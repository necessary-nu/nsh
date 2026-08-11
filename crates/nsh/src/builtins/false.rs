//! `false`, the builtin that fails.
//!
//! Port of `falsecmd` from `src/eval.c`.

use bstr::BStr;
use libc::c_int;

// [spec:dash:def:eval.falsecmd-fn]
// [spec:dash:sem:eval.falsecmd-fn]
pub unsafe fn falsecmd(_args: &[&BStr]) -> c_int {
    1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn always_fails() {
        unsafe {
            assert_eq!(falsecmd(&[BStr::new("false")]), 1);
            assert_eq!(falsecmd(&[BStr::new("false"), BStr::new("ignored")]), 1);
        }
    }
}
