//! `while`, `until`, `for`, `select` and `case`.
//!
//! The forms that run a list more than once, or choose which list to run.
//! What they share is the accounting `[spec:posix:req:cmd.while-exit-status]`
//! asks for -- a loop whose body never ran is zero, not the status of
//! whatever it last tested -- and the two flows that leave one early.

use super::*;

// [spec:dash:sem:eval.evalloop-fn]
// [spec:posix:req:cmd.while-execution]
// [spec:posix:req:cmd.while-exit-status]
// [spec:posix:req:cmd.until-execution]
// [spec:posix:req:cmd.until-exit-status]
pub(super) fn evaluate_loop(
    shell: &mut Shell,
    command: &BinaryCommand,
    until: bool,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let context = context.tested_only();

    shell.evaluation.loop_depth += 1;
    let outcome = (|| {
        let mut status = ExitStatus::SUCCESS;
        loop {
            let mut condition = match catch_one_loop(evaluate_tree(
                shell,
                Some(command.left.as_ref()),
                EvaluationContext::TESTED,
            )?) {
                LoopStep::Value(status) => status,
                LoopStep::Break(status) => return Ok(Flow::Done(status)),
                LoopStep::Continue(next_status) => {
                    status = next_status;
                    continue;
                }
                LoopStep::Propagate(control) => return Ok(control),
            };
            if until {
                condition = if condition.success() {
                    ExitStatus::FAILURE
                } else {
                    ExitStatus::SUCCESS
                };
            }
            if !condition.success() {
                return Ok(Flow::Done(status));
            }
            match catch_one_loop(evaluate_tree(shell, Some(command.right.as_ref()), context)?) {
                LoopStep::Value(body_status) => status = body_status,
                LoopStep::Break(break_status) => return Ok(Flow::Done(break_status)),
                LoopStep::Continue(next_status) => status = next_status,
                LoopStep::Propagate(control) => return Ok(control),
            }
        }
    })();
    shell.evaluation.loop_depth -= 1;
    outcome
}

// [spec:dash:sem:eval.evalfor-fn]
// [spec:posix:req:cmd.for-iteration]
// [spec:posix:req:cmd.for-omitted-in]
// [spec:posix:req:cmd.for-exit-status]
pub(super) fn evaluate_for(
    shell: &mut Shell,
    command: &ForCommand,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let mut expanded_fields = ExpandedFields::new();
    let mut status: ExitStatus;
    let context = context.tested_only();

    record_command_line(shell, command.line.get());

    for argument in &command.words {
        crate::expand::expand_argument(
            shell,
            argument,
            Some(&mut expanded_fields),
            ExpansionMode::SPLIT | ExpansionMode::TILDE,
        )?;
    }

    status = ExitStatus::SUCCESS;
    shell.evaluation.loop_depth += 1;
    for field in &expanded_fields.fields {
        /* Bash raises `DEBUG` once per iteration for a `for` command,
         * not once for the whole loop. */
        match repeat_debug_trap(shell, command.line.get())? {
            Flow::Done(_) => {}
            control => {
                shell.evaluation.loop_depth -= 1;
                return Ok(control);
            }
        }
        crate::variables::set_bytes(
            shell,
            command.variable.as_bstr(),
            Some(field.as_bstr()),
            VariableAttributes::NONE,
        )?;
        match catch_one_loop(evaluate_tree(shell, Some(command.body.as_ref()), context)?) {
            LoopStep::Value(body_status) => status = body_status,
            LoopStep::Break(break_status) => {
                status = break_status;
                break;
            }
            LoopStep::Continue(next_status) => status = next_status,
            LoopStep::Propagate(control) => {
                shell.evaluation.loop_depth -= 1;
                return Ok(control);
            }
        }
    }
    shell.evaluation.loop_depth -= 1;

    Ok(Flow::Done(status))
}

// [spec:dash:sem:eval.evalcase-fn]
// [spec:posix:req:cmd.case-selection]
// [spec:posix:req:cmd.case-pattern-expansion]
// [spec:posix:req:cmd.case-multiple-pattern-order-unspecified]
// [spec:posix:req:cmd.case-exit-status]
// [spec:posix:req:cmd.case-clause-terminators]
pub(super) fn evaluate_case(
    shell: &mut Shell,
    command: &CaseCommand,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    let mut expanded_fields = ExpandedFields::new();
    let mut status = ExitStatus::SUCCESS;
    let mut fallthrough = false;

    record_command_line(shell, command.line.get());
    // [spec:nsh:req:compat.bash.traps-introspection]
    flow!(crate::trap::bash::run_debug(shell));

    crate::expand::expand_argument(
        shell,
        command.word.as_ref(),
        Some(&mut expanded_fields),
        ExpansionMode::TILDE | ExpansionMode::PRESERVE_MULTIBYTE,
    )?;
    /* The C reads `arglist.list->text` with no null check, and is right to:
     * `expandarg` without EXP_FULL takes its single-field arm, which appends
     * exactly one entry whatever the word expands to. */
    debug_assert_eq!(
        expanded_fields.fields.len(),
        1,
        "an unsplit expansion is one field"
    );
    'case_done: {
        for clause in &command.clauses {
            let mut selected = fallthrough;
            if !selected {
                for pattern in &clause.patterns {
                    if crate::expand::case_pattern_matches(
                        shell,
                        pattern,
                        expanded_fields.fields[0].as_bstr(),
                    )? {
                        selected = true;
                        break;
                    }
                }
            }
            if !selected {
                continue;
            }
            /* Ensure body is non-empty as otherwise EV_EXIT may prevent us
             * from setting the exit status. */
            if clause.body.is_some() {
                status = flow!(evaluate_tree(shell, clause.body.as_deref(), context));
            }
            if clause.fallthrough {
                fallthrough = true;
            } else {
                break 'case_done;
            }
        }
    }
    // out:
    Ok(Flow::Done(status))
}
