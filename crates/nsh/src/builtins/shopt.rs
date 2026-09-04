//! Bash's shell-option discovery and mutation builtin.

use bstr::{BStr, ByteSlice as _};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::output::OutputDestination;

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
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let selection = match scan(args) {
        Scan::Selection(selection) => selection,
        Scan::Help => {
            shell.write_output(
                OutputDestination::Stdout,
                b"shopt: shopt [-pqsu] [-o] [optname ...]\n",
            )?;
            return Ok(Flow::Done((0).into()));
        }
        Scan::Invalid(option) => {
            invalid_option(shell, &option)?;
            return Ok(Flow::Done((2).into()));
        }
    };

    if matches!(selection.action, Action::Set | Action::Unset) && !selection.operands.is_empty() {
        return mutate(shell, &selection);
    }
    report(shell, &selection)
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

fn mutate(shell: &mut Shell, selection: &Selection<'_>) -> Result<Flow, Error> {
    let on = selection.action == Action::Set;
    let mut changed = false;
    let mut invalid = false;
    for name in selection.operands {
        if state(shell, selection.namespace, name).is_none() {
            invalid_name(shell, name)?;
            invalid = true;
            continue;
        }
        match selection.namespace {
            Namespace::Bash => {
                let traced = shell.options.shopt(crate::options::BashShopt::ExtDebug);
                let known = shell.options.set_bash_option(name, on);
                debug_assert!(known);
                /* Turning `extdebug` on is what makes a call record its
                 * arguments, and the reference installs `BASH_ARGV`'s
                 * bottom frame at that moment rather than waiting for
                 * the first read. Measured on the pinned 5.3.15:
                 * `shopt -s extdebug; set -- x y; declare -p BASH_ARGC`
                 * answers with the parameters the shell *started* with,
                 * so the install had already happened; the same two
                 * lines without the `shopt` answer `x y`. Turning it off
                 * does not install, and neither does any other option. */
                // [spec:nsh:req:compat.bash.names.call-stack]
                if !traced && shell.options.shopt(crate::options::BashShopt::ExtDebug) {
                    crate::variables::special::install_call_arguments(shell);
                }
            }
            Namespace::Shell => crate::options::set_option_by_name(shell, name, on)?,
        }
        changed = true;
    }
    if changed {
        crate::options::options_changed(shell)?;
    }
    Ok(Flow::Done((i32::from(invalid)).into()))
}

fn report(shell: &mut Shell, selection: &Selection<'_>) -> Result<Flow, Error> {
    let names = if selection.operands.is_empty() {
        names(shell, selection.namespace)
    } else {
        selection.operands.to_vec()
    };
    let mut all_on = true;
    let mut invalid = false;
    for name in names {
        let Some(on) = state(shell, selection.namespace, name) else {
            invalid_name(shell, name)?;
            invalid = true;
            continue;
        };
        all_on &= on;
        if selection.action != Action::Query && selected(selection.action, on) {
            write_state(shell, selection.namespace, selection.action, name, on)?;
        }
    }
    let named_status = !selection.operands.is_empty() && !all_on;
    Ok(Flow::Done((i32::from(invalid || named_status)).into()))
}

fn names(shell: &Shell, namespace: Namespace) -> Vec<&'static BStr> {
    match namespace {
        Namespace::Bash => crate::options::BASH_OPTION_NAMES
            .iter()
            .map(|name| BStr::new(*name))
            .collect(),
        Namespace::Shell => shell
            .options
            .shell_options()
            .map(|(name, _)| BStr::new(name))
            .collect(),
    }
}

fn state(shell: &Shell, namespace: Namespace, name: &BStr) -> Option<bool> {
    match namespace {
        Namespace::Bash => shell.options.bash_option(name),
        Namespace::Shell => shell.options.shell_option(name),
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

fn write_state(
    shell: &mut Shell,
    namespace: Namespace,
    action: Action,
    name: &BStr,
    on: bool,
) -> Result<(), Error> {
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
    shell.write_output(OutputDestination::Stdout, &line)
}

fn invalid_option(shell: &mut Shell, option: &[u8]) -> Result<(), Error> {
    let mut message = b"shopt: ".to_vec();
    message.extend_from_slice(option);
    message.extend_from_slice(b": invalid option\n");
    shell.write_output(OutputDestination::Stderr, &message)
}

fn invalid_name(shell: &mut Shell, name: &BStr) -> Result<(), Error> {
    let mut message = b"shopt: ".to_vec();
    message.extend_from_slice(name);
    message.extend_from_slice(b": invalid shell option name\n");
    shell.write_output(OutputDestination::Stderr, &message)
}
