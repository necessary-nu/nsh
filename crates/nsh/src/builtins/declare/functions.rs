//! `declare -f` and `declare -F`, and the `typeset` spellings of both.
//!
//! The two options answer different questions about the same table:
//! `-f` prints a definition's source, `-F` prints only the name it is
//! filed under -- plus, under `shopt -s extdebug`, the line and file it
//! was read from. Neither is a declaration, so both leave the variable
//! half of the built-in alone: `declare -f x` must never create `x`.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::execution::Command;
use crate::nodes::FunctionDefinition;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

/// The function half of `declare`, once the options have been read.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(super) fn run(shell: &mut Shell, names_only: bool, operands: &[&BStr]) -> Result<Flow, Error> {
    let mut listing = BString::new(Vec::new());
    let status = if operands.is_empty() {
        list_all(shell, names_only, &mut listing);
        ExitStatus::SUCCESS
    } else {
        select(shell, names_only, operands, &mut listing)
    };
    if !listing.is_empty() {
        shell.write_output(OutputDestination::Stdout, &listing)?;
    }
    Ok(Flow::Done(status))
}

/// The source `type` prints under `name is a function`.
pub(crate) fn source(shell: &Shell, name: &BStr) -> Option<BString> {
    let definition = definition(shell, name)?;
    Some(crate::nodes::source::function_definition(
        &shell.locale,
        name,
        &definition.body,
    ))
}

fn definition<'a>(shell: &'a Shell, name: &BStr) -> Option<&'a FunctionDefinition> {
    match shell.commands.get(name).map(|entry| &entry.command) {
        Some(Command::Function(definition)) => Some(definition),
        _ => None,
    }
}

/// Every defined function, in the name order the table already keeps.
fn list_all(shell: &Shell, names_only: bool, listing: &mut BString) {
    let functions: Vec<(BString, &FunctionDefinition)> = shell
        .commands
        .iter()
        .filter_map(|(name, entry)| match &entry.command {
            Command::Function(definition) => Some((name.clone(), definition)),
            _ => None,
        })
        .collect();
    for (name, definition) in functions {
        let name = BStr::new(name.as_slice());
        if names_only {
            // A bare `-F` prints the declaration that would recreate the
            // name, which is what Bash lists even with `extdebug` on.
            listing.extend_from_slice(b"declare -f ");
            listing.extend_from_slice(name);
        } else {
            listing.extend_from_slice(&crate::nodes::source::function_definition(
                &shell.locale,
                name,
                &definition.body,
            ));
        }
        listing.push(b'\n');
    }
}

/// The named functions, reporting failure for a name that has none.
fn select(
    shell: &Shell,
    names_only: bool,
    operands: &[&BStr],
    listing: &mut BString,
) -> ExitStatus {
    let mut status = ExitStatus::SUCCESS;
    for name in operands {
        let Some(definition) = definition(shell, name) else {
            status = ExitStatus::FAILURE;
            continue;
        };
        if names_only {
            listing.extend_from_slice(name);
            if shell.options.shopt(crate::options::BashShopt::ExtDebug) {
                listing.push(b' ');
                listing.extend_from_slice(definition.line.get().to_string().as_bytes());
                listing.push(b' ');
                listing.extend_from_slice(&crate::variables::call_stack::definition_source(
                    shell, name,
                ));
            }
        } else {
            listing.extend_from_slice(&crate::nodes::source::function_definition(
                &shell.locale,
                name,
                &definition.body,
            ));
        }
        listing.push(b'\n');
    }
    status
}
