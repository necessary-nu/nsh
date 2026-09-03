//! What an interactive shell does on a real terminal, by default.
//!
//! Both tests here run the shipped binary on a pseudo-terminal, because
//! that is what starts the line editor and draws a prompt. Nothing a
//! script on a pipe can do reaches either path: an interactive shell is
//! the one configuration the differential harness cannot put a script
//! to.

use std::fs::File;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

fn terminal_pair() -> (File, File) {
    nsh_platform::open_pseudoterminal().unwrap()
}

fn read_terminal(mut controller: File) -> Vec<u8> {
    let mut transcript = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match controller.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => transcript.extend_from_slice(&buffer[..count]),
            Err(error) if nsh_platform::is_pseudoterminal_end(&error) => {
                break;
            }
            Err(error) => panic!("cannot read pseudo-terminal: {error}"),
        }
    }
    transcript
}

// [spec:nsh:req:interactive.default-history-navigation/test]
#[test]
fn interactive_default_uses_arrow_history() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let (mut controller, terminal) = terminal_pair();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nsh"))
        .env("TERM", "xterm")
        .env("PS1", "$ ")
        .stdin(Stdio::from(terminal.try_clone().unwrap()))
        .stdout(Stdio::from(terminal.try_clone().unwrap()))
        .stderr(Stdio::from(terminal))
        .spawn()
        .unwrap();

    controller
        .write_all(
            b"n=0\n\
n=$((n + 1))\n\
\x1b[A\n\
a=DEFAULT; b=UP; test \"$n\" -eq 2 && printf '\\n%s-%s-OK\\n' \"$a\" \"$b\"\n\
a=DEFAULT; b=DOWN; \x1b[A\x1b[Btest \"$n\" -eq 2 && printf '\\n%s-%s-OK\\n' \"$a\" \"$b\"\n\
exit\n",
        )
        .unwrap();
    controller.flush().unwrap();

    let transcript = read_terminal(controller);
    let status = child.wait().unwrap();
    assert!(status.success(), "nsh exited with {status}: {transcript:?}");
    assert!(
        transcript
            .windows(b"DEFAULT-UP-OK".len())
            .any(|window| window == b"DEFAULT-UP-OK"),
        "up-arrow did not re-run the previous command: {transcript:?}"
    );
    assert!(
        transcript
            .windows(b"DEFAULT-DOWN-OK".len())
            .any(|window| window == b"DEFAULT-DOWN-OK"),
        "down-arrow did not restore the live command line: {transcript:?}"
    );
}

/// A prompt an echoed command line cannot contain, so finding it in the
/// transcript is the shell having drawn one.
const PROMPT: &str = "@nsh@ ";

/// An interactive Bash-mode shell keeps both prompts and draws with them,
/// where a shell nobody is watching has neither name at all.
///
/// It cannot be asked differentially. `pinned_bash::answer` puts a script
/// to a shell on a pipe, which is never interactive on either side, and
/// the reference's own interactive `PS1` is `\s-\v\$ ` -- a value this
/// shell has never claimed. So what is checked is that the two names
/// carry this shell's defaults and that the prompt reaches the terminal.
// [spec:nsh:req:compat.bash.names.only-what-the-reference-has/test]
#[test]
fn a_watched_bash_mode_shell_keeps_its_prompts() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let (mut controller, terminal) = terminal_pair();
    let mut child = Command::new(env!("CARGO_BIN_EXE_nsh"))
        .args(["-o", "bash"])
        .env("TERM", "xterm")
        .env_remove("PS1")
        .env_remove("PS2")
        .stdin(Stdio::from(terminal.try_clone().unwrap()))
        .stdout(Stdio::from(terminal.try_clone().unwrap()))
        .stderr(Stdio::from(terminal))
        .spawn()
        .unwrap();

    controller
        .write_all(b"declare -p PS1\ndeclare -p PS2\nPS1='@nsh@ '\necho DRAWN\nexit\n")
        .unwrap();
    controller.flush().unwrap();

    let transcript = read_terminal(controller);
    let status = child.wait().unwrap();
    let text = String::from_utf8_lossy(&transcript).into_owned();
    assert!(status.success(), "nsh exited with {status}: {text:?}");
    for expected in ["declare -- PS1=\"\\$ \"", "declare -- PS2=\"> \"", PROMPT] {
        assert!(
            text.contains(expected),
            "an interactive Bash-mode shell did not show {expected:?}: {text:?}"
        );
    }
}
