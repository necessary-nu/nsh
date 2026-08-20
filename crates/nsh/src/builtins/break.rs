//! `break` and `continue`.
//!
//! Port of `breakcmd` from `src/eval.c`. One function under two names: it
//! reads the word it was called as to tell them apart, which is why the
//! table can point both rows here.
//!
//! Loop control is returned to the evaluator as a typed [`Flow`].

// [spec:nsh:req:idiom.evaluator-control-flow]

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;

use crate::evaluation::Flow;

// [spec:dash:def:eval.breakcmd-fn]
// [spec:dash:sem:eval.breakcmd-fn]
// [spec:posix:syn:builtin.break.syn]
// [spec:posix:req:builtin.break.exit-nth-loop]
// [spec:posix:def:builtin.break.lexically-enclosing]
// [spec:posix:sem:builtin.break.non-lexical-loop-unspecified]
// [spec:posix:req:builtin.break.stderr]
// [spec:posix:req:builtin.break.interfaces]
// [spec:posix:req:builtin.break.exit-status]
// [spec:posix:syn:builtin.continue.syn]
// [spec:posix:req:builtin.continue.return-to-top]
// [spec:posix:req:builtin.continue.n-operand]
// [spec:posix:req:builtin.continue.stderr]
// [spec:posix:req:builtin.continue.interfaces]
// [spec:posix:req:builtin.continue.exit-status]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut levels = 1usize;

    if let Some(count) = args.get(1) {
        let parsed = crate::number::parse_nonnegative(&mut shell.diagnostics(), count)?;
        if parsed <= 0 {
            return Err(crate::number::invalid_number(
                &mut shell.diagnostics(),
                count,
            ));
        }
        levels = parsed as usize;
    }
    levels = levels.min(shell.evaluation.loop_depth);
    if levels > 0 {
        Ok(if args[0].first() == Some(&b'c') {
            Flow::Continue {
                levels,
                status: crate::status::ExitStatus::SUCCESS,
            }
        } else {
            Flow::Break {
                levels,
                status: crate::status::ExitStatus::SUCCESS,
            }
        })
    } else {
        Ok(Flow::Done(crate::status::ExitStatus::SUCCESS))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two names are told apart by the word the builtin was called
    /// as, so the flag each sets is the thing to check.
    fn invoke(name: &[u8], count: Option<&[u8]>, nest: usize) -> Flow {
        let _guard = crate::test_support::lock();
        let mut args = vec![BStr::new(name)];
        if let Some(count) = count {
            args.push(BStr::new(count));
        }
        /* One shell: the state the case arranges and the state the
         * builtin writes are the same shell's, which is the whole
         * point of the field. */
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        shell.evaluation.loop_depth = nest;
        super::run(shell, &args).unwrap()
    }

    #[test]
    fn break_and_continue_differ() {
        assert_eq!(
            invoke(b"break", None, 1),
            Flow::Break {
                levels: 1,
                status: 0.into()
            }
        );
        assert_eq!(
            invoke(b"continue", None, 1),
            Flow::Continue {
                levels: 1,
                status: 0.into()
            }
        );
    }

    /// "It should probably be an error to break out of more loops than
    /// exist, but it isn't in the standard shell so we don't make it one
    /// here" -- the count is clamped instead.
    #[test]
    fn the_count_clamps_to_the_nesting() {
        assert_eq!(
            invoke(b"break", Some(b"5"), 2),
            Flow::Break {
                levels: 2,
                status: 0.into()
            }
        );
    }

    /// Outside any loop there is nothing to skip, so no flag is set.
    #[test]
    fn outside_a_loop_nothing_is_set() {
        assert_eq!(invoke(b"break", None, 0), Flow::Done((0).into()));
    }
}
