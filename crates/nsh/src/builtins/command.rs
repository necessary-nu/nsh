//! `command`.
//!
//! Port of `commandcmd` from `src/exec.c`.
//!
//! Only the `-v`/`-V` half is here. Plain `command cmd` -- the form that
//! runs `cmd` while skipping functions -- is not a call to this function
//! at all: `evalcommand` recognises the word and re-runs its own lookup
//! with the flags changed, so the dispatch never reaches a builtin. What
//! this does is describe a name, which is `type`.

use crate::error::Error;
use bstr::BStr;
use core::ptr::null;
use libc::{c_char, c_int};

use crate::builtins::r#type::describe_command;
use crate::options::Options;

// [spec:dash:def:exec.commandcmd-fn]
// [spec:dash:sem:exec.commandcmd-fn]
pub unsafe fn commandcmd(args: &[&BStr]) -> Result<c_int, Error> {
    const VERIFY_BRIEF: c_int = 1;
    const VERIFY_VERBOSE: c_int = 2;
    let mut verify: c_int = 0;
    let mut path: *const c_char = null();

    let mut opts = crate::options::Options::new(args);
    while let Some(c) = opts.next(b"pvV") {
        if c == b'V' {
            verify |= VERIFY_VERBOSE;
        } else if c == b'v' {
            verify |= VERIFY_BRIEF;
        } else {
            /* DEBUG: `else if (c != 'p') abort();` */
            path = crate::var::defpath();
        }
    }

    if verify != 0 {
        if let Some(cmd) = opts.operands().first() {
            let cmd = crate::shell::cstring(cmd);
            return Ok(describe_command(
                crate::output::stdout(),
                cmd.as_ptr() as *mut c_char,
                path,
                verify - VERIFY_BRIEF,
            ));
        }
    }

    Ok(0)
}
