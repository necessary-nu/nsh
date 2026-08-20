//! `command`.
//!
//! Port of `commandcmd` from `src/exec.c`.
//!
//! Only the `-v`/`-V` half is here. Plain `command cmd` -- the form that
//! runs `cmd` while skipping functions -- is not a call to this function
//! at all: `evalcommand` recognises the word and re-runs its own lookup
//! with the flags changed, so the dispatch never reaches a builtin. What
//! this does is describe a name, which is `type`.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ffi::c_int;

use crate::builtins::r#type::describe_command;
use crate::eval::Flow;

// [spec:dash:def:exec.commandcmd-fn]
// [spec:dash:sem:exec.commandcmd-fn]
// [spec:posix:syn:builtin.command.synopsis]
// [spec:posix:req:builtin.command.v-options-report-interpretation]
// [spec:posix:req:builtin.command.utility-syntax-guidelines]
// [spec:posix:req:builtin.command.opt-p]
// [spec:posix:req:builtin.command.opt-v-uppercase]
// [spec:posix:req:builtin.command.env-locale]
// [spec:posix:sem:builtin.command.env-nlspath]
// [spec:posix:sem:builtin.command.env-path]
// [spec:posix:req:builtin.command.stdout-format]
// [spec:posix:req:builtin.command.stderr]
// [spec:posix:req:builtin.command.interfaces]
// [spec:posix:req:builtin.command.exit-status-v-options]
pub fn commandcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    const VERIFY_BRIEF: c_int = 1;
    const VERIFY_VERBOSE: c_int = 2;
    let mut verify: c_int = 0;
    let mut use_default_path = false;

    let mut opts = crate::options::Options::new(args);
    while let Some(c) = opts.next(sh, b"pvV")? {
        if c == b'V' {
            verify |= VERIFY_VERBOSE;
        } else if c == b'v' {
            verify |= VERIFY_BRIEF;
        } else {
            use_default_path = true;
        }
    }

    if verify != 0 {
        if let Some(cmd) = opts.operands().first() {
            let default_path = use_default_path.then(crate::var::defpath);
            return describe_command(
                sh,
                crate::output::Dest::Stdout,
                cmd,
                default_path.as_ref().map(|path| BStr::new(path.as_slice())),
                verify - VERIFY_BRIEF,
            );
        }
    }

    Ok(Flow::Done((0).into()))
}
