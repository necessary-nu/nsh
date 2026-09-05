//! `exec`.
//!
//! Port of `execcmd` from `src/eval.c`. With an operand it replaces the
//! shell's process image and never returns; with none it is the
//! redirection-only form, and the redirections have already been made
//! permanent by the time it runs.
//!
//! This is the builtin `[dec:nsh:public-surface]` singles out: an
//! embedded shell cannot survive it, so the API gates it behind a `Host`
//! method a frontend grants and an ordinary embedder refuses.
//!
//! # The three letters of `exec [-cl] [-a name]`
//!
//! POSIX gives `exec` no options and dash takes none: `exec -a N prog`
//! reads `-a` as the program and reports it missing. Bash gives it three.
//! Two say what the image will call itself and the third says what it is
//! handed:
//!
//! * `exec -a MYNAME sh -c 'echo $0'` prints `MYNAME` -- the program is
//!   still found by the word after the name, and only `argv[0]` moves.
//! * `exec -l sh -c 'echo $0'` prints `-/bin/sh`, and `exec -l -a N`
//!   prints `-N`: the letter prefixes a hyphen to whatever `argv[0]`
//!   would otherwise have been, which is how a login shell is spelled.
//! * `exec -c /usr/bin/env` prints nothing whatever the shell exported:
//!   the program is handed an empty environment. The *search* still reads
//!   the shell's, so `exec -c sh -c ...` finds `sh` on the shell's own
//!   `PATH` and the child then sets its own default.
//!
//! The letters live inside the dialect test for the reason `export -n`'s
//! do: they are Bash's alone, and taking them in the POSIX dialect would
//! move dash, which reads each of the three as a program that is not
//! there and ends the shell with 127.
//!
//! Every claim above is measured against the pinned Bash 5.3.15 by
//! `crates/nsh-cli/tests/bash_exec_argument_zero.rs`, which runs each case
//! through both shells and compares; nothing here is a recorded answer.

// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;

use bstr::{BStr, BString, ByteSlice};

use crate::evaluation::Flow;
use crate::execution::{ExecOverrides, execute_external_command_for_exec};

// [spec:dash:sem:eval.execcmd-fn]
// [spec:posix:syn:builtin.exec.syn]
// [spec:posix:req:builtin.exec.no-operands-redirections]
// [spec:posix:req:builtin.exec.utility-operand]
// [spec:posix:req:builtin.exec.failure-non-interactive-exits]
// [spec:posix:req:builtin.exec.failure-interactive-up]
// [spec:posix:req:builtin.exec.utility-syntax-guidelines]
// [spec:posix:req:builtin.exec.env-path]
// [spec:posix:req:builtin.exec.stderr]
// [spec:posix:req:builtin.exec.interfaces]
// [spec:posix:req:builtin.exec.exit-status]
// [spec:nsh:def:idiom.shell-options]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    let (utility, argument_zero, empty_environment) =
        if shell.options.dialect() == crate::options::Dialect::Bash {
            let mut option_scan = crate::options::Options::new(args);
            let mut name: Option<&BStr> = None;
            let mut login = false;
            let mut empty_environment = false;
            while let Some(letter) = option_scan.next(&mut shell.diagnostics(), b"a:cl")? {
                match letter {
                    b'a' => name = Some(option_scan.arg()),
                    b'c' => empty_environment = true,
                    _ => login = true,
                }
            }
            let operands = option_scan.operands();
            let argument_zero = spelled_name(name, login, operands.first().copied());
            (operands, argument_zero, empty_environment)
        } else {
            let mut utility = args.get(1..).unwrap_or_default();
            if utility
                .first()
                .is_some_and(|argument| *argument == BStr::new(b"--"))
            {
                utility = &utility[1..];
            }
            (utility, None, false)
        };

    if !utility.is_empty() {
        let interactive_root = shell
            .options
            .enabled(crate::options::ShellOption::Interactive)
            && shell.shell_level == 0;
        let saved_interactive = shell
            .options
            .enabled(crate::options::ShellOption::Interactive);
        let saved_monitor = shell.options.enabled(crate::options::ShellOption::Monitor);
        if !interactive_root {
            shell
                .options
                .set(crate::options::ShellOption::Interactive, false); /* exit on error */
        }
        shell
            .options
            .set(crate::options::ShellOption::Monitor, false);
        crate::options::apply_option_changes(shell)?;
        if !interactive_root {
            shell.flush_input();
        }
        /* Hoisted out of `shellexec`'s argument list, which also takes
         * the shell; see the note in `eval.rs`'s `evalcommand`. */
        let path = crate::variables::path_value(shell);
        let outcome = execute_external_command_for_exec(
            shell,
            utility,
            path.as_slice().as_bstr(),
            None,
            ExecOverrides {
                argument_zero: argument_zero.as_deref().map(BStr::new),
                empty_environment,
            },
        );

        if interactive_root {
            /* A successful exec never returns. On failure, restore the
             * interactive shell state before allowing evaluation to
             * continue. `Flow::Done` also takes the ordinary evalcommand
             * cleanup path, where exec's redirections are kept. */
            shell
                .options
                .set(crate::options::ShellOption::Interactive, saved_interactive);
            shell
                .options
                .set(crate::options::ShellOption::Monitor, saved_monitor);
            crate::options::apply_option_changes(shell)?;
            return match outcome? {
                Flow::Exit { .. } => Ok(Flow::Done(shell.status)),
                done @ Flow::Done(_) => Ok(done),
                control => Ok(control),
            };
        }

        return outcome;
    }
    Ok(Flow::Done((0).into()))
}

/// The `argv[0]` `-a` and `-l` between them ask for, or `None` when the
/// program keeps the name it was found by.
///
/// `-l` is not a name of its own: it prefixes a hyphen to whichever name
/// is in force, which is `-a`'s when there is one and the program word
/// otherwise -- `exec -l sh` runs as `-/bin/sh` in the reference and
/// `exec -l -a N sh` as `-N`. With no operand there is no program to run
/// under any name and the answer is never read.
fn spelled_name(name: Option<&BStr>, login: bool, program: Option<&BStr>) -> Option<BString> {
    if !login {
        return name.map(|name| BString::from(name.to_vec()));
    }
    let mut spelled = BString::from(vec![b'-']);
    spelled.extend_from_slice(name.or(program).unwrap_or_default());
    Some(spelled)
}
