//! `.`, the dot builtin.
//!
//! Port of `dotcmd` and `find_dot_file` from `src/main.c`.
//!
//! It re-enters evaluation by pushing the named file onto the input
//! stack and running the command loop over it, so like `eval` it depends
//! on its words not borrowing from the shell.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;
use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt;

use crate::eval::Flow;
use crate::exec::PathCursor;
use crate::shellmain::cmdloop;

// [spec:dash:def:main.find-dot-file-fn]
// [spec:dash:sem:main.find-dot-file-fn]
// [spec:posix:req:builtin.dot.path-search]
/// The C returns a `stalloc`'d copy of the candidate. Here the caller owns
/// the returned bytes directly, for exactly the same lifetime.
fn find_dot_file(sh: &mut crate::context::Shell, basename: &BStr) -> Option<BString> {
    let path_value = crate::var::pathval(sh);

    /* Explicit paths do not use PATH, but they still have to name a
     * readable regular file. Classifying a missing explicit operand here
     * keeps `.`'s own diagnostic/status contract instead of leaking the
     * input subsystem's generic open failure. */
    if basename.contains(&b'/') {
        let path = OsStr::from_bytes(basename);
        let regular_file = std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file());
        let readable = nsh_platform::effective_access(path, nsh_platform::AccessMode::READ_OK);
        return (regular_file && readable).then(|| basename.to_owned());
    }

    let mut path = PathCursor::new(path_value.as_slice().as_bstr());
    while let Some(candidate) = crate::exec::padvance(&mut path, basename) {
        let fullname = crate::mystring::cstr_prefix(&candidate.path);
        let path = OsStr::from_bytes(fullname);
        let regular_file = std::fs::metadata(path)
            .is_ok_and(|metadata| metadata.is_file());
        let readable = nsh_platform::effective_access(path, nsh_platform::AccessMode::READ_OK);
        if (candidate.option.is_none()
            || candidate.option.as_ref().and_then(|option| option.first()) == Some(&b'f'))
            && regular_file
            && readable
        {
            return Some(fullname.to_owned());
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
pub fn dotcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    dotcmd_with_missing_status(sh, args, 1)
}

// [spec:nsh:req:compat.smoosh.source-builtin]
pub fn sourcecmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    dotcmd_with_missing_status(sh, args, 1)
}

fn dotcmd_with_missing_status(
    sh: &mut Shell,
    args: &[&BStr],
    missing_status: c_int,
) -> Result<Flow, Error> {
    let mut status: c_int = 0;

    let mut opts = crate::options::Options::new(args);
    opts.next(sh, b"")?;

    if let Some(name) = opts.operands().first() {
        let Some(fullname) = find_dot_file(sh, name) else {
            let mut message = name.to_vec();
            message.extend_from_slice(b": not found");
            return Err(sh.builtin_error_value(missing_status, &message));
        };

        crate::input::setinputfile(
            sh,
            fullname.as_slice().as_bstr(),
            crate::input::INPUT_PUSH_FILE | crate::input::INPUT_DOT_FILE,
        )?;
        /* `evalbltin`'s epilogue reads `commandname` after this returns —
         * `flushall(); if (outerr(out1)) sh_warnx("%s: I/O error",
         * commandname);` — and the C is safe there only because the block
         * is `stalloc`'d and the enclosing mark has not popped yet.
         *
         * Now that `commandname` owns its bytes there is nothing to keep
         * alive: what the epilogue reads is a copy, so the buffer this
         * frame allocated can be freed with the frame like any other
         * local, and the static slot that used to hold it is gone. */
        sh.eval.commandname = Some(fullname);
        /* An `exit` inside a dotted file ends the shell, not the file, so
         * it leaves through here without the `popfile` -- exactly as the
         * C's longjmp did. The input stack is unwound to a mark by
         * whatever catches, not by the frame it passed through. */
        // A dot script is a fresh lexical command context for loop control:
        // loops active in its caller do not enclose commands read here.
        // [spec:nsh:req:compat.smoosh.control-boundaries]
        let caller_loopnest = sh.eval.loopnest;
        sh.eval.loopnest = 0;
        let outcome = cmdloop(sh, 0);
        sh.eval.loopnest = caller_loopnest;
        match outcome? {
            Flow::Done(s) => status = s,
            exit @ Flow::Exit { .. } => return Ok(exit),
        }
        crate::input::popfile(sh);
    }

    Ok(Flow::Done(status))
}
