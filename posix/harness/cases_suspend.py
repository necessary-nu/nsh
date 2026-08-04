"""Suspending a FOREGROUND job from the terminal.

These are the rules that survived the job-control conversion in
cases_jobctl.py. They needed a `^Z` typed at the terminal while the job is
running, which the pty writer could not do: it delivered the whole script
in one payload, and the line discipline acts on a suspend character when
the byte ARRIVES rather than when the shell reads it, so the ^Z fired
before the job it targeted existed.

`Case.pace` fixes that -- the script goes a line at a time, so `sleep 5`
is running by the time the ^Z lands. Everything here therefore drives a
foreground stop the way a user does, which is the one thing
`kill -STOP %1` on a background job could not reach.

The terminal-settings pair turned out to be observable after all. The
earlier reason said it was "not observable from the shell language",
which was true only if you stay inside the shell language: `stty` is a
utility, it reads and writes exactly the termios state these rules are
about, and it is on PATH.
"""

from __future__ import annotations

from model import Case


SUSPEND = "\x1a"  # ^Z


CASES: tuple[Case, ...] = (
    # [spec:posix:req:jobctl.suspended-job-message/test]
    Case(
        id="suspend-foreground-job-message",
        rules=("jobctl.suspended-job-message",),
        mode="interactive",
        pace=0.6,
        # "When a suspended job is created as a result of a foreground job
        # being stopped, it shall be assigned a job number, and an
        # interactive shell shall write ... a message to standard error,
        # formatted as described by the jobs utility (without the -l
        # option) for a suspended job."
        script=(
            "set -m\n"
            "sleep 5\n"
            f"{SUSPEND}\n"
            "jobs\n"
            "kill %1\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("Stopped", "[1]"),
        status="any",
        timeout=30.0,
    ),
    # [spec:posix:req:jobctl.suspend-on-catchable-signal/test]
    Case(
        id="suspend-on-catchable-signal",
        rules=("jobctl.suspend-on-catchable-signal",),
        mode="interactive",
        pace=0.6,
        # "If a process that the shell is waiting for is part of a
        # foreground job that was started as a foreground job and is
        # stopped by a catchable signal (SIGTSTP, SIGTTIN, or SIGTTOU) ...
        # the shell shall ... create a suspended job". ^Z is SIGTSTP from
        # the terminal, which is the catchable case; SIGSTOP is covered
        # separately by jobctl-suspend-on-sigstop.
        script=(
            "set -m\n"
            "sleep 5\n"
            f"{SUSPEND}\n"
            "jobs | grep -c Stopped\n"
            "kill %1\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("1",),
        status="any",
        timeout=30.0,
    ),
    # [spec:posix:req:jobctl.background-job-brought-to-foreground/test]
    Case(
        id="suspend-background-job-brought-to-foreground",
        rules=("jobctl.background-job-brought-to-foreground",),
        mode="interactive",
        pace=0.6,
        # "A background job ... can be brought into the foreground by means
        # of the fg utility ...; in this case the entire job shall become a
        # single foreground job. If a process that the shell subsequently
        # waits for is part of this foreground job and is stopped by a
        # signal, the entire job shall become a suspended job."
        #
        # So: start it in the background, fg it, stop it from the terminal,
        # and it must come back as a suspended job.
        script=(
            "set -m\n"
            "sleep 5 &\n"
            "fg\n"
            f"{SUSPEND}\n"
            "jobs\n"
            "kill %1\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("Stopped",),
        status="any",
        timeout=30.0,
    ),
    # [spec:posix:req:jobctl.save-terminal-settings/test]
    Case(
        id="suspend-shell-restores-its-own-terminal-settings",
        rules=("jobctl.save-terminal-settings",),
        mode="interactive",
        pace=0.7,
        # "When a suspended job is created as a result of a foreground job
        # being stopped, if the shell is interactive, it shall save the
        # terminal settings before changing them to the settings it needs
        # to read further commands."
        #
        # Observed through stty. What the rule requires is that the shell
        # move the terminal to "the settings it needs to read further
        # commands", so the test runs a job that takes the terminal out of
        # canonical mode -- the mode a shell needs to read a line -- stops
        # it, and checks the shell put canonical mode back.
        #
        # A first draft compared `stty -g` before and after for equality.
        # That asserts more than the standard does: the shell owes its own
        # working settings, not a byte-identical restoration of whatever
        # was there beforehand. dash reported restored=no and was entitled
        # to. (It also hung, because `exit` with a stopped job only warns
        # the first time -- hence the second one below.)
        #
        # It still fails, and this time dash is not entitled to it: the
        # shell is left in the job's non-canonical mode. dash calls
        # tcsetattr nowhere in the tree -- the single tcgetattr, at
        # input.c:138, is a tty test -- so it never moves the terminal to
        # the settings it needs. Recorded as a real non-conformance that
        # the port reproduces.
        script=(
            "set -m\n"
            "sh -c 'stty -icanon min 1 time 0; sleep 5'\n"
            f"{SUSPEND}\n"
            'case "$(stty -a)" in *-icanon*) printf \'canonical=no\\n\' ;; '
            "*) printf 'canonical=yes\\n' ;; esac\n"
            "kill %1\n"
            "exit\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("canonical=yes",),
        status="any",
        timeout=30.0,
    ),
    # [spec:posix:req:jobctl.fg-terminal-settings-restore/test]
    Case(
        id="suspend-fg-restores-job-terminal-settings",
        rules=("jobctl.fg-terminal-settings-restore",),
        mode="interactive",
        pace=0.7,
        # "If the fg utility is used from an interactive shell to bring
        # into the foreground a suspended job that was created from a
        # foreground job, before it sends the SIGCONT signal the fg utility
        # shall restore the terminal settings to [those] in effect when the
        # job was stopped."
        #
        # The job records the settings it set for itself, is stopped, and
        # records them again once fg resumes it. If fg restored the
        # terminal before SIGCONT, the two recordings match.
        #
        # An earlier draft only asserted that the first recording existed,
        # which dash passed while doing none of this. This version is
        # stronger but still not decisive against dash specifically, and
        # the reason is worth stating rather than leaving as a green tick:
        # dash calls tcsetattr nowhere in the tree (the single tcgetattr,
        # at input.c:138, is a tty test), so it never disturbs the
        # terminal between the stop and the resume -- and the job's
        # settings therefore survive by inaction rather than by fg
        # restoring them. The case does discriminate against a shell that
        # restores the WRONG settings, which is the failure mode worth
        # catching; it cannot separate "restored correctly" from "never
        # touched". The companion rule
        # jobctl.save-terminal-settings, which asserts what the shell owes
        # ITSELF after a stop, is the one that pins dash down, and it
        # fails.
        script=(
            "set -m\n"
            "sh -c 'stty -echo; stty -g > jobsettings; sleep 4; stty -g > resumed'\n"
            f"{SUSPEND}\n"
            "fg\n"
            "sleep 1\n"
            "cmp -s jobsettings resumed && printf 'restored=yes\\n' "
            "|| printf 'restored=no\\n'\n"
            "exit\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("restored=yes",),
        status="any",
        timeout=40.0,
    ),
    # [spec:posix:req:jobctl.foreground-job/test]
    Case(
        id="suspend-sequential-list-is-one-foreground-job",
        rules=("jobctl.foreground-job",),
        mode="interactive",
        pace=0.6,
        # "For a list consisting of one or more sequentially executed
        # AND-OR lists ... the whole list shall form a single foreground
        # job up until the sequentially executed AND-OR lists have all
        # completed execution."
        #
        # Stopping partway through must therefore yield ONE job, not one
        # per AND-OR list -- `jobs` printing a single line is the assertion.
        script=(
            "set -m\n"
            "sleep 4; sleep 4\n"
            f"{SUSPEND}\n"
            "jobs | wc -l\n"
            "kill %1\n"
            "exit\n"
        ),
        environment={"PS1": "", "PS2": ""},
        stdout=None,
        stdout_contains=("1",),
        status="any",
        timeout=30.0,
    ),
)
