//! What `$!` names after a process substitution, measured against the
//! pinned Bash 5.3.
//!
//! A process substitution is nobody's job, and `$!` names it anyway.
//! `echo hi > >(sleep 0.2; cat); echo "[$!]"` answers with the
//! substitution's own pid in the reference and answered with nothing
//! here. It is not a matter of filling in a blank either: a background
//! job's pid is *overwritten* by a substitution made after it, and taken
//! back by a background job made after that.
//!
//! NO ROW PRINTS A PID, because the two shells cannot produce the same
//! one and a test that compared them would be measuring the process
//! table. Every row prints a fact derived from one instead -- whether
//! `$!` is set, whether two readings of it differ, what `wait "$!"`
//! answers -- and each of those is the same in both shells or the shells
//! disagree.
//!
//! NO ROW LETS A SUBSTITUTION'S OUTPUT REACH THE COMPARED STREAM.
//! Neither shell waits for a `>(list)` child at exit, so a child that
//! writes to standard output is racing the shell's exit; that race is
//! `process-sub.test.sh:1` in the survey and both shells lose it under
//! load. The bodies here write to `/dev/null` or write nothing, so what
//! is compared is the shell's answer and not the machine's.
//!
//! WHAT `wait "$!"` STILL CANNOT ANSWER FOR is a substitution whose
//! status was collected before the wait, an older substitution than the
//! most recent, or one waited for from a subshell. This shell holds one
//! substitution's pid and no status where the reference holds every
//! substitution and what each reported; the difference is measured in
//! `wait-by-pid-for-an-older-process-substitution` and no row here
//! asserts either side of it.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// `$!` is set by a substitution that no job record mentions.
const A_SUBSTITUTION_SETS_IT: &[&str] = &[
    "echo hi > >(sleep 0.2; cat >/dev/null)\ntest -n \"$!\" && echo set || echo unset\n",
    /* The input direction sets it too: it is the fork that counts and
     * not which end of the pipe the shell kept. */
    "cat <(echo x) >/dev/null\ntest -n \"$!\" && echo set || echo unset\n",
    ": < <(sleep 0.2; echo p)\ntest -n \"$!\" && echo set || echo unset\n",
    /* A name opened by `exec` outlives the command that opened it, and
     * `$!` is written when the child is forked either way. */
    "exec 3> >(cat >/dev/null)\ntest -n \"$!\" && echo set || echo unset\nexec 3>&-\n",
    /* Reached through a function, and through a word that is not a
     * redirection at all. */
    "f(){ echo hi > >(sleep 0.2; cat >/dev/null); }\nf\ntest -n \"$!\" && echo set || echo unset\n",
    "x=<(echo hi)\ntest -n \"$!\" && echo set || echo unset\n",
    /* And nothing else in the neighbourhood sets it: a command
     * substitution and a here-document both fork and neither is named. */
    "y=$(echo sub)\ntest -n \"$!\" && echo set || echo unset\n",
    "cat <<EOF >/dev/null\nhere\nEOF\ntest -n \"$!\" && echo set || echo unset\n",
    "echo plain\ntest -n \"$!\" && echo set || echo unset\n",
];

/// Which of the two most recent forks `$!` names, when both a background
/// job and a substitution are in the running.
const THE_MOST_RECENT_FORK: &[&str] = &[
    /* A substitution overwrites a background job's pid. */
    "sleep 5 &\na=$!\necho hi > >(sleep 0.2; cat >/dev/null)\ntest \"$a\" = \"$!\" && echo same || echo different\n",
    /* And a background job started afterwards takes it back. */
    "echo hi > >(sleep 0.2; cat >/dev/null)\na=$!\nsleep 5 &\ntest \"$a\" = \"$!\" && echo same || echo different\n",
    "echo hi > >(sleep 0.2; cat >/dev/null)\nsleep 5 &\nb=$!\ntest \"$b\" = \"$!\" && echo same || echo different\n",
    /* Two substitutions do not name each other. */
    "echo a > >(sleep 0.2; cat >/dev/null)\na=$!\necho b > >(sleep 0.2; cat >/dev/null)\ntest \"$a\" = \"$!\" && echo same || echo different\n",
    /* A command substitution between the two leaves the substitution's
     * pid standing, which is the same rule `wait` follows. */
    "echo a > >(sleep 0.2; cat >/dev/null)\na=$!\ny=$(echo sub)\ntest \"$a\" = \"$!\" && echo same || echo different\n",
    /* A subshell reads the pid its parent had and does not disturb it. */
    "echo a > >(sleep 0.2; cat >/dev/null)\np=$!\n( test \"$p\" = \"$!\" && echo inherited || echo lost )\ntest \"$p\" = \"$!\" && echo kept || echo changed\n",
];

/// The pid is one `wait` can be given, and it answers for the
/// substitution's own exit.
const WAIT_TAKES_IT: &[&str] = &[
    "echo x > >(sleep 0.2; exit 7)\nwait \"$!\"\necho st=$?\n",
    "echo x > >(sleep 0.1; exit 0)\nwait \"$!\"\necho st=$?\n",
    /* It blocks, as `wait` with no operands does. */
    "echo hi > >(sleep 0.3; cat >/dev/null; echo written)\nwait \"$!\"\necho after\n",
    /* A background job beside it keeps its own pid answerable. */
    "echo x > >(sleep 0.3; exit 7)\np=$!\nsleep 0.1 &\nq=$!\nwait $p\necho p=$?\nwait $q\necho q=$?\n",
    /* A pid that is nobody's child at all is still 127. */
    "wait 999999\necho rc=$?\n",
    /* And it is still nobody's job while all of that is true. */
    "echo hi > >(sleep 0.2; cat >/dev/null)\njobs | wc -l\n",
    "sleep 5 &\necho hi > >(sleep 0.2; cat >/dev/null)\njobs | wc -l\n",
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

/// A process substitution puts its pid in `$!`.
// [spec:nsh:req:compat.bash.process-substitution/test]
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_substitution_is_named_in_the_bang_parameter() {
    agrees(A_SUBSTITUTION_SETS_IT);
}

/// `$!` names whichever of the two came last, in either order.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_bang_parameter_names_the_most_recent_fork() {
    agrees(THE_MOST_RECENT_FORK);
}

/// The pid `$!` holds is one `wait` accepts as an operand.
// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn wait_takes_the_substitutions_pid() {
    agrees(WAIT_TAKES_IT);
}
