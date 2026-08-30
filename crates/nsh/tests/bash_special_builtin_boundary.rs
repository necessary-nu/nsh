//! Where a special built-in's own failure stops, measured against the
//! pinned Bash 5.3.
//!
//! POSIX makes an error in a special built-in fatal to a non-interactive
//! shell, and the default dialect keeps that: the record's remaining
//! commands and every record after it are never read. Bash makes the same
//! failure an ordinary command failure -- it reports, takes a status, and
//! runs the next command of the same list -- and Bash mode does that now.
//!
//! Two frames could end a shell over one of these and both are exercised
//! here. `exec 3</nonesuch` and `: > /nodir/x` fail in the redirection
//! layer before the utility is entered; the rest fail inside it. The last
//! two cases are the ones Bash itself still ends the shell for, so they
//! pin the withdrawal as narrow rather than total.
//!
//! The reference's wording is not asserted anywhere below, because it
//! differs from this shell's in almost every case and is registered as
//! such in `docs/divergences.md`; what is asserted is whether execution
//! continued, and with what status.

mod pinned_bash;

use bstr::BStr;
use nsh::{Shell, Streams};
use std::io::Write as _;
use std::process::{Command, Stdio};

/// A special built-in that fails, and what each dialect does with it.
///
/// Every script is two records, the second of which only runs if the
/// first did not end the shell.
struct Case {
    /// The failing command. `echo after` is appended as its own record.
    failure: &'static [u8],
    /// The default dialect's exit status and standard output.
    posix: (i32, &'static [u8]),
    /// Bash's own answer, which Bash mode is required to match.
    bash: (i32, &'static [u8]),
}

const fn case(
    failure: &'static [u8],
    posix: (i32, &'static [u8]),
    bash: (i32, &'static [u8]),
) -> Case {
    Case {
        failure,
        posix,
        bash,
    }
}

/// One case per way a special built-in can fail: the redirection layer
/// before the utility, the utility's own refusal of an operand, its
/// option scan, and the loop count Bash refuses fatally.
const CASES: &[Case] = &[
    /* The redirection layer refuses before the utility is entered. */
    case(b"exec 1000000</dev/null", (1, b""), (0, b"after\n")),
    case(b"exec 3</nonesuch-nsh-boundary", (1, b""), (0, b"after\n")),
    case(
        b": > /nonesuch-dir-nsh-boundary/x",
        (1, b""),
        (0, b"after\n"),
    ),
    /* The utility itself refuses. */
    case(b"unset -v 'a['", (2, b""), (0, b"after\n")),
    case(b"local x=1", (2, b""), (0, b"after\n")),
    case(b"export 'a['=1", (2, b""), (0, b"after\n")),
    case(b"readonly 'a['=1", (2, b""), (0, b"after\n")),
    case(b". /nonesuch-file-nsh-boundary", (1, b""), (0, b"after\n")),
    case(b"eval 'syntax ((('", (2, b""), (0, b"after\n")),
    /* An unknown option is the same class: `set` and `unset` are special
     * built-ins, so refusing one of their options used to end the shell.
     * `shift 99` is here for the boundary only -- Bash makes an
     * over-shift a silent status 1 where this reports it, which is a
     * question about `shift` and not about where the shell stops. */
    case(b"set -o nosuchopt-nsh-boundary", (2, b""), (0, b"after\n")),
    case(b"unset -v -q x", (2, b""), (0, b"after\n")),
    case(b"shift 99", (2, b""), (0, b"after\n")),
    /* Bash reads a loop count through `get_numeric_arg`'s fatal flag,
     * which ends the shell rather than returning, so these two keep the
     * boundary in both dialects. Recovering here would leave the loop
     * that asked to be left still running. */
    case(
        b"while true; do echo hi; break oops; done",
        (2, b"hi\n"),
        (2, b"hi\n"),
    ),
    case(
        b"for i in 1 2; do echo hi; continue oops; done",
        (2, b"hi\n"),
        (2, b"hi\n"),
    ),
];

/// The two records of one case: the failure, then a command that only
/// runs if the shell survived it.
fn script(case: &Case) -> Vec<u8> {
    let mut source = case.failure.to_vec();
    source.extend_from_slice(b"\necho after\n");
    source
}

/// One script's exit status and standard output in the named dialect.
fn run(case: &Case, bash: bool) -> (i32, Vec<u8>) {
    let mut shell = Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"bash"), bash)
        .build()
        .expect("build shell");
    let status = shell
        .run(script(case).as_slice())
        .unwrap_or_else(|error| error.status())
        .code()
        .into();
    let printed = shell
        .take_captured_stdout()
        .expect("capture stdout")
        .to_vec();
    shell.take_captured_stderr().expect("capture stderr");
    (status, printed)
}

/// Bash mode reports the failure and runs the next record.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn a_special_builtins_failure_is_a_status() {
    for case in CASES {
        let (status, printed) = run(case, true);
        assert_eq!(
            (status, BStr::new(&printed)),
            (case.bash.0, BStr::new(case.bash.1)),
            "Bash mode disagrees with Bash about {}",
            BStr::new(case.failure)
        );
    }
}

/// The POSIX dialect keeps the fatal boundary. This is the half the
/// conformance harness is built on, so it is asserted here rather than
/// left to the harness to notice.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_posix_dialect_still_ends_the_shell() {
    for case in CASES {
        let (status, printed) = run(case, false);
        assert_eq!(
            (status, BStr::new(&printed)),
            (case.posix.0, BStr::new(case.posix.1)),
            "the POSIX dialect should have ended the shell at {}",
            BStr::new(case.failure)
        );
    }
}

/// The Bash column is the reference's answer, not this repository's
/// opinion.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn the_recorded_answers_are_the_references_own() {
    let bash = pinned_bash::path();
    for case in CASES {
        let mut child = Command::new(&bash)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the pinned Bash");
        child
            .stdin
            .take()
            .expect("the child's standard input")
            .write_all(&script(case))
            .expect("write the script");
        let output = child.wait_with_output().expect("wait for the pinned Bash");
        assert_eq!(
            (output.status.code(), BStr::new(&output.stdout)),
            (Some(case.bash.0), BStr::new(case.bash.1)),
            "the reference disagrees with the recorded answer for {}",
            BStr::new(case.failure)
        );
    }
}
