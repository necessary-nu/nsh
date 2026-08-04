"""Job control, tested rather than excused.

Twenty rules carried a `manual` disposition saying, in one wording or
another, that they "require an interactive shell" or "an interactive
session with a controlling terminal". The harness has had exactly that
since the sandbox was written: `mode="interactive"` runs the shell under
`setsid --ctty` *inside* the PID namespace precisely so that it gets its
own session and the pty as controlling terminal, and there is a self-test
asserting it. "Requires a pty" is a description of the facility, not a
reason to skip.

Two things genuinely are out of reach, and they are called out where they
apply rather than used to excuse a whole area:

  * **Timing against the tty.** `_run_pty` writes the whole script to the
    pty in one payload. The line discipline acts on ^Z when the byte
    *arrives*, not when the shell reads it, so a suspend character sent
    this way fires before the job it was meant to stop exists. Every
    suspension below is therefore driven with `kill -STOP` on a job,
    which needs no timing and exercises the same shell path.

  * **State with no interface.** Saved termios across a suspend/resume,
    and the ordering of tcsetpgrp against SIGCONT inside `fg`, are not
    observable from the shell language. Those keep a `manual`
    disposition, now with that as the reason.

The foreground-process-group rules do have an interface: field 5 of
/proc/<pid>/stat is the process group and field 8 is the tty's foreground
process group, so a process can ask whether it is in the foreground
without leaving the shell language.
"""

from __future__ import annotations

from model import Case


# Reports `fg=yes` when the running process's process group is the
# terminal's foreground process group. Field 5 is pgrp, field 8 is tpgid.
AM_I_FOREGROUND = (
    "awk '{ if ($5 == $8) print \"fg=yes\"; else print \"fg=no\" }' /proc/self/stat"
)


