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

// [spec:dash:def:main.find-dot-file-fn]
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

// [spec:dash:def:main.dotcmd-fn]
// [spec:dash:sem:main.dotcmd-fn]
// [spec:posix:syn:builtin.dot.syn]
// [spec:posix:req:builtin.dot.execute-in-current-environment]
// [spec:posix:req:builtin.dot.utility-syntax-guidelines]
// [spec:posix:req:builtin.dot.stderr]
// [spec:posix:req:builtin.dot.interfaces]
// [spec:posix:req:builtin.dot.exit-status]
pub fn run_dot(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    run_dot_with_missing_status(shell, args, crate::status::ExitStatus::FAILURE)
}

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

        let outcome = crate::resource::with_resources(shell, |shell, _resources| {
            crate::input::set_input_file(
                shell,
                full_path.as_slice().as_bstr(),
                crate::input::InputFileOptions::DOT,
            )?;
            /* `evalbltin`'s epilogue reads `commandname` after this returns.
             * The owned path remains valid independently of the input frame. */
            shell.evaluation.command_name = Some(full_path);

            // A dot script is a fresh lexical command context for loop control:
            // loops active in its caller do not enclose commands read here.
            // [spec:nsh:req:compat.smoosh.control-boundaries]
            let caller_loopnest = shell.evaluation.loop_depth;
            shell.evaluation.loop_depth = 0;
            let outcome = command_loop(shell, false);
            shell.evaluation.loop_depth = caller_loopnest;
            outcome
        });
        match outcome? {
            Flow::Done(s) => status = s,
            control => return Ok(control),
        }
    }

    Ok(Flow::Done((status).into()))
}
