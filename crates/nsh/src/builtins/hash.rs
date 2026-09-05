//! `hash`.
//!
//! Port of `hashcmd` and `printentry` from `src/exec.c`. The command
//! table it prints and clears stays in `crate::execution`, which is what fills
//! it during a PATH search.
//!
//! # The letters the reference adds, and the listing it prints
//!
//! POSIX gives `hash` only `-r` and dash takes only `-r`. Bash takes
//! `[-lr] [-p pathname] [-dt] [name ...]`, and the four extra letters are
//! the only script-visible handle on the table: `-p` pins a name to a path
//! without searching for it, `-t` reads one back, `-d` forgets one name
//! where `-r` forgets all of them, and `-l` prints the table as
//! `builtin hash -p` lines that re-enter it.
//!
//! The bare listing differs in the same dialect-shaped way: the reference
//! prints a `hits`/`command` header and a per-entry consultation count,
//! and says `hash table empty` rather than nothing, where dash prints the
//! path alone. So the whole listing is behind the dialect test, because
//! the POSIX dialect's format is dash's and
//! `[spec:posix:req:builtin.hash.stdout-report]` holds it there.
//!
//! Every claim above is measured against the pinned Bash 5.3.15 by
//! `crates/nsh-cli/tests/bash_hash_option_set.rs`, which runs each case
//! through both shells and compares; nothing here is a recorded answer.
//!
//! `--posix` does not move the letters or the columns in the reference --
//! only the `hash table empty` line, which it drops. Bash mode is measured
//! against plain `bash`, as `exec`'s letters are.

// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString, ByteSlice};

use crate::evaluation::Flow;
use crate::execution::{
    Command, CommandEntry, CommandSearch, PathCursor, clear_command_cache, find_command,
    remove_command_entry,
};
use crate::output::OutputDestination;

/// What the option letters between them asked for.
///
/// `-d`, `-t` and `-p` all act on the operands rather than on the table as
/// a whole, which is why an empty operand list is an error for each of
/// them and a listing for the other two.
#[derive(Default)]
struct Request<'a> {
    clear: bool,
    forget: bool,
    show: bool,
    portable: bool,
    pinned: Option<&'a BStr>,
}

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
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let mut request = Request::default();
    let mut option_scan = crate::options::Options::new(args);
    let letters: &[u8] = if bash { b"dlp:rt" } else { b"r" };
    while let Some(letter) = option_scan.next(&mut shell.diagnostics(), letters)? {
        match letter {
            b'r' => request.clear = true,
            b'd' => request.forget = true,
            b't' => request.show = true,
            b'l' => request.portable = true,
            _ => request.pinned = Some(option_scan.arg()),
        }
    }
    let operands = option_scan.operands();

    if request.clear {
        clear_command_cache(&mut shell.interrupt_deferral, &mut shell.commands);
        /* dash stops here whatever else was written: `hash -r ls` clears
         * and never looks at `ls`. The reference clears first and then
         * hashes the operands, so only the POSIX dialect returns early. */
        if !bash || operands.is_empty() {
            return Ok(Flow::Done((0).into()));
        }
    }

    if operands.is_empty() {
        return list_table(shell, bash, &request);
    }
    if bash && (request.forget || request.show || request.pinned.is_some()) {
        return act_on_names(shell, &request, operands);
    }
    populate(shell, operands)
}

/// `hash name ...`: fill the table by searching for each name.
///
/// The search is told not to count itself. The reference reports 0 hits
/// for a name it has just hashed even when the command has already run
/// several times, so writing the entry is not a use of it.
fn populate(shell: &mut Shell, operands: &[&BStr]) -> Result<Flow, Error> {
    let mut entry = Command::Unknown;
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
            CommandSearch::DEFAULT
                .reporting_errors()
                .not_counting_a_hit(),
            path.as_bstr(),
        )? {
            crate::evaluation::Flow::Done(_) => {}
            control => return Ok(control),
        }
        if matches!(&entry, Command::Unknown) {
            failed = true;
        } else {
            shell.commands.restart_hit_count(name);
        }
    }
    Ok(Flow::Done(i32::from(failed).into()))
}

