#![cfg(unix)]
//! Where the cursor lands after a prompt that is not plain ASCII.
//!
//! A prompt's *display* width is not its byte count and not its character
//! count. A colour sequence occupies no column, a window-title sequence
//! occupies no column and ends differently from a colour one, an East Asian
//! character occupies two, a combining mark occupies none, and a prompt with
//! a newline in it starts its last row over at column zero. Get any of them
//! wrong and the editor wraps the line in the wrong place and then redraws
//! over what it already wrote.
//!
//! None of that is visible to a test that compares strings, because a
//! mis-measured prompt produces the same bytes -- they land in the wrong
//! columns. So each case here types past the end of the row and asks where
//! the editor broke it, which is the display width the editor believed,
//! arrived at from outside the shell.
//!
//! Measured against a release build of `e04418d` on 2026-09-05, load 3 to 5:
//! a prompt carrying `ESC [ 1 ; 3 3 m ... ESC [ 0 m` around four columns of
//! text was drawn as seventeen columns of `^[[1;33mab^[[0m> `, so the row
//! broke thirteen columns early. The two title-sequence spellings were drawn
//! as sixteen and thirteen columns. Wide characters, combining marks and the
//! newline were already right, because the editor lays its own text out;
//! what nothing did was recognise a sequence.

use std::fs::File;
use std::io::{Read as _, Write as _};
use std::process::{Child, Command, ExitStatus, Stdio};
use std::time::{Duration, Instant};

/// How long one wait may take before the shell is declared hung. Generous,
/// because it bounds liveness rather than measuring anything: every wait
/// below ends as soon as the terminal falls quiet.
const PATIENCE: Duration = Duration::from_secs(20);

/// How long the terminal must say nothing before the editor is taken to have
/// finished drawing. A fixed sleep would be a guess about this machine's
/// load; this is a guess about how long a redraw takes to start.
const QUIET: Duration = Duration::from_millis(250);

/// Enough typed characters to fill a whole row between two others, so that
/// the longest unbroken run is one full row and therefore the width of the
/// terminal the pseudo-terminal pair chose. Nothing here may assume that
/// width: it belongs to the terminal, and it has to be measured before the
/// prompts can be.
const CALIBRATION: usize = 512;

/// How far past the end of the first row each measurement types. Enough that
/// the row certainly breaks, and few enough that the second row is shorter
/// than the first, which is what makes the longest run the first row's.
const OVERRUN: usize = 8;

/// Four columns, and no sequence of any kind. Every other prompt here is
/// measured against what this one leaves room for.
const PLAIN: &str = "ab> ";

/// The typed line is a comment, so that entering it runs nothing.
///
/// The line has to be entered rather than abandoned: an interrupt typed at
/// this terminal is text, because a shell spawned onto a pseudo-terminal this
/// way has no controlling terminal to raise a signal from, and the editor
/// binds the byte to nothing. Entering an ordinary command would put the
/// whole typed line back in a diagnostic naming what could not be found,
/// which holds a longer run of the typed character than any row does.
const COMMENT: &str = "#";

/// An interactive shell on its own pseudo-terminal, and the controller end.
struct Session {
    child: Child,
    controller: File,
    transcript: Vec<u8>,
}

impl Session {
    fn start(prompt: &str) -> Self {
        let (controller, terminal) = nsh_platform::open_pseudoterminal().expect("open a terminal");
        nsh_platform::set_nonblocking(&controller, true).expect("poll the controller");
        let child = Command::new(env!("CARGO_BIN_EXE_nsh"))
            .arg("-i")
            .env("TERM", "xterm")
            .env("PS1", prompt)
            // A session that reads the invoking user's start-up file or
            // their saved history is measuring their machine, not the shell.
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
            .expect("start an interactive shell");
        Self {
            child,
            controller,
            transcript: Vec::new(),
        }
    }

    fn send(&mut self, text: &str) {
        self.controller
            .write_all(text.as_bytes())
            .and_then(|()| self.controller.flush())
            .expect("type at the terminal");
    }

