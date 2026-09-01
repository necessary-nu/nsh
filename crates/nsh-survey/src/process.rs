use std::error::Error;
use std::ffi::{OsStr, OsString};
use std::fs;
use std::io::{Read, Write};
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const TERM_GRACE_MS: u64 = 100;
const POLL_MS: u64 = 5;

/// How much longer than the case's own budget the outer deadline waits.
///
/// The case's budget is enforced *inside* the boundary now, so this one
/// only has to outlast `timeout`'s own `TERM` and its `KILL` a second
/// later. It fires when the sandbox never reached the point of running
/// anything, which is the one situation the inner budget cannot cover.
const BACKSTOP_MS: u64 = 2_000;

/// What `timeout` reports when it fired.
const TIMED_OUT_STATUS: i32 = 124;
const CONTAINMENT_CANARY: &[u8] = b"__NSH_SURVEY_CONTAINED__\n";
const CONTAINMENT_PROBE: &str = r#"
fail() {
    printf 'containment probe: %s\n' "$1" >&2
    exit 91
}
host_pid=$1
host_pid_namespace=$2
host_net_namespace=$3
write_probe=$4
if test -e "/proc/$host_pid" && test "$(/usr/bin/readlink "/proc/$host_pid/ns/pid")" = "$host_pid_namespace"; then
    fail "host pid is visible"
fi
test "$(/usr/bin/readlink /proc/self/ns/pid)" != "$host_pid_namespace" || fail "PID namespace was not replaced"
test "$(/usr/bin/readlink /proc/self/ns/net)" != "$host_net_namespace" || fail "network namespace was not replaced"
if (umask 077; : > "$write_probe") 2>/dev/null; then
    fail "read-only root accepted a write"
fi
/usr/bin/awk '$1 == "Max" && $2 == "processes" { found=1; good=($3 == 64 && $4 == 64) } END { exit !(found && good) }' /proc/self/limits || fail "nproc limit is not 64"
set -- $(/usr/bin/cut -d ' ' -f 1,6,7 "/proc/$$/stat")
test "$2" -gt 0 || fail "command did not enter a session inside the PID namespace (sid=$2)"
test "$3" = 0 || fail "command retained a controlling terminal (tty=$3)"
set -- /proc/[0-9]*
test "$#" -le 16 || fail "too many host processes are visible ($#)"
printf '%s\n' __NSH_SURVEY_CONTAINED__
"#;
pub(crate) const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

pub(crate) struct Request<'a> {
    pub(crate) containment: &'a Containment,
    pub(crate) program: &'a Path,
    pub(crate) arguments: &'a [OsString],
    pub(crate) directory: &'a Path,
    pub(crate) environment: &'a [(OsString, OsString)],
    pub(crate) input: &'a [u8],
    pub(crate) timeout: Duration,
}

#[derive(Debug)]
pub(crate) struct Output {
    pub(crate) status: ExitStatus,
    pub(crate) stdout: Captured,
    pub(crate) stderr: Captured,
    pub(crate) timed_out: bool,
    pub(crate) duration: Duration,
    pub(crate) writer_error: Option<String>,
}

#[derive(Debug)]
pub(crate) struct Captured {
    pub(crate) bytes: Vec<u8>,
    pub(crate) truncated: bool,
}

/// A fail-closed namespace boundary for commands that consume survey input.
///
/// Construction succeeds only after a canary has proved that the configured
/// sandbox created fresh PID and network namespaces and hid the caller's PID.
/// The fields stay private so callers cannot manufacture an unverified token.
pub(crate) struct Containment {
    sandbox: PathBuf,
    writable_root: PathBuf,
}

impl Containment {
    pub(crate) fn verified(writable_root: &Path) -> Result<Self> {
        let writable_root = fs::canonicalize(writable_root).map_err(|error| {
            format!(
                "cannot resolve survey writable root {}: {error}",
                writable_root.display()
            )
        })?;
        if !writable_root.is_dir() {
            return Err(format!(
                "survey writable root {} is not a directory",
                writable_root.display()
            )
            .into());
        }
        path_argument(&writable_root, "survey writable root")?;
        let containment = Self {
            sandbox: resolve_sandbox()?,
            writable_root,
        };
        containment.verify_namespaces()?;
        Ok(containment)
    }

