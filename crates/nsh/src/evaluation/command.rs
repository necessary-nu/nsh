//! From a `SimpleCommand` node to it having run.
//!
//! Expanding the words, opening the redirections, deciding whether the
//! name is a built-in, a function or a file, running it, and putting the
//! shell back the way it was. `evaluate_command_in_scope` was 545 lines
//! of that on its own, which is the reason this file exists: it is one
//! subject, and it was in the middle of everything else.
//!
//! The subject is a driver and the phases it runs in order.
//! `evaluate_command_in_scope` owns the frame: `begin_local_variables`
//! and `apply_redirections` each have to be undone however the command
//! ends, and the code that undoes them is the last thing in it. So a
//! phase may not `return` past the driver, and [`CommandOutcome`] is
//! what it says instead. The C reached the epilogue by `goto out` and
//! the failure classifier by `goto bail`; those two labels are two of
//! that enum's three variants. The third is the departure that really
//! does skip the epilogue -- an `exit` raised from inside the utility --
//! which leaves `_` alone and a failed `exec`'s descriptors installed,
//! and left them that way when the labels were `goto`s too.

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

/// A simple command's words, and what the first of them turned out to be.
///
/// Fourteen values that were fourteen separate `let mut` bindings, most
/// of them declared together at the head of one function because C
/// declares at the top of a block rather than because anything tied them
/// together. They are one
/// value because one left-to-right pass over the words settles all of
/// them at once: expanding a word can run a command substitution, a
/// `command -p` prefix re-enters the lookup having consumed two more
/// words, and reaching a built-in is the moment that fixes the
/// specialness, the scope an assignment in front of the command lands
/// in, and the attributes it lands with.
///
/// That pass can stop in the middle of itself -- `find_command` runs a
/// `%func` file found on `PATH`, which is shell code and can `exit` --
/// so what it has settled so far has to live in a value rather than in
/// a stack frame the departure unwinds.
struct ExpandedCommand<'a> {
    /// The expanded words: the utility's name at `head`, its arguments
    /// after it, and anything a `command` prefix left in front.
    words: ExpandedFields,
    /// A declaration built-in's `a=(1 2)` operand cannot be expanded into
    /// a word: the value is structural, and its kind depends on the
    /// attributes the built-in has yet to apply. The name goes into the
    /// word list and the assignment waits here.
    held: Vec<bash_arrays::Declaration<'a>>,
    /// Which built-in the held operands are for: `export` refuses a
    /// subscripted one where `declare` accepts it.
    subscripted: bash_arrays::SubscriptedOperand,
    /// The C's `arglist.list`: where the utility's own name starts.
    /// `parse_command_args` moves it past the `command [-p]` words it
    /// consumed.
    head: usize,
    /// The C's `argc`: how many words the utility itself gets.
    argument_count: usize,
    /// The C's `osp`, the head before a `command` prefix moved it, which
    /// is where `set -x` traces from so that `command -p foo` traces as
    /// it was written. `None` when the words expanded to nothing at all,
    /// which is also how a redirection-only command is recognised.
    trace_start: Option<usize>,
    /// What the name resolved to. Starts as the empty built-in, which is
    /// the C's `&bltin` -- the entry a command with no word at all runs.
    resolved: Command,
    /// The `PATH` the lookup used, kept because the external branch has
    /// to hand the same one to `execve` and `command -p` substitutes the
    /// standard path for it.
    path: Option<BString>,
    /// Whether the utility is a special built-in, or `None` while the
    /// question is open. Only the first built-in reached answers it, and
    /// nothing behind that one reopens it -- which is the whole reason
    /// this is not a plain `bool`.
    special_builtin: Option<bool>,
    /// `exec` is the one utility whose redirections outlive the frame.
    is_exec_builtin: bool,
    /// What an assignment in front of the command carries: `exec cmd arg`
    /// exports where a bare `exec` does not.
    variable_attributes: VariableAttributes,
    /// Whether such an assignment is a temporary of this command rather
    /// than a change to the shell. A special built-in's persist.
    use_local_variables: bool,
    /// How the *next* lookup searches: `command` withdraws function
    /// lookup and `command -p` withdraws the caller's `PATH`.
    search: CommandSearch,
    /// Whether the utility takes its operands as assignments --
    /// `declare` and its neighbours -- which changes how the words after
    /// it expand.
    takes_assignments: bool,
}

