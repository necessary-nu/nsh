use std::error::Error;
use std::ffi::OsString;
use std::fs;
use std::io::{Read, Write};
use std::os::unix::process::CommandExt;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitStatus, Stdio};
use std::sync::atomic::{AtomicU64, Ordering};
use std::thread;
use std::time::{Duration, Instant};

type Result<T> = std::result::Result<T, Box<dyn Error>>;

const TERM_GRACE_MS: u64 = 100;
const POLL_MS: u64 = 5;
pub(crate) const OUTPUT_LIMIT: usize = 4 * 1024 * 1024;

pub(crate) struct Request<'a> {
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

pub(crate) fn run(request: &Request<'_>) -> Result<Output> {
    let mut command = Command::new(request.program);
    command
        .args(request.arguments)
        .current_dir(request.directory)
        .env_clear()
        .envs(request.environment.iter().cloned())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .process_group(0);
    let started = Instant::now();
    let mut child = command.spawn()?;
    let group = i32::try_from(child.id()).map_err(|_| "child pid does not fit i32")?;
    let mut input = child.stdin.take().ok_or("child stdin was not piped")?;
    let stdout = child.stdout.take().ok_or("child stdout was not piped")?;
    let stderr = child.stderr.take().ok_or("child stderr was not piped")?;
    let script = request.input.to_vec();
    let writer = thread::spawn(move || input.write_all(&script));
    let stdout_reader = thread::spawn(move || capture(stdout));
    let stderr_reader = thread::spawn(move || capture(stderr));

    let deadline = started + request.timeout;
    let mut timed_out = false;
    let status = 'wait: loop {
        if let Some(status) = child.try_wait()? {
            break status;
        }
        if Instant::now() >= deadline {
            timed_out = true;
            let signalled = nsh_platform::send_signal_to_process_group(
                group,
                nsh_platform::termination_signal(),
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
                    let _ = nsh_platform::send_signal_to_process_group(
                        group,
                        nsh_platform::kill_signal(),
                    );
                    let _ = child.kill();
                    break 'wait child.wait()?;
                }
                thread::sleep(Duration::from_millis(POLL_MS));
            }
        }
        thread::sleep(Duration::from_millis(POLL_MS));
    };
    let _ = nsh_platform::send_signal_to_process_group(group, nsh_platform::kill_signal());
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
        for _ in 0..100 {
            let serial = SCRATCH_COUNTER.fetch_add(1, Ordering::Relaxed);
            let path =
                std::env::temp_dir().join(format!("nsh-survey-{}-{serial}", std::process::id()));
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

    #[test]
    fn capture_discards_past_bound() {
        let input = vec![b'x'; OUTPUT_LIMIT + 17];
        let captured = capture(std::io::Cursor::new(input)).unwrap();
        assert!(captured.truncated);
        assert_eq!(captured.bytes.len(), OUTPUT_LIMIT);
    }
}
