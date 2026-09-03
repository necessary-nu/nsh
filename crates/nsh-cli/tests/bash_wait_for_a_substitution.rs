//! What `wait` with no operands waits for, measured against the pinned
//! Bash 5.3.
//!
//! A process substitution is nobody's job: there is no job record for
//! `jobs` to print, so `wait` could not see one at all and returned
//! before the child had written. `echo hi > >(sleep 0.4; cat); wait;
//! echo after` printed `after` first and `hi` after the shell had gone.
//!
//! IT IS THE MOST RECENT ONE AND NOT ALL OF THEM, which is the rule the
//! rows below are shaped around. `echo a > >(sleep 0.5; cat); echo c >
//! >(sleep 0.1; cat); wait; echo after` prints `c`, then `after`, then
//! `a` in the reference: `wait` returned as soon as the last
//! substitution reported and the one before it outlived the shell.
//! Bash's own name for what it blocks on is `last_procsub_child`.
//!
//! EVERY ROW'S ORDER IS DECIDED BY A SLEEP AND NOT BY A RACE. Two
//! substitutions with the *same* sleep interleave differently from run
//! to run in both shells -- measured five runs each, the reference
//! produced both orders -- so no row has two children due at once, and
//! the durations are far enough apart to survive a loaded machine.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A single substitution, which `wait` blocks on however the name
/// was reached.
const ONE_SUBSTITUTION: &[&str] = &[
    "echo hi > >(sleep 0.3; cat)\nwait\necho after\n",
    "exec 3> >(sleep 0.3; cat)\necho hi >&3\nexec 3>&-\nwait\necho after\n",
    "f(){ echo hi > >(sleep 0.3; cat); }\nf\nwait\necho after\n",
    "echo hi > >(sleep 0.3; cat)\nwait\necho st=$?\n",
    /* An input substitution the command never read is waited for too,
     * where one the command read to end-of-file has already gone. */
    ": < <(sleep 0.3; echo p)\necho mid\nwait\necho after\n",
    "cat < <(sleep 0.3; echo produced)\nwait\necho after\n",
    /* A job beside it is waited for by the job table's own loop. */
    "sleep 0.1 &\necho hi > >(sleep 0.3; cat)\nwait\necho after\n",
    /* And it is still nobody's job. */
    "echo hi > >(sleep 0.3; cat)\njobs | wc -l\nwait\n",
    "wait\necho empty=$?\n",
];

/// Several substitutions, of which `wait` blocks on the last.
const THE_MOST_RECENT_ONE: &[&str] = &[
    "echo a > >(sleep 0.5; cat)\necho c > >(sleep 0.1; cat)\nwait\necho after\n",
    "echo a > >(sleep 0.5; cat)\necho b > >(sleep 0.3; cat)\necho c > >(sleep 0.1; cat)\nwait\necho after\n",
    "exec 3> >(sleep 0.5; cat)\nexec 4> >(sleep 0.1; cat)\necho a >&3\necho b >&4\nexec 3>&- 4>&-\nwait\necho after\n",
    /* A second `wait` has nothing left to block on. */
    "echo a > >(sleep 0.5; cat)\necho c > >(sleep 0.1; cat)\nwait\nwait\necho after\n",
    /* And a substitution made after a `wait` is the one the next `wait`
     * blocks on. */
    "echo one > >(sleep 0.5; cat)\nwait\necho two > >(sleep 0.1; cat)\nwait\necho after\n",
    /* A command substitution in between is not one of these, and does
     * not become what `wait` blocks on. */
    "echo x > >(sleep 0.3; cat)\ny=$(echo sub)\nwait\necho after\n",
];

/// Both shells on one script, as `(what nsh said, what the pinned Bash
/// said)`.
fn both(script: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer(nsh, &["-o", "bash"], script),
        pinned_bash::answer(&bash, &[], script),
    )
}

/// Every script in `cases` produces the reference's bytes and status.
fn agrees(cases: &[&str]) {
    for script in cases {
        let (ours, theirs) = both(script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "output differed for\n{script}"
        );
        assert_eq!(ours.1, theirs.1, "status differed for\n{script}");
    }
}

/// `wait` blocks until the substitution has written.
// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn wait_blocks_on_a_substitution() {
    agrees(ONE_SUBSTITUTION);
}

/// With several live, `wait` blocks on the most recent and no other.
// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn wait_blocks_on_the_most_recent() {
    agrees(THE_MOST_RECENT_ONE);
}
