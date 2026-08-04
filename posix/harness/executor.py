"""Isolated shell execution and expectation matching."""

from __future__ import annotations

import errno
import fcntl
import os
import pty
import select
import signal
import subprocess
import tempfile
import termios
import time
from pathlib import Path

from model import Case, Observation


def _getconf(name: str) -> str | None:
    try:
        result = subprocess.run(
            ["getconf", name],
            check=False,
            capture_output=True,
            text=True,
            timeout=2,
        )
    except (OSError, subprocess.TimeoutExpired):
        return None
    if result.returncode != 0:
        return None
    value = result.stdout.strip()
    return value if value and value != "-1" and value != "undefined" else None


def host_capabilities() -> dict[str, bool]:
    """Resolve the standard option codes used by conditional rule wording."""

    return {
        "UP": _getconf("POSIX2_CHAR_TERM") is not None,
        "XSI": _getconf("_XOPEN_VERSION") is not None,
        # OB marks an obsolescent requirement, not an optional facility.
        "OB": True,
    }


def _decode(data: bytes | None) -> str:
    return (data or b"").decode("utf-8", errors="surrogateescape")


def _expand_expected(value: str, root: Path, home: Path, shell: Path) -> str:
    return (
        value.replace("{ROOT}", str(root))
        .replace("{HOME}", str(home))
        .replace("{SHELL}", str(shell))
    )


def _status_reason(expected: int | str, actual: int) -> str | None:
    if expected == "any":
        return None
    if expected == "nonzero":
        return None if actual != 0 else "expected nonzero status, got 0"
    if actual != expected:
        return f"expected status {expected}, got {actual}"
    return None


def _run_process(
    argv: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    stdin: str | None,
    timeout: float,
) -> tuple[int, str, str, bool]:
    process = subprocess.Popen(
        argv,
        cwd=cwd,
        env=env,
        stdin=subprocess.PIPE if stdin is not None else subprocess.DEVNULL,
        stdout=subprocess.PIPE,
        stderr=subprocess.PIPE,
        start_new_session=True,
    )
    try:
        stdout, stderr = process.communicate(
            None if stdin is None else stdin.encode(), timeout=timeout
        )
        return process.returncode, _decode(stdout), _decode(stderr), False
    except subprocess.TimeoutExpired:
        try:
            os.killpg(process.pid, signal.SIGKILL)
        except ProcessLookupError:
            pass
        stdout, stderr = process.communicate()
        return 124, _decode(stdout), _decode(stderr), True


def _make_controlling_terminal() -> None:
    """Start a session and make the child's standard input its controlling TTY."""

    os.setsid()
    fcntl.ioctl(0, termios.TIOCSCTTY, 0)


def _run_pty(
    argv: list[str],
    *,
    cwd: Path,
    env: dict[str, str],
    stdin: str,
    timeout: float,
) -> tuple[int, str, str, bool]:
    """Run an interactive shell on a real controlling terminal.

    A terminal has one output stream, so stderr is intentionally returned as
    empty and the complete terminal transcript is returned as stdout.
    """

    master, slave = pty.openpty()
    try:
        attributes = termios.tcgetattr(slave)
        attributes[3] &= ~(termios.ECHO | termios.ECHONL)
        termios.tcsetattr(slave, termios.TCSANOW, attributes)
        process = subprocess.Popen(
            argv,
            cwd=cwd,
            env=env,
            stdin=slave,
            stdout=slave,
            stderr=slave,
            close_fds=True,
            preexec_fn=_make_controlling_terminal,
        )
    except BaseException:
        os.close(master)
        os.close(slave)
        raise
    os.close(slave)

    payload = stdin.encode("utf-8", errors="surrogateescape") + b"\x04\x04"
    offset = 0
    output = bytearray()
    deadline = time.monotonic() + timeout
    terminal_eof = False
    timed_out = False
    os.set_blocking(master, False)

    try:
        while True:
            now = time.monotonic()
            if process.poll() is None and now >= deadline:
                timed_out = True
                try:
                    os.killpg(process.pid, signal.SIGKILL)
                except ProcessLookupError:
                    pass

            running = process.poll() is None
            readers = [] if terminal_eof else [master]
            writers = [master] if running and offset < len(payload) else []
            remaining = max(0.0, deadline - now) if running else 0.02
            wait = min(0.05, remaining) if running else 0.02
            try:
                readable, writable, _ = select.select(readers, writers, [], wait)
            except InterruptedError:
                continue

            if writable:
                try:
                    offset += os.write(master, payload[offset:])
                except BlockingIOError:
                    pass
                except OSError as error:
                    if error.errno not in {errno.EIO, errno.EBADF}:
                        raise

            if readable:
                try:
                    chunk = os.read(master, 65536)
                    if chunk:
                        output.extend(chunk)
                    else:
                        terminal_eof = True
                except BlockingIOError:
                    pass
                except OSError as error:
                    if error.errno == errno.EIO:
                        terminal_eof = True
                    else:
                        raise

            if process.poll() is not None:
                if terminal_eof:
                    break
                try:
                    readable, _, _ = select.select([master], [], [], 0.02)
                except InterruptedError:
                    continue
                if not readable:
                    break

        returncode = process.wait()
    finally:
        os.close(master)
        if process.poll() is None:
            try:
                os.killpg(process.pid, signal.SIGKILL)
            except ProcessLookupError:
                pass
            process.wait()

    transcript = _decode(bytes(output)).replace("\r\n", "\n")
    return (124 if timed_out else returncode), transcript, "", timed_out


