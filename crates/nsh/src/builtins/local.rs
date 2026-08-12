//! `local`.
//!
//! Port of `localcmd` from `src/var.c`. Making a variable local is
//! `crate::var`'s `mklocal` -- it has to save the old value where the
//! function's return will find it -- so this is the guard and the loop.
//!
//! It scans no options at all, which is why `local -x` localises a
//! variable called `-x` rather than complaining.

use crate::error::Error;
use bstr::BStr;
use libc::{c_char, c_int};

use crate::var::{localvar_stack_mut, mklocal};

// [spec:dash:def:var.localcmd-fn]
// [spec:dash:sem:var.localcmd-fn]
pub unsafe fn localcmd(args: &[&BStr]) -> Result<c_int, Error> {
    if localvar_stack_mut().is_empty() {
        return Err(crate::error::sh_error_value(b"not in a function"));
    }

    /* `local` scans no options at all, so every word after the command
     * name is a name to localise -- including one that starts with `-`. */
    for name in &args[1..] {
        let name = crate::shell::cstring(name);
        mklocal(name.as_ptr() as *mut c_char, 0)?;
    }
    Ok(0)
}