impl<'a> ExpandedCommand<'a> {
    /// The C's declarations at the top of `evalcommand`, and its
    /// `cmdentry.u.cmd = &bltin`.
    fn new() -> ExpandedCommand<'a> {
        ExpandedCommand {
            words: ExpandedFields::new(),
            held: Vec::new(),
            subscripted: bash_arrays::SubscriptedOperand::Accepted,
            head: 0,
            argument_count: 0,
            trace_start: None,
            resolved: Command::Builtin(&crate::builtins::EMPTY_BUILTIN),
            path: None,
            special_builtin: None,
            is_exec_builtin: false,
            variable_attributes: VariableAttributes::NONE,
            use_local_variables: false,
            search: CommandSearch::DEFAULT,
            takes_assignments: false,
        }
    }
}

/// Where running the command left off, in the frame that has to restore.
///
/// The C's `evalcommand` reaches its two labels by `goto`, and a
/// function cannot jump into its caller. Each label is a variant here, so
/// the phase that decided names the one it wants and the driver is the
/// only place that runs either.
///
/// The distinction is not cosmetic: `Ran` and `Abandoned` both reach the
/// code that retains an `exec`'s descriptors and sets `_`, and `Left`
/// does not. Every one of the C's `goto`s and every one of its longjmps
/// had to keep that straight by hand.
enum CommandOutcome {
    /// The C's `goto out`. The utility ran -- or was forked and waited
    /// for -- and this is the status the epilogue publishes.
    Ran(ExitStatus),
    /// The C's `goto bail`. Nothing ran: a redirection, an assignment or
    /// the lookup failed first, and the failure still has to be
    /// classified, because a special built-in's ends a non-interactive
    /// shell where an ordinary command's is only a status.
    Abandoned(ExitStatus),
    /// The C's longjmp past this frame. The epilogue does not run, so
    /// `_` keeps its old value and a failed `exec`'s descriptors stay
    /// installed. `crate::resource::with_resources` still restores, one
    /// frame further out.
    Left(Flow),
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

/// The frame a simple command runs in, and everything that puts it back.
///
/// Two things this opens have to be closed however the command ends: the
/// local-variable scope that `begin_local_variables` pushes, and the
/// descriptors `apply_redirections` moves aside. The code that closes
/// them is the epilogue at the end of this function, which is why the
/// phases it calls report a [`CommandOutcome`] rather than `return` past
/// it.
///
/// Two kinds of departure do skip that epilogue, and both are deliberate.
/// An `Err` is the diagnostic path, and the `Left` arm below is an `exit`
/// raised from inside the utility: both leave `_` alone and leave a
/// failed `exec`'s descriptors installed, which is what they left before
/// the labels were functions. Neither leaks the scope or the
/// redirections, because [`crate::resource::with_resources`] restores one
/// frame further out.
fn evaluate_command_in_scope(
    shell: &mut Shell,
    command: &SimpleCommand,
    context: EvaluationContext,
    resources: &mut crate::resource::ResourceScope,
) -> Result<Flow, Error> {
    /* Cleared before the DEBUG trap rather than with the rest of the
     * setup below: the trap runs shell code, and a command inside it
     * fills this on its own behalf. */
    shell.evaluation.refused_declarations.clear();
    shell.evaluation.declared_kind = None;
    shell.evaluation.held_declarations.clear();
    record_command_line(shell, command.line.get());
    // [spec:nsh:req:compat.bash.names.ordinary-state]
    crate::variables::special::record_command(shell, &command.tokens);
    // [spec:nsh:req:compat.bash.traps-introspection]
    flow!(crate::trap::bash::run_debug(shell));
    if has_unexecutable_bash_syntax(command) {
        return Err(shell
            .diagnostics()
            .shell_error(b"Bash syntax is parsed but not executable yet"));
    }

    let mut expansion = ExpandedCommand::new();
    flow!(expand_command_words(shell, command, &mut expansion));

    resources.begin_local_variables(shell, expansion.use_local_variables);

    let last_argument_index = underscore_target_field(shell, &expansion);

    let (status, redirection_error) = open_command_redirections(shell, command, resources)?;

    let mut command_control = None;
    let outcome = if status.success() {
        run_redirected_command(
            shell,
            command,
            context,
            &mut expansion,
            &mut command_control,
        )?
    } else {
        CommandOutcome::Abandoned(status)
    };
    let status = match outcome {
        CommandOutcome::Ran(status) => status,
        CommandOutcome::Left(flow) => return Ok(flow),
        // bail:
        CommandOutcome::Abandoned(status) => {
            classify_abandoned_command(shell, &expansion, status, redirection_error)?
        }
    };

    // out:
    if expansion.is_exec_builtin {
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
            Some(expansion.words.fields[last_argument_index].as_bstr()),
            VariableAttributes::NONE,
        )?;
    } else if shell.options.dialect() == crate::options::Dialect::Bash {
        /* A command with no words at all still moves the name: `x=5`
         * leaves `$_` empty in the reference rather than leaving the
         * previous command's last word standing. */
        // [spec:nsh:req:compat.bash.names.ordinary-state]
        crate::variables::set_bytes(
            shell,
            BStr::new(b"_"),
            Some(BStr::new(b"")),
            VariableAttributes::NONE,
        )?;
    }

