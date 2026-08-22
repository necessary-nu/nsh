//! Bash's `builtin`: run a built-in under a name a function has taken.
//!
//! `command` solves the neighbouring problem in the evaluator, because
//! it has to *skip* functions while still reaching the file system.
//! `builtin` never leaves the registry, so it does not need the
//! evaluator's lookup at all: it resolves the name in the same table
//! `execution::builtin` searches and calls the entry point. A name that
//! is not a built-in is an error rather than a `PATH` search, which is
//! the whole difference from `command`.

use bstr::{BStr, ByteSlice as _};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::{EvaluationContext, Flow};
use crate::expand::ExpandedField;
use crate::options::Dialect;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

use super::{BuiltinHandler, BuiltinSpec};

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let Some((name, rest)) = args[1.min(args.len())..].split_first() else {
        return Ok(Flow::Done(ExitStatus::SUCCESS));
    };
    let Some(spec) = resolve(shell, name) else {
        let mut message = b"builtin: ".to_vec();
        message.extend_from_slice(name.as_bytes());
        message.extend_from_slice(b": not a shell builtin\n");
        shell.write_output(OutputDestination::Stderr, &message)?;
        return Ok(Flow::Done(ExitStatus::NOT_FOUND));
    };

    let mut argv: Vec<&BStr> = Vec::with_capacity(rest.len() + 1);
    argv.push(*name);
    argv.extend_from_slice(rest);
    match spec.handler() {
        BuiltinHandler::Standard(entry) => entry(shell, &argv),
        BuiltinHandler::Eval => {
            super::eval::evaluate_arguments(shell, &argv, EvaluationContext::DEFAULT)
        }
        BuiltinHandler::History => {
            let mut fields: Vec<ExpandedField> = argv
                .iter()
                .map(|word| ExpandedField::from_bytes(word.as_bytes()))
                .collect();
            super::fc::run_fields(shell, &mut fields)
        }
    }
}

/// The same two-table search `execution::builtin` performs, without the
/// command cache: `builtin` names a built-in directly and a cached
/// external or function resolution for the same name is beside the point.
fn resolve(shell: &Shell, name: &BStr) -> Option<&'static BuiltinSpec> {
    if shell.options.dialect() == Dialect::Bash
        && let Ok(index) = super::BASH_BUILTINS.binary_search_by(|spec| spec.name().cmp(name))
    {
        return Some(&super::BASH_BUILTINS[index]);
    }
    super::BUILTINS
        .binary_search_by(|spec| spec.name().cmp(name))
        .ok()
        .map(|index| &super::BUILTINS[index])
}
