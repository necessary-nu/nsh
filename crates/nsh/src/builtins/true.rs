//! `true` and `:`, the builtins that succeed.
//!
//! Port of `truecmd` from `src/eval.c`. `:` is the same function under
//! the other name, which is what the C's two table rows say.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use bstr::BStr;

// [spec:dash:def:eval.truecmd-fn]
// [spec:dash:sem:eval.truecmd-fn]
pub fn truecmd(_sh: &mut Shell, _args: &[&BStr]) -> Result<Flow, Error> {
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both names, and any arguments, succeed. `:` taking arguments and
    /// ignoring them is what makes it the idiom it is.
    #[test]
    fn always_succeeds() {
        let sh = &mut Shell::new(crate::streams::Streams::INHERIT);
        assert_eq!(truecmd(sh, &[BStr::new("true")]).unwrap(), Flow::Done(0));
        assert_eq!(
            truecmd(sh, &[BStr::new(":"), BStr::new("ignored")]).unwrap(),
            Flow::Done(0)
        );
    }
}
