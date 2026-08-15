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
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io::Write;

use crate::error::{INTOFF, INTON};
use crate::eval::Flow;
use crate::options::Options;
use crate::trap::{NSIG, cbytes, clear_traps, decode_signal, decode_signum, setsignal};

// [spec:dash:def:trap.trapcmd-fn]
// [spec:dash:sem:trap.trapcmd-fn]
pub unsafe fn trapcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut signo: c_int;

    let mut opts = Options::new(args);
    opts.next(b"")?;
    let ap = opts.operands();
    if ap.is_empty() {
        signo = 0;
        while signo < NSIG as c_int {
            if let Some(t) = sh.traps.action(signo as usize) {
                let t = cbytes(t);
                let quoted = crate::mystring::single_quote(t.as_ptr() as *const c_char);
                let mut line = b"trap -- ".to_vec();
                line.extend_from_slice(CStr::from_ptr(quoted).to_bytes());
                line.push(b' ');
                line.extend_from_slice(
                    CStr::from_ptr(crate::signames::signal_names[signo as usize].as_ptr())
                        .to_bytes(),
                );
                line.push(b'\n');
                let _ = (*crate::output::stdout()).write_all(&line);
            }
            signo += 1;
        }
        return Ok(Flow::Done(0));
    }
    if sh.traps.ptrap != 0 {
        clear_traps(sh, None);
    }
    /* `trap SIG...` resets, and `trap ACTION SIG...` sets: the first word
     * is the action unless it is itself a signal, or the only word. */
    let first = crate::shell::cstring(ap[0]);
    let (mut action, signals) = if ap.len() < 2 || decode_signum(first.as_ptr()) >= 0 {
        (None, ap)
    } else {
        (Some(first), &ap[1..])
    };
    /* One guard for the whole command, which is the recorded granularity:
     * `trap 'act' INT TERM HUP` blocks once, not three times. It sits
     * outside the INTOFF/INTON pair below because that pair is per *word*
     * -- the design note read it as per-command, and the region it names
     * is this loop rather than that bracket. */
    let blocked = crate::siginbox::SignalsBlocked::new();
    for word in signals {
        let word = crate::shell::cstring(word);
        signo = decode_signal(word.as_ptr(), 0);
        if signo < 0 {
            let mut message = b"trap: ".to_vec();
            message.extend_from_slice(word.as_bytes());
            message.extend_from_slice(b": bad trap\n");
            let _ = (*crate::output::stderr()).write_all(&message);
            return Ok(Flow::Done(1));
        }
        INTOFF();
        /* The C's `action = savestr(action)` makes the next signal in the
         * list copy the previous copy; copying the argument word each time
         * gives the same bytes and leaves `action` pointing at what the
         * `'-'` test reads. */
        let mut newtrap: Option<BString> = None;
        if let Some(text) = &action {
            if text.as_bytes() == b"-" {
                action = None;
            } else {
                if !text.as_bytes().is_empty() {
                    sh.traps.trapcnt += 1;
                }
                newtrap = Some(BString::from(text.as_bytes()));
            }
        }
        /* Asked as a `bool` first: the count is a field of the table the
         * question is about, and reading one while writing the other is
         * two borrows of `sh.traps`. */
        let replacing_an_action = sh
            .traps
            .action(signo as usize)
            .map_or(false, |old| !old.is_empty());
        if replacing_an_action {
            sh.traps.trapcnt -= 1;
        }
        /* The C frees the old action and *then* stores the new one, so the
         * slot is briefly a dangling non-NULL pointer; `onsig` only tests it
         * for NULL, so it reads "a trap is set" throughout. A replace reads
         * the same way and never leaves a stale pointer for it to load --
         * and the presence bit `onsig` reads instead is published by the
         * same call, with signals blocked so the two cannot disagree. */
        drop(sh.traps.set(&blocked, signo as usize, newtrap));
        if signo != 0 {
            setsignal(sh, signo);
        }
        INTON();
    }
    Ok(Flow::Done(0))
}
