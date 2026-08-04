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
PORT = os.environ.get("PORT", f"{ROOT}/target/debug/dash")
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
                os.write(fd, (lines[i] + "\r").encode())
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
]


def main():
    root = f"{ROOT}/tests/.build/ptyrun"
    subprocess.run(["rm", "-rf", root], check=False)
    npass = nfail = 0
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
        else:
            nfail += 1
            print(f"  FAIL {name}")
            failures.append((name, a, b))
    for name, a, b in failures:
        print(f"\n### {name}\n  ref :\n{a}\n  port:\n{b}")
    print(f"\nINTERACTIVE (pty, sandboxed): PASS={npass} FAIL={nfail}")
    return 1 if nfail else 0


if __name__ == "__main__":
    sys.exit(main())
