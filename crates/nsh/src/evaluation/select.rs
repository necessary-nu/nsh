//! `select name in words; do list; done`.
//!
//! Bash's menu loop. The syntax is `for`'s exactly -- so is the node --
//! and everything that makes it a different command is here: the numbered
//! list, the `PS3` prompt, the read, and what the reply does to `name`
//! and `REPLY`.
//!
//! It is a *script* construct rather than an interactive one, which is why
//! it is in scope under [`dec:nsh:bash-compatibility-is-scripts`]: a
//! script containing `select` does not misbehave without it, it fails to
//! parse. The menu and the prompt go to standard error and the reply is
//! read from standard input, so it works the same whether or not either
//! is a terminal.

use bstr::{BStr, BString};

use super::{Flow, LoopStep, catch_one_loop, evaluate_tree, record_command_line};
use crate::context::Shell;
use crate::error::Error;
use crate::expand::{ExpandedFields, ExpansionMode};
use crate::nodes::ForCommand;
use crate::output::OutputDestination;
use crate::status::ExitStatus;
use crate::variables::VariableAttributes;

/// What Bash prompts with when `PS3` is unset.
const DEFAULT_PROMPT: &[u8] = b"#? ";

// [spec:nsh:req:compat.bash.select-time-grammar]
pub(super) fn evaluate_select(
    shell: &mut Shell,
    command: &ForCommand,
    context: super::EvaluationContext,
) -> Result<Flow, Error> {
    record_command_line(shell, command.line.get());
    let context = context.tested_only();

    let mut expanded_fields = ExpandedFields::new();
    for argument in &command.words {
        crate::expand::expand_argument(
            shell,
            argument,
            Some(&mut expanded_fields),
            ExpansionMode::SPLIT | ExpansionMode::TILDE,
        )?;
    }
    let choices: Vec<BString> = expanded_fields
        .fields
        .iter()
        .map(|field| field.as_bstr().to_owned())
        .collect();
    /* Nothing to choose from is not an empty menu, it is no loop at all:
     * Bash runs the body zero times and answers 0. */
    if choices.is_empty() {
        return Ok(Flow::Done(ExitStatus::SUCCESS));
    }

    let mut status = ExitStatus::SUCCESS;
    let mut show_menu = true;
    shell.evaluation.loop_depth += 1;
    let outcome = loop {
        if show_menu {
            write_menu(shell, &choices)?;
        }
        write_prompt(shell)?;
        let Some(reply) = read_reply(shell)? else {
            /* End of input closes the prompt line and ends the loop.
             * Bash answers 1 here whatever the body last did, which is
             * the one place `select`'s status is not the body's.
             *
             * The closing newline goes to standard *output*, where the
             * menu and the prompt went to standard error. That looks like
             * an inconsistency and is Bash's, measured: the prompt is
             * decoration and the newline is `read` finishing a line it
             * did not get. A script that captures the output of a
             * `select` sees the newline and not the menu. */
            // [spec:nsh:req:compat.bash.select-time-grammar]
            shell.write_output(OutputDestination::Stdout, b"\n")?;
            status = ExitStatus::FAILURE;
            break None;
        };
        /* A blank line asks for the menu again and runs nothing. */
        if reply.is_empty() {
            show_menu = true;
            continue;
        }
        show_menu = false;

        crate::variables::set_bytes(
            shell,
            BStr::new(b"REPLY"),
            Some(BStr::new(reply.as_slice())),
            VariableAttributes::NONE,
        )?;
        /* A reply that does not name one of the choices leaves the
         * variable empty and still runs the body -- `REPLY` is how the
         * script sees what was actually typed. */
        let selected = chosen(BStr::new(reply.as_slice()), &choices).unwrap_or(BStr::new(b""));
        crate::variables::set_bytes(
            shell,
            command.variable.as_bstr(),
            Some(selected),
            VariableAttributes::NONE,
        )?;

        match catch_one_loop(evaluate_tree(shell, Some(command.body.as_ref()), context)?) {
            LoopStep::Value(body_status) | LoopStep::Continue(body_status) => status = body_status,
            LoopStep::Break(break_status) => {
                status = break_status;
                break None;
            }
            LoopStep::Propagate(control) => break Some(control),
        }
    };
    shell.evaluation.loop_depth -= 1;
    Ok(outcome.unwrap_or(Flow::Done(status)))
}

/// The choice a reply names, or `None` for anything that is not one.
fn chosen<'a>(reply: &BStr, choices: &'a [BString]) -> Option<&'a BStr> {
    let index: usize = std::str::from_utf8(reply).ok()?.trim().parse().ok()?;
    index
        .checked_sub(1)
        .and_then(|index| choices.get(index))
        .map(|choice| BStr::new(choice.as_slice()))
}

/// `N) choice` per line, on standard error, as Bash writes it.
fn write_menu(shell: &mut Shell, choices: &[BString]) -> Result<(), Error> {
    let mut menu = BString::from(Vec::new());
    for (index, choice) in choices.iter().enumerate() {
        menu.extend_from_slice((index + 1).to_string().as_bytes());
        menu.extend_from_slice(b") ");
        menu.extend_from_slice(choice);
        menu.push(b'\n');
    }
    shell.write_output(OutputDestination::Stderr, &menu)
}

fn write_prompt(shell: &mut Shell) -> Result<(), Error> {
    let prompt = crate::variables::lookup_bytes(shell, BStr::new(b"PS3"))
        .unwrap_or_else(|| BString::from(DEFAULT_PROMPT));
    shell.write_output(OutputDestination::Stderr, &prompt)
}

/// One line of standard input, or `None` at end of input.
///
/// The trailing newline is not part of the reply, and a last line without
/// one is still a reply -- only a read that finds nothing at all ends the
/// loop.
fn read_reply(shell: &mut Shell) -> Result<Option<BString>, Error> {
    use crate::builtins::read::stream::ReadStream;
    use crate::syntax::InputUnit;

    let mut source = ReadStream::open(shell, None)?;
    let mut line = BString::from(Vec::new());
    let mut saw_any = false;
    let outcome = loop {
        match source.next_unit(shell, false) {
            Ok(InputUnit::Byte(b'\n')) => break Ok(Some(std::mem::take(&mut line))),
            Ok(InputUnit::Byte(byte)) => {
                saw_any = true;
                line.push(byte);
            }
            /* An alias boundary is not input, and `select` reads from a
             * descriptor rather than from the parser's stack, so it can
             * only arrive from a shared code path. Skip it. */
            Ok(InputUnit::EndOfAlias) => continue,
            Ok(InputUnit::EndOfInput) => {
                break Ok(saw_any.then(|| std::mem::take(&mut line)));
            }
            Err(error) => break Err(error),
        }
    };
    source.close(shell);
    outcome
}
