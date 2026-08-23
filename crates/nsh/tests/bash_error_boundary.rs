//! Where a failed assignment or a failed expansion stops.
//!
//! The two dialects part company here and the difference is the whole of
//! this file: POSIX ends a non-interactive shell with status 2, which is
//! XCU 2.8.1 and what the conformance harness observes, while Bash reports
//! the failure, answers 1 and abandons only the input record it was raised
//! in. Every expectation below was taken from GNU Bash side by side rather
//! than reasoned about.

use bstr::BStr;
use nsh::{Shell, Streams};

fn shell(bash: bool) -> Shell {
    Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"bash"), bash)
        .build()
        .expect("build shell")
}

fn run(shell: &mut Shell, script: &[u8]) -> (i32, Vec<u8>, Vec<u8>) {
    let status = shell.run(script).expect("run script").code().into();
    let stdout = shell
        .take_captured_stdout()
        .expect("capture stdout")
        .to_vec();
    let stderr = shell
        .take_captured_stderr()
        .expect("capture stderr")
        .to_vec();
    (status, stdout, stderr)
}

/// One Bash-mode script, its exit status and everything it printed.
fn bash_run(script: &[u8]) -> (i32, Vec<u8>, Vec<u8>) {
    run(&mut shell(true), script)
}

#[track_caller]
fn expect(script: &[u8], status: i32, stdout: &[u8]) {
    let (actual_status, printed, diagnostic) = bash_run(script);
    assert_eq!(
        (actual_status, BStr::new(&printed)),
        (status, BStr::new(stdout)),
        "script: {}",
        BStr::new(script)
    );
    assert!(
        !diagnostic.is_empty(),
        "the failure was recovered from without being reported: {}",
        BStr::new(script)
    );
}

/// The record is the unit Bash abandons, not the command and not the
/// shell: three commands on one line lose the two after the failure, and
/// the same three on three lines lose only the one that failed.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_read_only_assignment_abandons_its_record() {
    expect(
        b"readonly r=1\nr=2; echo same\necho next=$?\n",
        0,
        b"next=1\n",
    );
    expect(
        b"readonly r=1\nr=2\necho one=$?\nr=3\necho two=$?\n",
        0,
        b"one=1\ntwo=1\n",
    );
    // A function does not contain it; the caller's record goes too.
    expect(
        b"readonly r=1\nf() { echo a; r=2; echo b; }\nf; echo same\necho next=$?\n",
        0,
        b"a\nnext=1\n",
    );
    // A loop does not contain it either.
    expect(
        b"readonly r=1\nfor i in 1 2 3; do echo iter=$i; r=2; done\necho next=$?\n",
        0,
        b"iter=1\nnext=1\n",
    );
}

/// A subshell and a command substitution do contain it: the enclosing
/// shell sees a status and reads on.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_child_environment_contains_the_recovery() {
    expect(
        b"readonly r=1\n( echo a; r=2; echo b )\necho next=$?\n",
        0,
        b"a\nnext=1\n",
    );
    expect(
        b"a=(1 2)\nv=$(echo \"${a[0][0]}\"); echo same\necho next=$? v=$v\n",
        0,
        b"same\nnext=0 v=\n",
    );
}

/// `eval` recovers at its own record boundary, and takes nothing of the
/// caller with it -- the locals of the function that called it are still
/// there afterwards.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn eval_recovers_without_unwinding_its_caller() {
    expect(
        b"readonly r=1\nf() { local v=inner; eval 'r=2'; echo after=$v; }\nf\necho next=$?\n",
        0,
        b"after=inner\nnext=0\n",
    );
    // The enclosing loop still knows it is a loop.
    expect(
        b"readonly r=1\nfor i in 1 2; do eval 'r=2'; break; done\necho next=$?\n",
        0,
        b"next=0\n",
    );
}

/// A bad substitution is the same boundary; an arithmetic failure inside
/// a subscript is too, because the subscript is the expansion.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_bad_substitution_abandons_its_record() {
    expect(
        b"a=(1 2)\necho \"${a[0][0]}\"; echo same\necho next=$?\n",
        0,
        b"next=1\n",
    );
    expect(b"v=abcde\necho ${#v:1:3}\necho next=$?\n", 0, b"next=1\n");
    expect(b"echo $((1+)); echo same\necho next=$?\n", 0, b"next=1\n");
    expect(
        b"a=(x y)\nPWD=1\nref='a[~+]'\necho ${!ref}\necho next=$?\n",
        0,
        b"next=1\n",
    );
}

