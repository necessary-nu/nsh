#!/usr/bin/env python3
"""Interactive differential harness, contained.

dsdiff.sh drives both shells with -c or on a pipe, so it never reaches the
interactive paths: prompting, setinteractive, job control (which needs a
controlling terminal), the interactive branch of cmdloop, and -- now that
the port has a real line editor -- el_gets.

Two things this fixes over the previous version:

  * Every shell runs inside the same PID namespace the rest of the harness
    uses. The old pty harness exec'd the shells directly, which left it as
    the one place a test case could still reach the login session.

  * Output is compared with ANSI control sequences stripped. libedit and
    rustyline drive the terminal differently -- cursor moves, line clears,
    synchronized-output markers -- and none of that is shell behaviour.
    What is compared is the text the user would see.
"""
import os
import pty
import re
import select
import signal
import subprocess
import sys
import time

ROOT = os.environ.get("DASH_ROOT") or os.path.dirname(os.path.dirname(os.path.dirname(os.path.abspath(__file__))))
REF = os.environ.get("REF", f"{ROOT}/tests/.build/ref/src/dash")
PORT = os.environ.get("PORT", f"{ROOT}/target/debug/nsh")
SANDBOX = os.environ.get("DS_SANDBOX", "sandbox")


def sandbox_argv(workdir, shell, args):
    """The same containment as sandboxed.sh, minus --new-session: these
    tests need the pty to stay the controlling terminal."""
    return [
        SANDBOX, "--quiet",
        "--unshare", "all",
        "--die-with-parent",
        "--bind", "/:/:ro",
        "--dev", "/dev",
        "--proc", "/proc",
        "--bind", f"{workdir}:{workdir}",
        "--chdir", workdir,
        "--setenv", "TMPDIR", workdir,
        "--limit", "nproc=64",
        "--", shell, *args,
    ]


def run(shell, args, lines, workdir, timeout=10.0, settle=0.5):
    pid, fd = pty.fork()
    if pid == 0:
        os.environ["PS1"] = "$ "
        os.environ["PS2"] = "> "
        os.environ["TERM"] = "xterm"
        # Python ignores SIGPIPE, and a disposition survives exec, so
        # without this both shells inherit "ignore" from the harness and
        # a shell that gets SIGPIPE wrong looks correct. See the note in
        # sandboxed.sh -- this is the same leak, and it hid a real one.
        for sig in (signal.SIGPIPE, signal.SIGINT, signal.SIGQUIT, signal.SIGTERM,
                    signal.SIGHUP, signal.SIGTSTP, signal.SIGTTIN, signal.SIGTTOU):
            signal.signal(sig, signal.SIG_DFL)
        argv = sandbox_argv(workdir, shell, args)
        try:
            os.execvp(argv[0], argv)
        except Exception:
            os._exit(127)

    out = bytearray()
    deadline = time.time() + timeout
    i = 0
    time.sleep(0.4)
    try:
        while time.time() < deadline:
            r, w, _ = select.select([fd], [fd], [], 0.3)
            if w and i < len(lines):
                line = lines[i]
                # A bare control character is sent raw, with no Return:
                # "\x03" is how a user actually interrupts, and it is the
                # only way to reach onint() from a *blocked* read on the
                # tty rather than from shell code. That distinction is the
                # whole bug -- the port aborted with SIGABRT there,
                # because Rust will not let an unwind leave an extern "C"
                # frame, and a signal handler is one.
                if len(line) == 1 and ord(line) < 32:
                    os.write(fd, line.encode())
                else:
                    os.write(fd, (line + "\r").encode())
                i += 1
                time.sleep(settle)
            if r:
                try:
                    chunk = os.read(fd, 4096)
                except OSError:
                    break
                if not chunk:
                    break
                out += chunk
            if i >= len(lines) and not r:
                if not select.select([fd], [], [], 0.6)[0]:
                    break
    finally:
        try:
            os.kill(pid, signal.SIGKILL)
            os.waitpid(pid, 0)
        except Exception:
            pass
        os.close(fd)
    return bytes(out)


ANSI = re.compile(r"\x1b\[[0-9;?]*[a-zA-Z]|\x1b[()][A-Z0-9]|\x1b[=>]")


def norm(b, shell, workdir):
    s = b.decode("utf-8", "replace")
    s = ANSI.sub("", s)
    s = s.replace(shell, "SH").replace(os.path.basename(shell), "SH")
    s = s.replace(workdir, "WD")
    s = s.replace("\r\n", "\n").replace("\r", "")
    s = re.sub(r"\d{4,}", "PID", s)
    # Trailing blank lines differ with how fast each shell drains the pty.
    return s.rstrip() + "\n"


