//! `set`.
//!
//! Port of `setcmd` from `src/options.c`. The option scan it runs is
//! `crate::options::options`, shared with the shell's own command line so
//! that `set -x` and `sh -x` cannot drift apart; with operands left over
//! it replaces the positional parameters.
//!
//! With no arguments at all it prints the variables instead, which is the
//! one thing about `set` that has nothing to do with options.

use crate::error::Error;
use bstr::BStr;
use core::ptr::addr_of;
use libc::{c_char, c_int};

use crate::error::{INTOFF, INTON};
use crate::mystring::nullstr;
use crate::options::{options, optschanged, setparam};
use crate::var::{VUNSET, showvars};

// [spec:dash:def:options.setcmd-fn]
// [spec:dash:sem:options.setcmd-fn]
pub unsafe fn setcmd(args: &[&BStr]) -> Result<c_int, Error> {
    if args.len() == 1 {
        return Ok(showvars(addr_of!(nullstr) as *const c_char, 0, VUNSET));
    }
    INTOFF();
    let scan = options(args, 1, false);
    optschanged();
    if scan.next < args.len() {
        setparam(&args[scan.next..]);
    }
    INTON();
    Ok(0)
}
