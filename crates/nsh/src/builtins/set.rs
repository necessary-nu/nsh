//! `set`.
//!
//! Port of `setcmd` from `src/options.c`. The option scan it runs is
//! `crate::options::options`, shared with the shell's own command line so
//! that `set -x` and `sh -x` cannot drift apart; with operands left over
//! it replaces the positional parameters.
//!
//! With no arguments at all it prints the variables instead, which is the
//! one thing about `set` that has nothing to do with options.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ptr::addr_of;
use libc::c_char;

use crate::error::{INTOFF, INTON};
use crate::eval::Flow;
use crate::mystring::nullstr;
use crate::options::{options, optschanged, setparam};
use crate::var::{VUNSET, showvars};

// [spec:dash:def:options.setcmd-fn]
// [spec:dash:sem:options.setcmd-fn]
pub unsafe fn setcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    if args.len() == 1 {
        return Ok(Flow::Done(showvars(addr_of!(nullstr) as *const c_char, 0, VUNSET)));
    }
    INTOFF();
    let scan = options(sh, args, 1, false)?;
    /* The fourth `?` to return between this frame's INTOFF and its INTON,
     * and left leaking with the other three: 2.4 is explicit that pairing
     * them would move the instruction a pending SIGINT is delivered at. */
    optschanged(sh)?;
    if scan.next < args.len() {
        setparam(&args[scan.next..]);
    }
    INTON();
    Ok(Flow::Done(0))
}