    Ok(command_control
        .unwrap_or(Flow::Done(status))
        .with_status(status))
}

/// Which expanded word `$_` will be left holding, if any.
///
/// The C's `lastarg`, and all three of its conditions are the C's too.
/// `_` is a line-editing convenience there, so a non-interactive shell
/// never pays for it; a function body is not something the user typed, so
/// its commands do not set it either; and a command with no words has
/// nothing to name.
///
/// NONE OF THE THREE IS BASH'S. Measured on the pinned 5.3.15 with a
/// script on standard input, so non-interactive throughout: `echo hi`
/// leaves `hi`, `true a b c` leaves `c`, a bare `echo` leaves `echo` and
/// a bare `:` leaves `:` -- so the command word counts when nothing
/// follows it -- and `f(){ echo inner arg; ...; }` leaves `arg` inside
/// the body and `f` outside it. A command that is only an assignment
/// leaves the empty string, which is [`None`] here and the empty write in
/// the epilogue.
///
/// An index rather than the word, and settled here rather than in the
/// epilogue that uses it, because that is where the C settles it. It
/// cannot go stale in between: a built-in is handed its words as a
/// slice, whose length no callee can change.
// [spec:nsh:req:compat.bash.names.ordinary-state]
fn underscore_target_field(shell: &Shell, expansion: &ExpandedCommand<'_>) -> Option<usize> {
    if shell.options.dialect() == crate::options::Dialect::Bash {
        return expansion.words.fields.len().checked_sub(1);
    }
    if shell.options.enabled(ShellOption::Interactive)
        && shell.evaluation.function_line == 0
        && expansion.argument_count > 0
    {
        Some(expansion.words.fields.len() - 1)
    } else {
        None
    }
}

/// Whether the parser accepted more of Bash than the evaluator runs.
///
/// The Bash grammar is parsed ahead of being executable, so a node kind
/// the evaluator has no arm for can reach here as a well-formed tree.
/// Array assignments are the one Bash node this file does run, hence the
/// inner `matches!`: everything else in that family is refused up front,
/// with a diagnostic, rather than silently dropped by a `_ => {}` arm
/// somewhere further in.
fn has_unexecutable_bash_syntax(command: &SimpleCommand) -> bool {
    command
        .assignments
        .iter()
        .chain(command.arguments.iter())
        .any(|node| {
            matches!(node, Node::Bash(inner)
                if !matches!(inner, crate::nodes::BashNode::ArrayAssignment(_)))
        })
}

