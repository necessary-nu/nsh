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
use core::ffi::c_int;

use crate::error::{INTOFF, INTON};
use crate::eval::Flow;

// [spec:dash:def:options.shiftcmd-fn]
// [spec:dash:sem:options.shiftcmd-fn]
// [spec:posix:syn:builtin.shift.synopsis]
// [spec:posix:req:builtin.shift.positional-parameters]
// [spec:posix:req:builtin.shift.operand-value]
// [spec:posix:req:builtin.shift.stderr]
// [spec:posix:req:builtin.shift.exit-status]
// [spec:posix:sem:builtin.shift.utility-defaults]
pub fn shiftcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let n: c_int;

    n = match args.get(1) {
        Some(count) => crate::mystring::number(sh, count)?,
        None => 1,
    };
    if n > sh.options.shellparam.nparam {
        return Err(sh.sh_error_value(b"can't shift that many"));
    }
    INTOFF(sh);
    sh.options.shellparam.drop_first(n);
    INTON(sh);
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::options::setparam;
    use crate::testutil::lock;

    /// The parameters belong to the shell under test, not to the
    /// process: these take the receiver rather than reaching a global.
    fn params(sh: &mut Shell, words: &[&str]) {
        let words: Vec<&BStr> = words.iter().map(|w| BStr::new(*w)).collect();
        setparam(sh, &words);
    }

    fn remaining(sh: &mut Shell) -> Vec<Vec<u8>> {
        sh.options
            .shellparam
            .words()
            .into_iter()
            .map(Vec::from)
            .collect()
    }

    #[test]
    fn one_by_default() {
        let _g = lock();
        let sh = &mut Shell::new(crate::streams::Streams::INHERIT);
        params(sh, &["a", "b", "c"]);
        assert_eq!(shiftcmd(sh, &[BStr::new("shift")]).unwrap(), Flow::Done(0));
        assert_eq!(sh.options.shellparam.nparam, 2);
        assert_eq!(remaining(sh), vec![b"b".to_vec(), b"c".to_vec()]);
    }

    #[test]
    fn a_count_drops_that_many() {
        let _g = lock();
        let sh = &mut Shell::new(crate::streams::Streams::INHERIT);
        params(sh, &["a", "b", "c"]);
        assert_eq!(
            shiftcmd(sh, &[BStr::new("shift"), BStr::new("2")]).unwrap(),
            Flow::Done(0)
        );
        assert_eq!(sh.options.shellparam.nparam, 1);
        assert_eq!(remaining(sh), vec![b"c".to_vec()]);
    }

    /// Shifting exactly all of them is allowed; one more is not.
    #[test]
    fn shifting_past_the_end_raises() {
        let _g = lock();
        let sh = &mut Shell::new(crate::streams::Streams::INHERIT);
        params(sh, &["a", "b"]);
        assert_eq!(
            shiftcmd(sh, &[BStr::new("shift"), BStr::new("2")]).unwrap(),
            Flow::Done(0)
        );
        assert_eq!(sh.options.shellparam.nparam, 0);
        /* The diagnostic comes back as a value now rather than as an
         * unwind, so the assertion is on the error rather than on the
         * jump. The bytes are unchanged and still go to stderr. */
        let sh = &mut Shell::new(crate::streams::Streams::INHERIT);
        params(sh, &["a"]);
        let e = shiftcmd(sh, &[BStr::new("shift"), BStr::new("2")])
            .expect_err("shifting past the end fails");
        assert_eq!(e.message().to_vec(), b"can't shift that many".to_vec());
        assert_eq!(e.status(), 2);
    }
}
