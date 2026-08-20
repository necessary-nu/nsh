//! `return`.
//!
//! Port of `returncmd` from `src/eval.c`. Called outside a function it does
//! what ksh does and leaves the current command file.

// [spec:nsh:req:idiom.evaluator-control-flow]

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use bstr::BStr;

// [spec:dash:def:eval.returncmd-fn]
// [spec:dash:sem:eval.returncmd-fn]
// [spec:posix:syn:builtin.return.synopsis]
// [spec:posix:req:builtin.return.stop-function-or-dot-script]
// [spec:posix:req:builtin.return.stderr]
// [spec:posix:req:builtin.return.exit-status]
// [spec:posix:sem:builtin.return.utility-defaults]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let status: crate::status::ExitStatus;
    let explicit = args.get(1).is_some();

    /*
     * If called outside a function, do what ksh does;
     * skip the rest of the file.
     */
    if let Some(want) = args.get(1) {
        status = crate::status::ExitStatus::from_code(crate::number::parse_nonnegative(
            &mut shell.diagnostics(),
            want,
        )?);
    } else {
        status = shell.status;
    }
    Ok(Flow::Return { status, explicit })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// With an operand the status is the operand; without one the status is
    /// whatever `$?` already was.
    fn invoke(status: Option<&[u8]>, last: i32) -> Flow {
        let _guard = crate::test_support::lock();
        let mut args = vec![BStr::new("return")];
        if let Some(status) = status {
            args.push(BStr::new(status));
        }
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        shell.status = crate::status::ExitStatus::from_code(last);
        let returned = super::run(shell, &args).unwrap();
        returned
    }

    #[test]
    fn an_operand_is_the_status() {
        assert_eq!(
            invoke(Some(b"7"), 3),
            Flow::Return {
                status: 7.into(),
                explicit: true
            }
        );
    }

    #[test]
    fn without_one_the_last_status_stands() {
        assert_eq!(
            invoke(None, 3),
            Flow::Return {
                status: 3.into(),
                explicit: false
            }
        );
    }
}