/// Expand the command's words and find out what the first one names.
///
/// The words are expanded in two goes rather than one, and the C's
/// `fill_arglist` exists for the first of them: only enough words to
/// produce one field, because that field is the name, and looking the
/// name up is what says how many more words belong to the lookup rather
/// than to the utility -- `command -p ls` hands two of them to
/// [`parse_command_args`] before `ls` is even reached.
///
/// The name settles a second question that only this port asks. A
/// declaration built-in's operands expand as assignments, so `declare
/// x=~` keeps the tilde where `echo x=~` does not, and the words after
/// the name cannot be expanded until it is known which of those two the
/// command is.
///
/// `Flow::Done` means the caller carries on. Anything else is a control
/// transfer raised while expanding, which the caller passes straight out:
/// a command substitution can `exit`, and so can the shell code in a
/// `%func` file the lookup ran.
fn expand_command_words<'a>(
    shell: &mut Shell,
    command: &'a SimpleCommand,
    expansion: &mut ExpandedCommand<'a>,
) -> Result<Flow, Error> {
    /* First expand the arguments. */
    shell.evaluation.command_substitution_status = ExitStatus::SUCCESS;

    let mut remaining_arguments = command.arguments.as_slice();
    expansion.trace_start = append_expanded_arguments(
        shell,
        &mut expansion.words,
        &mut remaining_arguments,
        &mut expansion.held,
    )?;
    /* No word at all: the assignments and redirections are the whole
     * command, and there is nothing to look up. */
    if expansion.trace_start.is_none() {
        return Ok(Flow::Done(ExitStatus::SUCCESS));
    }

    flow!(resolve_command_prefix(
        shell,
        expansion,
        &mut remaining_arguments
    ));

    let assignment_operands = expansion.takes_assignments
        && crate::parser::bash::declaration_operands(shell, &command.arguments);
    for argument in remaining_arguments {
        bash_arrays::expand_command_argument(
            shell,
            argument,
            &mut expansion.words,
            assignment_operands,
            &mut expansion.held,
        )?;
    }

    expansion.argument_count = expansion.words.fields.len() - expansion.head;

    if expansion.is_exec_builtin && expansion.argument_count > 1 {
        expansion.variable_attributes = VariableAttributes::EXPORTED;
    }
    Ok(Flow::Done(ExitStatus::SUCCESS))
}

