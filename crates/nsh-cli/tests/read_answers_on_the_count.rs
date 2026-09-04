//! `read -n N` answers on the Nth character, measured against the
//! pinned Bash 5.3 on the two sources a script can be given.
//!
//! Every case here writes one byte and no newline, and leaves the source
//! open. That is the whole measurement: a shell that returns when the
//! delimiter arrives rather than when the count is reached has nothing
//! to return, and waits forever. So the answer is taken with a budget
//! and a shell that spends it is a failure -- these tests cannot be
//! written to fail fast, because the defect they cover is a shell that
//! never answers at all.
//!
//! The budget is generous rather than tight for the same reason a
//! blocking test must be: it is not measuring how long an answer takes,
//! only that one arrives, and a loaded machine that has to start two
//! shells must not be able to fail this by being slow.
//!
//! The reference is measured on every row it is asked about, and its
//! silence would be a failure rather than a skip.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
// [spec:nsh:req:compat.bash.builtins-special-variables]

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::{Read, Write};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::time::Duration;

/// How long an answer is waited for.
const BUDGET: Duration = Duration::from_secs(20);

/// The script under measurement. One character, then say what it was.
const SCRIPT: &str = r#"read -rn1 a; printf 'a=%s\n' "$a""#;

/// What the script prints once it has the character.
const ANSWERED: &str = "a=X";

/// Read `source` to its end on a thread, so the caller can give up.
///
/// A pipe from a shell that never returns is never closed either, so a
/// read of it cannot be abandoned by the thread doing it; the thread is
/// left behind holding the descriptor and the caller stops listening.
/// The child is killed either way, which is what lets that thread end.
fn transcript_within<R>(source: R, budget: Duration) -> Option<Vec<u8>>
where
    R: Read + Send + 'static,
{
    let (sender, receiver) = mpsc::channel();
    std::thread::spawn(move || {
        let mut source = source;
        let mut transcript = Vec::new();
        /* The end of a pseudo-terminal whose child has gone reports an
         * error rather than an end of input, and the transcript read so
         * far is the answer in both cases. */
        drop(source.read_to_end(&mut transcript));
        drop(sender.send(transcript));
    });
    receiver.recv_timeout(budget).ok()
}

/// Kill the shell and collect it, whatever it was doing.
fn stop(mut child: Child) {
    drop(child.kill());
    drop(child.wait());
}

/// One shell, one byte on a pipe, and whatever it says within the budget.
///
/// Standard input stays open across the wait: closing it would hand the
/// shell an end of input, and a shell that answers a count only because
/// its source ran out has not answered the count.
fn over_a_pipe(shell: &Path, dialect: &[&str], script: &str) -> Option<Vec<u8>> {
    let mut child = Command::new(shell)
        .args(dialect)
        .args(["-c", script])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    let mut source = child.stdin.take().expect("the shell's standard input");
    source.write_all(b"X").expect("write the character");
    source.flush().expect("flush the character");
    let output = child.stdout.take().expect("the shell's standard output");
    let transcript = transcript_within(output, BUDGET);
    stop(child);
    drop(source);
    transcript
}

/// The same on a pseudo-terminal, which is where a person types.
///
/// The terminal is left in the mode it was created with. A canonical
/// terminal hands over nothing at all until Enter, so a shell that reads
/// harder still gets nothing: answering here means the shell took the
/// terminal out of that mode for the length of the read, which is what
/// the reference does.
fn over_a_terminal(shell: &Path, dialect: &[&str], script: &str) -> Option<Vec<u8>> {
    let (mut controller, terminal) = nsh_platform::open_pseudoterminal().expect("a terminal pair");
    let child = Command::new(shell)
        .args(dialect)
        .args(["-c", script])
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .env("TERM", "xterm")
        .stdin(Stdio::from(
            terminal.try_clone().expect("clone the terminal"),
        ))
        .stdout(Stdio::from(
            terminal.try_clone().expect("clone the terminal"),
        ))
        .stderr(Stdio::from(terminal))
        .spawn()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    /* The shell has to have reached its `read` before the character is
     * written, or a canonical terminal buffers it where the shell is not
     * yet looking and the row measures the wait rather than the read. */
    std::thread::sleep(Duration::from_millis(500));
    controller.write_all(b"X").expect("type the character");
    controller.flush().expect("flush the character");
    let transcript = transcript_within(controller, BUDGET);
    stop(child);
    transcript
}

/// What a row says when nothing came back.
fn answer(named: &str, transcript: Option<Vec<u8>>) -> String {
    let transcript = transcript.unwrap_or_else(|| {
        panic!("{named} did not answer `read -rn1` within {BUDGET:?}, so the count never ended it")
    });
    String::from_utf8_lossy(&transcript).into_owned()
}

