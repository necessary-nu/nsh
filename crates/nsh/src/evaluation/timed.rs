//! `time [-p] pipeline`.
//!
//! A reserved word rather than a utility, which is the whole point of it:
//! `time` can prefix a built-in, a function or a whole pipeline, none of
//! which an external `time(1)` can see. POSIX reserves the word
//! (XCU 2.4), so it is grammar in both dialects.
//!
//! What is reported is the *elapsed* time of the pipeline and the
//! processor time charged to it. Processor time is read from the same
//! place `times` reads: the shell's own usage plus its reaped children's,
//! differenced across the command, so a pipeline that forks is accounted
//! for once its members have been waited on.

use super::{EvaluationContext, Flow, evaluate_tree, record_command_line};
use crate::context::Shell;
use crate::error::Error;
use crate::nodes::TimedCommand;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

/// The three durations one report is made of, in seconds.
#[derive(Clone, Copy)]
struct Durations {
    real: f64,
    user: f64,
    system: f64,
}

impl Durations {
    /// Read now, from the wall clock and the process accounting.
    ///
    /// A child's usage is only charged to the parent once it has been
    /// reaped, so `children_*` is included here and differenced later --
    /// that is what makes `time sleep 1` report the sleep rather than the
    /// shell's own idleness.
    fn now() -> Self {
        let times = nsh_platform::process_times();
        Self {
            real: nsh_platform::facts::monotonic_seconds(),
            user: times.user + times.children_user,
            system: times.system + times.children_system,
        }
    }

    /// How much of each elapsed between `self` and `later`.
    ///
    /// Clamped at zero: a monotonic clock does not go backwards, but the
    /// accounting clock has a coarse tick and can appear to.
    fn since(self, later: Self) -> Self {
        Self {
            real: (later.real - self.real).max(0.0),
            user: (later.user - self.user).max(0.0),
            system: (later.system - self.system).max(0.0),
        }
    }
}

// [spec:posix:req:token.reserved-word-time]
// [spec:nsh:req:compat.bash.select-time-grammar]
pub(super) fn evaluate_timed(
    shell: &mut Shell,
    command: &TimedCommand,
    context: EvaluationContext,
) -> Result<Flow, Error> {
    record_command_line(shell, command.line.get());

    let started = Durations::now();
    /* The pipeline's own outcome is what `time` answers with, including a
     * control flow that leaves the loop or the function: the report is
     * written on the way past either way, because a `return` inside a
     * timed pipeline still took the time it took. */
    /* `tested_only` clears `EV_EXIT`: the last command of a script may
     * otherwise replace this process image instead of returning, and then
     * there is nobody left to write the report. Bash cannot take that
     * shortcut under `time` either. */
    let outcome = match &command.command {
        Some(node) => evaluate_tree(shell, Some(node.as_ref()), context.tested_only()),
        None => Ok(Flow::Done(ExitStatus::SUCCESS)),
    };
    let report = started.since(Durations::now());

    let text = if command.posix_format {
        posix_report(report)
    } else {
        bash_report(report)
    };
    /* The report goes to standard error, so a timed pipeline's own output
     * stays usable in a substitution. A failure to write it is not the
     * pipeline's failure and must not replace the pipeline's status. */
    let written = shell.write_output(OutputDestination::Stderr, text.as_bytes());
    match outcome {
        Ok(flow) => {
            written?;
            Ok(flow)
        }
        Err(error) => Err(error),
    }
}

/// `real 0.05` -- seconds to two places, as `time -p` specifies.
fn posix_report(report: Durations) -> String {
    format!(
        "real {:.2}\nuser {:.2}\nsys {:.2}\n",
        report.real, report.user, report.system,
    )
}

/// `\nreal\t0m0.051s\n…` -- Bash's default `TIMEFORMAT`, rendered
/// directly.
///
/// `TIMEFORMAT` itself is not read, and the reason has moved:
/// [`dec:nsh:no-format-interpreters`] is obsolesced, and its successor
/// [`dec:nsh:printf-is-parsed-not-interpreted`] is what still forbids
/// this. That decision sanctions parsing a `%` conversion at runtime for
/// `printf` alone -- "scoped to `builtins::printf` and travels no
/// further" -- and says in as many words that nothing outside it may
/// format a value by a pattern chosen at runtime. A variable holding a
/// report layout is exactly such a pattern, so the two default formats
/// are written with `write!` at the site that knows the types.
fn bash_report(report: Durations) -> String {
    format!(
        "\nreal\t{}\nuser\t{}\nsys\t{}\n",
        minutes_and_seconds(report.real),
        minutes_and_seconds(report.user),
        minutes_and_seconds(report.system),
    )
}

fn minutes_and_seconds(seconds: f64) -> String {
    let minutes = (seconds / 60.0) as u64;
    format!("{}m{:.3}s", minutes, seconds - (minutes as f64) * 60.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_report_carries_minutes_over() {
        assert_eq!(minutes_and_seconds(0.0), "0m0.000s");
        assert_eq!(minutes_and_seconds(0.0512), "0m0.051s");
        assert_eq!(minutes_and_seconds(61.5), "1m1.500s");
        assert_eq!(minutes_and_seconds(3600.0), "60m0.000s");
    }

    #[test]
    fn the_two_formats_differ_as_specified() {
        let report = Durations {
            real: 1.5,
            user: 0.25,
            system: 0.0,
        };
        assert_eq!(posix_report(report), "real 1.50\nuser 0.25\nsys 0.00\n");
        assert_eq!(
            bash_report(report),
            "\nreal\t0m1.500s\nuser\t0m0.250s\nsys\t0m0.000s\n",
        );
    }
}