/// Walk the leading words until one of them is the utility itself.
///
/// The C's `for (;;)` around `find_command`, which goes round once per
/// `command` word: `command command -p ls` is three lookups, and each
/// one may hand the next a different search. Reaching any other built-in
/// ends it, and so does reaching a function or a file.
///
/// It is also where the built-in's attributes are read off, because
/// arriving at a built-in is exactly the moment the answers exist.
/// `special_builtin` is answered by the *first* built-in reached and
/// never re-answered, and that is the mechanism by which `command`
/// withdraws the specialness of what follows it: `command` is itself a
/// regular built-in, so `command exec ...` records `Some(false)` on the
/// first pass and the `exec` behind it does not get to say otherwise.
// [spec:posix:req:builtin.command.special-builtin-properties-suppressed]
fn resolve_command_prefix<'a>(
    shell: &mut Shell,
    expansion: &mut ExpandedCommand<'a>,
    remaining_arguments: &mut &'a [Node],
) -> Result<Flow, Error> {
    let standard_path = crate::variables::default_path();

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
            expansion.words.fields[expansion.head].as_bstr(),
            &mut expansion.resolved,
            expansion.search.regular_builtins_only(),
            BStr::new(active_path.as_slice()),
        )? {
            Flow::Done(_) => {}
            control => return Ok(control),
        }

        expansion.use_local_variables = true;

        /* implement bltin and command here */
        let Command::Builtin(builtin) = &expansion.resolved else {
            break;
        };
        let builtin = *builtin;

        expansion.takes_assignments = builtin.attributes().takes_assignments();
        if expansion.special_builtin.is_none() {
            let special = builtin.attributes().is_special();
            expansion.special_builtin = Some(special);
            expansion.use_local_variables = !special;
        }
        expansion.is_exec_builtin = builtin.id() == BuiltinId::Exec;
        /* `export` names variables, not elements: Bash refuses
         * `export a[7]=8` rather than assigning the element. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        if builtin.id() == BuiltinId::Export {
            expansion.subscripted = bash_arrays::SubscriptedOperand::Refused;
        }
        if builtin.id() != BuiltinId::Command {
            break;
        }

        let Some(next_search) = parse_command_args(
            shell,
            &mut expansion.words,
            remaining_arguments,
            &mut expansion.path,
            standard_path.as_slice().as_bstr(),
            &mut expansion.head,
            &mut expansion.held,
        )?
        else {
            break;
        };
        expansion.search = next_search;
    }

    Ok(Flow::Done(ExitStatus::SUCCESS))
}

/// Move the descriptors aside, and say what a failure to do so cost.
///
/// The C's `status = redirectsafe(..)`, which it computes as
/// `setjmp(..) * 2`. Both halves of the answer are returned: an `int`
/// cannot be re-raised, and [`classify_abandoned_command`] has to
/// re-raise this one when the utility is a special built-in. That is the
/// single place a redirection error is not swallowed.
///
/// An interrupt or an expansion error leaves as `Err` instead of
/// becoming the command's status, exactly as it does for a compound
/// command's redirections.
fn open_command_redirections(
    shell: &mut Shell,
    command: &SimpleCommand,
    resources: &mut crate::resource::ResourceScope,
) -> Result<(ExitStatus, Option<Error>), Error> {
    let stderr = shell.descriptors.slot(LogicalDescriptor::STDERR);
    shell.io.previous_stderr().set_destination(stderr);
    let expanded_redirections = expand_redirections(shell, &command.redirections)?;
    match resources.apply_redirections(shell, &expanded_redirections) {
        /* Same as compound-redirection evaluation: an interrupt leaves rather than
         * becoming this command's status. */
        Err(error) if error.is_interrupt() || error.is_expansion() => Err(error),
        /* From the value; see the compound-redirection arm. The status is read
         * before the move into the returned pair, which is where it is
         * re-raised from. */
        Err(error) => Ok((error.status(), Some(error))),
        Ok(()) => Ok((ExitStatus::SUCCESS, None)),
    }
}

/// Everything between the redirections landing and the epilogue.
///
/// Everything the C's `goto bail` skipped: assignments, the `set -x`
/// trace, the second lookup, and the utility itself. Only reached when
/// the redirections succeeded, so the status on entry is the success
/// they left -- which is why the `Ran` arm below can name that status
/// rather than have it passed in.
///
/// Every way out of here is a [`CommandOutcome`] rather than a `return`,
/// because the caller still has to put back what it opened.
fn run_redirected_command(
    shell: &mut Shell,
    command: &SimpleCommand,
    context: EvaluationContext,
    expansion: &mut ExpandedCommand<'_>,
    command_control: &mut Option<Flow>,
) -> Result<CommandOutcome, Error> {
    let mut assignment_fields = ExpandedFields::new();
    if !apply_command_assignments(
        shell,
        &command.assignments,
        &mut assignment_fields,
        expansion.use_local_variables,
        expansion.variable_attributes,
    )? {
        return Ok(CommandOutcome::Abandoned(ExitStatus::FAILURE));
    }

    /* Print the command if xflag is set. */
    if shell.options.enabled(ShellOption::Xtrace) && !shell.evaluation.expanding_trace_prompt {
        /* `eprintlist(sh, out, osp, sep)` prints from the *original*
         * head, so `command -p foo` traces as it was written and not
         * as `parse_command_args` left it.  A NULL `osp` prints
         * nothing, which is the empty slice. */
        let traced_from = expansion
            .trace_start
            .unwrap_or(expansion.words.fields.len());
        trace_expanded_command(
            shell,
            &assignment_fields.fields,
            &expansion.words.fields[traced_from..],
        )?;
    }

    /* Now locate the command. */
    if !matches!(
        &expansion.resolved,
        Command::Builtin(builtin) if builtin.attributes().is_regular()
    ) {
        if expansion.path.is_none() {
            expansion.path = Some(crate::variables::path_value(shell));
        }
        let search_path = BStr::new(
            expansion
                .path
                .as_ref()
                .expect("command lookup has a PATH")
                .as_slice(),
        );
        let command_name = expansion.words.fields[expansion.head].as_bstr();
        match find_command(
            shell,
            command_name,
            &mut expansion.resolved,
            expansion.search.reporting_errors(),
            search_path,
        )? {
            Flow::Done(_) => {}
            exit @ Flow::Exit { .. } => return Ok(CommandOutcome::Left(exit)),
            control => {
                *command_control = Some(control);
                return Ok(CommandOutcome::Ran(ExitStatus::SUCCESS));
            }
        }
    }

    run_resolved_command(shell, command, context, expansion, command_control)
}

