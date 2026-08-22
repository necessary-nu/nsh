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

use crate::builtins::r#type::describe_command;
use crate::evaluation::Flow;

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
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut describe = None;
    let mut use_default_path = false;

    let mut option_scan = crate::options::Options::new(args);
    while let Some(option) = option_scan.next(&mut shell.diagnostics(), b"pvV")? {
        if option == b'V' {
            describe = Some(true);
        } else if option == b'v' {
            describe.get_or_insert(false);
        } else {
            use_default_path = true;
        }
    }

    if let Some(verbose) = describe {
        // [spec:nsh:req:compat.bash.builtins-special-variables]
        if shell.options.dialect() == crate::options::Dialect::Bash && !use_default_path {
            let names = &option_scan.operands()[..1.min(option_scan.operands().len())];
            return crate::builtins::r#type::bash::run(
                shell,
                crate::builtins::r#type::bash::Requested::describing(verbose),
                crate::output::OutputDestination::Stdout,
                names,
            );
        }
        if let Some(command_name) = option_scan.operands().first() {
            let default_path = use_default_path.then(crate::variables::default_path);
            return describe_command(
                shell,
                crate::output::OutputDestination::Stdout,
                command_name,
                default_path.as_ref().map(|path| BStr::new(path.as_slice())),
                verbose,
            );
        }
    }

    Ok(Flow::Done((0).into()))
}
