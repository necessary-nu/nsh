//! `pwd`.
//!
//! Port of `pwdcmd` from `src/cd.c`. It prints what the shell believes
//! the current directory to be -- the logical path it has been
//! maintaining, or with `-P` the one the kernel would give.
//!
//! It shares `cd`'s option scan, because `-L` and `-P` mean the same
//! thing to both.

use crate::error::Error;
use bstr::BStr;
use core::ptr::addr_of;
use libc::c_int;
use std::io::Write;

use crate::builtins::cd::cdopt;
use crate::cd::{Pwd, cbytes, curdir, physdir, setpwd_inner};
use crate::eval::Flow;
use crate::options::Options;

// [spec:dash:def:cd.pwdcmd-fn]
// [spec:dash:sem:cd.pwdcmd-fn]
pub unsafe fn pwdcmd(args: &[&BStr]) -> Result<Flow, Error> {
    let flags: c_int;

    flags = cdopt(&mut Options::new(args))?;
    let mut dir = if flags != 0 {
        if (*addr_of!(physdir)).is_none() {
            setpwd_inner(Pwd::Current, 0)?;
        }
        cbytes(&*addr_of!(physdir))
    } else {
        cbytes(&*addr_of!(curdir))
    };
    dir.pop();
    dir.push(b'\n');
    let _ = (*crate::output::stdout()).write_all(&dir);
    Ok(Flow::Done(0))
}