/// Apply the assignments written in front of the command name.
///
/// `a=1 b=2 cmd` and `a=1 b=2` alone are the same words on the way in
/// and different things on the way out, and the difference was decided
/// before this runs: `use_local_variables` says whether the values are
/// this command's temporaries or the shell's, and `attributes` carries
/// the export a `exec cmd args` adds.
///
/// `false` is the C's `goto bail` -- an assignment Bash *reports*
/// rather than raises, such as a list assigned to a single element or a
/// write through a reference that names one. It abandons the command
/// with a status and leaves the shell alone, so it cannot be an `Err`.
fn apply_command_assignments(
    shell: &mut Shell,
    assignments: &[Node],
    assignment_fields: &mut ExpandedFields,
    use_local_variables: bool,
    attributes: VariableAttributes,
) -> Result<bool, Error> {
    for assignment in assignments {
        // A structural array assignment is applied whole; it has
        // no single expanded field to hand the scalar path. Its
        // written text still joins the ones `set -x` traces.
        if let Node::Bash(crate::nodes::BashNode::ArrayAssignment(array)) = assignment {
            assignment_fields.fields.push(ExpandedField::from_bytes(
                &bash_arrays::assignment_text(array),
            ));
            if !bash_arrays::assign_prefix(shell, array, use_local_variables)? {
                return Ok(false);
            }
            continue;
        }
        let assignment_index = assignment_fields.fields.len();
        crate::expand::expand_argument(
            shell,
            assignment,
            Some(&mut *assignment_fields),
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
                attributes,
            )?;
        }
    }
    Ok(true)
}

/// Write one `set -x` line: the prompt, the assignments, the words.
///
/// This is why [`OutputDestination`] exists. It used to open with
/// `out = previous_stderr()` and then hold that pointer across
/// `ps4val(sh)`, `expandstr(sh, ..)` and two `eprintlist` calls -- five
/// reborrows of the shell with a raw pointer into its I/O still live.
/// Sound while the pointer came from a static; undefined the moment it
/// comes from `&mut sh.io`. Naming the destination defers the resolution
/// to each write, so nothing spans a call.
///
/// The two halves quote differently on purpose. An assignment is traced
/// as it was *written*, so Bash prints `sp1+=(2)` bare where quoting it
/// as data would give `'sp1+=(2)'`. The words after it are expansion
/// results and do take Bash's trace quoting.
// [spec:posix:req:param.ps4]
fn trace_expanded_command(
    shell: &mut Shell,
    assignment_fields: &[ExpandedField],
    traced_words: &[ExpandedField],
) -> Result<(), Error> {
    let mut already_printed: bool;

    let dest = OutputDestination::PreviousStderr;
    shell.evaluation.expanding_trace_prompt = true;
    /* Hoisted out of `expandstr`'s argument list; see the
     * note in `evalcommand`. */
    let ps4 = crate::variables::trace_prompt_value(shell);
    let prompt = crate::parser::expand_string(shell, BStr::new(ps4.as_slice()))?;
    shell.write_output(dest, &prompt)?;
    shell.evaluation.expanding_trace_prompt = false;
    already_printed = false;
    already_printed = trace::write_assignments(shell, dest, assignment_fields, already_printed)?;
    trace::write_fields(shell, dest, traced_words, already_printed)?;
    shell.write_output(dest, b"\n")?;
    Ok(())
}

