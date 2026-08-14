//! `alias`.
//!
//! Port of `aliascmd` from `src/alias.c`. The alias table itself stays in
//! `crate::alias`, where the parser and the line editor read it; this is
//! the command that prints and defines entries in it.

use crate::error::Error;
use bstr::{BStr, ByteSlice};
use core::ffi::CStr;
use core::ptr::null_mut;
use libc::c_int;
use std::io::Write;

use crate::alias::{__lookupalias, alias, atab_mut, printalias, setalias};
use crate::eval::Flow;

// [spec:dash:def:alias.aliascmd-fn]
// [spec:dash:sem:alias.aliascmd-fn]
pub unsafe fn aliascmd(args: &[&BStr]) -> Result<Flow, Error> {
    let mut ret: c_int = 0;
    let mut ap: *mut alias;

    if args.len() == 1 {
        for ap in atab_mut().values() {
            printalias(&**ap as *const alias);
        }
        return Ok(Flow::Done(0));
    }
    for word in &args[1..] {
        /* `setalias` reads the value as an offset into the name, so the
         * two have to be one buffer, as they are in the word the shell
         * expanded. */
        let word = crate::shell::cstring(word);
        let n = word.as_ptr();
        /* n + 1: funny ksh stuff (from 44lite) */
        let vv = if *n == 0 {
            None
        } else {
            CStr::from_ptr(n.add(1))
                .to_bytes()
                .find_byte(b'=')
                .map(|at| n.add(1 + at))
        };
        if *n == 0 || vv.is_none() {
            ap = __lookupalias(n);
            if ap.is_null() {
                let mut message = b"alias: ".to_vec();
                message.extend_from_slice(word.as_bytes());
                message.extend_from_slice(b" not found\n");
                let _ = (*crate::output::stderr()).write_all(&message);
                ret = 1;
            } else {
                printalias(ap);
            }
        } else {
            setalias(n, vv.expect("the `=` branch").add(1))?;
        }
    }

    Ok(Flow::Done(ret))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::alias::lookupalias;

    /// Defining prints nothing and lands in the table the parser reads,
    /// which is the whole of what `alias name=value` is for.
    #[test]
    fn a_definition_reaches_the_table() {
        let _guard = crate::testutil::lock();
        unsafe {
            atab_mut().clear();
            assert_eq!(
                aliascmd(&[BStr::new("alias"), BStr::new("ll=ls -l")]).unwrap(),
                Flow::Done(0)
            );
            let found = lookupalias(c"ll".as_ptr(), 0);
            assert!(!found.is_null());
            atab_mut().clear();
        }
    }

    /// A name that is not defined is a diagnostic and a failing status,
    /// and it does not stop the words after it being defined.
    #[test]
    fn an_unknown_name_fails_without_stopping() {
        let _guard = crate::testutil::lock();
        unsafe {
            atab_mut().clear();
            assert_eq!(
                aliascmd(&[
                    BStr::new("alias"),
                    BStr::new("nosuchalias"),
                    BStr::new("after=1"),
                ])
                .unwrap(),
                Flow::Done(1)
            );
            assert!(!lookupalias(c"after".as_ptr(), 0).is_null());
            atab_mut().clear();
        }
    }
}
