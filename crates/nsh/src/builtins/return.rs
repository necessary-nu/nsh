//! `return`.
//!
//! Port of `returncmd` from `src/eval.c`. Like `break` it sets a skip
//! flag; called outside a function it does what ksh does and skips the
//! rest of the file.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ffi::c_int;

use crate::eval::{Flow, SKIPFUNC, SKIPFUNCDEF};

// [spec:dash:def:eval.returncmd-fn]
// [spec:dash:sem:eval.returncmd-fn]
// [spec:posix:syn:builtin.return.synopsis]
// [spec:posix:req:builtin.return.stop-function-or-dot-script]
// [spec:posix:req:builtin.return.stderr]
// [spec:posix:req:builtin.return.exit-status]
// [spec:posix:sem:builtin.return.utility-defaults]
pub fn returncmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let skip: c_int;
    let status: crate::status::ExitStatus;

    /*
     * If called outside a function, do what ksh does;
     * skip the rest of the file.
     */
    if let Some(want) = args.get(1) {
        skip = SKIPFUNC;
        status = crate::status::ExitStatus::from_code(crate::mystring::number(sh, want)?);
    } else {
        skip = SKIPFUNCDEF;
        status = sh.status;
    }
    sh.eval.evalskip = skip;

    Ok(Flow::Done((status).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With an operand the status is the operand and the skip is
    /// `SKIPFUNC`; without one the status is whatever `$?` already was,
    /// and the skip is the "not in a function" form that abandons the
    /// file instead.
    fn run(status: Option<&[u8]>, last: c_int) -> (c_int, Flow) {
        let _guard = crate::testutil::lock();
        let mut args = vec![BStr::new("return")];
        if let Some(status) = status {
            args.push(BStr::new(status));
        }
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        sh.status = crate::status::ExitStatus::from_code(last);
        let returned = returncmd(sh, &args).unwrap();
        (sh.eval.evalskip, returned)
    }

    #[test]
    fn an_operand_is_the_status() {
        assert_eq!(run(Some(b"7"), 3), (SKIPFUNC, Flow::Done((7).into())));
    }

    #[test]
    fn without_one_the_last_status_stands() {
        assert_eq!(run(None, 3), (SKIPFUNCDEF, Flow::Done((3).into())));
    }
}
