#![cfg(unix)]
//! A signal that arrives while the prompt is waiting must not end the session.
//!
//! This is the shape of defect no suite in this repository could see before
//! it. `gate-bash`, the Oils survey, the POSIX harness and smoosh all feed a
//! script on standard input or through `-c` and read what comes back, and a
//! shell that quits when a signal arrives passes every one of them: in a
//! script the shell is *supposed* to reach the end of its input. The defect
//! lived entirely in the interactive prompt read, which nothing drove.
//!
//! So these run the shipped binary on a real pseudo-terminal, which is what
//! starts the line editor, and send it a signal while it is blocked in the
//! read. `SIGCHLD` is the cheapest witness -- delivering one needs no child,
//! no timing and no job control -- and it was the reported symptom: an
//! interactive shell exited `0` at the exact moment a background job
//! finished, printing no `Done` notice and leaving the user with no session.
//!
//! The mechanism was that the editor's read reported every `EINTR` as
//! `HostFailure::Interrupted`, which completes as `ReadResult::Interrupted`,
//! which reached the parser as "no line" -- the same value a real end of
//! input produces. `SIGWINCH` and `SIGCONT` left the session alone only
//! because nothing catches them, so they never interrupted the read at all.

use nsh_platform::{ProcessId, ProcessTarget, SignalRequest};
use std::fs::File;
use std::io::{Read as _, Write as _};
use std::process::{Child, Command, Stdio};
use std::time::{Duration, Instant};

/// How long one wait for expected output may take before the shell is
/// declared hung. Generous, because it is a liveness bound and not a
/// measurement: every wait below ends as soon as its marker arrives.
const PATIENCE: Duration = Duration::from_secs(20);

/// Long enough for output the shell was going to write anyway to arrive.
const SETTLE: Duration = Duration::from_millis(300);

/// A prompt no echoed command line and no diagnostic can contain, so that
/// counting it counts prompts. "The shell re-prompted" is the only way to
/// say "it reacted where it stood" rather than one line later.
const PROMPT: &str = "@nsh@ ";

/// An interactive shell on its own pseudo-terminal, and the controller end.
struct Session {
    child: Child,
    controller: File,
    transcript: Vec<u8>,
}

