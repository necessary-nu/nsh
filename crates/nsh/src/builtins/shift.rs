//! `shift`.
//!
//! Port of `shiftcmd` from `src/options.c`. The positional parameters are
//! `crate::options`'s `shellparam`; this drops the first `n` of them.
//!
//! When the shell does not own the words -- inside a function, where they
//! are the caller's argument array -- the C shifts that array down in
//! place rather than the list, and so does this.

use bstr::BStr;
use core::ptr::addr_of_mut;
use libc::c_int;

use crate::error::{INTOFF, INTON};
use crate::options::shellparam;

// [spec:dash:def:options.shiftcmd-fn]
// [spec:dash:sem:options.shiftcmd-fn]
pub unsafe fn shiftcmd(args: &[&BStr]) -> c_int {
    let n: c_int;

    n = match args.get(1) {
        Some(count) => {
            let count = crate::shell::cstring(count);
            crate::mystring::number(count.as_ptr())
        }
        None => 1,
    };
    if n > shellparam.nparam {
        crate::error::sh_error(b"can't shift that many");
    }
    INTOFF();
    (*addr_of_mut!(shellparam)).drop_first(n);
    INTON();
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::options::setparam;
    use crate::testutil::{lock, raises};

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
            assert_eq!(shiftcmd(&[BStr::new("shift")]), 0);
            assert_eq!((*addr_of_mut!(shellparam)).nparam, 2);
            assert_eq!(remaining(), vec![b"b".to_vec(), b"c".to_vec()]);
        }
    }

    #[test]
    fn a_count_drops_that_many() {
        let _g = lock();
        unsafe {
            params(&["a", "b", "c"]);
            assert_eq!(shiftcmd(&[BStr::new("shift"), BStr::new("2")]), 0);
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
            assert_eq!(shiftcmd(&[BStr::new("shift"), BStr::new("2")]), 0);
            assert_eq!((*addr_of_mut!(shellparam)).nparam, 0);
        }
        assert!(raises(|| {
            unsafe {
                params(&["a"]);
                shiftcmd(&[BStr::new("shift"), BStr::new("2")]);
            }
        }));
    }
}
