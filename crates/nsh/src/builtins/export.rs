//! `export` and `readonly`.
//!
//! Port of `exportcmd` from `src/var.c`. One function under two names,
//! telling them apart by the word it was called as -- the two differ only
//! in which flag they set on the variable.
//!
//! The variable table stays in `crate::var`. What is here is the argument
//! handling: with no operands it prints the set, and with them it sets a
//! flag on names that exist and creates the ones that do not.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, ByteSlice};
use core::ffi::CStr;
use core::ptr;
use libc::{c_char, c_int};

use crate::eval::Flow;
use crate::options::Options;
use crate::var::{VEXPORT, VREADONLY, findvar, setvar, showvars, var};

// [spec:dash:def:var.exportcmd-fn]
// [spec:dash:sem:var.exportcmd-fn]
pub unsafe fn exportcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
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
    let notp = opts.next(sh, b"p")?.is_none();
    let operands = opts.operands();
    if notp && !operands.is_empty() {
        for word in operands {
            let word = crate::shell::cstring(word);
            let name = word.as_ptr() as *mut c_char;

            match CStr::from_ptr(name).to_bytes().find_byte(b'=') {
                /* `setvar` wants the value, which is the byte after the
                 * `=` in the same buffer -- the C keeps `strchr`'s
                 * pointer and steps it once. */
                Some(at) => p = name.add(at + 1),
                None => {
                    p = ptr::null();
                    vp = findvar(sh, name);
                    if !vp.is_null() {
                        (*vp).flags |= flag;
                        continue;
                    }
                }
            }
            setvar(sh, name, p, flag)?;
        }
    } else {
        let called = crate::shell::cstring(args[0]);
        showvars(sh, called.as_ptr(), flag, 0);
    }
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::ffi::CStr;

    use crate::testutil::{CStr0, lock};
    use crate::var::{VSTRFIXED, lookupvar, setvar};

    /// The shell is the caller's: `export` reads and writes the variable
    /// table, which belongs to an instance, so a `Shell` made in here
    /// would be a different set of variables from the one the test set up.
    fn run(sh: &mut Shell, name: &[u8], words: &[&[u8]]) -> Flow {
        let mut args = vec![BStr::new(name)];
        args.extend(words.iter().map(|w| BStr::new(*w)));
        unsafe { exportcmd(sh, &args).unwrap() }
    }

    /// The word the builtin was called as picks the flag, which is the
    /// whole of the difference between the two commands.
    #[test]
    fn the_calling_name_picks_the_flag() {
        let _g = lock();
        unsafe {
            let mut owned = Shell::new(crate::streams::Streams::INHERIT);
            let sh = &mut owned;
            let name = CStr0::new("Texport");
            setvar(sh, name.p(), CStr0::new("v").p(), VSTRFIXED);

            assert_eq!(run(sh, b"export", &[b"Texport"]), Flow::Done(0));
            assert_ne!((*findvar(sh, name.p())).flags & VEXPORT, 0);
            assert_eq!((*findvar(sh, name.p())).flags & VREADONLY, 0);

            assert_eq!(run(sh, b"readonly", &[b"Texport"]), Flow::Done(0));
            assert_ne!((*findvar(sh, name.p())).flags & VREADONLY, 0);
        }
    }

    /// An operand carrying a value assigns as well as flags, which is
    /// what makes `export` one of the assignment builtins.
    #[test]
    fn an_operand_may_assign() {
        let _g = lock();
        unsafe {
            let mut owned = Shell::new(crate::streams::Streams::INHERIT);
            let sh = &mut owned;
            assert_eq!(run(sh, b"export", &[b"Texport2=set"]), Flow::Done(0));
            let name = CStr0::new("Texport2");
            assert_eq!(CStr::from_ptr(lookupvar(sh, name.p())).to_bytes(), b"set");
            assert_ne!((*findvar(sh, name.p())).flags & VEXPORT, 0);
        }
    }
}
