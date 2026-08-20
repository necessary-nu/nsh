//! `hash`.
//!
//! Port of `hashcmd` and `printentry` from `src/exec.c`. The command
//! table it prints and clears stays in `crate::exec`, which is what fills
//! it during a PATH search.

// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, ByteSlice};

use crate::eval::Flow;
use crate::exec::{
    Command, CommandSearch, PathCursor, clearcmdentry, delete_cmd_entry, find_command, padvance,
};
use crate::output::Dest;

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
// [spec:nsh:req:idiom.command-dispatch]
pub fn hashcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut entry = Command::Unknown;
    let mut clear: bool;

    clear = false;
    let mut opts = crate::options::Options::new(args);
    while opts.next(&mut sh.diagnostics(), b"r")?.is_some() {
        clear = true;
    }
    if clear {
        clearcmdentry(&mut sh.interrupt_deferral, &mut sh.commands);
        return Ok(Flow::Done((0).into()));
    }

    let operands = opts.operands();
    if operands.is_empty() {
        /* `PATH` is read before the walk rather than inside
         * `printentry`: the walk holds `sh.commands` borrowed, and
         * reading `sh.vars` through the receiver inside it would borrow
         * the shell twice. This is the "copy the scalar out before the
         * walk" technique the command table already needed for
         * built-in location, with a pointer in place of a flag -- and the
         * value is the same one `printentry` read for itself, since
         * nothing in the loop assigns to `PATH`. */
        let path = crate::var::pathval(sh);
        let lines: Vec<Vec<u8>> = sh
            .commands
            .iter()
            .filter_map(|(name, cmdp)| {
                let Command::External { path_index } = &cmdp.command else {
                    return None;
                };
                let Some(path_index) = *path_index else {
                    return None;
                };
                Some(printentry(
                    name.as_slice().as_bstr(),
                    path_index,
                    cmdp.rehash,
                    path.as_slice().as_bstr(),
                ))
            })
            .collect();
        for line in lines {
            sh.write_output(Dest::Stdout, &line)?;
        }
        return Ok(Flow::Done((0).into()));
    }
    let mut failed = false;
    for name in operands {
        if sh
            .commands
            .get(name)
            .is_some_and(|cmdp| sh.commands.path_dependent(cmdp))
        {
            delete_cmd_entry(&mut sh.interrupt_deferral, &mut sh.commands, name);
        }
        /* Hoisted out of the argument list; see the note in `eval.rs`'s
         * `evalcommand`. */
        let path = crate::var::pathval(sh);
        match find_command(
            sh,
            name,
            &mut entry,
            CommandSearch::DEFAULT.reporting_errors(),
            path.as_bstr(),
        )? {
            crate::eval::Flow::Done(_) => {}
            control => return Ok(control),
        }
        if matches!(&entry, Command::Unknown) {
            failed = true;
        }
    }
    Ok(Flow::Done(i32::from(failed).into()))
}

// [spec:dash:def:exec.printentry-fn]
// [spec:dash:sem:exec.printentry-fn]
/// `path` is `pathval()`, read by the caller.
///
/// The C reads it here. It cannot be read here any more: the only caller
/// that walks the command table holds it borrowed across this call, so the
/// read has to happen before the walk starts. Passing it in is what makes
/// that visible rather than a surprise.
fn printentry(name: &BStr, path_index: usize, rehash: bool, pathval: &BStr) -> Vec<u8> {
    let mut path = PathCursor::new(pathval);
    let candidate = (0..=path_index)
        .map(|_| padvance(&mut path, name))
        .last()
        .flatten();
    let fullname = candidate
        .expect("a cached PATH index must still name a PATH element")
        .path;
    /* Rendered rather than written, for the reason the note above gives
     * about `pathval`: the walk holds `sh.commands` borrowed, and a write
     * wants `sh.io` at the same time. */
    let mut line = fullname.to_vec();
    line.extend_from_slice(if rehash { b"*\n" } else { b"\n" });
    line
}
