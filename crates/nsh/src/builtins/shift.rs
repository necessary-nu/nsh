//! `shift`.
//!
//! Port of `shiftcmd` from `src/options.c`. The positional parameters are
//! `crate::options`'s `shellparam`; this drops the first `n` of them.
//!
//! When the shell does not own the words -- inside a function, where they
//! are the caller's argument array -- the C shifts that array down in
//! place rather than the list, and so does this.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ptr::addr_of_mut;
use libc::c_int;

use crate::error::{INTOFF, INTON};
use crate::eval::Flow;
use crate::options::shellparam;

// [spec:dash:def:options.shiftcmd-fn]
// [spec:dash:sem:options.shiftcmd-fn]
pub unsafe fn shiftcmd(_sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let n: c_int;

    n = match args.get(1) {
        Some(count) => {
            let count = crate::shell::cstring(count);
            crate::mystring::number(count.as_ptr())?
        }
        None => 1,
    };
    if n > shellparam.nparam {
        return Err(crate::error::sh_error_value(b"can't shift that many"));
    }
    INTOFF();
    (*addr_of_mut!(shellparam)).drop_first(n);
    INTON();
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::options::setparam;
    use crate::testutil::lock;

    unsafe fn params(words: &[&str]) {
        let words: Vec<&BStr> = words.iter().map(|w| BStr::new(*w)).collect();
        setparam(&words);
    }

    unsafe fn remaining() -> Vec<Vec<u8>> {
        let mut out = Vec::new();
        let mut p = crate::options::shellparam_p();
        while !(*p).is_null() {
            out.push(std::ffi::CStr::from_ptr(*p).to_bytes().to_vec());
            p = p.add(1);
        }
        out
    }

    #[test]
    fn one_by_default() {
        let _g = lock();
        unsafe {
            params(&["a", "b", "c"]);
            assert_eq!(shiftcmd(&mut Shell::new(), &[BStr::new("shift")]).unwrap(), Flow::Done(0));
            assert_eq!((*addr_of_mut!(shellparam)).nparam, 2);
            assert_eq!(remaining(), vec![b"b".to_vec(), b"c".to_vec()]);
        }
    }

    #[test]
    fn a_count_drops_that_many() {
        let _g = lock();
        unsafe {
            params(&["a", "b", "c"]);
            let sh = &mut Shell::new();
            assert_eq!(
                shiftcmd(sh, &[BStr::new("shift"), BStr::new("2")]).unwrap(),
                Flow::Done(0)
            );
            assert_eq!((*addr_of_mut!(shellparam)).nparam, 1);
            assert_eq!(remaining(), vec![b"c".to_vec()]);
        }
    }

    /// Shifting exactly all of them is allowed; one more is not.
    #[test]
    fn shifting_past_the_end_raises() {
        let _g = lock();
        unsafe {
            params(&["a", "b"]);
            let sh = &mut Shell::new();
            assert_eq!(
                shiftcmd(sh, &[BStr::new("shift"), BStr::new("2")]).unwrap(),
                Flow::Done(0)
            );
            assert_eq!((*addr_of_mut!(shellparam)).nparam, 0);
        }
        /* The diagnostic comes back as a value now rather than as an
         * unwind, so the assertion is on the error rather than on the
         * jump. The bytes are unchanged and still go to stderr. */
        unsafe {
            params(&["a"]);
            let e = shiftcmd(&mut Shell::new(), &[BStr::new("shift"), BStr::new("2")])
                .expect_err("shifting past the end fails");
            assert_eq!(e.message().to_vec(), b"can't shift that many".to_vec());
            assert_eq!(e.status(), 2);
        }
    }
}
