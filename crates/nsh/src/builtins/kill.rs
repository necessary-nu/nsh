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
use std::io::Write;

use crate::eval::Flow;
use crate::jobs::{JobId, getjob, ps_pid};
use crate::output::Dest;
use crate::trap::SignalSpec;

// [spec:nsh:def:idiom.job-control-model]

fn process_target(value: i32) -> nsh_platform::ProcessTarget {
    match value {
        1.. => nsh_platform::ProcessTarget::Process(
            nsh_platform::ProcessId::new(value as u32).expect("a positive PID is nonzero"),
        ),
        0 => nsh_platform::ProcessTarget::CurrentProcessGroup,
        -1 => nsh_platform::ProcessTarget::AllProcesses,
        _ => nsh_platform::ProcessTarget::ProcessGroup(
            nsh_platform::ProcessGroupId::new(value.unsigned_abs())
                .expect("a negative process-group operand has nonzero magnitude"),
        ),
    }
}

// [spec:dash:def:jobs.killcmd-fn]
// [spec:dash:sem:jobs.killcmd-fn]
// [spec:posix:syn:builtin.kill.synopsis]
// [spec:posix:syn:builtin.kill.synopsis-xsi]
// [spec:posix:req:builtin.kill.send-signal]
// [spec:posix:req:builtin.kill.utility-syntax-guidelines]
// [spec:posix:req:builtin.kill.option-l]
// [spec:posix:req:builtin.kill.option-s]
// [spec:posix:req:builtin.kill.option-signal-name]
// [spec:posix:req:builtin.kill.option-signal-number]
// [spec:posix:req:builtin.kill.negative-first-argument]
// [spec:posix:req:builtin.kill.operand-pid-number]
// [spec:posix:def:builtin.kill.operand-pid-job-id]
// [spec:posix:def:builtin.kill.operand-exit-status]
// [spec:posix:req:builtin.kill.env-vars]
// [spec:posix:sem:builtin.kill.env-nlspath]
// [spec:posix:req:builtin.kill.stdout-unused-without-l]
// [spec:posix:req:builtin.kill.stdout-signal-list-format]
// [spec:posix:req:builtin.kill.stdout-exit-status-format]
// [spec:posix:req:builtin.kill.stderr]
// [spec:posix:req:builtin.kill.interfaces]
// [spec:posix:req:builtin.kill.exit-status]
pub fn killcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    /* the `usage:` label is a backward goto whose body only raises, so it
     * is reproduced as two returns of the same message. */
    const USAGE: &[u8] =
        b"Usage: kill [-s sigspec | -signum | -sigspec] [pid | job]... or\nkill -l [exitstatus]";
    let mut signal = None;
    let mut list = false;
    let mut jp: JobId;

    if args.len() <= 1 {
        // usage:
        return Err(sh.diagnostics().sh_error_value(USAGE));
    }

    let mut opts = crate::options::Options::new(args);
    /* `-9` and `-TERM` are a signal, not an option, so the option scan
     * runs only once the signal reading has failed -- and then from the
     * same word, which is where `Options` starts. */
    let mut operands: &[&BStr] = &args[1..];
    if args[1].first() == Some(&b'-') {
        signal = crate::trap::decode_signal(BStr::new(&args[1][1..]), false);
        if signal.is_none() {
            while let Some(c) = opts.next(&mut sh.diagnostics(), b"ls:")? {
                match c {
                    b's' => {
                        let name = opts.arg();
                        signal = crate::trap::decode_signal(name, false);
                        if signal.is_none() {
                            let mut message = b"invalid signal number or name: ".to_vec();
                            message.extend_from_slice(name);
                            return Err(sh.diagnostics().sh_error_value(&message));
                        }
                    }
                    _ /* default, 'l' */ => {
                        list = true;
                    }
                }
            }
            operands = opts.operands();
        } else {
            operands = &args[2..];
        }
    }

    if !list && signal.is_none() {
        signal = Some(SignalSpec::Signal(
            nsh_platform::termination_signal().into(),
        ));
    }

    if (signal.is_none() || operands.is_empty()) != list {
        // goto usage
        return Err(sh.diagnostics().sh_error_value(USAGE));
    }

    if list {
        let Some(status) = operands.first() else {
            let _ = sh.io.get(Dest::Stdout).write_all(b"0\n");
            for index in 1..crate::signames::NSIG {
                let mut record = crate::signames::signal_names[index].to_bytes().to_vec();
                record.push(b'\n');
                let _ = sh.io.get(Dest::Stdout).write_all(&record);
            }
            return Ok(Flow::Done((0).into()));
        };
        let number = crate::number::parse_nonnegative(&mut sh.diagnostics(), status)?;
        let number = if number > 128 { number - 128 } else { number };
        if let Some(signal) = crate::status::Signal::from_number(number)
            .filter(|signal| signal.number() < crate::signames::NSIG as i32)
        {
            let mut record = crate::signames::signal_names[signal.number() as usize]
                .to_bytes()
                .to_vec();
            record.push(b'\n');
            let _ = sh.io.get(Dest::Stdout).write_all(&record);
        } else {
            let mut message = b"invalid signal number or exit status: ".to_vec();
            message.extend_from_slice(status);
            return Err(sh.diagnostics().sh_error_value(&message));
        }
        return Ok(Flow::Done((0).into()));
    }

    let mut failed = false;
    for spec in operands {
        let target = if spec.first() == Some(&b'%') {
            // [spec:nsh:req:compat.smoosh.interactive-job-prompt]
            // A `%job` names a process group, not the first process that
            // happened to be recorded for a background command. Requiring
            // a job-control job here prevents the latter from silently
            // becoming the former while monitor mode is disabled.
            jp = getjob(sh, Some(spec), true)?;
            let Some(leader) = ps_pid(sh, jp, 0) else {
                sh.diagnostics().sh_warnx(b"No such process\n");
                failed = true;
                continue;
            };
            nsh_platform::ProcessTarget::ProcessGroup(nsh_platform::ProcessGroupId::from_leader(
                leader,
            ))
        } else {
            let value = if spec.first() == Some(&b'-') {
                -crate::number::parse_nonnegative(&mut sh.diagnostics(), BStr::new(&spec[1..]))?
            } else {
                crate::number::parse_nonnegative(&mut sh.diagnostics(), spec)?
            };
            process_target(value)
        };
        let request = match signal.expect("kill validates a signal before delivery") {
            SignalSpec::Exit => nsh_platform::SignalRequest::Probe,
            SignalSpec::Signal(signal) => nsh_platform::SignalRequest::Deliver(signal.platform()),
        };
        if let Err(error) = nsh_platform::send_signal(target, request) {
            let mut message = sh.locale.error_message(&error).into_bytes();
            message.push(b'\n');
            sh.diagnostics().sh_warnx(&message);
            failed = true;
        }
    }

    Ok(Flow::Done(i32::from(failed).into()))
}