def _invocation(shell: Path, case: Case) -> tuple[list[str], str | None, bool]:
    """Translate a case execution mode into argv, input, and PTY use."""

    options = list(case.shell_options)
    if case.mode == "command":
        return [str(shell), *options, "-c", case.script, *case.args], case.stdin, False
    session = case.stdin if case.stdin is not None else case.script
    if case.mode == "stdin":
        return [str(shell), *options, "-s", *case.args], session, False
    return [str(shell), *options, "-i", *case.args], session, True


def run_case(
    shell: Path,
    case: Case,
    capabilities: dict[str, bool] | None = None,
) -> Observation:
    """Run one case in a fresh directory with a deterministic environment."""

    capabilities = capabilities or host_capabilities()
    unavailable = tuple(code for code in case.requires if not capabilities.get(code, False))
    if unavailable:
        return Observation(
            case=case.id,
            rules=case.rules,
            verdict="SKIP",
            status=None,
            stdout="",
            stderr="",
            reasons=[f"host does not advertise {code}" for code in unavailable],
            duration_ms=0,
            skipped_by=unavailable,
        )

    shell = shell.resolve()
    started = time.monotonic()
    try:
        with tempfile.TemporaryDirectory(prefix="dash-posix-") as directory:
            root = Path(directory)
            home = root / ".home"
            home.mkdir()
            bin_dir = root / ".bin"
            bin_dir.mkdir()
            (bin_dir / "sh").symlink_to(shell)

            for relative, fixture in case.files.items():
                path = root / relative
                path.parent.mkdir(parents=True, exist_ok=True)
                path.write_text(fixture.content, encoding="utf-8")
                path.chmod(fixture.mode)

            env = {
                "HOME": str(home),
                "PATH": f"{bin_dir}:/usr/bin:/bin",
                "LC_ALL": "C",
                "LANG": "C",
                "PWD": str(root),
                "TMPDIR": str(root),
                "TERM": "dumb",
            }
            env.update(
                {
                    name: _expand_expected(value, root, home, shell)
                    for name, value in case.environment.items()
                }
            )
            argv, stdin, interactive = _invocation(shell, case)
            if interactive:
                status, stdout, stderr, timed_out = _run_pty(
                    argv,
                    cwd=root,
                    env=env,
                    stdin=stdin or "",
                    timeout=case.timeout,
                )
            else:
                status, stdout, stderr, timed_out = _run_process(
                    argv,
                    cwd=root,
                    env=env,
                    stdin=stdin,
                    timeout=case.timeout,
                )

            reasons: list[str] = []
            if timed_out:
                reasons.append(f"timed out after {case.timeout:g}s")
            status_mismatch = _status_reason(case.status, status)
            if status_mismatch:
                reasons.append(status_mismatch)

            if case.stdout is not None:
                expected = _expand_expected(case.stdout, root, home, shell)
                if stdout != expected:
                    reasons.append(f"stdout differs (expected {expected!r})")
            if case.stderr is not None:
                expected = _expand_expected(case.stderr, root, home, shell)
                if stderr != expected:
                    reasons.append(f"stderr differs (expected {expected!r})")
            for needle in case.stdout_contains:
                expected = _expand_expected(needle, root, home, shell)
                if expected not in stdout:
                    reasons.append(f"stdout missing {expected!r}")
            for needle in case.stderr_contains:
                expected = _expand_expected(needle, root, home, shell)
                if expected not in stderr:
                    reasons.append(f"stderr missing {expected!r}")
            for needle in case.stdout_excludes:
                expected = _expand_expected(needle, root, home, shell)
                if expected in stdout:
                    reasons.append(f"stdout unexpectedly contains {expected!r}")
            for needle in case.stderr_excludes:
                expected = _expand_expected(needle, root, home, shell)
                if expected in stderr:
                    reasons.append(f"stderr unexpectedly contains {expected!r}")

            elapsed = round((time.monotonic() - started) * 1000)
            return Observation(
                case=case.id,
                rules=case.rules,
                verdict="PASS" if not reasons else "FAIL",
                status=status,
                stdout=stdout,
                stderr=stderr,
                reasons=reasons,
                duration_ms=elapsed,
            )
    except OSError as error:
        elapsed = round((time.monotonic() - started) * 1000)
        return Observation(
            case=case.id,
            rules=case.rules,
            verdict="ERROR",
            status=None,
            stdout="",
            stderr="",
            reasons=[str(error)],
            duration_ms=elapsed,
        )