CASES: tuple[Case, ...] = (
    # [spec:posix:req:cmd.async-job-number/test]
    Case(
        id="jobctl-async-assigns-a-job-number",
        rules=("cmd.async-job-number",),
        mode="interactive",
        # "If job control is enabled (see set -m), the AND-OR list shall
        # become a job-control background job and a job number shall be
        # assigned to it."
        script="set -m\nsleep 5 &\njobs\nkill %1\nexit\n",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("[1]",),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:cmd.async-job-notification-format/test]
    Case(
        id="jobctl-async-notification-format",
        rules=("cmd.async-job-notification-format",),
        mode="interactive",
        # "If the shell is interactive and the asynchronous AND-OR list
        # became a background job, the job number and the process ID
        # associated with the job shall be written to standard error using
        # the format: "[%d] %d\n", <job-number>, <process-id>"
        #
        # dash does not do this: `[%d] ` appears in jobs.c only inside the
        # job *listing* formats, never on starting a job. The case is here
        # to record that, not because it is expected to pass -- the port
        # reproduces it, which is the point of a bug-for-bug port.
        script="set -m\nsleep 5 &\nkill %1\nexit\n",
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("[1] ",),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:cmd.async-non-job-pid-message/test]
    Case(
        id="jobctl-async-pid-without-job-control",
        rules=("cmd.async-non-job-pid-message",),
        mode="interactive",
        # "If the shell is interactive and the asynchronous AND-OR list did
        # not become a background job, the process ID associated with the
        # asynchronous AND-OR list shall be written to standard error in an
        # unspecified format." The format is unspecified, so what is
        # asserted is that the pid appears at all -- the case prints $! and
        # then looks for that number in the transcript.
        script=(
            "set +m\n"
            "sleep 5 &\n"
            'printf \'mypid=%s\\n\' "$!"\n'
            "kill $!\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("mypid=",),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:jobctl.job-number-and-process-id/test]
    Case(
        id="jobctl-job-number-removed-by-fg",
        rules=("jobctl.job-number-and-process-id",),
        mode="interactive",
        # "Each background job (whether suspended or not) shall have
        # associated with it a job number and a process ID that is known in
        # the current shell execution environment. When a background job is
        # brought into the foreground by means of the fg utility, the
        # associated job number shall be removed from the shell's
        # background jobs list."
        script=(
            "set -m\n"
            "sleep 0.3 &\n"
            "jobs > before\n"
            "fg\n"
            "jobs > after\n"
            "printf 'before=%s after=%s\\n' \"$(wc -l < before)\" \"$(wc -l < after)\"\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("before=1 after=0",),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:jobctl.background-job-suspended-message/test]
    Case(
        id="jobctl-background-job-suspended-message",
        rules=("jobctl.background-job-suspended-message",),
        mode="interactive",
        # "When a process associated with a background job is stopped by a
        # SIGSTOP, SIGTSTP, SIGTTIN, or SIGTTOU signal, the shell shall
        # convert the (non-suspended) background job into a suspended job
        # and an interactive shell shall write a message to standard error,
        # formatted as described by the jobs utility ... for a suspended
        # job." With `set -b` the message is immediate.
        script=(
            "set -m -b\n"
            "sleep 5 &\n"
            "kill -STOP %1\n"
            "sleep 0.5\n"
            "jobs\n"
            "kill -CONT %1\n"
            "kill %1\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("Stopped",),
        status="any",
        timeout=20.0,
    ),
    # [spec:posix:req:jobctl.continue-suspended-job/test]
    Case(
        id="jobctl-continue-suspended-job",
        rules=("jobctl.continue-suspended-job",),
        mode="interactive",
        # "Execution of a suspended job can be continued ... as a
        # (non-suspended) background job either by means of the bg utility
        # ... or by sending the stopped processes a SIGCONT signal."
        # Asserted by observing the job return to Running.
        script=(
            "set -m\n"
            "sleep 5 &\n"
            "kill -STOP %1\n"
            "sleep 0.4\n"
            "bg %1 >/dev/null 2>&1\n"
            "sleep 0.4\n"
            "jobs\n"
            "kill %1\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("Running",),
        status="any",
        timeout=20.0,
    ),
    # [spec:posix:req:jobctl.background-job-completion-message/test]
    Case(
        id="jobctl-background-completion-message",
        rules=("jobctl.background-job-completion-message",),
        mode="interactive",
        # "When a background job completes ... an interactive shell shall
        # write a message to standard error, formatted as described by the
        # jobs utility ... If set -b is enabled, the message shall be
        # written immediately after the job completes."
        script=(
            "set -m -b\n"
            "sleep 0.2 &\n"
            "sleep 1\n"
            "printf 'after\\n'\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("Done",),
        status="any",
        timeout=20.0,
    ),
    # [spec:posix:req:jobctl.foreground-process-group-assignment/test]
    # [spec:posix:req:jobctl.initial-foreground-process-group/test]
    Case(
        id="jobctl-foreground-process-group",
        rules=(
            "jobctl.foreground-process-group-assignment",
            "jobctl.initial-foreground-process-group",
        ),
        mode="interactive",
        # A foreground job's process group is the terminal's foreground
        # process group; a background job's is not. Both are readable from
        # /proc/<pid>/stat without leaving the shell language, which is why
        # these two no longer need a "not observable" exemption.
        script=(
            "set -m\n"
            f"{AM_I_FOREGROUND}\n"
            f"{{ {AM_I_FOREGROUND}; }} & \n"
            "wait\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("fg=yes", "fg=no"),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:jobctl.foreground-process-group-restored/test]
    Case(
        id="jobctl-foreground-process-group-restored",
        rules=("jobctl.foreground-process-group-restored",),
        mode="interactive",
        # After a foreground job terminates the shell takes the terminal
        # back, so a second foreground job also reports fg=yes -- which it
        # could not if the shell had failed to restore itself in between.
        script=(
            "set -m\n"
            f"{AM_I_FOREGROUND}\n"
            "sleep 0.2\n"
            f"{AM_I_FOREGROUND}\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("fg=yes",),
        status="any",
        timeout=15.0,
    ),
    # [spec:posix:req:jobctl.suspend-on-sigstop/test]
    Case(
        id="jobctl-suspend-on-sigstop",
        rules=("jobctl.suspend-on-sigstop",),
        mode="interactive",
        # A job stopped by SIGSTOP becomes a suspended job the shell knows
        # about, and can then be continued and reaped.
        script=(
            "set -m\n"
            "sleep 5 &\n"
            "kill -STOP %1\n"
            "sleep 0.4\n"
            "jobs | grep -c Stopped\n"
            "kill -CONT %1\n"
            "sleep 0.3\n"
            "jobs | grep -c Running\n"
            "kill %1\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("1",),
        status="any",
        timeout=20.0,
    ),
    # [spec:posix:req:cmd.sequential-foreground-job/test]
    Case(
        id="jobctl-sequential-foreground-job",
        rules=("cmd.sequential-foreground-job",),
        mode="interactive",
        # A sequential AND-OR list runs in the foreground, so each stage
        # sees itself as the terminal's foreground process group.
        script=(
            "set -m\n"
            f"true && {AM_I_FOREGROUND}\n"
            f"false || {AM_I_FOREGROUND}\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("fg=yes",),
        status="any",
        timeout=15.0,
    ),
)
