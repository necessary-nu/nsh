//! `hash`.
//!
//! Port of `hashcmd` and `printentry` from `src/exec.c`. The command
//! table it prints and clears stays in `crate::exec`, which is what fills
//! it during a PATH search.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, ByteSlice};
use core::ffi::c_int;
use std::ffi::CStr;
use std::io::Write;

use crate::eval::Flow;
use crate::exec::{
    CMDNORMAL, CMDUNKNOWN, DO_ERR, PathCursor, clearcmdentry, cmdentry, delete_cmd_entry,
    find_command, padvance, tblentry,
};

// [spec:dash:def:exec.hashcmd-fn]
// [spec:dash:sem:exec.hashcmd-fn]
// [spec:posix:syn:builtin.hash.synopsis]
// [spec:posix:req:builtin.hash.remembered-locations]
// [spec:posix:req:builtin.hash.builtins-and-functions-not-reported]
// [spec:posix:req:builtin.hash.utility-syntax-guidelines]
// [spec:posix:req:builtin.hash.opt-r]
// [spec:posix:def:builtin.hash.operand-utility]
// [spec:posix:sem:builtin.hash.operand-utility-unspecified]
// [spec:posix:req:builtin.hash.env-locale]
// [spec:posix:sem:builtin.hash.env-nlspath]
// [spec:posix:sem:builtin.hash.env-path]
// [spec:posix:req:builtin.hash.stdout-report]
// [spec:posix:req:builtin.hash.list-cleared-on-path-change]
// [spec:posix:req:builtin.hash.stderr]
// [spec:posix:req:builtin.hash.interfaces]
// [spec:posix:req:builtin.hash.exit-status]
pub fn hashcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut c: c_int;
    let mut entry = cmdentry::unknown();
    let mut clear: bool;

    clear = false;
    let mut opts = crate::options::Options::new(args);
    while opts.next(sh, b"r")?.is_some() {
        clear = true;
    }
    if clear {
        clearcmdentry(sh);
        return Ok(Flow::Done((0).into()));
    }

    let operands = opts.operands();
    if operands.is_empty() {
        /* `PATH` is read before the walk rather than inside
         * `printentry`: the walk holds `sh.commands` borrowed, and
         * reading `sh.vars` through the receiver inside it would borrow
         * the shell twice. This is the "copy the scalar out before the
         * walk" technique the command table already needed for
         * `builtinloc`, with a pointer in place of a flag -- and the
         * value is the same one `printentry` read for itself, since
         * nothing in the loop assigns to `PATH`. */
        let path = crate::var::pathval(sh);
        let lines: Vec<Vec<u8>> = sh
            .commands
            .iter()
            .filter(|(_, cmdp)| cmdp.cmdtype() == CMDNORMAL)
            .map(|(name, cmdp)| {
                printentry(name.as_slice().as_bstr(), cmdp, path.as_slice().as_bstr())
            })
            .collect();
        for line in lines {
            let _ = sh.io.stdout().write_all(&line);
        }
        return Ok(Flow::Done((0).into()));
    }
    c = 0;
    for name in operands {
        if sh
            .commands
            .get(name)
            .is_some_and(|cmdp| sh.commands.path_dependent(cmdp))
        {
            delete_cmd_entry(sh, name);
        }
        /* Hoisted out of the argument list; see the note in `eval.rs`'s
         * `evalcommand`. */
        let path = crate::var::pathval(sh);
        match find_command(sh, name, &mut entry, DO_ERR, path.as_bstr())? {
            crate::eval::Flow::Done(_) => {}
            exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
        }
        if entry.cmdtype() == CMDUNKNOWN {
            c = 1;
        }
    }
    Ok(Flow::Done((c).into()))
}

// [spec:dash:def:exec.printentry-fn]
// [spec:dash:sem:exec.printentry-fn]
/// `path` is `pathval()`, read by the caller.
///
/// The C reads it here. It cannot be read here any more: the only caller
/// that walks the command table holds it borrowed across this call, so the
/// read has to happen before the walk starts. Passing it in is what makes
/// that visible rather than a surprise.
fn printentry(name: &BStr, cmdp: &tblentry, pathval: &BStr) -> Vec<u8> {
    let mut idx: c_int;
    let mut candidate = None;

    idx = cmdp.path_index();
    let mut path = PathCursor::new(pathval);
    loop {
        candidate = padvance(&mut path, name);
        idx -= 1;
        if idx < 0 {
            break;
        }
    }
    let fullname = candidate
        .expect("a cached PATH index must still name a PATH element")
        .path;
    /* Rendered rather than written, for the reason the note above gives
     * about `pathval`: the walk holds `sh.commands` borrowed, and a write
     * wants `sh.io` at the same time. */
    let mut line = CStr::from_bytes_with_nul(&fullname)
        .expect("padvance returns one terminated candidate")
        .to_bytes()
        .to_vec();
    line.extend_from_slice(if cmdp.rehash { b"*\n" } else { b"\n" });
    line
}
