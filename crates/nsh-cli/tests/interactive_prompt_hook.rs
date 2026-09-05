#![cfg(unix)]
//! `PROMPT_COMMAND`: a hook the shell runs in its own environment before
//! each primary prompt, and the state it is given to read.
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
use std::time::Duration;

/// Long enough that a shell which timed the gap between two prompts
/// instead of the command between them would report it, and short enough
/// to pay once per test that asks for it.
const PAUSE: Duration = Duration::from_millis(700);

/// Run an interactive shell on its own terminal, feed it `chunks` with a
/// [`PAUSE`] between each, and return everything the terminal saw.
///
/// A single chunk is written in one go and pauses nowhere. Every script
/// must end by leaving the shell: nothing here waits for a marker, so a
/// session that does not exit would hang on the read below rather than
/// fail.
fn transcript_of(chunks: &[&[u8]]) -> String {
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

    for (index, chunk) in chunks.iter().enumerate() {
        if index > 0 {
            std::thread::sleep(PAUSE);
        }
        controller.write_all(chunk).expect("feed the shell");
        controller.flush().expect("feed the shell");
    }

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
    let text = transcript_of(&[b"PROMPT_COMMAND='n=$((n + 1)); PS1=\"[$n]@nsh@ \"'\n\
          :\n\
          exit\n"]);
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
    let text = transcript_of(&[b"PROMPT_COMMAND='seen=$?'\n\
          false\n\
          printf '\\nSTATUS-%s-%s-OK\\n' \"$seen\" \"$?\"\n\
          exit\n"]);
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
    let text = transcript_of(&[b"PROMPT_COMMAND='fi'\n\
          printf '\\nALIVE-OK\\n'\n\
          exit\n"]);
    assert!(
        text.contains("ALIVE-OK"),
        "a hook that failed to parse ended the session: {text:?}"
    );
    assert!(
        text.contains("PROMPT_COMMAND: Syntax error"),
        "the hook's diagnostic did not name the hook: {text:?}"
    );
}

/// Every millisecond figure the hook reported, oldest first.
///
/// The echoed command line carries the literal `DUR-%s-OK` too, and it
/// falls out here rather than being special-cased: `%s` is not a number.
fn reported_durations(text: &str) -> Vec<u64> {
    text.split("DUR-")
        .skip(1)
        .filter_map(|tail| tail.split('-').next())
        .filter_map(|digits| digits.parse().ok())
        .collect()
}

/// The hook is told how many jobs the shell is tracking.
///
/// The background job holds none of the terminal's three descriptors, so
/// the transcript still ends when the shell does rather than when the
/// `sleep` does.
// [spec:nsh:req:interactive.prompt-state/test]
#[test]
fn the_hook_is_told_the_job_count() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let text = transcript_of(&[
        b"PROMPT_COMMAND='printf \"\\nJOBS-%s-OK\\n\" \"$NSH_JOBS\"'\n\
          sleep 3 </dev/null >/dev/null 2>&1 &\n\
          exit\n",
    ]);
    for expected in ["JOBS-0-OK", "JOBS-1-OK"] {
        assert!(
            text.contains(expected),
            "the hook never reported {expected:?}: {text:?}"
        );
    }
}

/// The duration is the shell's own measurement around the whole record.
///
/// `slow | slow | fast` is the case the rule names, and it separates
/// three implementations. A hook sampling a clock at each prompt would
/// answer with the time since the last prompt -- which here, where the
/// whole session is written at once, is the same thing and so proves
/// nothing; a *pre-execution* hook per command would time only the
/// `true` and answer near zero; measuring around the record answers with
/// the pipeline, whose members start together. Half a second is asked
/// for and four hundred milliseconds asserted, because a busy machine
/// makes a lower bound safer, never tighter.
// [spec:nsh:req:interactive.prompt-state/test]
#[test]
fn the_duration_covers_the_whole_pipeline() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let text = transcript_of(&[
        b"PROMPT_COMMAND='printf \"\\nDUR-%s-OK\\n\" \"$NSH_DURATION_MS\"'\n\
          sleep 0.5 | sleep 0.5 | true\n\
          exit\n",
    ]);
    let reported = reported_durations(&text);
    assert_eq!(
        reported.first().copied(),
        Some(0),
        "the prompt before any command should report no duration: {text:?}"
    );
    assert!(
        reported.iter().any(|milliseconds| *milliseconds >= 400),
        "no prompt reported the pipeline's own duration, only {reported:?}: {text:?}"
    );
}

/// The two names are the hook's, and the shell takes them back.
///
/// Deliberate, and this is where the decision is pinned:
/// `[spec:nsh:req:compat.bash.names.only-what-the-reference-has]` forbids
/// Bash mode publishing a name the reference has not got, so the state
/// the hook reads cannot be left on the table for `declare -p` to list.
/// A value the user had put there is given back rather than lost.
// [spec:nsh:req:interactive.prompt-state/test]
#[test]
fn the_lent_state_is_given_back() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let text = transcript_of(&[b"NSH_JOBS=mine\n\
          PROMPT_COMMAND='printf \"\\nSEEN-%s-OK\\n\" \"$NSH_JOBS\"'\n\
          printf '\\nKEPT-%s-%s-OK\\n' \"$NSH_JOBS\" \"${NSH_DURATION_MS-unset}\"\n\
          exit\n"]);
    assert!(
        text.contains("SEEN-0-OK"),
        "the hook did not read the shell's job count: {text:?}"
    );
    assert!(
        text.contains("KEPT-mine-unset-OK"),
        "the state outlived the hook, or the borrowed name was not given back: {text:?}"
    );
}

/// The duration is the command's, not the gap between two prompts.
///
/// This is the half `the_duration_covers_the_whole_pipeline` cannot ask:
/// there the whole session is written at once, so a shell timing
/// prompt-to-prompt would answer the same. Here the session pauses in
/// the middle, with the shell blocked at a prompt for [`PAUSE`], and then
/// runs a command that takes no time at all. A shell that sampled a
/// clock at each prompt would charge the pause to `true`.
// [spec:nsh:req:interactive.prompt-state/test]
#[test]
fn the_duration_is_not_the_gap_between_prompts() {
    if !nsh_platform::supports_bidirectional_pseudoterminal_pair() {
        return;
    }
    let text = transcript_of(&[
        b"PROMPT_COMMAND='printf \"\\nDUR-%s-OK\\n\" \"$NSH_DURATION_MS\"'\n",
        b"true\n\
          exit\n",
    ]);
    let reported = reported_durations(&text);
    assert!(
        reported.len() >= 2,
        "expected a duration at each prompt, got {reported:?}: {text:?}"
    );
    assert!(
        reported.iter().all(|milliseconds| *milliseconds < 400),
        "a prompt charged the wait for input to the command: {reported:?}: {text:?}"
    );
}
