//! From a `SimpleCommand` node to it having run.
//!
//! Expanding the words, opening the redirections, deciding whether the
//! name is a built-in, a function or a file, running it, and putting the
//! shell back the way it was. `evaluate_command_in_scope` is 545 lines of
//! that on its own, which is the reason this file exists: it is one
//! subject, and it was in the middle of everything else.

use super::*;

// [spec:dash:sem:eval.fill-arglist-fn]
//
// The C's `argpp` is a `union node **` cursor walking `narg.next`; the
// argument list is a slice now, so the cursor is the unconsumed tail of it.
// The return value is the C's `*lastp`: the first entry this call appended,
// or NULL if the argument list ran out without producing one. As an index it
// is the length the list had on entry, so the answer is `Some` exactly when
// the list grew.
fn append_expanded_arguments<'a>(
    shell: &mut Shell,
    expanded_fields: &mut ExpandedFields,
    remaining_argument_nodes: &mut &'a [Node],
    held: &mut Vec<bash_arrays::Declaration<'a>>,
) -> Result<Option<usize>, Error> {
    let initial_field_count = expanded_fields.fields.len();

    while let Some((argument, rest)) = remaining_argument_nodes.split_first() {
        bash_arrays::expand_command_argument(shell, argument, expanded_fields, false, held)?;
        *remaining_argument_nodes = rest;
        if expanded_fields.fields.len() != initial_field_count {
            break;
        }
    }

    if expanded_fields.fields.len() != initial_field_count {
        Ok(Some(initial_field_count))
    } else {
        Ok(None)
    }
}

// [spec:dash:sem:eval.parse-command-args-fn]
// [spec:posix:req:builtin.command.suppress-function-lookup]
// [spec:posix:req:builtin.command.special-builtin-properties-suppressed]
// [spec:posix:req:builtin.command.equivalent-to-omitting-command]
// [spec:posix:req:builtin.command.declaration-utility]
// [spec:posix:def:builtin.command.operands]
// [spec:posix:req:builtin.command.exit-status-invocation]
// [spec:posix:req:param.ps4]
// `head` is the C's `arglist->list`, which this function reassigns to skip
// the `command [-p]` words it consumed. A `Vec`'s start does not move, so the
// head is an index the caller keeps; see [`crate::expand::arglist`].
fn parse_command_args<'a>(
    shell: &mut Shell,
    expanded_fields: &mut ExpandedFields,
    remaining_argument_nodes: &mut &'a [Node],
    path: &mut Option<BString>,
    standard_path: &BStr,
    head: &mut usize,
    held: &mut Vec<bash_arrays::Declaration<'a>>,
) -> Result<Option<CommandSearch>, Error> {
    let mut argument_index = *head;

    loop {
        /* `sp = sp->next ? sp->next : fill_arglist(arglist, argpp)` */
        argument_index = if argument_index + 1 < expanded_fields.fields.len() {
            argument_index + 1
        } else {
            match append_expanded_arguments(shell, expanded_fields, remaining_argument_nodes, held)?
            {
                Some(field_index) => field_index,
                None => return Ok(None),
            }
        };
        let word = expanded_fields.fields[argument_index].as_bstr();
        if word.first() != Some(&b'-') {
            break;
        }
        let options = &word[1..];
        if options.is_empty() {
            break;
        }
        if options == b"-" {
            if argument_index + 1 >= expanded_fields.fields.len()
                && append_expanded_arguments(
                    shell,
                    expanded_fields,
                    remaining_argument_nodes,
                    held,
                )?
                .is_none()
            {
                return Ok(None);
            }
            argument_index += 1;
            break;
        }
        for &option in options.as_bytes() {
            match option {
                b'p' => {
                    *path = Some(standard_path.to_owned());
                }
                _ => {
                    /* run 'typecmd' for other options */
                    return Ok(None);
                }
            }
        }
    }

    *head = argument_index;
    Ok(Some(CommandSearch::DEFAULT.skipping_functions()))
}

/*
 * Execute a simple command.
 */

