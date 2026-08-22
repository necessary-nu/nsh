//! `trap`.
//!
//! Port of `trapcmd` from `src/trap.c`. The trap table, the dispositions
//! it asks the host for, and `dotrap` -- which runs an action between
//! commands rather than from here -- all stay in `crate::trap`.
//!
//! `trap` with no operands prints the table in a form that can be read
//! back, which is why the action is single-quoted on the way out.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString};

use crate::evaluation::Flow;
use crate::options::{Dialect, Options};
use crate::output::OutputDestination;
use crate::trap::bash::BashCondition;
use crate::trap::{
    SIGNAL_SLOT_COUNT, SignalSpec, TrapAction, clear_traps, configure_signal, decode_signal,
    parse_signal_number,
};

// [spec:posix:req:builtin.trap.opt-p-suitable-for-reinput]
fn listing_line(condition_name: &[u8], action: &TrapAction) -> Vec<u8> {
    let mut line = b"trap -- ".to_vec();
    match action {
        TrapAction::Default => line.push(b'-'),
        TrapAction::Ignore => line.extend_from_slice(b"''"),
        TrapAction::Command(command) => {
            line.extend_from_slice(&crate::escape::shell_quote(BStr::new(command.as_slice())));
        }
    }
    line.push(b' ');
    line.extend_from_slice(condition_name);
    line.push(b'\n');
    line
}

fn write_listing(
    shell: &mut Shell,
    signal_number: usize,
    include_default: bool,
) -> Result<(), Error> {
    let action = shell.traps.listed_action(signal_number);
    if !include_default && matches!(action, TrapAction::Default) {
        return Ok(());
    }
    let line = listing_line(
        crate::signal_names::SIGNAL_NAMES[signal_number].to_bytes(),
        action,
    );
    shell.write_output(OutputDestination::Stdout, &line)
}

/// The Bash pseudo-condition `word` names, in the dialect that has them.
// [spec:nsh:req:compat.bash.traps-introspection]
fn bash_condition(shell: &Shell, word: &BStr) -> Option<BashCondition> {
    if shell.options.dialect() != Dialect::Bash {
        return None;
    }
    crate::trap::bash::decode(word)
}

// [spec:nsh:req:compat.bash.traps-introspection]
fn write_bash_listing(
    shell: &mut Shell,
    condition: BashCondition,
    include_default: bool,
) -> Result<(), Error> {
    let action = shell.traps.bash.listed_action(condition);
    if !include_default && matches!(action, TrapAction::Default) {
        return Ok(());
    }
    let line = listing_line(condition.name(), action);
    shell.write_output(OutputDestination::Stdout, &line)
}

/// `trap -l`: every signal the shell knows, numbered.
// [spec:nsh:req:compat.bash.traps-introspection]
fn write_signal_names(shell: &mut Shell) -> Result<(), Error> {
    for number in 1..SIGNAL_SLOT_COUNT {
        let mut line = number.to_string().into_bytes();
        line.extend_from_slice(b") SIG");
        line.extend_from_slice(crate::signal_names::SIGNAL_NAMES[number].to_bytes());
        line.push(b'\n');
        shell.write_output(OutputDestination::Stdout, &line)?;
    }
    Ok(())
}

/// The action a `trap` operand word selects, consuming the reset
/// spelling exactly as the C's `action = NULL` does.
fn selected_action(action: &mut Option<BString>) -> TrapAction {
    let Some(text) = action.as_ref() else {
        return TrapAction::Default;
    };
    if text.as_slice() == b"-" {
        *action = None;
        return TrapAction::Default;
    }
    if text.is_empty() {
        return TrapAction::Ignore;
    }
    TrapAction::Command(text.clone())
}

