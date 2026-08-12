//! Literal port of `src/alias.c` / `src/alias.h`.
//! Rules: `docs/spec/port/src/alias.md`.
//!
//! `atab` is a `BTreeMap` keyed by alias name, not the C's 39 chained hash
//! buckets, so `alias` with no operands prints in name order. `alias.c`
//! carries a standing one-line request for sorted output above `aliascmd`;
//! this answers it in the container rather than at print time. Registered
//! in `docs/divergences.md`.

use crate::error::Error;
use bstr::{BStr, BString};
use core::ptr::{addr_of_mut, null_mut};
use libc::{c_char, c_int};
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::io::Write;

use crate::error::{INTOFF, INTON};
use crate::options::Options;
use crate::var::varname;

pub const ALIASINUSE: c_int = 1;
pub const ALIASDEAD: c_int = 2;

// [spec:dash:def:alias.alias]
/// The C's `struct alias *next` is gone with the hash chain; `atab` orders
/// the entries itself.
///
/// `text` owns the complete NUL-terminated `name=value` byte string. `name`
/// and `val` are cached views into it for the parser/input interface that still
/// speaks C pointers; neither pointer owns or frees anything. `input.rs`
/// copies `val` into its `StrPush`, so replacing `text` cannot invalidate an
/// in-flight alias expansion.
pub struct alias {
    pub name: *mut c_char,
    pub val: *mut c_char,
    pub flag: c_int,
    text: BString,
    value_offset: usize,
}

impl alias {
    fn new(text: BString, value_offset: usize) -> Self {
        let mut ap = alias {
            name: null_mut(),
            val: null_mut(),
            flag: 0,
            text,
            value_offset,
        };
        ap.refresh_views();
        ap
    }

    fn replace_text(&mut self, text: BString, value_offset: usize) {
        self.text = text;
        self.value_offset = value_offset;
        self.refresh_views();
    }

    fn refresh_views(&mut self) {
        self.name = self.text.as_mut_ptr() as *mut c_char;
        debug_assert!(self.value_offset < self.text.len());
        self.val = unsafe { self.name.add(self.value_offset) };
    }
}

/// Every alias, by name. Keyed the same way variables are — see
/// `var::varname`, which is dash's own choice: `alias.c` reaches into
/// `var.h` for `hashval` and `varequal`.
///
/// The `Box` is what `input.rs` needs: it holds an entry's address in a
/// `parsefile` for as long as the alias is being read from, and a map's
/// values move when the map rebalances.
static mut atab: BTreeMap<BString, Box<alias>> = BTreeMap::new();

#[inline]
pub(crate) unsafe fn atab_mut() -> &'static mut BTreeMap<BString, Box<alias>> {
    &mut *addr_of_mut!(atab)
}

// [spec:dash:def:alias.setalias-fn]
// [spec:dash:sem:alias.setalias-fn]
pub(crate) unsafe fn setalias(name: *const c_char, val: *const c_char) -> Result<(), Error> {
    let ap: *mut alias;
    let mut p: *const c_char = name;
    let value_offset: usize;

    loop {
        if crate::syntax::BASESYNTAX(*p as i8 as c_int) != crate::syntax::CWORD {
            let mut message = b"Invalid alias name: ".to_vec();
            message.extend_from_slice(CStr::from_ptr(name).to_bytes());
            return Err(crate::error::sh_error_value(&message));
        }
        p = p.add(1);
        if *p == b'=' as c_char {
            break;
        }
    }

    ap = __lookupalias(name);
    INTOFF();
    value_offset = val as usize - name as usize;
    let text = BString::from(core::ffi::CStr::from_ptr(name).to_bytes_with_nul());
    if !ap.is_null() {
        /* `input.rs` reads its own copy, so dropping the replaced `BString`
         * cannot pull bytes out from under an in-flight expansion. */
        (*ap).replace_text(text, value_offset);
        (*ap).flag &= !ALIASDEAD;
    } else {
        /* not found.  The address comes back out of the map rather than out
         * of the `Box` that went in, so nothing derived from it predates the
         * move. */
        atab_mut()
            .entry(varname(name).to_owned())
            .or_insert_with(|| Box::new(alias::new(text, value_offset)));
    }
    INTON();
    Ok(())
}