// [spec:dash:sem:eval.evalcommand-fn]
// [spec:posix:req:builtin.special.error-may-abort-shell]
// [spec:posix:req:builtin.special.preceding-assignments-persist]
// [spec:posix:sem:shell.command-execution]
// [spec:posix:req:grammar.word-expansion-timing]
// [spec:posix:req:grammar.assignment-word-processing]
// [spec:posix:req:shenv.utility-does-not-change-shell-environment]
// [spec:posix:req:cmd.simple-processing-order]
// [spec:posix:req:cmd.simple-command-name-determination]
// [spec:posix:req:cmd.simple-declaration-utility-expansion]
// [spec:posix:req:cmd.simple-argument-expansion]
// [spec:posix:req:cmd.simple-redirections-performed]
// [spec:posix:req:cmd.simple-assignment-expansion]
// [spec:posix:req:cmd.simple-step-order-reversal]
// [spec:posix:req:cmd.declaration-utility-lexical-analysis]
// [spec:posix:req:cmd.assign-no-command-name]
// [spec:posix:req:cmd.assign-exported-to-command]
// [spec:posix:req:cmd.assign-standard-utility-as-function]
// [spec:posix:req:cmd.assign-special-builtin]
// [spec:posix:req:cmd.assign-function]
// [spec:posix:req:cmd.assign-readonly-error]
// [spec:posix:req:cmd.no-name-redirections-subshell]
// [spec:posix:req:cmd.no-name-redirection-failure]
// [spec:posix:req:cmd.no-name-exit-status]
// [spec:nsh:req:idiom.command-dispatch]
pub(super) fn evaluate_command(
    shell: &mut Shell,
    command: &SimpleCommand,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    crate::resource::with_resources(shell, |shell, resources| {
        evaluate_command_in_scope(shell, command, context, resources)
    })
}