// [spec:dash:sem:trap.trapcmd-fn]
// [spec:posix:syn:builtin.trap.synopsis]
// [spec:posix:req:builtin.trap.operand-interpretation]
// [spec:posix:req:builtin.trap.action-values]
// [spec:posix:def:builtin.trap.condition]
// [spec:posix:req:builtin.trap.signal-name-extensions]
// [spec:posix:req:builtin.trap.kill-stop-undefined]
// [spec:posix:req:builtin.trap.list-condition-set]
// [spec:posix:syn:builtin.trap.list-format]
// [spec:posix:req:builtin.trap.list-in-subshell]
// [spec:posix:req:builtin.trap.list-suitable-for-reinput]
// [spec:posix:req:builtin.trap.opt-p]
// [spec:posix:req:builtin.trap.xsi-signal-numbers]
// [spec:posix:req:builtin.trap.invalid-condition-warning]
// [spec:posix:req:builtin.trap.utility-syntax-guidelines]
// [spec:posix:req:builtin.trap.utility-defaults]
// [spec:posix:req:builtin.trap.stderr-usage]
// [spec:posix:req:builtin.trap.exit-status]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut print = false;
    let mut list = false;
    /* `-l` is Bash's, and the dialect decides whether the letter exists
     * at all rather than whether it does anything. */
    // [spec:nsh:req:compat.bash.traps-introspection]
    let accepted: &[u8] = if shell.options.dialect() == Dialect::Bash {
        b"lp"
    } else {
        b"p"
    };
    let mut option_scan = Options::new(args);
    while let Some(option) = option_scan.next(&mut shell.diagnostics(), accepted)? {
        print |= option == b'p';
        list |= option == b'l';
    }
    if list {
        write_signal_names(shell)?;
        return Ok(Flow::Done((0).into()));
    }
    let operands = option_scan.operands();
    if operands.is_empty() {
        for index in 0..SIGNAL_SLOT_COUNT {
            write_listing(shell, index, print)?;
        }
        // [spec:nsh:req:compat.bash.traps-introspection]
        if shell.options.dialect() == Dialect::Bash {
            for condition in crate::trap::bash::CONDITIONS {
                write_bash_listing(shell, condition, print)?;
            }
        }
        return Ok(Flow::Done((0).into()));
    }
    if print {
        for word in operands {
            if let Some(condition) = bash_condition(shell, word) {
                write_bash_listing(shell, condition, true)?;
                continue;
            }
            let Some(signal) = decode_signal(word, true) else {
                let mut message = b"trap: ".to_vec();
                message.extend_from_slice(word);
                message.extend_from_slice(b": bad trap\n");
                shell.write_output(OutputDestination::Stderr, &message)?;
                return Ok(Flow::Done((1).into()));
            };
            write_listing(shell, signal.index(), true)?;
        }
        return Ok(Flow::Done((0).into()));
    }
    shell.traps.end_subshell_listing();
    if shell.traps.parent_traps_pending {
        clear_traps(shell, None);
    }
    /* `trap SIG...` resets, and `trap ACTION SIG...` sets: the first word
     * is the action unless it is itself a signal, or the only word. */
    let first = operands[0];
    let (mut action, signals) = if operands.len() < 2 || parse_signal_number(first).is_some() {
        (None, operands)
    } else {
        (Some(BString::from(first)), &operands[1..])
    };
    /* One signal-mask guard for the whole command, which is the recorded
     * granularity: `trap 'act' INT TERM HUP` blocks once, not three times.
     * Interrupt deferral remains per word inside the loop. */
    let blocked = crate::signal_inbox::SignalsBlocked::new();
    for word in signals {
        /* The C's `action = savestr(action)` makes the next signal in the
         * list copy the previous copy; copying the argument word each time
         * gives the same bytes and leaves `action` pointing at what the
         * `'-'` test reads. */
        let new_action = selected_action(&mut action);
        // [spec:nsh:req:compat.bash.traps-introspection]
        if let Some(condition) = bash_condition(shell, word) {
            crate::error::with_interrupts_deferred(shell, |shell| {
                crate::trap::bash::set(shell, condition, new_action);
            });
            continue;
        }
        let Some(signal) = decode_signal(word, true) else {
            let mut message = b"trap: ".to_vec();
            message.extend_from_slice(word);
            message.extend_from_slice(b": bad trap\n");
            shell.write_output(OutputDestination::Stderr, &message)?;
            return Ok(Flow::Done((1).into()));
        };
        crate::error::with_interrupts_deferred(shell, |shell| {
            if matches!(new_action, TrapAction::Command(_)) {
                shell.traps.trap_count += 1;
            }
            /* Asked as a `bool` first: the count is a field of the table the
             * question is about, and reading one while writing the other is
             * two borrows of `sh.traps`. */
            let replacing_an_action =
                matches!(shell.traps.action(signal.index()), TrapAction::Command(_));
            if replacing_an_action {
                shell.traps.trap_count -= 1;
            }
            /* The C frees the old action and *then* stores the new one, so the
             * slot is briefly a dangling non-NULL pointer; `onsig` only tests it
             * for NULL, so it reads "a trap is set" throughout. A replace reads
             * the same way and never leaves a stale pointer for it to load --
             * and the presence bit `onsig` reads instead is published by the
             * same call, with signals blocked so the two cannot disagree. */
            drop(shell.traps.set(&blocked, signal.index(), new_action));
            if let SignalSpec::Signal(signal) = signal {
                configure_signal(shell, signal);
            }
        });
    }
    Ok(Flow::Done((0).into()))
}
