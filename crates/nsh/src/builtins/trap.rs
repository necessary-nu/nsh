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
use std::io::Write;

use crate::eval::Flow;
use crate::options::Options;
use crate::trap::{
    NSIG, SignalSpec, TrapAction, clear_traps, decode_signal, decode_signum, setsignal,
};

// [spec:posix:req:builtin.trap.opt-p-suitable-for-reinput]
fn listing_line(signo: usize, action: &TrapAction) -> Vec<u8> {
    let mut line = b"trap -- ".to_vec();
    match action {
        TrapAction::Default => line.push(b'-'),
        TrapAction::Ignore => line.extend_from_slice(b"''"),
        TrapAction::Command(command) => {
            line.extend_from_slice(&crate::mystring::single_quote(BStr::new(
                command.as_slice(),
            )));
        }
    }
    line.push(b' ');
    line.extend_from_slice(crate::signames::signal_names[signo].to_bytes());
    line.push(b'\n');
    line
}

fn write_listing(sh: &mut Shell, signo: usize, include_default: bool) {
    let action = sh.traps.listed_action(signo);
    if !include_default && matches!(action, TrapAction::Default) {
        return;
    }
    let line = listing_line(signo, action);
    let _ = sh.io.stdout().write_all(&line);
}

// [spec:dash:def:trap.trapcmd-fn]
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
pub fn trapcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut print = false;
    let mut opts = Options::new(args);
    while let Some(option) = opts.next(sh, b"p")? {
        print |= option == b'p';
    }
    let ap = opts.operands();
    if ap.is_empty() {
        for index in 0..NSIG {
            write_listing(sh, index, print);
        }
        return Ok(Flow::Done((0).into()));
    }
    if print {
        for word in ap {
            let Some(signal) = decode_signal(word, true) else {
                let mut message = b"trap: ".to_vec();
                message.extend_from_slice(word);
                message.extend_from_slice(b": bad trap\n");
                let _ = sh.io.stderr().write_all(&message);
                return Ok(Flow::Done((1).into()));
            };
            write_listing(sh, signal.index(), true);
        }
        return Ok(Flow::Done((0).into()));
    }
    sh.traps.end_subshell_listing();
    if sh.traps.ptrap != 0 {
        clear_traps(sh, None);
    }
    /* `trap SIG...` resets, and `trap ACTION SIG...` sets: the first word
     * is the action unless it is itself a signal, or the only word. */
    let first = ap[0];
    let (mut action, signals) = if ap.len() < 2 || decode_signum(first).is_some() {
        (None, ap)
    } else {
        (Some(BString::from(first)), &ap[1..])
    };
    /* One signal-mask guard for the whole command, which is the recorded
     * granularity: `trap 'act' INT TERM HUP` blocks once, not three times.
     * Interrupt deferral remains per word inside the loop. */
    let blocked = crate::siginbox::SignalsBlocked::new();
    for word in signals {
        let Some(signal) = decode_signal(word, true) else {
            let mut message = b"trap: ".to_vec();
            message.extend_from_slice(word);
            message.extend_from_slice(b": bad trap\n");
            let _ = sh.io.stderr().write_all(&message);
            return Ok(Flow::Done((1).into()));
        };
        crate::error::with_interrupts_deferred(sh, |sh| {
            /* The C's `action = savestr(action)` makes the next signal in the
             * list copy the previous copy; copying the argument word each time
             * gives the same bytes and leaves `action` pointing at what the
             * `'-'` test reads. */
            let mut newtrap = TrapAction::Default;
            if let Some(text) = &action {
                if text.as_slice() == b"-" {
                    action = None;
                } else if text.is_empty() {
                    newtrap = TrapAction::Ignore;
                } else {
                    sh.traps.trapcnt += 1;
                    newtrap = TrapAction::Command(text.clone());
                }
            }
            /* Asked as a `bool` first: the count is a field of the table the
             * question is about, and reading one while writing the other is
             * two borrows of `sh.traps`. */
            let replacing_an_action =
                matches!(sh.traps.action(signal.index()), TrapAction::Command(_));
            if replacing_an_action {
                sh.traps.trapcnt -= 1;
            }
            /* The C frees the old action and *then* stores the new one, so the
             * slot is briefly a dangling non-NULL pointer; `onsig` only tests it
             * for NULL, so it reads "a trap is set" throughout. A replace reads
             * the same way and never leaves a stale pointer for it to load --
             * and the presence bit `onsig` reads instead is published by the
             * same call, with signals blocked so the two cannot disagree. */
            drop(sh.traps.set(&blocked, signal.index(), newtrap));
            if let SignalSpec::Signal(signal) = signal {
                setsignal(sh, signal);
            }
        });
    }
    Ok(Flow::Done((0).into()))
}
