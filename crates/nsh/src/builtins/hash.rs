//! `hash`.
//!
//! Port of `hashcmd` and `printentry` from `src/exec.c`. The command
//! table it prints and clears stays in `crate::execution`, which is what fills
//! it during a PATH search.

// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, ByteSlice};

use crate::evaluation::Flow;
use crate::execution::{
    Command, CommandSearch, PathCursor, clear_command_cache, find_command, remove_command_entry,
};
use crate::output::OutputDestination;

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
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut entry = Command::Unknown;
    let mut clear: bool;

    clear = false;
    let mut option_scan = crate::options::Options::new(args);
    while option_scan.next(&mut shell.diagnostics(), b"r")?.is_some() {
        clear = true;
    }
    if clear {
        clear_command_cache(&mut shell.interrupt_deferral, &mut shell.commands);
        return Ok(Flow::Done((0).into()));
    }

    let operands = option_scan.operands();
    if operands.is_empty() {
        /* `PATH` is read before the walk rather than inside
         * `printentry`: the walk holds `sh.commands` borrowed, and
         * reading `sh.vars` through the receiver inside it would borrow
         * the shell twice. This is the "copy the scalar out before the
         * walk" technique the command table already needed for
         * built-in location, with a pointer in place of a flag -- and the
         * value is the same one `printentry` read for itself, since
         * nothing in the loop assigns to `PATH`. */
        let path = crate::variables::path_value(shell);
        let lines: Vec<Vec<u8>> = shell
            .commands
            .iter()
            .filter_map(|(name, command_entry)| {
                let Command::External { path_index } = &command_entry.command else {
                    return None;
                };
                let Some(path_index) = *path_index else {
                    return None;
                };
                Some(format_hash_entry(
                    name.as_slice().as_bstr(),
                    path_index,
                    command_entry.rehash,
                    path.as_slice().as_bstr(),
                ))
            })
            .collect();
        for line in lines {
            shell.write_output(OutputDestination::Stdout, &line)?;
        }
        return Ok(Flow::Done((0).into()));
    }
    let mut failed = false;
    for name in operands {
        if shell
            .commands
            .get(name)
            .is_some_and(|command_entry| shell.commands.path_dependent(command_entry))
        {
            remove_command_entry(&mut shell.interrupt_deferral, &mut shell.commands, name);
        }
        /* Hoisted out of the argument list; see the note in `eval.rs`'s
         * `evalcommand`. */
        let path = crate::variables::path_value(shell);
        match find_command(
            shell,
            name,
            &mut entry,
            CommandSearch::DEFAULT.reporting_errors(),
            path.as_bstr(),
        )? {
            crate::evaluation::Flow::Done(_) => {}
            control => return Ok(control),
        }
        if matches!(&entry, Command::Unknown) {
            failed = true;
        }
    }
    Ok(Flow::Done(i32::from(failed).into()))
}

// [spec:dash:sem:exec.printentry-fn]
/// `path` is `pathval()`, read by the caller.
///
/// The C reads it here. It cannot be read here any more: the only caller
/// that walks the command table holds it borrowed across this call, so the
/// read has to happen before the walk starts. Passing it in is what makes
/// that visible rather than a surprise.
fn format_hash_entry(name: &BStr, path_index: usize, rehash: bool, path_value: &BStr) -> Vec<u8> {
    let mut path = PathCursor::new(path_value);
    let candidate = (0..=path_index)
        .map(|_| path.advance(name))
        .last()
        .flatten();
    let full_path = candidate
        .expect("a cached PATH index must still name a PATH element")
        .path;
    /* Rendered rather than written, for the reason the note above gives
     * about `pathval`: the walk holds `sh.commands` borrowed, and a write
     * wants `sh.io` at the same time. */
    let mut line = full_path.to_vec();
    line.extend_from_slice(if rehash { b"*\n" } else { b"\n" });
    line
}
