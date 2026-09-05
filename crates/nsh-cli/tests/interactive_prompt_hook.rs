#![cfg(unix)]
//! `PROMPT_COMMAND`: a hook the shell runs in its own environment before
//! each primary prompt.
//!
//! Every case here needs a real pseudo-terminal, for the reason
//! `interactive_signal_at_the_prompt.rs` gives at greater length: a
//! prompt exists only in a session someone is watching, and the whole
//! differential harness feeds scripts to shells on pipes. A hook that
//! never ran would pass every suite in this repository.
//!
//! Nor can these be asked differentially. The reference runs
//! `PROMPT_COMMAND` too, but its interactive `PS1` is `\s-\v\$ ` and its
//! `PROMPT_COMMAND` is an array whose elements it joins -- so a
//! transcript comparison would be measuring two shells' prompt defaults,
//! not the rule. `[spec:nsh:req:interactive.prompt-hook]` says in as many
//! words that the name is borrowed and the semantics are not.

use std::fs::File;
use std::io::{Read, Write};
use std::process::{Command, Stdio};

/// Run an interactive shell on its own terminal, feed it `script`, and
/// return everything the terminal saw.
///
/// The script is written in one go and must end by leaving the shell:
/// nothing here waits for a marker, so a session that does not exit
/// would hang on the read below rather than fail.
fn transcript_of(script: &[u8]) -> String {
    let (mut controller, terminal) = nsh_platform::open_pseudoterminal().expect("open a terminal");
    let mut child = Command::new(env!("CARGO_BIN_EXE_nsh"))
        .arg("-i")
        .env("TERM", "xterm")
        .env("PS1", "$ ")
        // A session that reads the invoking user's start-up file or their
        // saved history is measuring their machine, not the shell.
        .env_remove("ENV")
        .env_remove("PROMPT_COMMAND")
        .env("HISTFILE", "")
        .stdin(Stdio::from(terminal.try_clone().expect("share it")))
        .stdout(Stdio::from(terminal.try_clone().expect("share it")))
        .stderr(Stdio::from(terminal))
        .spawn()
        .expect("start an interactive shell");

    controller.write_all(script).expect("feed the shell");
    controller.flush().expect("feed the shell");

    let text = read_to_end(controller);
    let status = child.wait().expect("reap the shell");
    assert!(status.success(), "nsh exited with {status}: {text:?}");
    text
}

fn read_to_end(mut controller: File) -> String {
    let mut transcript = Vec::new();
    let mut buffer = [0; 4096];
    loop {
        match controller.read(&mut buffer) {
            Ok(0) => break,
            Ok(count) => transcript.extend_from_slice(&buffer[..count]),
            Err(error) if nsh_platform::is_pseudoterminal_end(&error) => break,
            Err(error) => panic!("cannot read pseudo-terminal: {error}"),
        }
    }
    String::from_utf8_lossy(&transcript).into_owned()
}

/// The hook runs before every prompt, and it runs in the shell's own
/// execution environment.
///
/// Both halves are one assertion. `n` surviving from one prompt to the
/// next is the environment -- a hook run in a subshell would leave every
/// prompt at `[1]` -- and the counter reaching two is the "before *each*
/// prompt" half. That the prompt shows the value at all is the third:
/// what the hook assigned is what the prompt expansion then read.
// [spec:nsh:req:interactive.prompt-hook/test]
#[test]
fn each_prompt_runs_the_hook_in_this_environment() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let text = transcript_of(
        b"PROMPT_COMMAND='n=$((n + 1)); PS1=\"[$n]@nsh@ \"'\n\
          :\n\
          exit\n",
    );
    for expected in ["[1]@nsh@ ", "[2]@nsh@ "] {
        assert!(
            text.contains(expected),
            "the prompt never showed {expected:?}: {text:?}"
        );
    }
}

/// The hook sees the last command's status, and the next command still
/// sees it too.
///
/// The trap this is about: the hook's own last command is an assignment,
/// which succeeds. A shell that did not restore the status would answer
/// `0` to the second field and make `false; echo $?` report the prompt's
/// success instead of the command's failure.
// [spec:nsh:req:interactive.prompt-hook/test]
#[test]
fn the_hook_reads_the_status_without_replacing_it() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let text = transcript_of(
        b"PROMPT_COMMAND='seen=$?'\n\
          false\n\
          printf '\\nSTATUS-%s-%s-OK\\n' \"$seen\" \"$?\"\n\
          exit\n",
    );
    assert!(
        text.contains("STATUS-1-1-OK"),
        "the hook did not see, or did not restore, the failing status: {text:?}"
    );
}

/// A hook that cannot even be parsed leaves the session standing, and
/// says whose failure it was.
///
/// `fi` is a syntax error in both dialects, so this asks nothing of Bash
/// mode. The name in the diagnostic is the point: the user has just run a
/// command of their own, and a bare `Syntax error` under its prompt reads
/// as that command having failed.
// [spec:nsh:req:interactive.prompt-hook/test]
#[test]
fn a_failing_hook_names_itself_and_prompts_again() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let text = transcript_of(
        b"PROMPT_COMMAND='fi'\n\
          printf '\\nALIVE-OK\\n'\n\
          exit\n",
    );
    assert!(
        text.contains("ALIVE-OK"),
        "a hook that failed to parse ended the session: {text:?}"
    );
    assert!(
        text.contains("PROMPT_COMMAND: Syntax error"),
        "the hook's diagnostic did not name the hook: {text:?}"
    );
}
