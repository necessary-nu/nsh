//! `break` and `continue`.
//!
//! Port of `breakcmd` from `src/eval.c`. One function under two names: it
//! reads the word it was called as to tell them apart, which is why the
//! table can point both rows here.
//!
//! Breaking a loop is a flag the evaluation routines check rather than a
//! control transfer -- `evalskip` and `skipcount` live in `crate::eval`
//! because that is what reads them.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use libc::c_int;

use crate::eval::{Flow, SKIPBREAK, SKIPCONT};

// [spec:dash:def:eval.breakcmd-fn]
// [spec:dash:sem:eval.breakcmd-fn]
pub unsafe fn breakcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut n: c_int = 1;

    if let Some(count) = args.get(1) {
        let count = crate::shell::cstring(count);
        n = crate::mystring::number(count.as_ptr())?;
        if n <= 0 {
            return Err(crate::mystring::badnum(count.as_ptr()));
        }
    }
    if n > sh.eval.loopnest {
        n = sh.eval.loopnest;
    }
    if n > 0 {
        sh.eval.evalskip = if args[0].first() == Some(&b'c') {
            SKIPCONT
        } else {
            SKIPBREAK
        };
        sh.eval.skipcount = n;
    }
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The two names are told apart by the word the builtin was called
    /// as, so the flag each sets is the thing to check.
    fn run(name: &[u8], count: Option<&[u8]>, nest: c_int) -> (c_int, c_int) {
        let _guard = crate::testutil::lock();
        let mut args = vec![BStr::new(name)];
        if let Some(count) = count {
            args.push(BStr::new(count));
        }
        unsafe {
            /* One shell: the state the case arranges and the state the
             * builtin writes are the same shell's, which is the whole
             * point of the field. */
            let mut owned = Shell::new();
            let sh = &mut owned;
            sh.eval.loopnest = nest;
            assert_eq!(breakcmd(sh, &args).unwrap(), Flow::Done(0));
            (sh.eval.evalskip, sh.eval.skipcount)
        }
    }

    #[test]
    fn break_and_continue_differ() {
        assert_eq!(run(b"break", None, 1), (SKIPBREAK, 1));
        assert_eq!(run(b"continue", None, 1), (SKIPCONT, 1));
    }

    /// "It should probably be an error to break out of more loops than
    /// exist, but it isn't in the standard shell so we don't make it one
    /// here" -- the count is clamped instead.
    #[test]
    fn the_count_clamps_to_the_nesting() {
        assert_eq!(run(b"break", Some(b"5"), 2), (SKIPBREAK, 2));
    }

    /// Outside any loop there is nothing to skip, so no flag is set.
    #[test]
    fn outside_a_loop_nothing_is_set() {
        assert_eq!(run(b"break", None, 0), (0, 0));
    }
}