impl Session {
    /// Start an interactive shell whose terminal declares `term`.
    ///
    /// That name is what decides which of the two reading paths runs: the
    /// editor starts only for a terminal whose declared capabilities can
    /// host redisplay, so `xterm` exercises the editor's read and `dumb`
    /// exercises `input::read_input_descriptor` directly. `nsh-cli`
    /// enables the `edit` feature unconditionally, so this is how the
    /// plain path is reached from a test at all.
    fn start(term: &str) -> Self {
        let (controller, terminal) = nsh_platform::open_pseudoterminal().expect("open a terminal");
        nsh_platform::set_nonblocking(&controller, true).expect("poll the controller");
        let child = Command::new(env!("CARGO_BIN_EXE_nsh"))
            .arg("-i")
            .env("TERM", term)
            .env("PS1", PROMPT)
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

    fn identity(&self) -> ProcessId {
        ProcessId::new(self.child.id()).expect("a spawned child has a positive identity")
    }

    /// Type a line at the terminal. A closed session refuses the write, and
    /// that is a result for the assertions below rather than a failure here.
    fn send(&mut self, line: &str) {
        let _ = self
            .controller
            .write_all(line.as_bytes())
            .and_then(|()| self.controller.flush());
    }

    /// Collect output until `marker` has been seen, the shell has gone, or
    /// `limit` has run out. Reports whether the marker arrived.
    fn collect(&mut self, marker: &str, limit: Duration) -> bool {
        let deadline = Instant::now() + limit;
        let mut buffer = [0; 4096];
        while Instant::now() < deadline {
            match self.controller.read(&mut buffer) {
                Ok(0) => break,
                Ok(count) => self.transcript.extend_from_slice(&buffer[..count]),
                // The controller reads `EIO` once the last terminal end has
                // been closed, which is this pair's end of file.
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

    fn wait_for(&mut self, marker: &str) -> bool {
        self.collect(marker, PATIENCE)
    }

    /// Take in whatever the shell has to say, without waiting for anything
    /// in particular. The negative assertions need this: "it has not written
    /// that yet" says nothing until there has been time to write it.
    fn settle(&mut self) {
        self.collect("", SETTLE);
    }

    fn contains(&self, marker: &str) -> bool {
        !marker.is_empty()
            && self
                .transcript
                .windows(marker.len())
                .any(|window| window == marker.as_bytes())
    }

    fn text(&self) -> String {
        String::from_utf8_lossy(&self.transcript).into_owned()
    }

    /// How many prompts the shell has written so far.
    fn prompts(&self) -> usize {
        self.text().matches(PROMPT).count()
    }

    /// How many times `text` appears in everything the terminal has shown,
    /// which for a command line the shell never redisplays is how often the
    /// terminal echoed it.
    fn occurrences(&self, text: &str) -> usize {
        self.text().matches(text).count()
    }

    /// Run a command whose output names `marker` without the typed line
    /// containing it, so that seeing the marker means the shell *ran* the
    /// line rather than merely echoed it back.
    fn run_printing(&mut self, marker: &str) {
        let (head, tail) = marker.split_at(marker.len() / 2);
        self.send(&format!("printf '%s%s\\n' {head} {tail}\n"));
    }

    /// The shell has run a command and written its prompt back, so its next
    /// act is to block in the read these tests are about.
    fn wait_at_the_prompt(&mut self, marker: &str) {
        assert!(
            self.wait_for(marker),
            "the shell never ran the command that puts it at the prompt: {:?}",
            self.text()
        );
        // The prompt is written immediately before the read blocks, and the
        // marker above was written before the prompt. Settling closes the
        // remaining microseconds between the two, so the signal lands in the
        // read rather than in front of it.
        self.settle();
    }

    /// Deliver one signal and let the shell finish reacting to it.
    ///
    /// The settle is what makes this test able to fail, and it was found by
    /// watching it not: `kill` returns once the signal is queued, so typing
    /// the next line immediately afterwards races it. The line lands in the
    /// terminal first, the interrupted read comes back with a byte instead
    /// of `EINTR`, and the signal is taken later at a point that was never
    /// under test. Even the shell with the defect passed that way.
    fn deliver(&mut self, signal: nsh_platform::Signal) {
        nsh_platform::send_signal(
            ProcessTarget::Process(self.identity()),
            SignalRequest::Deliver(signal),
        )
        .expect("deliver the signal");
        self.settle();
    }

    /// Ask the shell to leave with a status only a live session can
    /// produce, and report what it left with. `None` is "it did not".
    ///
    /// Bounded, and that is not tidiness. A shell whose input buffer has
    /// been left misaligned does not necessarily act on the `exit` it is
    /// sent -- the defect below splits `exit 7` across two reads, so the
    /// `exit` runs without its argument or not at all -- and closing the
    /// controller does not reach it either, because a child spawned this
    /// way is not a session leader and the pty is not its controlling
    /// terminal, so it sees `EIO` rather than a hangup. Waiting for it
    /// unbounded reports a defect as a hung test, which is the one
    /// outcome a test must never have.
    fn finish(mut self) -> Option<i32> {
        self.send("exit 7\n");
        self.settle();
        drop(self.controller);
        let deadline = Instant::now() + PATIENCE;
        loop {
            match self.child.try_wait().expect("wait for the shell") {
                Some(status) => return status.code(),
                None if Instant::now() >= deadline => {
                    let _ = self.child.kill();
                    let _ = self.child.wait();
                    return None;
                }
                None => std::thread::sleep(Duration::from_millis(5)),
            }
        }
    }
}

// [spec:nsh:req:interactive.signal-does-not-end-the-session/test]
#[test]
fn a_child_signal_does_not_end_the_session() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let mut session = Session::start("xterm");
    session.run_printing("READYREADY");
    session.wait_at_the_prompt("READYREADY");

    session.deliver(nsh_platform::child_signal());

    session.run_printing("ALIVEALIVE");
    let survived = session.wait_for("ALIVEALIVE");
    let transcript = session.text();
    let status = session.finish();

    assert!(
        survived,
        "a child signal at the prompt ended the session: {transcript:?}"
    );
    assert_eq!(
        status,
        Some(7),
        "the shell did not leave through its own `exit`: {transcript:?}"
    );
}

// [spec:nsh:req:interactive.signal-does-not-end-the-session/test]
#[test]
fn a_trapped_signal_runs_after_the_next_line() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let mut session = Session::start("xterm");
    // The action prints its marker in halves for the same reason
    // `run_printing` does: the terminal echoes the `trap` command itself, so
    // an action spelling its own output would be "seen" before it ever ran.
    session.send("trap 'printf \"%s%s\\n\" TRAP RAN' HUP\n");
    session.run_printing("SETSET");
    session.wait_at_the_prompt("SETSET");

    session.deliver(nsh_platform::hangup_signal());

    // dash and the pinned Bash 5.3.15 both leave the action for the next
    // `dotrap`, which is reached once the line being typed has been entered.
    // Measured 2026-09-02 on a pty: both print the action's output in front
    // of the *following* command's output, and neither prints it here.
    let at_the_prompt = session.contains("TRAPRAN");

    session.run_printing("AFTERAFTER");
    let survived = session.wait_for("AFTERAFTER");
    let ran = session.contains("TRAPRAN");
    let transcript = session.text();
    let status = session.finish();

    assert!(
        survived,
        "a trapped hangup at the prompt ended the session: {transcript:?}"
    );
    assert!(
        !at_the_prompt,
        "the trap action ran at the prompt, which is neither dash's order \
         nor the pinned Bash's: {transcript:?}"
    );
    assert!(ran, "the trap action never ran: {transcript:?}");
    assert_eq!(
        status,
        Some(7),
        "the shell did not leave through its own `exit`: {transcript:?}"
    );
}

/// With the line editor off, an interrupt has to be taken where it lands.
///
/// The mirror of the two cases above, and reachable only when
/// `editing_active` is false: `TERM=dumb`, a terminal whose terminfo cannot
/// host redisplay, or a `set +o emacs +o vi` session on such a terminal.
/// `input::read_input_descriptor` retried its interrupted read, and the
/// retry blocked again inside the deferral scope that
/// `refill_input_buffer` polls at the end of -- so the interrupt could not
/// be delivered until a line had been read, which means until the user had
/// typed one. Measured on a pty against `/usr/bin/dash` before the fix:
/// the `^C` printed nothing, the next command was swallowed, and the one
/// after it was echoed twice with the swallowed command's output welded to
/// the front of it. dash is right here only because `onint` longjmped out
/// of the read from inside the handler, which
/// `[dec:nsh:errors-are-values]` removed.
// [spec:nsh:req:interactive.signal-does-not-end-the-session/test]
#[test]
fn a_plain_read_takes_its_interrupt_at_once() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let mut session = Session::start("dumb");
    session.run_printing("READYREADY");
    session.wait_at_the_prompt("READYREADY");
    let before = session.prompts();

    session.deliver(nsh_platform::interrupt_signal());

    // dash prints a newline and a fresh prompt the moment the interrupt
    // arrives, with nothing typed in between, and `deliver` has already
    // waited for anything the shell was going to write.
    let reprompted = session.prompts() > before;

    session.run_printing("ALIVEALIVE");
    let survived = session.wait_for("ALIVEALIVE");
    // A line the shell reads correctly is echoed once by the terminal.
    // Twice is the misalignment: the defect left the input buffer holding
    // bytes the parser had already consumed.
    let echoes = session.occurrences("ALIVE ALIVE");
    let transcript = session.text();
    let status = session.finish();

    assert!(
        reprompted,
        "the interrupt printed no fresh prompt where it arrived: {transcript:?}"
    );
    assert!(
        survived,
        "the command typed after the interrupt never ran: {transcript:?}"
    );
    assert_eq!(
        echoes, 1,
        "the line after the interrupt was echoed {echoes} times, so the \
         input buffer was left misaligned: {transcript:?}"
    );
    assert_eq!(
        status,
        Some(7),
        "the shell did not leave through its own `exit`: {transcript:?}"
    );
}