/// A subscript that names no element parts company from the other two: it
/// is reported and expands to nothing, and the command it was written in
/// still runs. Only an assignment through it refuses.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_missing_subscript_reads_as_nothing() {
    expect(
        b"a=(1 2)\necho one \"${a[-5]}\" two; echo same\necho next=$?\n",
        0,
        b"one  two\nsame\nnext=0\n",
    );
    expect(
        b"a=(1 2)\necho $(( a[-5] ))\necho next=$?\n",
        0,
        b"0\nnext=0\n",
    );
    expect(
        b"a=(1 2)\na[-5]=x; echo same\necho next=$?\necho a=${a[*]}\n",
        0,
        b"next=1\na=1 2\n",
    );
}

/// A special built-in's refusal of a read-only name becomes that
/// command's status. POSIX makes a special built-in's error fatal; Bash
/// runs the next command of the same list.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_special_builtin_refusal_is_a_status() {
    expect(
        b"readonly r=1\nunset r; echo same\necho next=$?\n",
        0,
        b"same\nnext=0\n",
    );
    expect(b"readonly r=1\nunset r\necho next=$?\n", 0, b"next=1\n");
    expect(b"readonly r=1\nexport r=2\necho next=$?\n", 0, b"next=1\n");
}

/// `set -e` is the script saying it wants to stop at the first error, and
/// the recovery must not be the thing that swallows one. Bash ends the
/// shell at every one of these when the option is on, and so does this.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn errexit_keeps_the_failure_fatal() {
    for script in [
        &b"set -e\nreadonly r=1\nr=2\necho next=$?\n"[..],
        b"set -e\nreadonly r=1\nr=2 || true\necho next=$?\n",
        b"set -e\nreadonly r=1\nif r=2; then echo t; fi\necho next=$?\n",
        b"set -e\na=(1 2)\necho \"${a[0][0]}\"\necho next=$?\n",
        // Reported without raising when the option is off, fatal when on.
        b"set -e\na=(1 2)\necho \"${a[-5]}\"\necho next=$?\n",
        b"set -e\na=(1 2)\na[-5]=x\necho next=$?\n",
    ] {
        let mut shell = shell(true);
        let error = shell.run(script).expect_err("errexit ends the shell");
        assert_eq!(
            i32::from(error.status().code()),
            1,
            "script: {}",
            BStr::new(script)
        );
        let printed = shell
            .take_captured_stdout()
            .expect("capture stdout")
            .to_vec();
        assert_eq!(
            BStr::new(&printed),
            BStr::new(b""),
            "script: {}",
            BStr::new(script)
        );
    }
}

/// The fatal form: the shell does not survive the failure, so `run`
/// answers with the error rather than a status. Both the status it
/// carries and the output written before it are of interest.
#[track_caller]
fn expect_fatal(bash: bool, script: &[u8]) {
    let mut shell = shell(bash);
    let error = shell.run(script).expect_err("the record ends the shell");
    assert_eq!(i32::from(error.status().code()), 2);
    let printed = shell
        .take_captured_stdout()
        .expect("capture stdout")
        .to_vec();
    assert_eq!(BStr::new(&printed), BStr::new(b""));
}

/// The default dialect keeps the boundary XCU 2.8.1 requires, and so does
/// Bash mode the moment `set -o posix` leaves the dialect.
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:compat.bash.posix-option/test]
#[test]
fn the_posix_dialect_keeps_the_fatal_boundary() {
    expect_fatal(false, b"readonly r=1\nr=2\necho next=$?\n");
    expect_fatal(true, b"set -o posix\nreadonly r=1\nr=2\necho next=$?\n");

    // And `set +o posix` puts the dialect, and the boundary, back.
    expect(
        b"set -o posix\nset +o posix\nreadonly r=1\nr=2\necho next=$?\n",
        0,
        b"next=1\n",
    );
}