    pub(crate) fn label(&self) -> &'static str {
        "sandbox-pid-net-ro-root"
    }

    fn verify_namespaces(&self) -> Result<()> {
        let host_pid = std::process::id().to_string();
        let host_pid_namespace = fs::read_link("/proc/self/ns/pid")?.into_os_string();
        let host_net_namespace = fs::read_link("/proc/self/ns/net")?.into_os_string();
        let root_name = self
            .writable_root
            .file_name()
            .ok_or("survey writable root has no final component")?
            .to_string_lossy();
        let write_probe = self
            .writable_root
            .parent()
            .ok_or("survey writable root has no parent")?
            .join(format!(".containment-write-probe-{root_name}"));
        if write_probe.exists() {
            return Err(format!(
                "containment write probe {} already exists",
                write_probe.display()
            )
            .into());
        }
        let arguments = [
            OsString::from("-eu"),
            OsString::from("-c"),
            OsString::from(CONTAINMENT_PROBE),
            OsString::from("nsh-survey-containment-probe"),
            OsString::from(host_pid),
            host_pid_namespace,
            host_net_namespace,
            write_probe.as_os_str().to_owned(),
        ];
        let environment = Vec::new();
        let output = run(&Request {
            containment: self,
            program: Path::new("/bin/sh"),
            arguments: &arguments,
            directory: &self.writable_root,
            environment: &environment,
            input: b"",
            timeout: Duration::from_secs(5),
        });
        let root_was_writable = write_probe.exists();
        if root_was_writable {
            fs::remove_file(&write_probe).map_err(|error| {
                format!(
                    "sandbox wrote outside its scratch root and cleanup of {} failed: {error}",
                    write_probe.display()
                )
            })?;
            return Err(format!(
                "sandbox root was writable outside {}",
                self.writable_root.display()
            )
            .into());
        }
        let output = output?;
        if output.timed_out {
            return Err("containment canary timed out".into());
        }
        if !output.status.success()
            || output.stdout.bytes != CONTAINMENT_CANARY
            || !output.stderr.bytes.is_empty()
            || output.stdout.truncated
            || output.stderr.truncated
            || output.writer_error.is_some()
        {
            return Err(format!(
                "containment canary failed: status={:?}, stdout={:?}, stderr={:?}",
                output.status.code(),
                String::from_utf8_lossy(&output.stdout.bytes),
                String::from_utf8_lossy(&output.stderr.bytes)
            )
            .into());
        }
        Ok(())
    }
}