CASES = [
    ("prompt + simple",        [], ["echo one", "exit"]),
    ("PS2 continuation",       [], ["for i in a b", "do", "echo $i", "done", "exit"]),
    ("exit status carry",      [], ["false", "echo $?", "exit"]),
    ("function then call",     [], ["f() { echo fn; }", "f", "exit"]),
    ("here-doc interactive",   [], ["cat <<EOF", "line", "EOF", "exit"]),
    ("alias interactive",      [], ["alias a='echo aliased'", "a", "exit"]),
    ("cd and pwd",             [], ["cd /", "pwd", "exit"]),
    ("trap EXIT",              [], ["trap 'echo bye' EXIT", "echo body", "exit"]),
    ("syntax error recovery",  [], ["for", "echo survived", "exit"]),
    ("unset -u error",         [], ["set -u", "echo $nosuch", "echo survived", "exit"]),
    ("bg job + jobs",          [], ["sleep 0.1 &", "jobs", "wait", "exit"]),
    # --- the job table itself, which dsdiff.sh cannot reach because job
    # control needs a controlling terminal. The fifth background job is
    # the one that grows jobtab, and growing it is what jobs.c relocated
    # curjob, every prev_job and every ps-into-ps0 for. The rest pin the
    # current-job chain's ordering (the + and - markers), the two ways a
    # job is named, and a job whose process array is longer than one.
    ("five bg jobs + jobs",    [], ["sleep 3 & sleep 3 & sleep 3 & sleep 3 & sleep 3 &",
                                    "jobs", "wait", "echo done", "exit"]),
    ("job refs by number",     [], ["sleep 3 &", "sleep 3 &", "jobs %1", "jobs %2",
                                    "wait %2", "wait %1", "echo done", "exit"]),
    ("job refs by name",       [], ["sleep 3 &", "jobs %sleep", "jobs %?lee", "wait", "exit"]),
    ("bg pipeline + jobs",     [], ["sleep 3 | cat &", "jobs", "wait", "echo done", "exit"]),
    # [spec:nsh:req:compat.smoosh.interactive-job-prompt/test]
    ("monitor flag on tty",    [], ["case $- in *m*) echo MONITOR-ON;; *) echo MONITOR-OFF;; esac",
                                    "exit"]),
    ("fg a bg job",            [], ["sleep 2 &", "fg", "echo done", "exit"]),
    ("stop then fg",           [], ["sleep 2", "\x1a", "jobs", "fg", "echo done", "exit"]),
    # --- the line-editor paths, which only exist with -E/-V ---
    ("emacs mode: basic",      ["-E"], ["echo X", "exit"]),
    ("emacs mode: fc -l",      ["-E"], ["echo one", "echo two", "fc -l", "exit"]),
    ("emacs mode: fc -s",      ["-E"], ["echo redo", "fc -s", "exit"]),
    ("emacs mode: fc -ln",     ["-E"], ["echo a", "fc -ln 1 1", "exit"]),
    ("vi mode: basic",         ["-V"], ["echo Y", "exit"]),
    ("vi mode: fc -l",         ["-V"], ["echo v1", "fc -l", "exit"]),
    ("editing off then on",    [], ["set -o emacs", "echo late", "fc -l", "exit"]),
    ("HISTSIZE in editing",    ["-E"], ["HISTSIZE=2", "echo a", "echo b", "echo c", "fc -l 1 9", "exit"]),
    ("multiline into history", ["-E"], ["for i in 1 2", "do", "echo $i", "done", "fc -l", "exit"]),
    # --- onint() leaving the signal handler ---
    # C longjmps out of the handler back to main_handler. The port raises
    # that jump as an unwind, which Rust turns into an abort unless the
    # handler is declared extern "C-unwind" -- so the shell died with
    # status 134 where dash printed a fresh prompt.
    ("^C at the prompt",       [], ["\x03", "echo ALIVE-$?", "exit"]),
    ("^C mid-line",            [], ["echo notsent", "\x03", "echo ALIVE-$?", "exit"]),
    ("^C during a loop",       [], ["while :; do :; done", "\x03", "echo ALIVE-$?", "exit"]),
    ("^C in a here-doc",       [], ["cat <<EOF", "body", "\x03", "echo ALIVE-$?", "exit"]),
    ("^C during PS2",          [], ["for i in a b", "\x03", "echo ALIVE-$?", "exit"]),
    ("^C with a trap set",     [], ["trap 'echo caught' INT", "\x03", "echo ALIVE-$?", "exit"]),
    ("^C then ^C again",       [], ["\x03", "\x03", "echo ALIVE-$?", "exit"]),
    ("^C in emacs mode",       ["-E"], ["\x03", "echo ALIVE-$?", "exit"]),
    ("^C in vi mode",          ["-V"], ["\x03", "echo ALIVE-$?", "exit"]),
    ("^C with EXIT trap",      [], ["trap 'echo bye' EXIT", "\x03", "exit"]),
    # A subshell inside an EXIT trap: the child raises EXEXIT into main's
    # frame via forkreset(). Non-interactively this is covered by
    # tests/corpus/aud_exception_paths.txt; here it also has to survive
    # job control being on.
    ("subshell in EXIT trap",  [], ["trap '(exit 7); echo t=$?' EXIT", "exit"]),

    # --- where the interrupt is noticed ---
    # These exist for `errors-are-values` step F, and they are here
    # *before* the change rather than after it, which is what
    # docs/errors-are-values.md 6B asks for: the failure mode of getting
    # the interrupt wrong is not a crash but a shell that stops answering
    # ^C, and no batch harness can see that because a batch harness never
    # sends one.
    #
    # Each one blocks in a different syscall, because the change replaces
    # "unwind out of the signal handler" with "notice at the EINTR return"
    # and the five EINTR sites are what has to notice. `sa_flags = 0`
    # (trap.rs:288) is why there is always an EINTR to notice at: dash
    # never sets SA_RESTART, so every interruptible syscall returns.
    ("^C during a blocked read",  [], ["read v", "\x03", "echo ALIVE-$?", "exit"]),
    ("^C during a slow child",    [], ["sleep 5", "\x03", "echo ALIVE-$?", "exit"]),
    ("^C during wait",            [], ["sleep 5 &", "wait", "\x03",
                                       "echo ALIVE-$?", "exit"]),
    ("^C during a substitution",  [], ["x=$(sleep 5)", "\x03",
                                       "echo ALIVE-$?", "exit"]),
    # The counter, not the syscall. 2.4 records that `evalbltin` and
    # `evalfun` restore neither `suppressint` nor the saved value after a
    # swallowed builtin error, so the counter comes back one higher than
    # it started -- dash's behaviour, and 8.4 lists "is it really dash's"
    # as one of the things this document is not sure about. A stuck
    # counter is observable in exactly one configuration: an interactive
    # shell that has stopped answering ^C.
    ("^C after a builtin error",  [], ["cd /nonexistent-dir", "\x03",
                                       "echo ALIVE-$?", "exit"]),
    ("^C after a nested error",   [], ["f() { cd /nonexistent-dir; }", "f", "\x03",
                                       "echo ALIVE-$?", "exit"]),

    # --- job control, and the two presence bits `onsig` reads ---
    # Here *before* [dec:nsh:host-owns-signals] moves the trap table onto
    # the shell, for the reason the block above acts on and
    # docs/errors-are-values.md 6B states: the 61,592-case corpus sends
    # no signals and has no controlling terminal, so it cannot execute
    # any of this, and a suite that is green about a path it never runs
    # is not evidence about that path.
    #
    # `onsig` reads the trap table at exactly two indices, and both reads
    # are presence tests rather than reads of the action --
    # `trap[SIGCHLD].is_none()` and `trap[SIGINT].is_none()`
    # (trap.rs:287, :295). When those two bits move into the signal inbox
    # the mirror can disagree with the table, and it is observable in
    # *both* directions, so each direction gets the case that catches it:
    #
    #   SIGCHLD, trap set     "trap CHLD on a fg command" / "on a bg job"
    #   SIGCHLD, trap clear   "trap CHLD cleared", read against "wait
    #                         status no CHLD trap" as its control: a
    #                         mirror that still says "trapped" sets
    #                         pending_sig = SIGCHLD, and `wait` answers
    #                         128 + pending_sig = 145 (bltin/wait.rs:51)
    #   SIGINT, trap set      "^C a fg job with INT trap" -- the trap
    #                         runs; a stale mirror takes the interrupt
    #                         instead
    #   SIGINT, trap clear    "trap INT cleared then ^C" -- ALIVE-130 is
    #                         the interrupt being taken; a stale mirror
    #                         swallows the ^C entirely
    #
    # "trap ignore INT then ^C" is the third state of the C's three:
    # NULL, "" and an action. The bit is `is_some()`, so an *ignored*
    # signal has it set; a mirror keyed on `is_empty()` instead passes
    # every case above and fails this one.
    #
    # The rest is job control proper, which nothing above reaches:
    # suspend and resume, the terminal changing hands, SIGTTIN, and the
    # reaping a shell only reports at a prompt.
    ("stop then bg",              [], ["sleep 1", "\x1a", "bg", "wait", "echo done", "exit"]),
    ("fg then read the tty",      [], ["sleep 1", "\x1a", "fg", "read v", "hello",
                                       "echo got-$v", "exit"]),
    ("exit with a stopped job",   [], ["sleep 3", "\x1a", "exit", "exit"]),
    ("^Z at the prompt",          [], ["\x1a", "echo ALIVE-$?", "exit"]),
    ("trap TSTP then ^Z",         [], ["trap 'echo tstp' TSTP", "\x1a", "echo ALIVE-$?", "exit"]),
    ("bg job stopped on tty read",[], ["cat /dev/tty &", "sleep 1", "jobs", "exit", "exit"]),
    ("kill %1 and its notice",    [], ["sleep 5 &", "kill %1", "sleep 1", "echo after", "exit"]),
    ("trap CHLD on a fg command", [], ["trap 'echo CHLD' CHLD", "/bin/true", "echo done", "exit"]),
    ("trap CHLD on a bg job",     [], ["trap 'echo CHLD' CHLD", "sleep 1 &", "wait",
                                       "echo done", "exit"]),
    ("trap CHLD cleared",         [], ["trap 'echo CHLD' CHLD", "trap - CHLD", "sleep 1 &",
                                       "wait", "echo done", "exit"]),
    ("wait status w/ CHLD trap",  [], ["trap 'echo CHLD' CHLD", "sleep 1 &", "wait",
                                       "echo st=$?", "exit"]),
    ("wait status no CHLD trap",  [], ["sleep 1 &", "wait", "echo st=$?", "exit"]),
    ("^C a fg job with INT trap", [], ["trap 'echo caught' INT", "sleep 5", "\x03",
                                       "echo ALIVE-$?", "exit"]),
    ("trap INT cleared then ^C",  [], ["trap 'echo caught' INT", "trap - INT", "\x03",
                                       "echo ALIVE-$?", "exit"]),
    ("trap ignore INT then ^C",   [], ["trap '' INT", "\x03", "echo ALIVE-$?", "exit"]),
    # SIGQUIT had no coverage at all. An interactive shell ignores it
    # (trap.rs:160-167, the S_IGN arm), and a foreground child does not
    # -- `forkchild` only ignores INT and QUIT for FORK_BG (jobs.rs:994).
    # `ulimit -c 0` keeps the dump out of the message and off the disk;
    # without it both shells print "Quit (core dumped)" and both write a
    # core through the host's core_pattern on every run.
    ("^backslash at the prompt",  [], ["\x1c", "echo ALIVE-$?", "exit"]),
    ("^backslash kills a fg job", [], ["ulimit -c 0", "sleep 5", "\x1c",
                                       "echo ALIVE-$?", "exit"]),
]


