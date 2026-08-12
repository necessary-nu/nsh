//! `true` and `:`, the builtins that succeed.
//!
//! Port of `truecmd` from `src/eval.c`. `:` is the same function under
//! the other name, which is what the C's two table rows say.

use crate::error::Error;
use bstr::BStr;
use libc::c_int;

// [spec:dash:def:eval.truecmd-fn]
// [spec:dash:sem:eval.truecmd-fn]
pub unsafe fn truecmd(_args: &[&BStr]) -> Result<c_int, Error> {
    Ok(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Both names, and any arguments, succeed. `:` taking arguments and
    /// ignoring them is what makes it the idiom it is.
    #[test]
    fn always_succeeds() {
        unsafe {
            assert_eq!(truecmd(&[BStr::new("true")]).unwrap(), 0);
            assert_eq!(truecmd(&[BStr::new(":"), BStr::new("ignored")]).unwrap(), 0);
        }
    }
}