// [spec:dash:def:alias.unalias-fn]
// [spec:dash:sem:alias.unalias-fn]
pub unsafe fn unalias(name: *const c_char) -> c_int {
    if __lookupalias(name).is_null() {
        return 1;
    }

    INTOFF();
    /* Take the entry out before freeing anything and put it back if it
     * survives. `name` is often `(*ap).name` itself -- `input.rs` unaliases
     * a dead alias by its own text -- so the key has to be owned before
     * `freealias` can free the buffer it points into, and `remove_entry`
     * hands it over without a copy. */
    let (key, mut ap) = atab_mut().remove_entry(varname(name)).unwrap();
    if freealias(&mut ap) {
        atab_mut().insert(key, ap);
    }
    INTON();

    0
}

// [spec:dash:def:alias.rmaliases-fn]
// [spec:dash:sem:alias.rmaliases-fn]
pub unsafe fn rmaliases() {
    INTOFF();
    atab_mut().retain(|_, ap| freealias(ap));
    INTON();
}

// [spec:dash:def:alias.lookupalias-pub-fn]
// [spec:dash:sem:alias.lookupalias-pub-fn]
/// Public lookup.  Absent from `plan/.port-manifest.styx` — the extractor
/// folded it into `alias.lookupalias-fn` after stripping the leading
/// underscores from the distinct static `__lookupalias`.
pub unsafe fn lookupalias(name: *const c_char, check: c_int) -> *mut alias {
    let ap: *mut alias = __lookupalias(name);

    if check != 0 && !ap.is_null() && ((*ap).flag & ALIASINUSE) != 0 {
        return null_mut();
    }
    ap
}

/*
 * The C's standing request for sorted output sat here.  `atab` is ordered,
 * so no-operand `alias` prints sorted without a sort.
 */

// [spec:dash:def:alias.freealias-fn]
// [spec:dash:sem:alias.freealias-fn]
/// Free `ap`, unless it is being read from — then only mark it dead, so the
/// reader can finish and `input.rs` can unalias it afterwards.
///
/// The C returns the link that replaces `ap` in its chain: `ap` itself when
/// it survives, `ap->next` when it does not. With no chain that is one bit
/// of information, so this returns "the entry stays in the table" — and the
/// caller drops the `Box` when it does not, releasing both the node and its
/// owned bytes.
unsafe fn freealias(ap: &mut alias) -> bool {
    if (ap.flag & ALIASINUSE) != 0 {
        ap.flag |= ALIASDEAD;
        return true;
    }

    false
}

// [spec:dash:def:alias.printalias-fn]
// [spec:dash:sem:alias.printalias-fn]
pub unsafe fn printalias(ap: *const alias) {
    let quoted = crate::mystring::single_quote((*ap).name);
    let mut line = CStr::from_ptr(quoted).to_bytes().to_vec();
    line.push(b'\n');
    let _ = (*crate::output::stdout()).write_all(&line);
}