/// `-p`, `-t` and `-d`, each of which reads or writes the table directly.
///
/// None of the three searches `PATH`: `hash -t ls` on an empty table says
/// `not found` rather than finding `ls`, and `hash -p` is the whole point
/// of not searching.
fn act_on_names(
    shell: &mut Shell,
    request: &Request<'_>,
    operands: &[&BStr],
) -> Result<Flow, Error> {
    if let Some(pinned) = request.pinned {
        for name in operands {
            shell.commands.pin(name, pinned);
        }
        return Ok(Flow::Done((0).into()));
    }
    let path = crate::variables::path_value(shell);
    let mut failed = false;
    for name in operands {
        let resolved = shell
            .commands
            .get(name)
            .and_then(|entry| entry_path(entry, name, path.as_bstr()));
        let Some(resolved) = resolved else {
            let mut message = b"hash: ".to_vec();
            message.extend_from_slice(name);
            message.extend_from_slice(b": not found\n");
            shell.write_output(OutputDestination::Stderr, &message)?;
            failed = true;
            continue;
        };
        if request.forget {
            remove_command_entry(&mut shell.interrupt_deferral, &mut shell.commands, name);
            continue;
        }
        let mut line = Vec::new();
        /* One operand prints the path alone; two or more label each with
         * the name it was asked about. */
        if operands.len() > 1 {
            line.extend_from_slice(name);
            line.push(b'\t');
        }
        line.extend_from_slice(&resolved);
        line.push(b'\n');
        shell.write_output(OutputDestination::Stdout, &line)?;
        shell.commands.note_hit(name);
    }
    Ok(Flow::Done(i32::from(failed).into()))
}

/// The path a table entry stands for, or `None` when it names no external.
///
/// A pinned entry keeps its path verbatim; every other external is stored
/// as an index into `PATH` and has to be walked back to a path from there.
fn entry_path(entry: &CommandEntry, name: &BStr, path_value: &BStr) -> Option<BString> {
    if let Some(pinned) = &entry.pinned {
        return Some(pinned.clone());
    }
    let Command::External { path_index } = &entry.command else {
        return None;
    };
    let mut path = PathCursor::new(path_value);
    let candidate = (0..=(*path_index)?)
        .map(|_| path.advance(name))
        .last()
        .flatten();
    Some(BString::from(candidate?.path.to_vec()))
}

/// `hash` and `hash -l` with no operand.
fn list_table(shell: &mut Shell, bash: bool, request: &Request<'_>) -> Result<Flow, Error> {
    if bash {
        if request.pinned.is_some() {
            let usage = b"hash: usage: hash [-lr] [-p pathname] [-dt] [name ...]\n";
            shell.write_output(OutputDestination::Stderr, usage)?;
            return Ok(Flow::Done((2).into()));
        }
        if request.forget || request.show {
            let letter: &[u8] = if request.forget { b"-d" } else { b"-t" };
            let mut message = b"hash: ".to_vec();
            message.extend_from_slice(letter);
            message.extend_from_slice(b": option requires an argument\n");
            shell.write_output(OutputDestination::Stderr, &message)?;
            return Ok(Flow::Done((1).into()));
        }
    }
    /* `PATH` is read before the walk rather than inside `printentry`: the
     * walk holds `sh.commands` borrowed, and reading `sh.vars` through the
     * receiver inside it would borrow the shell twice. This is the "copy
     * the scalar out before the walk" technique the command table already
     * needed for built-in location, with a pointer in place of a flag --
     * and the value is the same one `printentry` read for itself, since
     * nothing in the loop assigns to `PATH`. */
    let path = crate::variables::path_value(shell);
    let lines: Vec<Vec<u8>> = shell
        .commands
        .iter()
        .filter_map(|(name, command_entry)| {
            let name = name.as_slice().as_bstr();
            let resolved = entry_path(command_entry, name, path.as_slice().as_bstr())?;
            Some(format_hash_entry(
                name,
                resolved.as_slice().as_bstr(),
                command_entry,
                bash,
                request.portable,
            ))
        })
        .collect();
    if bash && !request.portable {
        if lines.is_empty() {
            shell.write_output(OutputDestination::Stdout, b"hash: hash table empty\n")?;
            return Ok(Flow::Done((0).into()));
        }
        shell.write_output(OutputDestination::Stdout, b"hits\tcommand\n")?;
    }
    for line in lines {
        shell.write_output(OutputDestination::Stdout, &line)?;
    }
    Ok(Flow::Done((0).into()))
}

// [spec:dash:sem:exec.printentry-fn]
/// One listing line, in whichever of the three shapes was asked for.
///
/// dash's is the path with a `*` when the entry is due a rehash. The
/// reference's plain listing is the consultation count in a four-column
/// field and the path, and its `-l` form is a command that re-enters the
/// entry -- with `builtin` in front, so that it still means this built-in
/// where a function has taken the name.
fn format_hash_entry(
    name: &BStr,
    resolved: &BStr,
    entry: &CommandEntry,
    bash: bool,
    portable: bool,
) -> Vec<u8> {
    if !bash {
        let mut line = resolved.to_vec();
        line.extend_from_slice(if entry.rehash { b"*\n" } else { b"\n" });
        return line;
    }
    if portable {
        let mut line = b"builtin hash -p ".to_vec();
        line.extend_from_slice(resolved);
        line.push(b' ');
        line.extend_from_slice(name);
        line.push(b'\n');
        return line;
    }
    let mut line = format!("{:4}\t", entry.hits).into_bytes();
    line.extend_from_slice(resolved);
    line.push(b'\n');
    line
}
