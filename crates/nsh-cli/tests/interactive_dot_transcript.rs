#![cfg(unix)]
//! What `.` writes to a terminal, measured against dash on a terminal.
//!
//! The extra blank line this holds down is invisible to every other suite in
//! the repository, and the reason is structural: `gate-bash`, the Oils
//! survey, the POSIX harness and the differential corpora all feed a script
//! on standard input or through `-c`, and the newline is written only when
//! the shell is interactive. A non-interactive `.` is byte-identical in both
//! shells, so nothing that runs a script can report this.
//!
//! Nothing here is a recorded expectation. Each case runs in both shells on
//! their own pseudo-terminals with the same prompt and the same typed bytes,
//! and the two transcripts are compared, so there is no literal to go stale.
//!
//! `TERM=dumb` on purpose: it keeps the line editor's redisplay out of the
//! transcript, so what is left is the shell's own writes, which is what the
//! comparison is about.
//!
//! The last case is the one that makes the first two safe. The newline is
//! not unwanted everywhere: reaching the end of the shell's *own* input
//! with `-i` live writes one in dash too, so a fix that suppresses it
//! wholesale satisfies every `.` case above and is still wrong.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// A liveness bound, not a measurement: every wait ends as soon as its
/// marker arrives.
const PATIENCE: Duration = Duration::from_secs(20);

/// A prompt neither shell would write for any other reason, so a
/// difference in the compared transcripts is a difference in what the
/// shells wrote rather than in what the environment gave them.
const PROMPT: &str = "@sh@ ";

/// What the shell prints last, so both sides can be read to the same point.
const DONE: &str = "TRANSCRIPT-END";

/// The system dash, which is what an interactive transcript is judged
/// against.
///
/// Not optional. `.` is dash's built-in here, and a terminal transcript
/// compared against nothing is exactly the shape of defect this file exists
/// for -- it went unnoticed for a day because the suites that ran could not
/// see it.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
fn system_dash() -> &'static Path {
    let dash = Path::new("/usr/bin/dash");
    assert!(
        dash.exists(),
        "no /usr/bin/dash, so an interactive transcript has nothing to be \
         checked against and this file would pass without measuring anything"
    );
    dash
}

/// One interactive shell on its own pseudo-terminal.
struct Session {
    child: Child,
    controller: File,
    transcript: Vec<u8>,
}

impl Session {
    fn start(shell: &Path, directory: &Path) -> Self {
        let (controller, terminal) = nsh_platform::open_pseudoterminal().expect("open a terminal");
        nsh_platform::set_nonblocking(&controller, true).expect("poll the controller");
        let child = Command::new(shell)
            .arg("-i")
            .current_dir(directory)
            .env("TERM", "dumb")
            .env("PS1", PROMPT)
            .env("PS2", "> ")
            .env("LC_ALL", "C")
            /* A session that reads the invoking user's start-up file or
             * their saved history measures their machine, not the shell. */
            .env_remove("ENV")
            .env("HISTFILE", "")
            .stdin(Stdio::from(
                terminal.try_clone().expect("share the terminal"),
            ))
            .stdout(Stdio::from(
                terminal.try_clone().expect("share the terminal"),
            ))
            .stderr(Stdio::from(terminal))
            .spawn()
            .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
        Self {
            child,
            controller,
            transcript: Vec::new(),
        }
    }

    fn send(&mut self, line: &str) {
        let _ = self
            .controller
            .write_all(line.as_bytes())
            .and_then(|()| self.controller.flush());
    }

    fn contains(&self, marker: &str) -> bool {
        self.transcript
            .windows(marker.len())
            .any(|window| window == marker.as_bytes())
    }

    /// Read until `marker` has been written, the shell has gone, or the
    /// patience runs out.
    fn wait_for(&mut self, marker: &str) -> bool {
        let deadline = Instant::now() + PATIENCE;
        let mut buffer = [0; 4096];
        while Instant::now() < deadline {
            match self.controller.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => self.transcript.extend_from_slice(&buffer[..count]),
                // `EIO` once the last terminal end is closed: end of file.
                Err(error) if nsh_platform::is_pseudoterminal_end(&error) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    if self.contains(marker) {
                        return true;
                    }
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => panic!("cannot read the terminal: {error}"),
            }
            if self.contains(marker) {
                return true;
            }
        }
        self.contains(marker)
    }

    /// Everything the terminal showed, up to and including the marker that
    /// ends the comparison. Reading to a fixed point rather than to end of
    /// file keeps the shells' different exit-time writes out of it.
    fn transcript_through(&self, marker: &str) -> String {
        let text = String::from_utf8_lossy(&self.transcript);
        match text.find(marker) {
            Some(at) => text[..at + marker.len()].to_owned(),
            None => panic!("the transcript never reached {marker:?}: {text:?}"),
        }
    }

    /// Wait until the shell has drawn its first prompt, then forget
    /// everything written so far.
    ///
    /// Two things land before that prompt and belong to neither shell's
    /// answer: a job-control notice from whichever shell cannot claim the
    /// terminal it was handed, and the echo of anything typed before the
    /// shell was ready to read. Discarding to the first prompt starts both
    /// transcripts at the same place.
    fn ready(&mut self) {
        assert!(
            self.wait_for(PROMPT),
            "the shell never drew a prompt: {:?}",
            String::from_utf8_lossy(&self.transcript)
        );
        self.transcript.clear();
    }

    fn finish(mut self) {
        self.send("exit\n");
        let _ = self.child.wait();
    }
}

