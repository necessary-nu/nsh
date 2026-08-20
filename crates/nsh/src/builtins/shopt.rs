//! Bash's shell-option discovery and mutation builtin.

use std::io::Write as _;

use bstr::{BStr, ByteSlice as _};

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;

#[derive(Clone, Copy, Eq, PartialEq)]
enum Action {
    List,
    Set,
    Unset,
    Print,
    Query,
}

#[derive(Clone, Copy)]
enum Namespace {
    Bash,
    Shell,
}

struct Selection<'a> {
    action: Action,
    namespace: Namespace,
    operands: &'a [&'a BStr],
}

enum Scan<'a> {
    Selection(Selection<'a>),
    Help,
    Invalid(Vec<u8>),
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch]
pub fn shoptcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let selection = match scan(args) {
        Scan::Selection(selection) => selection,
        Scan::Help => {
            let _ = sh
                .io
                .stdout()
                .write_all(b"shopt: shopt [-pqsu] [-o] [optname ...]\n");
            return Ok(Flow::Done((0).into()));
        }
        Scan::Invalid(option) => {
            invalid_option(sh, &option);
            return Ok(Flow::Done((2).into()));
        }
    };

    if matches!(selection.action, Action::Set | Action::Unset) && !selection.operands.is_empty() {
        return mutate(sh, &selection);
    }
    report(sh, &selection)
}

fn scan<'a>(args: &'a [&'a BStr]) -> Scan<'a> {
    let mut action = Action::List;
    let mut namespace = Namespace::Bash;
    let mut next = 1;
    while let Some(word) = args.get(next) {
        if word.as_bytes() == b"--" {
            next += 1;
            break;
        }
        if word.as_bytes() == b"--help" {
            return Scan::Help;
        }
        if let Some(long) = word.as_bytes().strip_prefix(b"--") {
            action = match long {
                b"set" => Action::Set,
                b"unset" => Action::Unset,
                b"print" => Action::Print,
                b"quiet" => Action::Query,
                _ => return Scan::Invalid(word.to_vec()),
            };
            next += 1;
            continue;
        }
        let bytes = word.as_bytes();
        if bytes.len() < 2 || bytes[0] != b'-' {
            break;
        }
        for &option in &bytes[1..] {
            match option {
                b'o' => namespace = Namespace::Shell,
                b'p' => action = Action::Print,
                b'q' => action = Action::Query,
                b's' => action = Action::Set,
                b'u' => action = Action::Unset,
                _ => return Scan::Invalid(vec![b'-', option]),
            }
        }
        next += 1;
    }
    Scan::Selection(Selection {
        action,
        namespace,
        operands: &args[next..],
    })
}

fn mutate(sh: &mut Shell, selection: &Selection<'_>) -> Result<Flow, Error> {
    let on = selection.action == Action::Set;
    let mut changed = false;
    let mut invalid = false;
    for name in selection.operands {
        if state(sh, selection.namespace, name).is_none() {
            invalid_name(sh, name);
            invalid = true;
            continue;
        }
        match selection.namespace {
            Namespace::Bash => {
                let known = sh.options.set_bash_option(name, on);
                debug_assert!(known);
            }
            Namespace::Shell => crate::options::set_option_by_name(sh, name, on)?,
        }
        changed = true;
    }
    if changed {
        crate::options::options_changed(sh)?;
    }
    Ok(Flow::Done((i32::from(invalid)).into()))
}

fn report(sh: &mut Shell, selection: &Selection<'_>) -> Result<Flow, Error> {
    let names = if selection.operands.is_empty() {
        names(sh, selection.namespace)
    } else {
        selection.operands.to_vec()
    };
    let mut all_on = true;
    let mut invalid = false;
    for name in names {
        let Some(on) = state(sh, selection.namespace, name) else {
            invalid_name(sh, name);
            invalid = true;
            continue;
        };
        all_on &= on;
        if selection.action != Action::Query && selected(selection.action, on) {
            write_state(sh, selection.namespace, selection.action, name, on);
        }
    }
    let named_status = !selection.operands.is_empty() && !all_on;
    Ok(Flow::Done((i32::from(invalid || named_status)).into()))
}

fn names(sh: &Shell, namespace: Namespace) -> Vec<&'static BStr> {
    match namespace {
        Namespace::Bash => crate::options::BASH_OPTION_NAMES
            .iter()
            .map(|name| BStr::new(*name))
            .collect(),
        Namespace::Shell => sh
            .options
            .shell_options()
            .map(|(name, _)| BStr::new(name))
            .collect(),
    }
}

fn state(sh: &Shell, namespace: Namespace, name: &BStr) -> Option<bool> {
    match namespace {
        Namespace::Bash => sh.options.bash_option(name),
        Namespace::Shell => sh.options.shell_option(name),
    }
}

fn selected(action: Action, on: bool) -> bool {
    match action {
        Action::Set => on,
        Action::Unset => !on,
        Action::List | Action::Print => true,
        Action::Query => false,
    }
}

fn write_state(sh: &mut Shell, namespace: Namespace, action: Action, name: &BStr, on: bool) {
    let mut line = Vec::new();
    if action == Action::Print {
        match namespace {
            Namespace::Bash => line.extend_from_slice(if on { b"shopt -s " } else { b"shopt -u " }),
            Namespace::Shell => line.extend_from_slice(if on { b"set -o " } else { b"set +o " }),
        }
        line.extend_from_slice(name);
    } else {
        line.extend_from_slice(name);
        if name.len() < 20 {
            line.resize(20, b' ');
        }
        line.push(b'\t');
        line.extend_from_slice(if on { b"on" } else { b"off" });
    }
    line.push(b'\n');
    let _ = sh.io.stdout().write_all(&line);
}

fn invalid_option(sh: &mut Shell, option: &[u8]) {
    let mut message = b"shopt: ".to_vec();
    message.extend_from_slice(option);
    message.extend_from_slice(b": invalid option\n");
    let _ = sh.io.stderr().write_all(&message);
}

fn invalid_name(sh: &mut Shell, name: &BStr) {
    let mut message = b"shopt: ".to_vec();
    message.extend_from_slice(name);
    message.extend_from_slice(b": invalid shell option name\n");
    let _ = sh.io.stderr().write_all(&message);
}
