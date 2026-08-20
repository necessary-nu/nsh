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

// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;

use bstr::{BStr, ByteSlice};

use crate::eval::Flow;
use crate::exec::shellexec;

// [spec:dash:def:eval.execcmd-fn]
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
pub fn execcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut utility = args.get(1..).unwrap_or_default();
    if utility
        .first()
        .is_some_and(|argument| *argument == BStr::new(b"--"))
    {
        utility = &utility[1..];
    }

    if !utility.is_empty() {
        let interactive_root =
            sh.options.enabled(crate::options::ShellOption::Interactive) && sh.shell_level == 0;
        let saved_interactive = sh.options.enabled(crate::options::ShellOption::Interactive);
        let saved_monitor = sh.options.enabled(crate::options::ShellOption::Monitor);
        if !interactive_root {
            sh.options
                .set(crate::options::ShellOption::Interactive, false); /* exit on error */
        }
        sh.options.set(crate::options::ShellOption::Monitor, false);
        crate::options::optschanged(sh)?;
        if !interactive_root {
            sh.flush_input();
        }
        /* Hoisted out of `shellexec`'s argument list, which also takes
         * the shell; see the note in `eval.rs`'s `evalcommand`. */
        let path = crate::var::pathval(sh);
        let outcome = shellexec(sh, utility, path.as_slice().as_bstr(), None);

        if interactive_root {
            /* A successful exec never returns. On failure, restore the
             * interactive shell state before allowing evaluation to
             * continue. `Flow::Done` also takes the ordinary evalcommand
             * cleanup path, where exec's redirections are kept. */
            sh.options
                .set(crate::options::ShellOption::Interactive, saved_interactive);
            sh.options
                .set(crate::options::ShellOption::Monitor, saved_monitor);
            crate::options::optschanged(sh)?;
            return match outcome? {
                Flow::Exit { .. } => Ok(Flow::Done((sh.status).into())),
                done @ Flow::Done(_) => Ok(done),
                control => Ok(control),
            };
        }

        return outcome;
    }
    Ok(Flow::Done((0).into()))
}
