//! Literal port of `src/alias.c` / `src/alias.h`.
//! Rules: `docs/spec/port/src/alias.md`.
//!
//! `atab` is a `BTreeMap` keyed by alias name, not the C's 39 chained hash
//! buckets, so `alias` with no operands prints in name order. `alias.c`
//! carries a standing one-line request for sorted output above `aliascmd`;
//! this answers it in the container rather than at print time. Registered
//! in `docs/divergences.md`.

use bstr::BString;
use libc::{c_char, c_int, c_void, size_t};
use std::collections::BTreeMap;
use core::ptr::{addr_of, addr_of_mut, null_mut};

use crate::error::{INTOFF, INTON};
use crate::memalloc::{ckfree, ckmalloc, savestr};
use crate::options::{argptr, nextopt};
use crate::output::VaArg;
use crate::shell::cstr;
use crate::var::varname;

pub const ALIASINUSE: c_int = 1;
pub const ALIASDEAD: c_int = 2;

// [spec:dash:def:alias.alias]
/// The C's `struct alias *next` is gone with the hash chain; `atab` orders
/// the entries itself. The struct stays separately allocated and never
/// moves, because `input.rs` holds its address in a `parsefile` for as long
/// as the alias is being read from.
#[repr(C)]
pub struct alias {
    pub name: *mut c_char,
    pub val: *mut c_char,
    pub flag: c_int,
}

/// Every alias, by name. Keyed the same way variables are — see
/// `var::varname`, which is dash's own choice: `alias.c` reaches into
/// `var.h` for `hashval` and `varequal`.
static mut atab: BTreeMap<BString, *mut alias> = BTreeMap::new();

#[inline]
unsafe fn atab_mut() -> &'static mut BTreeMap<BString, *mut alias> {
    &mut *addr_of_mut!(atab)
}

// [spec:dash:def:alias.setalias-fn]
// [spec:dash:sem:alias.setalias-fn]
unsafe fn setalias(name: *const c_char, val: *const c_char) {
    let mut ap: *mut alias;
    let mut p: *const c_char = name;
    let namelen: size_t;

    loop {
        if crate::syntax::BASESYNTAX(*p as i8 as c_int) != crate::syntax::CWORD {
            crate::error::sh_error(cstr(b"Invalid alias name: %s\0"), &[VaArg::Str(name)]);
        }
        p = p.add(1);
        if *p == b'=' as c_char {
            break;
        }
    }

    ap = __lookupalias(name);
    INTOFF();
    if !ap.is_null() {
        /* The C skips this free while the alias is being expanded, because
         * `input.c` is then reading out of this very buffer and its
         * `strpush` has taken over the freeing (`sp->string != sp->ap->name`
         * in `popstring`). `input.rs` reads a copy, so nobody else holds
         * the buffer and the guard would only leak it. */
        ckfree((*ap).name as *mut c_void);
        (*ap).flag &= !ALIASDEAD;
    } else {
        /* not found */
        ap = ckmalloc(core::mem::size_of::<alias>() as size_t) as *mut alias;
        (*ap).flag = 0;
        atab_mut().insert(varname(name).to_owned(), ap);
    }
    namelen = (val as usize - name as usize) as size_t;
    (*ap).name = savestr(name);
    (*ap).val = (*ap).name.add(namelen as usize);
    INTON();
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
    let (key, ap) = atab_mut().remove_entry(varname(name)).unwrap();
    if freealias(ap) {
        atab_mut().insert(key, ap);
    }
    INTON();

    0
}

// [spec:dash:def:alias.rmaliases-fn]
// [spec:dash:sem:alias.rmaliases-fn]
pub unsafe fn rmaliases() {
    INTOFF();
    atab_mut().retain(|_, &mut ap| freealias(ap));
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

// [spec:dash:def:alias.aliascmd-fn]
// [spec:dash:sem:alias.aliascmd-fn]
pub unsafe fn aliascmd(argc: c_int, mut argv: *mut *mut c_char) -> c_int {
    let mut n: *mut c_char;
    let mut ret: c_int = 0;
    let mut ap: *mut alias;

    if argc == 1 {
        for &ap in atab_mut().values() {
            printalias(ap);
        }
        return 0;
    }
    loop {
        argv = argv.add(1);
        n = *argv;
        if n.is_null() {
            break;
        }
        /* n + 1: funny ksh stuff (from 44lite) */
        let vv = if *n == 0 {
            null_mut()
        } else {
            libc::strchr(n.add(1), b'=' as c_int)
        };
        if *n == 0 || vv.is_null() {
            ap = __lookupalias(n);
            if ap.is_null() {
                crate::output::outfmt(
                    crate::output::out2,
                    cstr(b"%s: %s not found\n\0"),
                    &[VaArg::Str(cstr(b"alias\0")), VaArg::Str(n)],
                );
                ret = 1;
            } else {
                printalias(ap);
            }
        } else {
            setalias(n, vv.add(1));
        }
    }

    ret
}

// [spec:dash:def:alias.unaliascmd-fn]
// [spec:dash:sem:alias.unaliascmd-fn]
pub unsafe fn unaliascmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut i: c_int;

    loop {
        i = nextopt(b"a\0".as_ptr() as *const c_char);
        if i == b'\0' as c_int {
            break;
        }
        if i == b'a' as c_int {
            rmaliases();
            return 0;
        }
    }
    i = 0;
    while !(*argptr).is_null() {
        if unalias(*argptr) != 0 {
            crate::output::outfmt(
                crate::output::out2,
                cstr(b"%s: %s not found\n\0"),
                &[VaArg::Str(cstr(b"unalias\0")), VaArg::Str(*argptr)],
            );
            i = 1;
        }
        argptr = argptr.add(1);
    }

    i
}

// [spec:dash:def:alias.freealias-fn]
// [spec:dash:sem:alias.freealias-fn]
/// Free `ap`, unless it is being read from — then only mark it dead, so the
/// reader can finish and `input.rs` can unalias it afterwards.
///
/// The C returns the link that replaces `ap` in its chain: `ap` itself when
/// it survives, `ap->next` when it does not. With no chain that is one bit
/// of information, so this returns "the entry stays in the table".
unsafe fn freealias(ap: *mut alias) -> bool {
    if ((*ap).flag & ALIASINUSE) != 0 {
        (*ap).flag |= ALIASDEAD;
        return true;
    }

    ckfree((*ap).name as *mut c_void);
    ckfree(ap as *mut c_void);
    false
}

// [spec:dash:def:alias.printalias-fn]
// [spec:dash:sem:alias.printalias-fn]
pub unsafe fn printalias(ap: *const alias) {
    crate::output::out1fmt(
        addr_of!(crate::mystring::snlfmt) as *const c_char,
        &[VaArg::Str(crate::mystring::single_quote((*ap).name))],
    );
}

// [spec:dash:def:alias.lookupalias-fn]
// [spec:dash:sem:alias.lookupalias-fn]
/// The C returns the address of the link holding the entry, never NULL, so
/// callers test `*result`; a map removes by key, so this returns the entry
/// itself and NULL when there is none.
unsafe fn __lookupalias(name: *const c_char) -> *mut alias {
    match atab_mut().get(varname(name)) {
        Some(&ap) => ap,
        None => null_mut(),
    }
}