pub(crate) fn run(request: &Request<'_>) -> Result<Output> {
    let directory = fs::canonicalize(request.directory).map_err(|error| {
        format!(
            "cannot resolve case directory {}: {error}",
            request.directory.display()
        )
    })?;
    if !directory.starts_with(&request.containment.writable_root) {
        return Err(format!(
            "case directory {} escapes writable root {}",
            directory.display(),
            request.containment.writable_root.display()
        )
        .into());
    }
    let writable = path_argument(&request.containment.writable_root, "survey writable root")?;
    let directory_argument = path_argument(&directory, "case directory")?;
    let mut command = Command::new(&request.containment.sandbox);
    command
        .arg("--quiet")
        .arg("--unshare")
        .arg("all")
        .arg("--die-with-parent")
        .arg("--new-session")
        .arg("--close-fds")
        .arg("--bind")
        .arg("/:/:ro")
        .arg("--dev")
        .arg("/dev")
        .arg("--proc")
        .arg("/proc")
        .arg("--tmpfs")
        .arg("/tmp")
        .arg("--bind")
        .arg(format!("{writable}:{writable}"))
        .arg("--chdir")
        .arg(directory_argument)
        .arg("--limit")
        .arg("nproc=64")
        .arg("--clearenv");
    for (name, value) in request.environment {
        command.arg("--setenv").arg(name).arg(value);
    }
    /* The budget is spent inside the boundary rather than by signalling
     * it from outside. Killing the sandbox process is only reliable once
     * the sandbox has finished setting up: a signal delivered during that
     * window reaps the process this side holds and leaves the tree inside
     * it running, which leaked a background descendant 6 times in 20 at
     * load 30 and never once when the same signal was 50 ms later. There
     * is no readiness this side can wait for, so nothing is timed from
     * here that can be timed from in there -- and a case whose command
     * exits is torn down by the sandbox's own reaper, which is the path
     * `normal_exit_kills_background_descendants` already covers. */
    command
        .arg("--")
        .arg("/usr/bin/timeout")
        .arg("--signal=TERM")
        .arg("--kill-after=1")
        .arg(format!("{:.3}", request.timeout.as_secs_f64()))
        .arg("/usr/bin/env")
        .arg("--default-signal")
        .arg("--")
        .arg(request.program)
        .args(request.arguments)
        .current_dir(&directory)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let started = Instant::now();
    let mut child = command.spawn()?;
    let child_pid = nsh_platform::ProcessId::new(child.id()).ok_or("child returned PID zero")?;
    let mut input = child.stdin.take().ok_or("child stdin was not piped")?;
    let stdout = child.stdout.take().ok_or("child stdout was not piped")?;
    let stderr = child.stderr.take().ok_or("child stderr was not piped")?;
    let script = request.input.to_vec();
    let writer = thread::spawn(move || input.write_all(&script));
    let stdout_reader = thread::spawn(move || capture(stdout));
    let stderr_reader = thread::spawn(move || capture(stderr));

    let deadline = started + request.timeout + Duration::from_millis(BACKSTOP_MS);
    let mut timed_out = false;
    let status = 'wait: loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let signalled = nsh_platform::send_signal(
                nsh_platform::ProcessTarget::Process(child_pid),
                nsh_platform::SignalRequest::Deliver(nsh_platform::termination_signal()),
            )
            .is_ok();
            if !signalled {
                let _ = child.kill();
            }
            let grace_deadline = Instant::now() + Duration::from_millis(TERM_GRACE_MS);
            loop {
                if let Some(status) = child.try_wait()? {
                    break 'wait status;
                }
                if Instant::now() >= grace_deadline {
                    let _ = child.kill();
                    break 'wait child.wait()?;
                }
                thread::sleep(Duration::from_millis(POLL_MS));
            }
        }
        thread::sleep(Duration::from_millis(POLL_MS));
    };
    /* `timeout` reports 124 when it fired, and a case is free to exit 124
     * on its own -- so the elapsed time has to agree before that is read
     * as a timeout. */
    let timed_out = timed_out
        || (status.code() == Some(TIMED_OUT_STATUS) && started.elapsed() >= request.timeout);
    let writer_error = writer
        .join()
        .map_err(|_| "stdin writer thread panicked")?
        .err()
        .filter(|error| error.kind() != std::io::ErrorKind::BrokenPipe)
        .map(|error| error.to_string());
    let stdout = stdout_reader
        .join()
        .map_err(|_| "stdout reader thread panicked")??;
    let stderr = stderr_reader
        .join()
        .map_err(|_| "stderr reader thread panicked")??;
    Ok(Output {
        status,
        stdout,
        stderr,
        timed_out,
        duration: started.elapsed(),
        writer_error,
    })
}

fn resolve_sandbox() -> Result<PathBuf> {
    let requested =
        std::env::var_os("NSH_SURVEY_SANDBOX").unwrap_or_else(|| OsString::from("sandbox"));
    let search_path = std::env::var_os("PATH");
    resolve_sandbox_from(&requested, search_path.as_deref())
}

fn resolve_sandbox_from(requested: &OsStr, search_path: Option<&OsStr>) -> Result<PathBuf> {
    let requested_path = Path::new(&requested);
    let candidate = if requested_path.components().count() > 1 {
        requested_path.to_owned()
    } else {
        let path =
            search_path.ok_or("PATH is unset; set NSH_SURVEY_SANDBOX to the sandbox executable")?;
        std::env::split_paths(path)
            .map(|directory| directory.join(requested))
            .find(|candidate| is_executable(candidate))
            .ok_or_else(|| {
                format!(
                    "sandbox executable {:?} was not found; refusing to run survey cases unsandboxed",
                    requested
                )
            })?
    };
    if !is_executable(&candidate) {
        return Err(format!(
            "configured sandbox {} is not an executable file",
            candidate.display()
        )
        .into());
    }
    Ok(fs::canonicalize(candidate)?)
}

fn is_executable(path: &Path) -> bool {
    fs::metadata(path)
        .is_ok_and(|metadata| metadata.is_file() && metadata.permissions().mode() & 0o111 != 0)
}

fn path_argument<'a>(path: &'a Path, description: &str) -> Result<&'a str> {
    let value = path
        .to_str()
        .ok_or_else(|| format!("{description} {} is not UTF-8", path.display()))?;
    if value.contains(':') {
        return Err(format!("{description} {value:?} contains ':'").into());
    }
    Ok(value)
}