/// `read -rn1` on a pipe returns on the character, in both shells.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_counted_pipe_read_answers_at_once() {
    let reference = pinned_bash::path();
    let reference_said = answer("the pinned Bash", over_a_pipe(&reference, &[], SCRIPT));
    assert!(
        reference_said.contains(ANSWERED),
        "the pinned Bash answered {reference_said:?}, not {ANSWERED:?}"
    );

    let ours = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let we_said = answer("nsh -o bash", over_a_pipe(ours, &["-o", "bash"], SCRIPT));
    assert!(
        we_said.contains(ANSWERED),
        "nsh answered {we_said:?}, not {ANSWERED:?}"
    );
}

/// The descriptor `-u` names has always answered on the character, and
/// is the row that localised the defect to the shared input stack.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_counted_read_of_a_named_descriptor_answers() {
    const NAMED: &str = r#"exec 3<&0; read -rn1 -u3 a; printf 'a=%s\n' "$a""#;

    let reference = pinned_bash::path();
    let reference_said = answer("the pinned Bash", over_a_pipe(&reference, &[], NAMED));
    assert!(
        reference_said.contains(ANSWERED),
        "the pinned Bash answered {reference_said:?}, not {ANSWERED:?}"
    );

    let ours = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let we_said = answer("nsh -o bash", over_a_pipe(ours, &["-o", "bash"], NAMED));
    assert!(
        we_said.contains(ANSWERED),
        "nsh answered {we_said:?}, not {ANSWERED:?}"
    );
}

/// `read -rn1` on a canonical terminal returns on the character, in both
/// shells -- the row a menu or a "press any key" is made of.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_counted_terminal_read_answers_at_once() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let reference = pinned_bash::path();
    let reference_said = answer("the pinned Bash", over_a_terminal(&reference, &[], SCRIPT));
    assert!(
        reference_said.contains(ANSWERED),
        "the pinned Bash answered {reference_said:?}, not {ANSWERED:?}"
    );

    let ours = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let we_said = answer(
        "nsh -o bash",
        over_a_terminal(ours, &["-o", "bash"], SCRIPT),
    );
    assert!(
        we_said.contains(ANSWERED),
        "nsh answered {we_said:?}, not {ANSWERED:?}"
    );
}

/// A count is not the only record that can end before its line: `-d`
/// moves the terminator, and the bytes before it are just as complete.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_delimited_read_answers_at_the_delimiter() {
    const DELIMITED: &str = r#"read -r -d X a; printf 'a=%s\n' "${a}X""#;

    let reference = pinned_bash::path();
    let reference_said = answer("the pinned Bash", over_a_pipe(&reference, &[], DELIMITED));
    assert!(
        reference_said.contains(ANSWERED),
        "the pinned Bash answered {reference_said:?}, not {ANSWERED:?}"
    );

    let ours = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let we_said = answer("nsh -o bash", over_a_pipe(ours, &["-o", "bash"], DELIMITED));
    assert!(
        we_said.contains(ANSWERED),
        "nsh answered {we_said:?}, not {ANSWERED:?}"
    );

    /* `-d` is POSIX.1-2024's, not an extension of the Bash dialect, and
     * the input stack it waits on is the same one. */
    // [spec:posix:req:builtin.read.option-d]
    let posix_said = answer("nsh", over_a_pipe(ours, &[], DELIMITED));
    assert!(
        posix_said.contains(ANSWERED),
        "nsh in its default dialect answered {posix_said:?}, not {ANSWERED:?}"
    );
}

/// A read with no count still waits for its line, on both sources.
///
/// The change that makes a counted read answer early is one flag, and a
/// flag left set would make the shell hand the parser half a line. This
/// is the row that would notice.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn an_uncounted_read_still_waits_for_the_line() {
    const WHOLE_LINE: &str = r#"read -r a; printf 'a=%s\n' "$a""#;

    let ours = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let short = Duration::from_secs(2);
    for dialect in [&[][..], &["-o", "bash"][..]] {
        let mut child = Command::new(ours)
            .args(dialect)
            .args(["-c", WHOLE_LINE])
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .env("LC_ALL", "C")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start nsh");
        let mut source = child.stdin.take().expect("the shell's standard input");
        source.write_all(b"X").expect("write the character");
        source.flush().expect("flush the character");
        let output = child.stdout.take().expect("the shell's standard output");
        let transcript = transcript_within(output, short);
        stop(child);
        drop(source);
        assert!(
            transcript.is_none(),
            "nsh {dialect:?} answered an uncounted `read` from {ANSWERED:?} with no newline: \
             {transcript:?}"
        );
    }
}
