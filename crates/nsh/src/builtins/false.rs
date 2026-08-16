//! `false`, the builtin that fails.
//!
//! Port of `falsecmd` from `src/eval.c`.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use bstr::BStr;

// [spec:dash:def:eval.falsecmd-fn]
// [spec:dash:sem:eval.falsecmd-fn]
pub unsafe fn falsecmd(_sh: &mut Shell, _args: &[&BStr]) -> Result<Flow, Error> {
    Ok(Flow::Done(1))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_fails() {
        unsafe {
            let sh = &mut Shell::new(crate::streams::Streams::INHERIT);
            assert_eq!(falsecmd(sh, &[BStr::new("false")]).unwrap(), Flow::Done(1));
            assert_eq!(
                falsecmd(sh, &[BStr::new("false"), BStr::new("ignored")]).unwrap(),
                Flow::Done(1)
            );
        }
    }
}
