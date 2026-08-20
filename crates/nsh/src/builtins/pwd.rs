//! `pwd`.
//!
//! Port of `pwdcmd` from `src/cd.c`. It prints what the shell believes
//! the current directory to be -- the logical path it has been
//! maintaining, or with `-P` the one the kernel would give.
//!
//! It shares `cd`'s option scan, because `-L` and `-P` mean the same
//! thing to both.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ffi::c_int;
use std::io::Write;

use crate::builtins::cd::cdopt;
use crate::cd::{Pwd, setpwd_inner};
use crate::eval::Flow;
use crate::options::Options;

// [spec:dash:def:cd.pwdcmd-fn]
// [spec:dash:sem:cd.pwdcmd-fn]
pub fn pwdcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let flags: c_int;

    flags = cdopt(sh, &mut Options::new(args))?;
    let mut dir = if flags != 0 {
        if sh.cwd.physdir.is_none() {
            setpwd_inner(sh, Pwd::Current, 0)?;
        }
        sh.cwd.physdir.clone().unwrap_or_default()
    } else {
        sh.cwd.curdir.clone().unwrap_or_default()
    };
    dir.push(b'\n');
    let _ = sh.io.stdout().write_all(&dir);
    Ok(Flow::Done((0).into()))
}
