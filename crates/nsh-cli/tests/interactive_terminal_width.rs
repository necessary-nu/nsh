#![cfg(unix)]
//! `COLUMNS`: the width of the terminal the shell is being watched on.
//!
//! A pseudo-terminal is the only way to ask. The width lives in the
//! terminal rather than in the shell, so a session on a pipe has no
//! answer and every other harness in this repository puts a script to a
//! shell on a pipe.
//!
//! The resizes are driven from *outside* the session, through `stty` on
//! the controller end of the pair -- which is what a terminal emulator
//! does when a window is dragged, and is the only way to move the width
//! while the shell is sitting in its read. A resize the session performs
//! on itself would prove much less: it happens at a moment the shell
//! chose.
//!
//! The pinned GNU Bash 5.3.15 -- `pinned_bash::path()`, never
//! /usr/bin/bash, which is 5.2 -- is driven through the identical
//! sequence, and what it answers is asserted rather than described.
//!
//! IT ANSWERS 80 TO ALL THREE, and both halves of that are the finding.
//! The 80 is a width no terminal reported: the reference falls back to
//! one where this shell publishes nothing, which is
//! `[spec:nsh:req:interactive.terminal-width]`'s "a fabricated one is
//! worse than none" measured rather than assumed. And it does not move
//! afterwards, because the reference learns of a resize from `SIGWINCH`
//! and a child started this way has no controlling terminal, so the
//! signal is never delivered to it -- verified separately through a
//! `pty.fork()` harness, where the same reference answers 80, 100, 133.
//! A shell that asks the terminal at a boundary needs no signal and no
//! controlling terminal, which is why this one answers the resizes here
//! and the reference cannot.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

/// This shell, interactive, reading nothing of the invoking user's.
const OURS: &[&str] = &["-i"];

/// The reference, interactive, reading nothing of the invoking user's.
/// `--norc --noprofile` is the reference's spelling of what this shell
/// gets from an empty `ENV`, and its long options must precede `-i` or
/// it prints its usage and leaves.
const THEIRS: &[&str] = &["--norc", "--noprofile", "-i"];

/// Long enough for the shell to have finished a line and drawn the next
/// prompt on a machine under load, and paid several times per session.
const SETTLE: Duration = Duration::from_millis(400);

/// An interactive shell on its own pseudo-terminal, with the controller
/// end kept so the window can be resized under it.
struct Session {
    child: Child,
    controller: File,
}

impl Session {
    fn start(shell: &Path, arguments: &[&str]) -> Self {
        let (controller, terminal) =
            nsh_platform::open_pseudoterminal().expect("open a pseudo-terminal");
        let child = Command::new(shell)
            .args(arguments)
            .env("TERM", "xterm")
            .env("PS1", "$ ")
            // A session that reads the invoking user's start-up file or
            // their saved history is measuring their machine.
            .env_remove("ENV")
            .env_remove("COLUMNS")
            .env_remove("PROMPT_COMMAND")
            .env("HISTFILE", "")
            .stdin(Stdio::from(terminal.try_clone().expect("share it")))
            .stdout(Stdio::from(terminal.try_clone().expect("share it")))
            .stderr(Stdio::from(terminal))
            .spawn()
            .expect("start an interactive shell");
        Self { child, controller }
    }

    /// Resize the window from the controller end, as an emulator does.
    ///
    /// `stty` acts on its standard input, so handing it the controller
    /// is the whole of it. A failure here is a failure of the check:
    /// a resize that did not happen would let every assertion below pass
    /// for the wrong reason.
    fn resize(&self, columns: u16, rows: u16) {
        let status = Command::new("stty")
            .args(["columns", &columns.to_string(), "rows", &rows.to_string()])
            .stdin(Stdio::from(
                self.controller.try_clone().expect("share the controller"),
            ))
            .stdout(Stdio::null())
            .stderr(Stdio::inherit())
            .status()
            .expect("run stty on the controller");
        assert!(status.success(), "stty could not resize the window");
        /* A shell that answered a resize asynchronously would need a
         * moment to do it, and one that is asked at a boundary does not.
         * The pause is here so that a failure below is the width and not
         * the wait. */
        std::thread::sleep(SETTLE);
    }

    fn send(&mut self, line: &[u8]) {
        self.controller.write_all(line).expect("feed the shell");
        self.controller.flush().expect("feed the shell");
        std::thread::sleep(SETTLE);
    }

    /// Everything the terminal saw, once the shell has gone.
    fn finish(mut self) -> String {
        let mut transcript = Vec::new();
        let mut buffer = [0; 4096];
        loop {
            match self.controller.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => transcript.extend_from_slice(&buffer[..count]),
                Err(error) if nsh_platform::is_pseudoterminal_end(&error) => break,
                Err(error) => panic!("cannot read pseudo-terminal: {error}"),
            }
        }
        let text = String::from_utf8_lossy(&transcript).into_owned();
        let status = self.child.wait().expect("reap the shell");
        assert!(status.success(), "nsh exited with {status}: {text:?}");
        text
    }
}

/// The width is published from the terminal, and follows it afterwards.
///
/// Three answers from one session, run through both shells and compared
/// rather than written down. A pair nobody has sized reports no width at
/// all, and the rule says a fabricated one is worse than none, so
/// `COLUMNS` is absent rather than guessed. Each resize then lands while
/// the shell is waiting at a prompt -- which is where a window is
/// actually dragged -- and the next command reads the size it was
/// dragged to.
///
/// The literal expectations are asserted as well as the agreement,
/// because two shells that both published nothing would agree.
///
/// The reference's three answers are asserted whole rather than
/// compared, for the reason the module comment gives: it never learns
/// that this window moved.
// [spec:nsh:req:interactive.terminal-width/test]
#[test]
fn columns_follows_the_terminal_it_is_drawn_on() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let ours = three_widths(Path::new(env!("CARGO_BIN_EXE_nsh")), OURS);
    assert_eq!(ours, ["unset", "100", "133"], "this shell");
    let theirs = three_widths(&pinned_bash::path(), THEIRS);
    assert_eq!(
        theirs,
        ["80", "80", "80"],
        "the reference no longer fabricates a width, or has grown a way \
         to see a resize with no controlling terminal"
    );
}