// [spec:dash:def:alias.lookupalias-fn]
// [spec:dash:sem:alias.lookupalias-fn]
/// The C returns the address of the link holding the entry, never NULL, so
/// callers test `*result`; a map removes by key, so this returns the entry
/// itself and NULL when there is none.
pub(crate) unsafe fn __lookupalias(name: *const c_char) -> *mut alias {
    match atab_mut().get_mut(varname(name)) {
        Some(ap) => &mut **ap as *mut alias,
        None => null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::CStr0;

    /// A name the alias table will not take comes back as a value, and the
    /// table is left as it was.
    #[test]
    fn an_invalid_name_returns_its_complaint() {
        let _g = crate::testutil::lock();
        unsafe {
            atab_mut().clear();
            let definition = CStr0::new("a b=value");

            let e = setalias(definition.p(), definition.p().add(6))
                .expect_err("a space is not a word character");

            assert_eq!(e.message().to_vec(), b"Invalid alias name: a b=value".to_vec());
            assert!(atab_mut().is_empty());
        }
    }

    // [spec:dash:sem:alias.setalias-fn/test]
    // [spec:dash:sem:alias.lookupalias-pub-fn/test]
    // [spec:dash:sem:alias.unalias-fn/test]
    // [spec:dash:sem:alias.rmaliases-fn/test]
    // [spec:dash:sem:alias.freealias-fn/test]
    #[test]
    fn owned_alias_views_remain_stable() {
        let _g = crate::testutil::lock();
        unsafe {
            atab_mut().clear();

            let initial = CStr0::new("a=old");
            setalias(initial.p(), initial.p().add(2)).unwrap();
            let ap = lookupalias(CStr0::new("a").p(), 0);
            assert!(!ap.is_null());
            let address = ap as usize;

            for i in 0..64 {
                let definition = CStr0::new(&format!("name{i}=value{i}"));
                let offset = format!("name{i}=").len();
                setalias(definition.p(), definition.p().add(offset)).unwrap();
            }
            assert_eq!(lookupalias(CStr0::new("a").p(), 0) as usize, address);

            let replacement = [b'a' as c_char, b'=' as c_char, -1, 0];
            setalias(replacement.as_ptr(), replacement.as_ptr().add(2)).unwrap();
            let ap = lookupalias(CStr0::new("a").p(), 0);
            assert_eq!(ap as usize, address);
            assert_eq!(core::ffi::CStr::from_ptr((*ap).name).to_bytes(), b"a=\xff");
            assert_eq!(core::ffi::CStr::from_ptr((*ap).val).to_bytes(), b"\xff");
            assert_eq!((*ap).name as *const u8, (*ap).text.as_ptr());

            assert_eq!(unalias(CStr0::new("a").p()), 0);

            let held_definition = CStr0::new("held=old");
            let held_name = CStr0::new("held");
            setalias(held_definition.p(), held_definition.p().add(5)).unwrap();
            let held = lookupalias(held_name.p(), 0);
            (*held).flag |= ALIASINUSE;
            assert!(lookupalias(held_name.p(), 1).is_null());

            assert_eq!(unalias(held_name.p()), 0);
            let deferred = lookupalias(held_name.p(), 0);
            assert_eq!(deferred, held);
            assert_ne!((*deferred).flag & ALIASINUSE, 0);
            assert_ne!((*deferred).flag & ALIASDEAD, 0);

            let held_replacement = CStr0::new("held=new");
            setalias(held_replacement.p(), held_replacement.p().add(5)).unwrap();
            let revived = lookupalias(held_name.p(), 0);
            assert_eq!(revived, held);
            assert_ne!((*revived).flag & ALIASINUSE, 0);
            assert_eq!((*revived).flag & ALIASDEAD, 0);
            assert_eq!(core::ffi::CStr::from_ptr((*revived).val).to_bytes(), b"new");

            (*revived).flag &= !ALIASINUSE;
            assert_eq!(unalias(held_name.p()), 0);
            assert!(lookupalias(held_name.p(), 0).is_null());

            let kept_definition = CStr0::new("kept=value");
            let kept_name = CStr0::new("kept");
            let dropped_definition = CStr0::new("dropped=value");
            let dropped_name = CStr0::new("dropped");
            setalias(kept_definition.p(), kept_definition.p().add(5)).unwrap();
            setalias(dropped_definition.p(), dropped_definition.p().add(8)).unwrap();
            let kept = lookupalias(kept_name.p(), 0);
            (*kept).flag |= ALIASINUSE;

            rmaliases();
            assert!(lookupalias(dropped_name.p(), 0).is_null());
            assert_eq!(lookupalias(kept_name.p(), 0), kept);
            assert_ne!((*kept).flag & ALIASDEAD, 0);

            (*kept).flag &= !ALIASINUSE;
            rmaliases();
            assert!(lookupalias(kept_name.p(), 0).is_null());
            atab_mut().clear();
        }
    }
}
