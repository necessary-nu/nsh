//! `kill`.
//!
//! Port of `killcmd` from `src/jobs.c`. The signal table is
//! `crate::signames` and decoding a name is `crate::trap`'s, which the
//! `trap` builtin shares; what is here is the argument grammar, and it is
//! the awkward one -- `-9` and `-TERM` are a signal where every other
//! builtin would read an option.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ffi::{c_int};
use std::io::Write;

use crate::eval::Flow;
use crate::jobs::{getjob, ps_pid};
use crate::output::Dest;

// [spec:dash:def:jobs.killcmd-fn]
// [spec:dash:sem:jobs.killcmd-fn]
pub fn killcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    /* the `usage:` label is a backward goto whose body only raises, so it
     * is reproduced as two returns of the same message. */
    const USAGE: &[u8] =
        b"Usage: kill [-s sigspec | -signum | -sigspec] [pid | job]... or\nkill -l [exitstatus]\0";
    let mut signo: c_int = -1;
    let mut list: c_int = 0;
    let mut i: c_int;
    let mut pid: i32;
    let mut jp: usize;

    if args.len() <= 1 {
        // usage:
        return Err(sh.sh_error_value(&USAGE[..USAGE.len() - 1]));
    }

    let mut opts = crate::options::Options::new(args);
    /* `-9` and `-TERM` are a signal, not an option, so the option scan
     * runs only once the signal reading has failed -- and then from the
     * same word, which is where `Options` starts. */
    let mut operands: &[&BStr] = &args[1..];
    if args[1].first() == Some(&b'-') {
        signo = crate::trap::decode_signal(BStr::new(&args[1][1..]), 1);
        if signo < 0 {
            while let Some(c) = opts.next(sh, b"ls:")? {
                match c {
                    b's' => {
                        let name = opts.arg();
                        signo = crate::trap::decode_signal(name, 1);
                        if signo < 0 {
                            let mut message = b"invalid signal number or name: ".to_vec();
                            message.extend_from_slice(name);
                            return Err(sh.sh_error_value(&message));
                        }
                    }
                    /* `default:` (DEBUG: abort()) falls through into 'l' */
                    _ /* default, 'l' */ => {
                        list = 1;
                    }
                }
            }
            operands = opts.operands();
        } else {
            operands = &args[2..];
        }
    }

    if list == 0 && signo < 0 {
        signo = nsh_platform::termination_signal();
    }

    if (((signo < 0 || operands.is_empty()) as c_int) ^ list) != 0 {
        // goto usage
        return Err(sh.sh_error_value(&USAGE[..USAGE.len() - 1]));
    }

    if list != 0 {
        let Some(status) = operands.first() else {
            let _ = sh.io.get(Dest::Stdout).write_all(b"0\n");
            let mut i = 1;
            while i < crate::signames::NSIG as c_int {
                let mut record = crate::signames::signal_names[i as usize]
                    .to_bytes()
                    .to_vec();
                record.push(b'\n');
                let _ = sh.io.get(Dest::Stdout).write_all(&record);
                i += 1;
            }
            return Ok(Flow::Done(0));
        };
        signo = crate::mystring::number(sh, status)?;
        if signo > 128 {
            signo -= 128;
        }
        if 0 < signo && signo < crate::signames::NSIG as c_int {
            let mut record = crate::signames::signal_names[signo as usize]
                .to_bytes()
                .to_vec();
            record.push(b'\n');
            let _ = sh.io.get(Dest::Stdout).write_all(&record);
        } else {
            let mut message = b"invalid signal number or exit status: ".to_vec();
            message.extend_from_slice(status);
            return Err(sh.sh_error_value(&message));
        }
        return Ok(Flow::Done(0));
    }

    i = 0;
    for spec in operands {
        if spec.first() == Some(&b'%') {
            jp = getjob(sh, Some(spec), 0)?;
            pid = -ps_pid(sh, jp, 0);
        } else {
            pid = if spec.first() == Some(&b'-') {
                -crate::mystring::number(sh, BStr::new(&spec[1..]))?
            } else {
                crate::mystring::number(sh, spec)?
            };
        }
        if let Err(error) = nsh_platform::send_signal(pid, signo) {
            let mut message = sh.locale.error_message(&error).into_bytes();
            message.push(b'\n');
            sh.sh_warnx(&message);
            i = 1;
        }
    }

    Ok(Flow::Done(i))
}
