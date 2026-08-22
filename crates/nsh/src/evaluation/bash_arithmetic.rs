//! Execution of Bash's `(( ... ))` command and `for (( ... ))` loop.
//!
//! Both spell their expressions as source text that has not been expanded
//! yet, because `(( A[$key] += $2 ))` needs parameter expansion and command
//! substitution *before* it is arithmetic. Evaluating one is therefore the
//! same operation as expanding `$(( ... ))`: the shared path is the point,
//! not an implementation shortcut, since a second arithmetic entry would be
//! a second set of answers.

use bstr::{BStr, BString, ByteSlice as _};

use super::{
    EvaluationContext, Flow, LoopStep, catch_one_loop, evaluate_tree, flow, record_command_line,
    repeat_debug_trap,
};
use crate::context::Shell;
use crate::error::Error;
use crate::nodes::{BashArithmeticCommand, BashArithmeticFor};
use crate::status::ExitStatus;

/// Run `(( expression ))`: true when the value is non-zero.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(crate) fn command(
    shell: &mut Shell,
    node: &BashArithmeticCommand,
) -> Result<ExitStatus, Error> {
    Ok(match value(shell, node.expression.as_bstr())? {
        Some(0) | None => ExitStatus::FAILURE,
        Some(_) => ExitStatus::SUCCESS,
    })
}

/// Run `for (( init; test; update )); do ... done`.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(crate) fn for_loop(
    shell: &mut Shell,
    node: &BashArithmeticFor,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    record_command_line(shell, node.line);
    let context = context.tested_only();
    /* Bash raises `DEBUG` for each of the three expressions, so the
     * header's line is re-recorded before each one: the body between
     * them has moved `$LINENO` on. */
    // [spec:nsh:req:compat.bash.traps-introspection]
    flow!(repeat_debug_trap(shell, node.line));
    if value(shell, node.init.as_bstr())?.is_none() {
        return Ok(Flow::Done(ExitStatus::ERROR));
    }

    let mut status = ExitStatus::SUCCESS;
    shell.evaluation.loop_depth += 1;
    let outcome = (|| {
        loop {
            flow!(repeat_debug_trap(shell, node.line));
            match condition(shell, node.test.as_bstr())? {
                None => return Ok(Flow::Done(ExitStatus::ERROR)),
                Some(false) => return Ok(Flow::Done(status)),
                Some(true) => {}
            }
            match catch_one_loop(evaluate_tree(shell, Some(node.body.as_ref()), context)?) {
                LoopStep::Value(body_status) => status = body_status,
                LoopStep::Break(break_status) => return Ok(Flow::Done(break_status)),
                LoopStep::Continue(next_status) => status = next_status,
                LoopStep::Propagate(control) => return Ok(control),
            }
            flow!(repeat_debug_trap(shell, node.line));
            if value(shell, node.update.as_bstr())?.is_none() {
                return Ok(Flow::Done(ExitStatus::ERROR));
            }
        }
    })();
    shell.evaluation.loop_depth -= 1;
    outcome
}

/// An omitted loop condition is true, which is what `for (( ; ; ))` means.
fn condition(shell: &mut Shell, text: &BStr) -> Result<Option<bool>, Error> {
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(Some(true));
    }
    Ok(value(shell, text)?.map(|value| value != 0))
}

/// Expand and evaluate one expression, or say that it failed.
///
/// `None` is a failure that has already been reported: Bash keeps running
/// after an arithmetic error in these commands and only the status records
/// it, which is exactly what prompt-style expansion does with a diagnostic.
/// A successful arithmetic expansion always renders a decimal integer, so
/// anything else is that reported failure coming back.
fn value(shell: &mut Shell, text: &BStr) -> Result<Option<i64>, Error> {
    let mut source = BString::from(&b"$(( "[..]);
    source.extend_from_slice(text);
    source.extend_from_slice(b" ))");
    let rendered = crate::parser::expand_string(shell, source.as_bstr())?;
    Ok(core::str::from_utf8(&rendered)
        .ok()
        .and_then(|rendered| rendered.parse::<i64>().ok()))
}
