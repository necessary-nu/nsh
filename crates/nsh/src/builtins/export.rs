//! `export` and `readonly`.
//!
//! Port of `exportcmd` from `src/var.c`. One function under two names,
//! telling them apart by the word it was called as -- the two differ only
//! in which flag they set on the variable.
//!
//! The variable table stays in `crate::var`. What is here is the argument
//! handling: with no operands it prints the set, and with them it sets a
//! flag on names that exist and creates the ones that do not.

use bstr::BStr;
use libc::{c_char, c_int};

use crate::options::Options;
use crate::var::{VEXPORT, VREADONLY, findvar, setvar, showvars, var};

// [spec:dash:def:var.exportcmd-fn]
// [spec:dash:sem:var.exportcmd-fn]
pub unsafe fn exportcmd(args: &[&BStr]) -> c_int {
    let mut vp: *mut var;
    let mut p: *const c_char;
    /* `export` and `readonly` are one builtin telling itself apart by the
     * word it was called as. */
    let flag: c_int = if args[0].first() == Some(&b'r') {
        VREADONLY
    } else {
        VEXPORT
    };

    let mut opts = Options::new(args);
    let notp = opts.next(b"p").is_none();
    let operands = opts.operands();
    if notp && !operands.is_empty() {
        for word in operands {
            let word = crate::shell::cstring(word);
            let name = word.as_ptr() as *mut c_char;

            p = libc::strchr(name, b'=' as c_int);
            if !p.is_null() {
                p = p.add(1);
            } else {
                vp = findvar(name);
                if !vp.is_null() {
                    (*vp).flags |= flag;
                    continue;
                }
            }
            setvar(name, p, flag);
        }
    } else {
        let called = crate::shell::cstring(args[0]);
        showvars(called.as_ptr(), flag, 0);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::CStr;

    use crate::testutil::{CStr0, lock};
    use crate::var::{VSTRFIXED, lookupvar, setvar};

    fn run(name: &[u8], words: &[&[u8]]) -> c_int {
        let mut args = vec![BStr::new(name)];
        args.extend(words.iter().map(|w| BStr::new(*w)));
        unsafe { exportcmd(&args) }
    }

    /// The word the builtin was called as picks the flag, which is the
    /// whole of the difference between the two commands.
    #[test]
    fn the_calling_name_picks_the_flag() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("Texport");
            setvar(name.p(), CStr0::new("v").p(), VSTRFIXED);

            assert_eq!(run(b"export", &[b"Texport"]), 0);
            assert_ne!((*findvar(name.p())).flags & VEXPORT, 0);
            assert_eq!((*findvar(name.p())).flags & VREADONLY, 0);

            assert_eq!(run(b"readonly", &[b"Texport"]), 0);
            assert_ne!((*findvar(name.p())).flags & VREADONLY, 0);
        }
    }

    /// An operand carrying a value assigns as well as flags, which is
    /// what makes `export` one of the assignment builtins.
    #[test]
    fn an_operand_may_assign() {
        let _g = lock();
        unsafe {
            assert_eq!(run(b"export", &[b"Texport2=set"]), 0);
            let name = CStr0::new("Texport2");
            assert_eq!(
                CStr::from_ptr(lookupvar(name.p())).to_bytes(),
                b"set"
            );
            assert_ne!((*findvar(name.p())).flags & VEXPORT, 0);
        }
    }
}
