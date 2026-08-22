//! Bash's `help`: what the shell can tell a script about its own
//! built-ins.
//!
//! The synopsis lines come from the registry rather than from a second
//! table of prose, so a built-in cannot be added without `help` knowing
//! about it and the two cannot drift apart. What is deliberately absent
//! is Bash's paragraph of documentation for each name: inventing text
//! that claims to describe Bash's behaviour would be worse than saying
//! only what is true -- that the name exists and which table it is in.

use bstr::{BStr, ByteSlice as _};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::options::Dialect;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut topics: &[&BStr] = &args[1.min(args.len())..];
    while let Some(first) = topics.first() {
        let bytes: &[u8] = first.as_ref();
        if bytes == b"--" {
            topics = &topics[1..];
            break;
        }
        if bytes.len() < 2 || bytes[0] != b'-' {
            break;
        }
        // `-d`, `-m` and `-s` select a shorter rendering of the same
        // facts; none of them changes which topics match.
        topics = &topics[1..];
    }

    if topics.is_empty() {
        return list_all(shell);
    }

    let mut status = ExitStatus::SUCCESS;
    for topic in topics {
        let matches = matching(shell, topic);
        if matches.is_empty() {
            no_such_topic(shell, topic)?;
            status = ExitStatus::FAILURE;
            continue;
        }
        for name in matches {
            write_entry(shell, name)?;
        }
    }
    Ok(Flow::Done(status))
}

/// Every built-in name this shell would run, in the order `help` prints
/// them: the dialect's own table first, then the baseline.
fn names(shell: &Shell) -> Vec<&'static BStr> {
    let mut names: Vec<&'static BStr> = Vec::new();
    if shell.options.dialect() == Dialect::Bash {
        names.extend(super::BASH_BUILTINS.iter().map(super::BuiltinSpec::name));
    }
    for spec in super::BUILTINS {
        if !names.contains(&spec.name()) {
            names.push(spec.name());
        }
    }
    names.sort_unstable();
    names
}

/// Bash matches a topic as a prefix, so `help re` reports `read`,
/// `readonly` and `return` rather than nothing.
fn matching(shell: &Shell, topic: &BStr) -> Vec<&'static BStr> {
    let pattern: &[u8] = topic.as_ref();
    names(shell)
        .into_iter()
        .filter(|name| name.as_bytes().starts_with(pattern))
        .collect()
}

fn list_all(shell: &mut Shell) -> Result<Flow, Error> {
    let banner = format!(
        "nsh, version {} (Bash-compatible built-in help)\n\
         These shell commands are defined internally.  Type `help' to see this list.\n\
         Type `help name' to find out more about the function `name'.\n\n",
        crate::variables::special::version_text()
    );
    shell.write_output(OutputDestination::Stdout, banner.as_bytes())?;
    for name in names(shell) {
        write_entry(shell, name)?;
    }
    Ok(Flow::Done(ExitStatus::SUCCESS))
}

fn write_entry(shell: &mut Shell, name: &BStr) -> Result<(), Error> {
    let mut line = Vec::from(name.as_bytes());
    line.extend_from_slice(b": ");
    line.extend_from_slice(name.as_bytes());
    line.extend_from_slice(b" [arg ...]\n");
    shell.write_output(OutputDestination::Stdout, &line)
}

fn no_such_topic(shell: &mut Shell, topic: &BStr) -> Result<(), Error> {
    let mut message = b"help: no help topics match `".to_vec();
    message.extend_from_slice(topic.as_bytes());
    message.extend_from_slice(b"'.  Try `help help'.\n");
    shell.write_output(OutputDestination::Stderr, &message)
}