/// What one shell answers before any size is set, after a resize to 100,
/// and after a resize to 133 -- each resize landing while it waits at a
/// prompt.
fn three_widths(shell: &Path, arguments: &[&str]) -> [String; 3] {
    let mut session = Session::start(shell, arguments);
    std::thread::sleep(SETTLE);
    session.send(b"echo A=${COLUMNS-unset}\n");
    session.resize(100, 24);
    session.send(b"echo B=${COLUMNS-unset}\n");
    session.resize(133, 40);
    session.send(b"echo C=${COLUMNS-unset}\nexit\n");

    let text = session.finish();
    ["A=", "B=", "C="].map(|marker| reading(&text, marker))
}

/// The value on the one transcript line that begins with `marker`.
///
/// The echoed command line carries the marker too, but behind a prompt
/// and an `echo`, so a line that *starts* with it is the answer. A
/// carriage return within the line starts it over -- the reference ends
/// its bracketed-paste sequence that way -- so what counts as the start
/// is what follows the last one.
fn reading(text: &str, marker: &str) -> String {
    let found = text
        .lines()
        .filter_map(|line| line.rsplit('\r').next())
        .find_map(|line| line.strip_prefix(marker));
    found
        .unwrap_or_else(|| panic!("no line answering {marker:?}: {text:?}"))
        .to_owned()
}

/// A window dragged while a command is running is the width the next
/// prompt is drawn into.
///
/// The other half, and the one the refresh before the prompt exists for.
/// Reading the width once the line has been entered cannot answer this:
/// that reading was taken before `sleep` started, and the window moved
/// after it.
// [spec:nsh:req:interactive.terminal-width/test]
#[test]
fn a_prompt_sees_a_resize_during_the_command() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let mut session = Session::start(Path::new(env!("CARGO_BIN_EXE_nsh")), OURS);
    session.resize(100, 24);
    session.send(b"PROMPT_COMMAND='printf \"\\nWIDE-%s-OK\\n\" \"${COLUMNS-unset}\"'\n");
    session
        .controller
        .write_all(b"sleep 1\n")
        .expect("feed the shell");
    session.controller.flush().expect("feed the shell");
    std::thread::sleep(Duration::from_millis(300));
    session.resize(155, 24);
    std::thread::sleep(Duration::from_millis(1200));
    session.send(b"exit\n");

    let text = session.finish();
    assert!(
        text.contains("WIDE-155-OK"),
        "no prompt was drawn against the width the window ended at: {text:?}"
    );
}

/// A shell with no terminal leaves the name exactly as it found it.
///
/// Both directions, because "leave it alone" is two claims: nothing is
/// invented where there is nothing to report, and an inherited value is
/// the caller's and is not overwritten. The pinned Bash 5.3.15 answers
/// the same to both.
// [spec:nsh:req:interactive.terminal-width/test]
#[test]
fn a_shell_with_no_terminal_leaves_columns_alone() {
    let absent = answer(None, Script::Operand("echo ${COLUMNS-unset}"));
    assert_eq!(absent, "unset\n", "a shell on a pipe invented a width");
    let inherited = answer(Some("77"), Script::Operand("echo ${COLUMNS-unset}"));
    assert_eq!(
        inherited, "77\n",
        "a shell on a pipe overwrote the width it was given"
    );
    /* `-i` on a pipe is what separates "interactive" from "attached to
     * a terminal": this shell prompts and runs the prompt hook, reaching
     * every place a width would be published from, and still has no
     * width to report. */
    let watched = answer(None, Script::WatchedInput("echo ${COLUMNS-unset}\nexit\n"));
    assert_eq!(
        watched, "unset\n",
        "an interactive shell on a pipe invented a width"
    );
}

/// How the script reaches the shell, which is what decides whether the
/// command loop -- and so the width publication -- runs at all.
enum Script<'a> {
    /// `-c text`: no command loop, no prompt.
    Operand(&'a str),
    /// `-i` with the text on standard input: the command loop, prompting
    /// into a pipe.
    WatchedInput(&'a str),
}

/// Run the shell with no terminal anywhere, optionally carrying
/// `columns` in the environment, and return what it wrote to standard
/// output. The prompt goes to standard error and stays out of the
/// answer.
fn answer(columns: Option<&str>, script: Script<'_>) -> String {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nsh"));
    command.env_remove("ENV").env("HISTFILE", "");
    match columns {
        Some(value) => command.env("COLUMNS", value),
        None => command.env_remove("COLUMNS"),
    };
    let mut child = match script {
        Script::Operand(text) => command.args(["-c", text]),
        Script::WatchedInput(_) => command.arg("-i"),
    }
    .stdin(Stdio::piped())
    .stdout(Stdio::piped())
    .stderr(Stdio::null())
    .spawn()
    .expect("run the shell");
    let mut standard_input = child.stdin.take().expect("the shell has a standard input");
    if let Script::WatchedInput(text) = script {
        standard_input
            .write_all(text.as_bytes())
            .expect("feed the shell");
    }
    drop(standard_input);
    let output = child.wait_with_output().expect("reap the shell");
    assert!(output.status.success(), "nsh failed: {output:?}");
    String::from_utf8_lossy(&output.stdout).into_owned()
}