pub(crate) fn capture(mut reader: impl Read) -> std::io::Result<Captured> {
    let mut bytes = Vec::new();
    let mut truncated = false;
    let mut buffer = [0_u8; 16 * 1024];
    loop {
        let count = reader.read(&mut buffer)?;
        if count == 0 {
            break;
        }
        let remaining = OUTPUT_LIMIT.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..count.min(remaining)]);
        truncated |= count > remaining;
    }
    Ok(Captured { bytes, truncated })
}

pub(crate) fn duration_millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

static SCRATCH_COUNTER: AtomicU64 = AtomicU64::new(0);

pub(crate) struct ScratchTree {
    path: PathBuf,
}

impl ScratchTree {
    pub(crate) fn new() -> std::io::Result<Self> {
        let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../target/nsh-survey-work");
        fs::create_dir_all(&base)?;
        let base = fs::canonicalize(base)?;
        for _ in 0..100 {
            let serial = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = base.join(format!("run-{}-{serial}", std::process::id()));
            match fs::create_dir(&path) {
                Ok(()) => return Ok(Self { path }),
                Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
                Err(error) => return Err(error),
            }
        }
        Err(std::io::Error::new(
            std::io::ErrorKind::AlreadyExists,
            "could not allocate a unique survey scratch directory",
        ))
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for ScratchTree {
    fn drop(&mut self) {
        let _ = fs::remove_dir_all(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn run_shell(containment: &Containment, directory: &Path, script: &str) -> Result<Output> {
        let arguments = [OsString::from("-c"), OsString::from(script)];
        run(&Request {
            containment,
            program: Path::new("/bin/sh"),
            arguments: &arguments,
            directory,
            environment: &[],
            input: b"",
            timeout: Duration::from_secs(2),
        })
    }

    #[test]
    fn capture_discards_past_bound() {
        let input = vec![b'x'; OUTPUT_LIMIT + 17];
        let captured = capture(std::io::Cursor::new(input)).unwrap();
        assert!(captured.truncated);
        assert_eq!(captured.bytes.len(), OUTPUT_LIMIT);
    }

    #[test]
    fn missing_sandbox_has_no_fallback() {
        let scratch = ScratchTree::new().unwrap();
        let error = resolve_sandbox_from(
            OsStr::new("definitely-not-an-installed-sandbox"),
            Some(scratch.path().as_os_str()),
        )
        .unwrap_err();
        assert!(
            error
                .to_string()
                .contains("refusing to run survey cases unsandboxed")
        );
    }

    #[test]
    fn kill_all_cannot_reach_host_processes() {
        let scratch = ScratchTree::new().unwrap();
        let containment = Containment::verified(scratch.path()).unwrap();
        let mut sentinel = Command::new("/bin/sleep").arg("30").spawn().unwrap();

        let output = run_shell(
            &containment,
            scratch.path(),
            "kill -KILL -1 2>/dev/null || :; printf survived",
        )
        .unwrap();
        let sentinel_survived = sentinel.try_wait().unwrap().is_none();
        let _ = sentinel.kill();
        let _ = sentinel.wait();

        assert!(sentinel_survived);
        assert!(!output.timed_out);
        assert_eq!(output.stdout.bytes, b"survived");
        /* The case's own budget stands inside the boundary with it, so
         * `kill -KILL -1` reaches that too and the sandbox reports the
         * wrapper's death rather than the shell's exit. That ends the
         * case early rather than extending it, which is the direction
         * that matters: a case cannot disarm its budget and keep
         * running. The shell still ran, which is what the output says. */
        assert_eq!(output.status.code(), Some(137));
    }

    #[test]
    fn normal_exit_kills_background_descendants() {
        let scratch = ScratchTree::new().unwrap();
        let containment = Containment::verified(scratch.path()).unwrap();
        let output = run_shell(
            &containment,
            scratch.path(),
            "(sleep 1; printf leaked > leak) & exit 0",
        )
        .unwrap();

        assert!(!output.timed_out);
        assert_eq!(output.status.code(), Some(0));
        std::thread::sleep(Duration::from_millis(1_100));
        assert!(!scratch.path().join("leak").exists());
    }

    #[test]
    fn outside_working_directory_is_rejected() {
        let scratch = ScratchTree::new().unwrap();
        let containment = Containment::verified(scratch.path()).unwrap();
        let arguments = [];
        let error = run(&Request {
            containment: &containment,
            program: Path::new("/bin/true"),
            arguments: &arguments,
            directory: Path::new("/tmp"),
            environment: &[],
            input: b"",
            timeout: Duration::from_secs(1),
        })
        .unwrap_err();

        assert!(error.to_string().contains("escapes writable root"));
    }
}