    /// Collect output until the terminal has been quiet for [`QUIET`], the
    /// shell has gone, or [`PATIENCE`] has run out.
    fn settle(&mut self) {
        let deadline = Instant::now() + PATIENCE;
        let mut spoke = Instant::now();
        let mut buffer = [0; 4096];
        while Instant::now() < deadline && spoke.elapsed() < QUIET {
            match self.controller.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => {
                    self.transcript.extend_from_slice(&buffer[..count]);
                    spoke = Instant::now();
                }
                // The controller reads `EIO` once the last terminal end has
                // been closed, which is this pair's end of file.
                Err(error) if nsh_platform::is_pseudoterminal_end(&error) => break,
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {
                    std::thread::sleep(Duration::from_millis(5));
                }
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
                Err(error) => panic!("cannot read the terminal: {error}"),
            }
        }
    }

    /// Reap the shell, killing it if it did not leave on its own.
    ///
    /// Waiting without a bound is how one broken case in this file becomes a
    /// `cargo test --workspace` that never returns for whoever else is in the
    /// checkout. A shell that had to be killed reports `None`, which is a
    /// failure carrying its transcript rather than a wait nobody can see.
    fn finish(&mut self) -> Option<ExitStatus> {
        let deadline = Instant::now() + PATIENCE;
        while Instant::now() < deadline {
            if let Some(status) = self.child.try_wait().expect("ask after the shell") {
                return Some(status);
            }
            std::thread::sleep(Duration::from_millis(20));
        }
        let _ = self.child.kill();
        let _ = self.child.wait();
        None
    }

    /// The longest run of one byte the terminal was ever shown unbroken.
    ///
    /// The editor draws a row at a time and breaks the row itself rather than
    /// letting the terminal wrap, so a run of typed characters is never
    /// longer than the row holding it.
    fn longest_run(&self, byte: u8) -> usize {
        let mut longest = 0;
        let mut run = 0;
        for found in &self.transcript {
            run = if *found == byte { run + 1 } else { 0 };
            longest = longest.max(run);
        }
        longest
    }
}

/// Type `typed` characters after `prompt` and report the longest run of them
/// the terminal was shown, together with everything it was shown.
fn beside(prompt: &str, typed: usize) -> (usize, Vec<u8>) {
    let mut session = Session::start(prompt);
    session.settle();
    session.send(&format!("{COMMENT}{}", "X".repeat(typed)));
    session.settle();
    let run = session.longest_run(b'X');
    session.send("\n");
    session.settle();
    session.send("exit\n");
    session.settle();
    let status = session.finish();
    assert!(
        status.is_some_and(|status| status.success()),
        "the shell did not survive a wrapped line under {prompt:?}: {:?}",
        String::from_utf8_lossy(&session.transcript)
    );
    (run, session.transcript)
}

/// The width of the terminal these sessions get, measured rather than
/// assumed: with more than two rows of typed text, one whole row is drawn
/// unbroken and that row is the width.
fn terminal_columns() -> usize {
    let columns = beside(PLAIN, CALIBRATION).0;
    assert!(
        columns >= 40 && 2 * columns <= CALIBRATION,
        "a terminal {columns} columns wide is not one this file can calibrate against"
    );
    columns
}

/// How many columns of typed text fitted beside `prompt`, on a terminal
/// already known to be `columns` wide.
fn columns_beside(prompt: &str, columns: usize) -> usize {
    let typed = columns + OVERRUN;
    let (run, _) = beside(prompt, typed);
    assert!(
        run < columns && run > typed - run,
        "{prompt:?} did not break its first row where a first row breaks: \
         {run} of {typed} typed on a terminal {columns} wide"
    );
    run
}

/// Each of these occupies the four columns [`PLAIN`] does, by a different
/// route, and one occupies six.
// [spec:nsh:req:interactive.prompt-display-width/test]
#[test]
fn a_prompt_is_measured_in_columns_not_bytes() {
    let columns = terminal_columns();
    let plain = columns_beside(PLAIN, columns);

    for (shape, prompt) in [
        ("a colour sequence", "\u{1b}[1;33mab\u{1b}[0m> "),
        ("a title sequence ended by a bell", "\u{1b}]0;t\u{7}ab> "),
        (
            "a title sequence ended by a string terminator",
            "\u{1b}]0;t\u{1b}\\ab> ",
        ),
        ("a combining mark", "e\u{301}b> "),
        ("a newline", "a first row\nab> "),
    ] {
        assert_eq!(
            columns_beside(prompt, columns),
            plain,
            "{shape} did not leave a four-column prompt's room: {prompt:?}"
        );
    }

    assert_eq!(
        columns_beside("\u{65e5}\u{672c}> ", columns),
        plain - 2,
        "two East Asian Wide characters did not take two columns each"
    );
}

/// The sequences are handed to the terminal, not shown to the reader.
///
/// This is the half of the requirement a column count cannot see: a shell
/// that drew `^[[1;33m` in eight columns and left eight columns for it would
/// place its cursor correctly and still have no colour in its prompt.
// [spec:nsh:req:interactive.prompt-display-width/test]
#[test]
fn a_prompts_sequences_reach_the_terminal_unshown() {
    let (_, transcript) = beside("\u{1b}[1;33mab\u{1b}[0m> ", OVERRUN);
    for sequence in [b"\x1b[1;33m".as_slice(), b"\x1b[0m".as_slice()] {
        assert!(
            transcript
                .windows(sequence.len())
                .any(|window| window == sequence),
            "the terminal was never sent {sequence:?}: {:?}",
            String::from_utf8_lossy(&transcript)
        );
    }
    assert!(
        !transcript.windows(2).any(|window| window == b"^["),
        "the escape was drawn as visible text: {:?}",
        String::from_utf8_lossy(&transcript)
    );
}
