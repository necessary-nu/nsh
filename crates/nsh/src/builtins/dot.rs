//! `.`, the dot builtin.
//!
//! Port of `dotcmd` and `find_dot_file` from `src/main.c`.
//!
//! It re-enters evaluation by pushing the named file onto the input
//! stack and running the command loop over it, so like `eval` it depends
//! on its words not borrowing from the shell.

// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString, ByteSlice};
use nsh_platform::ShellBytesExt as _;

use crate::evaluation::Flow;
use crate::execution::PathCursor;
use crate::runtime::command_loop;

// [spec:dash:sem:main.find-dot-file-fn]
// [spec:posix:req:builtin.dot.path-search]
/// The C returns a `stalloc`'d copy of the candidate. Here the caller owns
/// the returned bytes directly, for exactly the same lifetime.
fn find_dot_file(shell: &mut crate::context::Shell, basename: &BStr) -> Option<BString> {
    let path_value = crate::variables::path_value(shell);

    /* Explicit paths do not use PATH, but they still have to name a
     * readable regular file. Classifying a missing explicit operand here
     * keeps `.`'s own diagnostic/status contract instead of leaking the
     * input subsystem's generic open failure. */
    if nsh_platform::shell_path_has_separator(basename) {
        let Ok(path) = basename.try_to_path_buf() else {
            return None;
        };
        let regular_file = nsh_platform::path_is_file(&path);
        let readable = nsh_platform::effective_access(&path, nsh_platform::AccessMode::READ_OK);
        return (regular_file && readable).then(|| basename.to_owned());
    }

    let mut path = PathCursor::new(path_value.as_slice().as_bstr());
    while let Some(candidate) = path.advance(basename) {
        let full_path = candidate.path.as_bstr();
        let Ok(native) = full_path.try_to_path_buf() else {
            continue;
        };
        let regular_file = nsh_platform::path_is_file(&native);
        let readable = nsh_platform::effective_access(&native, nsh_platform::AccessMode::READ_OK);
        if (candidate.option.is_none()
            || candidate.option.as_ref().and_then(|option| option.first()) == Some(&b'f'))
            && regular_file
            && readable
        {
            return Some(full_path.to_owned());
        }
    }
    None
}

// [spec:dash:sem:main.dotcmd-fn]
// [spec:posix:syn:builtin.dot.syn]
// [spec:posix:req:builtin.dot.execute-in-current-environment]
// [spec:posix:req:builtin.dot.utility-syntax-guidelines]
// [spec:posix:req:builtin.dot.stderr]
// [spec:posix:req:builtin.dot.interfaces]
// [spec:posix:req:builtin.dot.exit-status]
pub fn run_dot(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    /* Status 2 in the POSIX dialect, not the 1 the imported Smoosh bytes
     * record. `.` is a POSIX special built-in, so XCU 2.8.1 makes a file
     * it cannot find fatal to a non-interactive shell and
     * `[spec:posix:req:builtin.dot.exit-status]` asks only for "non-zero";
     * dash answers 2 and `[spec:nsh:req:compat.bash.error-boundary]`
     * writes 2 down for this dialect. Only the number moves: the
     * diagnostic stays the prefix-less `.: NAME: not found` the Smoosh
     * contract fixes, in both dialects. Bash reports and carries on with
     * 1, which is what the Bash dialect keeps. */
    // [spec:nsh:req:compat.bash.error-boundary]
    let missing_status = shell.options.dialect().refusal_status();
    run_dot_with_missing_status(shell, args, missing_status)
}

/// `source` is not a POSIX built-in and dash has no answer for it, so the
/// Smoosh contract stands unopposed here: a missing file is 1 in both
/// dialects. The collision `run_dot` resolves needs two oracles, and this
/// name has one.
// [spec:nsh:req:compat.smoosh.source-builtin]
pub fn run_source(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    run_dot_with_missing_status(shell, args, crate::status::ExitStatus::FAILURE)
}

fn run_dot_with_missing_status(
    shell: &mut Shell,
    args: &[&BStr],
    missing_status: crate::status::ExitStatus,
) -> Result<Flow, Error> {
    let mut status = crate::status::ExitStatus::SUCCESS;

    let mut option_scan = crate::options::Options::new(args);
    option_scan.next(&mut shell.diagnostics(), b"")?;

    if let Some(name) = option_scan.operands().first() {
        let Some(full_path) = find_dot_file(shell, name) else {
            let mut message = name.to_vec();
            message.extend_from_slice(b": not found");
            return Err(shell
                .diagnostics()
                .builtin_error_value(missing_status, &message));
        };

        /* The line the `.` was written on, which is what this frame
         * contributes to `BASH_LINENO`. */
        // [spec:nsh:req:compat.bash.traps-introspection]
        let call_line = shell.variables.line_number;
        let outcome = crate::resource::with_resources(shell, |shell, _resources| {
            crate::input::set_input_file(
                shell,
                full_path.as_slice().as_bstr(),
                crate::input::InputFileOptions::DOT,
            )?;
            /* `evalbltin`'s epilogue reads `commandname` after this returns.
             * The owned path remains valid independently of the input frame. */
            shell.evaluation.command_name = Some(full_path.clone());

            // A dot script is a fresh lexical command context for loop control:
            // loops active in its caller do not enclose commands read here.
            // [spec:nsh:req:compat.smoosh.control-boundaries]
            /* `. self.sh` sourcing itself nests the evaluator exactly as a
             * call does and is unbounded in every shell here -- nsh, dash
             * and Bash all segfault on it. Same counter, same bound, and
             * the same reason as a function's: an embedder gets an `Err`
             * where the stack would otherwise go. */
            // [spec:nsh:req:idiom.bounded-recursion]
            if crate::variables::call_stack::evaluation_depth(shell)
                >= crate::evaluation::MAX_EVALUATION_DEPTH
            {
                let mut message = b"Maximum function recursion depth (".to_vec();
                message.extend_from_slice(
                    crate::evaluation::MAX_EVALUATION_DEPTH
                        .to_string()
                        .as_bytes(),
                );
                message.extend_from_slice(b") reached");
                return Err(shell.diagnostics().shell_error(&message));
            }
            let caller_loopnest = shell.evaluation.loop_depth;
            shell.evaluation.loop_depth = 0;
            crate::variables::call_stack::push_source(
                shell,
                full_path.as_slice().as_bstr(),
                call_line,
            );
            let outcome = command_loop(shell, crate::runtime::InputFrame::Pushed);
            /* A dot script's `RETURN` action runs whatever `functrace`
             * says: Bash withholds the trap from *functions*, and this is
             * not one. It runs while the frame is still innermost. */
            // [spec:nsh:req:compat.bash.traps-introspection]
            let outcome = match outcome {
                Ok(flow) => crate::trap::bash::run_return(shell).map(|action| match action {
                    Flow::Done(_) => flow,
                    control => control,
                }),
                failed => failed,
            };
            crate::variables::call_stack::pop(shell);
            crate::evaluation::restore_caller_line(shell, call_line);
            shell.evaluation.loop_depth = caller_loopnest;
            outcome
        });
        match outcome? {
            Flow::Done(s) => status = s,
            control => return Ok(control),
        }
    }

    Ok(Flow::Done(status))
}