fn evaluate_command_in_scope(
    shell: &mut Shell,
    command: &SimpleCommand,
    context: EvaluationContext,
    resources: &mut crate::resource::ResourceScope,
) -> Result<Flow, Error> {
    let mut remaining_arguments: &[Node];
    /* A declaration built-in's `a=(1 2)` operand cannot be expanded into
     * a word: the value is structural, and its kind depends on the
     * attributes the built-in has yet to apply. The name goes into the
     * argument list and the assignment waits here. */
    let mut held_declarations = Vec::new();
    /* Which built-in the held operands are for: `export` refuses a
     * subscripted one where `declare` accepts it. */
    let mut subscripted_operands = bash_arrays::SubscriptedOperand::Accepted;
    shell.evaluation.refused_declarations.clear();
    let mut expanded_fields = ExpandedFields::new();
    let mut assignment_fields = ExpandedFields::new();
    let mut argument_count: usize;
    /* The C's `arglist.list`, which `parse_command_args` moves past the
     * `command [-p]` words while `osp` keeps the original head for `set -x`. */
    let mut head: usize = 0;
    let mut resolved_command = Command::Builtin(&crate::builtins::EMPTY_BUILTIN);
    let job_id: Option<JobId>;
    let mut path: Option<BString> = None;
    let standard_path = crate::variables::default_path();
    let mut special_builtin: Option<bool>;
    let mut command_search: CommandSearch;
    let mut is_exec_builtin: bool;
    let mut status: ExitStatus;
    let mut variable_attributes: VariableAttributes;
    let mut use_local_variables: bool;
    let mut command_control: Option<Flow> = None;

    record_command_line(shell, command.line.get());
    // [spec:nsh:req:compat.bash.traps-introspection]
    flow!(crate::trap::bash::run_debug(shell));
    if command
        .assignments
        .iter()
        .chain(command.arguments.iter())
        .any(|node| {
            matches!(node, Node::Bash(inner)
                if !matches!(inner, crate::nodes::BashNode::ArrayAssignment(_)))
        })
    {
        return Err(shell
            .diagnostics()
            .shell_error(b"Bash syntax is parsed but not executable yet"));
    }

    /* First expand the arguments. */
    shell.evaluation.command_substitution_status = ExitStatus::SUCCESS;

    command_search = CommandSearch::DEFAULT;
    is_exec_builtin = false;
    special_builtin = None;
    variable_attributes = VariableAttributes::NONE;
    use_local_variables = false;
    argument_count = 0;
    remaining_arguments = command.arguments.as_slice();
    let original_fields_start = append_expanded_arguments(
        shell,
        &mut expanded_fields,
        &mut remaining_arguments,
        &mut held_declarations,
    )?;
    if original_fields_start.is_some() {
        let mut assignments_are_arguments = false;

        loop {
            /* `find_command` can run a `%func` PATH file, which is shell
             * code and can `exit`; the C's longjmp took that past this
             * frame and so does this. */
            /* `pathval` and the call both take the shell, so the read is
             * hoisted out of the argument list rather than nested in it.
             * Arguments evaluate left to right and nothing before it here
             * has an effect, so it is read at the same point as before.
             * Do not re-inline it. */
            let active_path = crate::variables::path_value(shell);
            match find_command(
                shell,
                expanded_fields.fields[head].as_bstr(),
                &mut resolved_command,
                command_search.regular_builtins_only(),
                BStr::new(active_path.as_slice()),
            )? {
                Flow::Done(_) => {}
                control => return Ok(control),
            }

            use_local_variables = true;

            /* implement bltin and command here */
            let Command::Builtin(builtin) = &resolved_command else {
                break;
            };
            let builtin = *builtin;

            assignments_are_arguments = builtin.attributes().takes_assignments();
            if special_builtin.is_none() {
                let special = builtin.attributes().is_special();
                special_builtin = Some(special);
                use_local_variables = !special;
            }
            is_exec_builtin = builtin.id() == BuiltinId::Exec;
            /* `export` names variables, not elements: Bash refuses
             * `export a[7]=8` rather than assigning the element. */
            // [spec:nsh:req:compat.bash.arrays-declarations]
            if builtin.id() == BuiltinId::Export {
                subscripted_operands = bash_arrays::SubscriptedOperand::Refused;
            }
            if builtin.id() != BuiltinId::Command {
                break;
            }

            let Some(next_search) = parse_command_args(
                shell,
                &mut expanded_fields,
                &mut remaining_arguments,
                &mut path,
                standard_path.as_slice().as_bstr(),
                &mut head,
                &mut held_declarations,
            )?
            else {
                break;
            };
            command_search = next_search;
        }

        let assignment_operands = assignments_are_arguments
            && crate::parser::bash::declaration_operands(shell, &command.arguments);
        for argument in remaining_arguments {
            bash_arrays::expand_command_argument(
                shell,
                argument,
                &mut expanded_fields,
                assignment_operands,
                &mut held_declarations,
            )?;
        }

        argument_count = expanded_fields.fields.len() - head;

        if is_exec_builtin && argument_count > 1 {
            variable_attributes = VariableAttributes::EXPORTED;
        }
    }

    resources.begin_local_variables(shell, use_local_variables);

    let last_argument_index = if shell.options.enabled(ShellOption::Interactive)
        && shell.evaluation.function_line == 0
        && argument_count > 0
    {
        Some(expanded_fields.fields.len() - 1)
    } else {
        None
    };

    let stderr = shell.descriptors.slot(LogicalDescriptor::STDERR);
    shell.io.previous_stderr().set_destination(stderr);
    let expanded_redirections = expand_redirections(shell, &command.redirections)?;
    /* `status = redirectsafe(..)`, which the C computes as `setjmp(..) *
     * 2`. The value is kept as well as the status, because `bail:` below
     * re-raises it when the command is a special built-in — that is the
     * one place a redirection error is *not* swallowed, and an `int`
     * cannot be re-raised. */
    let mut redirection_error: Option<Error> = None;
    match resources.apply_redirections(shell, &expanded_redirections) {
        /* Same as compound-redirection evaluation: an interrupt leaves rather than
         * becoming this command's status. */
        Err(error) if error.is_interrupt() || error.is_expansion() => return Err(error),
        Err(error) => {
            /* From the value; see the compound-redirection arm. Read before the move
             * into `redir_err`, which is where it is re-raised from. */
            status = error.status();
            redirection_error = Some(error);
        }
        Ok(()) => status = ExitStatus::SUCCESS,
    }

    'command_done: {
        'abort_command: {
            if !status.success() {
                break 'abort_command;
            }

            for assignment in &command.assignments {
                // A structural array assignment is applied whole; it has
                // no single expanded field to hand the scalar path. Its
                // written text still joins the ones `set -x` traces.
                if let Node::Bash(crate::nodes::BashNode::ArrayAssignment(array)) = assignment {
                    assignment_fields.fields.push(ExpandedField::from_bytes(
                        &bash_arrays::assignment_text(array),
                    ));
                    /* A refusal Bash reports rather than raises -- a
                     * list assigned to one element, a write through a
                     * reference that names one -- abandons the command
                     * with its status rather than the shell. */
                    if !bash_arrays::assign_prefix(shell, array, use_local_variables)? {
                        status = ExitStatus::FAILURE;
                        break 'abort_command;
                    }
                    continue;
                }
                let assignment_index = assignment_fields.fields.len();
                crate::expand::expand_argument(
                    shell,
                    assignment,
                    Some(&mut assignment_fields),
                    ExpansionMode::ASSIGNMENT_TILDE,
                )?;
                /* `(*spp)->text` with no null check: EXP_VARTILDE has no
                 * EXP_FULL, so `expandarg` appended exactly one entry. */
                debug_assert_eq!(
                    assignment_fields.fields.len(),
                    assignment_index + 1,
                    "an unsplit expansion is one field"
                );

                if use_local_variables {
                    crate::variables::make_local_bytes(
                        shell,
                        assignment_fields.fields[assignment_index].as_bstr(),
                        VariableAttributes::EXPORTED,
                    )?;
                } else {
                    crate::variables::set_assignment_bytes(
                        shell,
                        assignment_fields.fields[assignment_index].as_bstr(),
                        variable_attributes,
                    )?;
                }
            }

            /* Print the command if xflag is set. */
            if shell.options.enabled(ShellOption::Xtrace)
                && !shell.evaluation.expanding_trace_prompt
            {
                let mut already_printed: bool;

                /* This block is why `Dest` exists. It used to open with
                 * `out = previous_stderr()` and then hold that pointer
                 * across `ps4val(sh)`, `expandstr(sh, ..)` and two
                 * `eprintlist` calls — five reborrows of the shell with a
                 * raw pointer into its I/O still live. Sound while the
                 * pointer came from a static; undefined the moment it
                 * comes from `&mut sh.io`. Naming the destination defers
                 * the resolution to each write, so nothing spans a call. */
                let dest = OutputDestination::PreviousStderr;
                shell.evaluation.expanding_trace_prompt = true;
                /* Hoisted out of `expandstr`'s argument list; see the
                 * note in `evalcommand`. */
                let ps4 = crate::variables::trace_prompt_value(shell);
                let prompt = crate::parser::expand_string(shell, BStr::new(ps4.as_slice()))?;
                shell.write_output(dest, &prompt)?;
                shell.evaluation.expanding_trace_prompt = false;
                already_printed = false;
                /* An assignment is traced as it was *written*, not as a
                 * word that has to read back: Bash prints `sp1+=(2)`
                 * bare, where quoting it as data would give
                 * `'sp1+=(2)'`. The words after it are expansion
                 * results and do take Bash's trace quoting. */
                already_printed = trace::write_assignments(
                    shell,
                    dest,
                    &assignment_fields.fields,
                    already_printed,
                )?;
                /* `eprintlist(sh, out, osp, sep)` prints from the *original*
                 * head, so `command -p foo` traces as it was written and not
                 * as `parse_command_args` left it.  A NULL `osp` prints
                 * nothing, which is the empty slice. */
                trace::write_fields(
                    shell,
                    dest,
                    &expanded_fields.fields
                        [original_fields_start.unwrap_or(expanded_fields.fields.len())..],
                    already_printed,
                )?;
                shell.write_output(dest, b"\n")?;
            }

            /* Now locate the command. */
            if !matches!(
                &resolved_command,
                Command::Builtin(builtin) if builtin.attributes().is_regular()
            ) {
                if path.is_none() {
                    path = Some(crate::variables::path_value(shell));
                }
                let search_path =
                    BStr::new(path.as_ref().expect("command lookup has a PATH").as_slice());
                let command_name = expanded_fields.fields[head].as_bstr();
                match find_command(
                    shell,
                    command_name,
                    &mut resolved_command,
                    command_search.reporting_errors(),
                    search_path,
                )? {
                    Flow::Done(_) => {}
                    exit @ Flow::Exit { .. } => return Ok(exit),
                    control => {
                        command_control = Some(control);
                        break 'command_done;
                    }
                }
            }

            job_id = None;

            /* Execute the command. */
            match resolved_command {
                Command::Unknown => {
                    status = ExitStatus::NOT_FOUND;
                    break 'abort_command;
                }

                Command::Builtin(builtin) => {
                    /* `if (evalbltin(..) && !(exception == EXERROR && spclbltin <= 0))
                     *      goto raise;`
                     *
                     * The C asks two questions of one integer and a global:
                     * did the builtin leave by the exception mechanism, and
                     * was it the one kind of exception this frame is allowed
                     * to swallow. Both are answered by the type now. A
                     * diagnostic is `Err`, and swallowing it -- reporting it
                     * and carrying on with its status -- is POSIX's rule that
                     * only a *special* builtin's error ends a non-interactive
                     * shell, which is `docs/api-design.md` 3.3's contract and
                     * the mechanism that decides which errors an embedder
                     * ever sees. Anything else leaves as it arrived. */
                    match evaluate_builtin(
                        shell,
                        builtin,
                        &mut expanded_fields.fields[head..],
                        context,
                    ) {
                        Ok(flow) => {
                            if let Err(exit) = capture_local_control(flow, &mut command_control) {
                                return Ok(exit);
                            }
                        }
                        Err(error) => {
                            /* The C's `!(exception == EXERROR && spclbltin
                             * <= 0)`. An interrupt is not an EXERROR and
                             * was never swallowed here; now that it is a
                             * value, saying so is a test on the value.
                             *
                             * A signal trap adds one catch boundary. Its
                             * command failures must reach `dotrap` as a
                             * status so the interrupted status can be
                             * restored; returning the typed special-builtin
                             * error here would instead abort the shell and
                             * skip this command's ordinary cleanup. */
                            // [spec:nsh:req:compat.smoosh.trap-status]
                            if builtin_error_is_fatal(
                                shell,
                                builtin.id(),
                                special_builtin.unwrap_or(false),
                                &error,
                            ) {
                                return Err(error);
                            }
                            /* Reported already, and `evalbltin`'s epilogue
                             * has run. The status it took travels in the
                             * error, so this frame -- the one that catches
                             * it -- is the one that writes it. It reaches
                             * `status` through `waitforjob(sh, None)`,
                             * which returns `exitstatus` when there is no
                             * job; `bail:` does not touch it on this path
                             * because the C reaches `out:` here. */
                            shell.status = error.status();
                            drop(error);
                        }
                    }
                }

                Command::Function(function) => {
                    /* `if (evalfun(..)) goto raise;` -- a function body is
                     * not a builtin, so there is nothing to swallow: both an
                     * exit and a diagnostic leave through this frame. */
                    let args = crate::builtins::args(&expanded_fields.fields[head..]);
                    if let Err(exit) = capture_local_control(
                        evaluate_function(shell, &function, &args, context)?,
                        &mut command_control,
                    ) {
                        return Ok(exit);
                    }
                }

                Command::External { path_index } => {
                    shell.flush_input();
                    let args = crate::builtins::args(&expanded_fields.fields[head..]);

                    /* Fork off a child process if necessary. */
                    if !context.exits() || crate::trap::has_traps(shell) {
                        let syntax = Node::Command(Box::new(command.clone()));
                        status = crate::error::with_interrupts_deferred(shell, |shell| {
                            let job = crate::jobs::fork_and_execute(
                                shell,
                                &syntax,
                                &args,
                                BStr::new(
                                    path.as_ref()
                                        .expect("external command has a PATH")
                                        .as_slice(),
                                ),
                                path_index,
                            )?;
                            crate::jobs::wait_for_job(shell, Some(job))
                        })?;
                        crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
                        break 'command_done;
                    } else {
                        /* `shellexec` replaces the process image or fails;
                         * failing, it reports and is the C's EXEND. */
                        return execute_external_command(
                            shell,
                            &args,
                            BStr::new(
                                path.as_ref()
                                    .expect("external command has a PATH")
                                    .as_slice(),
                            ),
                            path_index,
                        );
                    }
                }
            }

            /* The attributes exist now, so the structural values the
             * declaration was written with can finally land. A
             * declaration that failed -- an unknown option, a name it
             * may not touch -- stores nothing, and has already said so. */
            bash_arrays::apply_declarations(
                shell,
                &held_declarations,
                subscripted_operands,
                shell.status.success(),
            )?;
            status = crate::jobs::wait_for_job(shell, job_id)?;
            crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
            break 'command_done;
        }
        // bail:
        /* A redirection-only command has no builtin entry whose specialness
         * can classify the failure. The adopted Smoosh contract uses the
         * shell-error status 1 for that path; this is the foreground half of
         * the parsed `exec 9&<-` case. */
        // [spec:nsh:req:compat.smoosh.error-contracts]
        status = redirection_only_status(
            status,
            redirection_error.as_ref(),
            original_fields_start.is_some(),
        );
        shell.status = status;

        /* We have a redirection error. */
        /* The dialect test is the same withdrawal of specialness that
         * `builtin_error_is_fatal` makes, at the other of the two frames
         * that can end a shell over a special built-in. This one is
         * reached before the built-in runs at all -- `exec 3</nonesuch`
         * and `: > /nonesuch-dir/x` never enter their utility -- so it
         * cannot be folded into that one. Falling through to `out:`
         * leaves the status the redirection layer took, which is the 1
         * Bash reports.
         *
         * What falling through does *not* reproduce is what a failed
         * `exec` leaves open: `out:` retains the descriptors the
         * successful redirections of the list already installed, where
         * Bash undoes all of them, so `exec 3</dev/null 4</nonesuch`
         * leaves 3 open here and closed there. That is the redirection
         * layer's rule and not the boundary's; it was unobservable while
         * the shell ended at the failure. */
        // [spec:nsh:req:compat.bash.error-boundary]
        if special_builtin == Some(true) && shell.options.dialect() != crate::options::Dialect::Bash
        {
            /* POSIX's "an error in a special built-in exits a
             * non-interactive shell", and the C's textless
             * `exraise(EXERROR)`: no diagnostic is written here because
             * whatever failed wrote its own.
             *
             * `redirectsafe` hands its error back, so the usual way in
             * carries the value. The other way in is an unknown command with
             * status 127, where there is no value to carry: `find_command`
             * reported "not found" and returned normally, which is
             * `docs/api-design.md` 3.3's "reported and carried on past".
             * `Error::reported` is that case -- a value with no text,
             * because the text has already been written. */
            let error = match redirection_error.take() {
                Some(error) => {
                    debug_assert_eq!(
                        error.status(),
                        status,
                        "a redirection error keeps its status"
                    );
                    error
                }
                None => crate::error::Error::reported(shell.evaluation.diagnostic_line, status),
            };
            debug_assert!(
                !error.is_expansion(),
                "expansion errors bypass redirection status"
            );
            // Smoosh's adopted POSIX closure profile assigns status 1 to a
            // redirection failure on a directly invoked special builtin.
            // Its diagnostic was already written by the redirection layer.
            // [spec:nsh:req:compat.smoosh.error-contracts]
            shell.status = ExitStatus::FAILURE;
            return Err(crate::error::Error::reported(
                shell.evaluation.diagnostic_line,
                1,
            ));
        }

        // goto out
    }
    // out:
    if is_exec_builtin {
        resources.retain_redirections(shell);
    }
    resources.restore(shell);
    if let Some(last_argument_index) = last_argument_index {
        /* dsl: I think this is intended to be used to support
         * '_' in 'vi' command mode during line editing...
         * However I implemented that within libedit itself.
         */
        crate::variables::set_bytes(
            shell,
            BStr::new(b"_"),
            Some(expanded_fields.fields[last_argument_index].as_bstr()),
            VariableAttributes::NONE,
        )?;
    }

    Ok(command_control
        .unwrap_or(Flow::Done(status))
        .with_status(status))
}