def main():
    root = f"{ROOT}/tests/.build/ptyrun"
    subprocess.run(["rm", "-rf", root], check=False)
    npass = nfail = nflaky = 0
    failures = []
    for idx, (name, args, lines) in enumerate(CASES):
        wa = f"{root}/{idx}/w"
        wb = f"{root}/{idx}/w2"
        for d in (wa, wb):
            os.makedirs(d, exist_ok=True)
        a = norm(run(REF, args, lines, wa), REF, wa)
        b = norm(run(PORT, args, lines, wb), PORT, wb)
        if a == b:
            npass += 1
            print(f"  ok   {name}")
            continue
        # A single mismatch on a pty is not yet a divergence. How much of
        # a prompt has been drained when the reader gives up is a timing
        # question, and this harness once reported 18/20 on a tree that
        # was fine. Re-run both sides and only call it a failure when the
        # two sets of observed outputs are disjoint -- the same rule
        # dscase.sh applies. (Documented as missing in tests/README.md;
        # it no longer is.)
        refset, portset = {a}, {b}
        for _ in range(3):
            refset.add(norm(run(REF, args, lines, wa), REF, wa))
            portset.add(norm(run(PORT, args, lines, wb), PORT, wb))
        if refset & portset:
            nflaky += 1
            npass += 1
            print(f"  ok   {name}  (flaky: the port produced an output the reference also produces)")
            continue
        nfail += 1
        print(f"  FAIL {name}")
        failures.append((name, a, b))
    for name, a, b in failures:
        print(f"\n### {name}\n  ref :\n{a}\n  port:\n{b}")
    print(f"\nINTERACTIVE (pty, sandboxed): PASS={npass} FAIL={nfail} FLAKY={nflaky}")
    return 1 if nfail else 0


if __name__ == "__main__":
    sys.exit(main())
