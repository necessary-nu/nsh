//! `return`.
//!
//! Port of `returncmd` from `src/eval.c`. Like `break` it sets a skip
//! flag; called outside a function it does what ksh does and skips the
//! rest of the file.

use crate::error::Error;
use bstr::BStr;
use libc::c_int;

use crate::eval::{SKIPFUNC, SKIPFUNCDEF, evalskip, exitstatus};

// [spec:dash:def:eval.returncmd-fn]
// [spec:dash:sem:eval.returncmd-fn]
pub unsafe fn returncmd(args: &[&BStr]) -> Result<c_int, Error> {
    let skip: c_int;
    let status: c_int;

    /*
     * If called outside a function, do what ksh does;
     * skip the rest of the file.
     */
    if let Some(want) = args.get(1) {
        let want = crate::shell::cstring(want);
        skip = SKIPFUNC;
        status = crate::mystring::number(want.as_ptr());
    } else {
        skip = SKIPFUNCDEF;
        status = exitstatus;
    }
    evalskip = skip;

    Ok(status)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With an operand the status is the operand and the skip is
    /// `SKIPFUNC`; without one the status is whatever `$?` already was,
    /// and the skip is the "not in a function" form that abandons the
    /// file instead.
    fn run(status: Option<&[u8]>, last: c_int) -> (c_int, c_int) {
        let _guard = crate::testutil::lock();
        let mut args = vec![BStr::new("return")];
        if let Some(status) = status {
            args.push(BStr::new(status));
        }
        unsafe {
            exitstatus = last;
            evalskip = 0;
            let returned = returncmd(&args).unwrap();
            (evalskip, returned)
        }
    }

    #[test]
    fn an_operand_is_the_status() {
        assert_eq!(run(Some(b"7"), 3), (SKIPFUNC, 7));
    }

    #[test]
    fn without_one_the_last_status_stands() {
        assert_eq!(run(None, 3), (SKIPFUNCDEF, 3));
    }
}