/// Run whatever the name turned out to be, and collect its status.
///
/// The C's switch on `cmdentry.cmdtype`, which is a `Command` here, so
/// the tag and the payload cannot disagree. The three live arms differ
/// in what may leave through them, which is the whole reason they are
/// not one: a built-in's diagnostic may be swallowed, a function body's
/// may not, and an external command may not even come back.
fn run_resolved_command(
    shell: &mut Shell,
    command: &SimpleCommand,
    context: EvaluationContext,
    expansion: &mut ExpandedCommand<'_>,
    command_control: &mut Option<Flow>,
) -> Result<CommandOutcome, Error> {
    /* The C's `jp`, and NULL on every path that reaches the wait below:
     * the forked branch waits for its own job and leaves before it. */
    let job_id: Option<JobId> = None;

    /* Execute the command. */
    match expansion.resolved.clone() {
        Command::Unknown => return Ok(CommandOutcome::Abandoned(ExitStatus::NOT_FOUND)),

        Command::Builtin(builtin) => {
            if let Some(flow) = run_builtin_command(
                shell,
                builtin,
                &mut expansion.words.fields[expansion.head..],
                context,
                expansion.special_builtin.unwrap_or(false),
                command_control,
            )? {
                return Ok(CommandOutcome::Left(flow));
            }
        }

        Command::Function(function) => {
            /* `if (evalfun(..)) goto raise;` -- a function body is
             * not a builtin, so there is nothing to swallow: both an
             * exit and a diagnostic leave through this frame. */
            let args = crate::builtins::args(&expansion.words.fields[expansion.head..]);
            if let Err(exit) = capture_local_control(
                evaluate_function(shell, &function, &args, context)?,
                command_control,
            ) {
                return Ok(CommandOutcome::Left(exit));
            }
        }

        Command::External { path_index } => {
            return run_external_command(shell, command, context, expansion, path_index);
        }
    }

    /* The attributes exist now, so the structural values the
     * declaration was written with can finally land. A
     * declaration that failed -- an unknown option, a name it
     * may not touch -- stores nothing, and has already said so. */
    bash_arrays::apply_declarations(
        shell,
        &expansion.held,
        expansion.subscripted,
        shell.status.success(),
    )?;
    let status = crate::jobs::wait_for_job(shell, job_id)?;
    crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
    Ok(CommandOutcome::Ran(status))
}

