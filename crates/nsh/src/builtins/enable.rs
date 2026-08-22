//! Bash's `enable`: which built-ins the command lookup is allowed to
//! find.
//!
//! A disabled name is not removed from the registry -- the table is
//! `'static` and shared by every shell in the process, so it could not
//! be -- but recorded per shell in [`DisabledBuiltins`], which
//! `execution::builtin` consults. That is also what makes `enable -n`
//! reversible and confines it to one shell instance
//! ([dec:nsh:no-ambient-state]).
//!
//! Loading a built-in from a shared object (`enable -f`) is refused
//! rather than implemented. It is an ambient data-to-code path -- a
//! script naming a file the shell then executes inside itself -- which
//! is exactly what `[dec:nsh:safety-trumps-compatibility]` says not to
//! import.

use std::collections::BTreeSet;

use bstr::{BStr, BString, ByteSlice as _};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::options::Dialect;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

/// The built-in names one shell has switched off.
///
/// Empty for every shell that never runs `enable -n`, which is what
/// keeps the lookup's fast path free of a second table probe.
#[derive(Default)]
pub struct DisabledBuiltins {
    names: BTreeSet<BString>,
}

impl DisabledBuiltins {
    pub(crate) const fn new() -> Self {
        Self {
            names: BTreeSet::new(),
        }
    }

    pub(crate) fn contains(&self, name: &BStr) -> bool {
        !self.names.is_empty() && self.names.contains(name)
    }
}

/// What one invocation asks for.
#[derive(Clone, Copy, Default)]
struct Requested {
    disable: bool,
    all: bool,
    print: bool,
    special_only: bool,
    load: bool,
}

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let (requested, names) = match parse(args) {
        Ok(parsed) => parsed,
        Err(letter) => {
            let mut message = b"enable: -".to_vec();
            message.push(letter);
            message.extend_from_slice(b": invalid option\n");
            shell.write_output(OutputDestination::Stderr, &message)?;
            return Ok(Flow::Done(ExitStatus::ERROR));
        }
    };
    if requested.load {
        return Err(shell
            .diagnostics()
            .shell_error(b"enable: -f: loading built-ins from a file is not supported"));
    }
    if names.is_empty() {
        return report(shell, requested);
    }

    let mut status = ExitStatus::SUCCESS;
    for name in names {
        if lookup(shell, name).is_none() {
            let mut message = b"enable: ".to_vec();
            message.extend_from_slice(name.as_bytes());
            message.extend_from_slice(b": not a shell builtin\n");
            shell.write_output(OutputDestination::Stderr, &message)?;
            status = ExitStatus::FAILURE;
            continue;
        }
        if requested.print {
            write_state(shell, name)?;
            continue;
        }
        if requested.disable {
            shell
                .disabled_builtins
                .names
                .insert(BString::from(name.as_bytes()));
        } else {
            shell
                .disabled_builtins
                .names
                .remove(BStr::new(name.as_bytes()));
        }
    }
    Ok(Flow::Done(status))
}

fn parse<'a>(args: &'a [&'a BStr]) -> Result<(Requested, &'a [&'a BStr]), u8> {
    let mut requested = Requested::default();
    let mut at = 1;
    while at < args.len() {
        let word: &[u8] = args[at].as_ref();
        let Some(letters) = word.strip_prefix(b"-").filter(|rest| !rest.is_empty()) else {
            break;
        };
        if letters == b"-" {
            at += 1;
            break;
        }
        for letter in letters {
            match letter {
                b'n' => requested.disable = true,
                b'a' => requested.all = true,
                b'p' => requested.print = true,
                b's' => requested.special_only = true,
                b'f' | b'd' => requested.load = true,
                other => return Err(*other),
            }
        }
        at += 1;
    }
    Ok((requested, &args[at..]))
}

/// Whether the registry this shell searches holds `name` at all.
fn lookup(shell: &Shell, name: &BStr) -> Option<&'static super::BuiltinSpec> {
    let bash = (shell.options.dialect() == Dialect::Bash)
        .then(|| {
            super::BASH_BUILTINS
                .binary_search_by(|spec| spec.name().cmp(name))
                .ok()
                .map(|index| &super::BASH_BUILTINS[index])
        })
        .flatten();
    bash.or_else(|| {
        super::BUILTINS
            .binary_search_by(|spec| spec.name().cmp(name))
            .ok()
            .map(|index| &super::BUILTINS[index])
    })
}

fn report(shell: &mut Shell, requested: Requested) -> Result<Flow, Error> {
    let mut names: Vec<(&'static BStr, bool)> = Vec::new();
    if shell.options.dialect() == Dialect::Bash {
        for spec in super::BASH_BUILTINS {
            names.push((spec.name(), spec.attributes().is_special()));
        }
    }
    for spec in super::BUILTINS {
        if !names.iter().any(|(name, _)| *name == spec.name()) {
            names.push((spec.name(), spec.attributes().is_special()));
        }
    }
    names.sort_unstable();
    for (name, special) in names {
        if requested.special_only && !special {
            continue;
        }
        let disabled = shell.disabled_builtins.contains(name);
        if !requested.all && disabled != requested.disable {
            continue;
        }
        write_state(shell, name)?;
    }
    Ok(Flow::Done(ExitStatus::SUCCESS))
}

fn write_state(shell: &mut Shell, name: &BStr) -> Result<(), Error> {
    let disabled = shell.disabled_builtins.contains(name);
    let mut line = b"enable ".to_vec();
    if disabled {
        line.extend_from_slice(b"-n ");
    }
    line.extend_from_slice(name.as_bytes());
    line.push(b'\n');
    shell.write_output(OutputDestination::Stdout, &line)
}
