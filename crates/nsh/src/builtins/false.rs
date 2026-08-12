//! `false`, the builtin that fails.
//!
//! Port of `falsecmd` from `src/eval.c`.

use crate::error::Error;
use bstr::BStr;
use libc::c_int;

// [spec:dash:def:eval.falsecmd-fn]
// [spec:dash:sem:eval.falsecmd-fn]
pub unsafe fn falsecmd(_args: &[&BStr]) -> Result<c_int, Error> {
    Ok(1)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_fails() {
        unsafe {
            assert_eq!(falsecmd(&[BStr::new("false")]).unwrap(), 1);
            assert_eq!(falsecmd(&[BStr::new("false"), BStr::new("ignored")]).unwrap(), 1);
        }
    }
}