/// Run a built-in, and decide whether its failure ends the shell.
///
/// The C is `if (evalbltin(..) && !(exception == EXERROR && spclbltin <= 0))
/// goto raise;` -- two questions asked of one integer and a global: did
/// the built-in leave by the exception mechanism, and was it the one kind
/// of exception this frame may swallow. Both are answered by the type
/// now. A diagnostic is `Err`, and swallowing it -- reporting it and
/// carrying on with its status -- is POSIX's rule that only a *special*
/// built-in's error ends a non-interactive shell, which is
/// `docs/api-design.md` 3.3's contract and the mechanism that decides
/// which errors an embedder ever sees. [`builtin_error_is_fatal`] is that
/// question on its own; this is the frame that asks it.
///
/// `Some` is a departure the caller must not run its epilogue for.
/// `None` carries on to the wait.
fn run_builtin_command(
    shell: &mut Shell,
    builtin: &'static BuiltinSpec,
    fields: &mut [ExpandedField],
    context: EvaluationContext,
    special_builtin: bool,
    command_control: &mut Option<Flow>,
) -> Result<Option<Flow>, Error> {
    match evaluate_builtin(shell, builtin, fields, context) {
        Ok(flow) => {
            if let Err(exit) = capture_local_control(flow, command_control) {
                return Ok(Some(exit));
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
            if builtin_error_is_fatal(shell, builtin.id(), special_builtin, &error) {
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
    Ok(None)
}

/// Fork for a file on `PATH`, or become it.
///
/// The C's `case CMDNORMAL`, whose two halves are two different
/// commitments. Forking keeps this shell alive, so the child's status
/// comes back through the wait and the epilogue runs as usual. Not
/// forking is `shellexec`, which replaces the process image or fails,
/// and failing it has already reported and is the C's EXEND -- there is
/// no frame left to restore anything into, which is why it leaves rather
/// than returns a status.
///
/// The choice is not an optimisation: a trap has to survive the command
/// to be able to run, so a shell with traps forks even where the last
/// command of a `-c` string would otherwise be exec'd in place.
fn run_external_command(
    shell: &mut Shell,
    command: &SimpleCommand,
    context: EvaluationContext,
    expansion: &ExpandedCommand<'_>,
    path_index: Option<usize>,
) -> Result<CommandOutcome, Error> {
    shell.flush_input();
    let args = crate::builtins::args(&expansion.words.fields[expansion.head..]);
    let search_path = BStr::new(
        expansion
            .path
            .as_ref()
            .expect("external command has a PATH")
            .as_slice(),
    );

    /* Fork off a child process if necessary. */
    if !context.exits() || crate::trap::has_traps(shell) {
        let syntax = Node::Command(Box::new(command.clone()));
        let status = crate::error::with_interrupts_deferred(shell, |shell| {
            let job =
                crate::jobs::fork_and_execute(shell, &syntax, &args, search_path, path_index)?;
            crate::jobs::wait_for_job(shell, Some(job))
        })?;
        crate::error::clear_interrupt_deferral(&mut shell.interrupt_deferral);
        Ok(CommandOutcome::Ran(status))
    } else {
        /* `shellexec` replaces the process image or fails;
         * failing, it reports and is the C's EXEND. */
        Ok(CommandOutcome::Left(execute_external_command(
            shell,
            &args,
            search_path,
            path_index,
        )?))
    }
}

/// The C's `bail:`: what a failure before the utility ran meant.
///
/// A redirection-only command has no built-in entry whose specialness
/// can classify the failure, so neither shell ends over a bare
/// `> /nonesuch-dir/x`. The adopted Smoosh contract gives that path the
/// shell-error status 1 through [`redirection_only_status`], where dash
/// answers 2 -- the one status in this frame that is still Smoosh's, and
/// `bash.divergences.redirection-status-without-a-command` holds it.
///
/// The dialect test below is the same withdrawal of specialness that
/// [`builtin_error_is_fatal`] makes, at the other of the two frames that
/// can end a shell over a special built-in. This one is reached before
/// the built-in runs at all -- `exec 3</nonesuch` and
/// `: > /nonesuch-dir/x` never enter their utility -- so it cannot be
/// folded into that one. Returning the status instead leaves what the
/// redirection layer took, which is the 1 Bash reports.
///
/// What returning does *not* reproduce is what a failed `exec` leaves
/// open: the epilogue retains the descriptors the successful
/// redirections of the list already installed, where Bash undoes all of
/// them, so `exec 3</dev/null 4</nonesuch` leaves 3 open here and closed
/// there. That is the redirection layer's rule and not the boundary's;
/// it was unobservable while the shell ended at the failure.
// [spec:nsh:req:compat.smoosh.error-contracts]
// [spec:nsh:req:compat.bash.error-boundary]
fn classify_abandoned_command(
    shell: &mut Shell,
    expansion: &ExpandedCommand<'_>,
    status: ExitStatus,
    redirection_error: Option<Error>,
) -> Result<ExitStatus, Error> {
    let status = redirection_only_status(
        status,
        redirection_error.as_ref(),
        expansion.trace_start.is_some(),
    );
    shell.status = status;

    /* We have a redirection error. */
    if expansion.special_builtin == Some(true)
        && shell.options.dialect() != crate::options::Dialect::Bash
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
        let error = match redirection_error {
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
        /* The status is the redirection layer's, which the
         * `debug_assert_eq!` above pins: it has already taken the
         * dialect's number -- 2 for `: <missing` in the POSIX dialect,
         * where dash also answers 2 -- and re-deriving one here could
         * only contradict the diagnostic that was written with it.
         * Smoosh records 1 for this case; that is a sanctioned
         * divergence in `docs/divergences.md` rather than a number to
         * reinstate. */
        // [spec:nsh:req:compat.bash.error-boundary]
        shell.status = error.status();
        return Err(error);
    }

    Ok(status)
}