/// Type `probe` at both shells and hand back the two transcripts.
fn both(directory: &Path, probe: &str) -> (String, String) {
    let mut answers = Vec::new();
    for shell in [Path::new(env!("CARGO_BIN_EXE_nsh")), system_dash()] {
        let mut session = Session::start(shell, directory);
        session.ready();
        session.send(&format!("{probe}\n"));
        /* Spelled in two pieces so the typed line the terminal echoes does
         * not itself contain the marker: waiting for it would then stop
         * before the shell had answered anything. */
        session.send("printf '%s%s\\n' TRANSCRIPT -END\n");
        assert!(
            session.wait_for(DONE),
            "{} never finished the probe {probe:?}",
            shell.display()
        );
        answers.push(session.transcript_through(DONE));
        session.finish();
    }
    (answers.remove(0), answers.remove(0))
}

/// A directory holding the two operands, private to the calling check.
///
/// `[spec:nsh:req:oracle.checks-do-not-share-state]`: the harness runs
/// these as threads of one process, so a shared working directory would be
/// shared mutable state and the shells are started with `current_dir`
/// rather than by changing this process's own.
fn operands(name: &str) -> std::path::PathBuf {
    let directory = std::env::temp_dir().join(format!("nsh-dot-transcript-{name}"));
    std::fs::create_dir_all(&directory).expect("make a directory for the operands");
    std::fs::write(directory.join("speaks.sh"), b"echo IN-DOT\n").expect("write the operand");
    std::fs::write(directory.join("silent.sh"), b":\n").expect("write the operand");
    directory
}

/// `.` on a file that prints writes what dash writes.
// [spec:nsh:sem:idiom.specified-defects/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn dot_on_a_speaking_file_matches_dash() {
    let directory = operands("speaks");
    let (ours, theirs) = both(&directory, ". ./speaks.sh");
    assert_eq!(ours, theirs, "the transcripts differed");
}

/// `.` on a file that prints nothing writes what dash writes.
///
/// The one that shows the blank line is the built-in's and not the
/// operand's: there is no output for a missing final newline to be missing
/// from.
// [spec:nsh:sem:idiom.specified-defects/test]
#[test]
fn dot_on_a_silent_file_matches_dash() {
    let directory = operands("silent");
    let (ours, theirs) = both(&directory, ". ./silent.sh");
    assert_eq!(ours, theirs, "the transcripts differed");
}

/// The two ways of running the same text that are not `.` are unmoved.
///
/// They agreed before and have to still agree, because a fix that reached
/// them would be changing the prompt rather than the built-in.
// [spec:nsh:sem:idiom.specified-defects/test]
#[test]
fn eval_and_substitution_still_match_dash() {
    let directory = operands("others");
    for probe in ["eval \"$(cat ./speaks.sh)\"", "x=$(echo sub); echo $x"] {
        let (ours, theirs) = both(&directory, probe);
        assert_eq!(ours, theirs, "the transcripts differed for {probe}");
    }
}

/// Reaching the end of the shell's own input with `-i` live still writes
/// dash's newline.
///
/// Not a terminal case: it needs the shell to *run out* of input, which a
/// session that is typed at does not do. Both spellings are here because
/// they take different paths into the same loop -- a script operand and
/// standard input -- and only one of them was at risk.
// [spec:nsh:sem:idiom.specified-defects/test]
// [spec:posix:req:builtin.set.opt-o-ignoreeof/test]
#[test]
fn own_input_ending_still_writes_the_newline() {
    let directory = operands("own-input");
    let script = directory.join("turns-interactive.sh");
    std::fs::write(&script, b"set -i\necho done\n").expect("write the script");

    for shell in [Path::new(env!("CARGO_BIN_EXE_nsh")), system_dash()] {
        let as_operand = Command::new(shell)
            .arg(&script)
            .current_dir(&directory)
            .env("PS1", PROMPT)
            .env_remove("ENV")
            .output()
            .expect("run the script as an operand");
        let on_standard_input = Command::new(shell)
            .current_dir(&directory)
            .env("PS1", PROMPT)
            .env_remove("ENV")
            .stdin(Stdio::from(File::open(&script).expect("open the script")))
            .output()
            .expect("run the script on standard input");
        for (how, produced) in [
            ("operand", as_operand),
            ("standard input", on_standard_input),
        ] {
            let mut written = produced.stdout;
            written.extend_from_slice(&produced.stderr);
            assert!(
                written.ends_with(b"\n"),
                "{} lost dash's line termination reading its own input as {how}: {:?}",
                shell.display(),
                String::from_utf8_lossy(&written)
            );
        }
    }
}
