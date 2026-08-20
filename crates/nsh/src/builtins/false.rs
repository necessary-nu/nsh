//! `false`, the builtin that fails.
//!
//! Port of `falsecmd` from `src/eval.c`.

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use bstr::BStr;

// [spec:dash:sem:eval.falsecmd-fn]
pub fn run(_shell: &mut Shell, _args: &[&BStr]) -> Result<Flow, Error> {
    Ok(Flow::Done((1).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_fails() {
        let shell = &mut Shell::new(crate::streams::Streams::INHERIT);
        assert_eq!(
            run(shell, &[BStr::new("false")]).unwrap(),
            Flow::Done((1).into())
        );
        assert_eq!(
            run(shell, &[BStr::new("false"), BStr::new("ignored")]).unwrap(),
            Flow::Done((1).into())
        );
    }
}
