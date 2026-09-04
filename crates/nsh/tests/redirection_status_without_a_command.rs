//! What a redirection failure answers when no command word was written.
//!
//! A simple command's failed redirection has always answered the
//! dialect's number -- 2 in the POSIX dialect, which is dash's, and 1 in
//! Bash mode, which is the reference's. Seven shapes with no command word
//! did not: a redirected group, a redirected `if`, a command word that
//! expanded to nothing, three spellings of a null command's redirection,
//! and a `noclobber` refusal. Each forced a literal
//! `ExitStatus::FAILURE` over the status `OpenFailureContext::status`
//! had already computed, so the shell gave 2 where a command word was
//! written and 1 where one was not -- a distinction neither reference
//! makes. `bash.divergences.redirection-status-without-a-command` is the
//! node.
//!
//! The POSIX column is dash's, measured against `/usr/bin/dash` 0.5.12-12
//! and `tests/.build/ref/src/dash` of the same version on 2026-09-04:
//! every shape here answers 2 there, with a diagnostic this shell already
//! matched byte for byte. It is recorded rather than run because dash is
//! not wired into this crate's test harness the way `pinned_bash` wires
//! the reference Bash; the whole-corpus differential sweep is what runs
//! it. The Bash column is not recorded from anywhere -- the last test
//! below asks the pinned Bash 5.3.15 itself.

mod pinned_bash;

use bstr::BStr;
use nsh::{Shell, Streams};

/// One redirection failure with no command word in front of it.
struct Case {
    /// The failing record. `echo "s=$?"` is appended as its own record,
    /// because what moves here is the status the failure *leaves* rather
    /// than the status the shell ends with: none of these is fatal in
    /// either dialect.
    failure: &'static str,
    /// The default dialect's answer, which is dash's.
    posix: i32,
    /// Bash's own answer, which Bash mode is required to match.
    bash: i32,
}

const fn case(failure: &'static str, posix: i32, bash: i32) -> Case {
    Case {
        failure,
        posix,
        bash,
    }
}

/// One case per shape that reaches a site without a command word: the
/// two compound forms go through `evaluate_tree`'s `Node::Redirect |
/// Node::Group` arm, the other three through
/// `classify_abandoned_command`.
const CASES: &[Case] = &[
    case("{ echo hi; } < /nonexistent-nsh-redir/zzz", 2, 1),
    case("if : ; then echo t; fi < /nonexistent-nsh-redir/zzz", 2, 1),
    case("u=; $u < /nonexistent-nsh-redir/zzz", 2, 1),
    case("> /nonesuch-dir-nsh-redir/x", 2, 1),
    case("< /nonexistent-nsh-redir/zzz", 2, 1),
    case("<-", 2, 1),
];

/// The failure, then a record that only reports what it left behind.
fn script(failure: &str) -> String {
    format!("{failure}\necho \"s=$?\"\n")
}

/// One script's standard output in the named dialect.
fn run(source: &str, bash: bool) -> Vec<u8> {
    let mut shell = Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"bash"), bash)
        .build()
        .expect("build shell");
    shell
        .run(source)
        .unwrap_or_else(|error| error.status())
        .code();
    let printed = shell
        .take_captured_stdout()
        .expect("capture stdout")
        .to_vec();
    shell.take_captured_stderr().expect("capture stderr");
    printed
}

/// A file that exists, so `set -C` has something to refuse.
///
/// Written from Rust rather than by the script under test: a `noclobber`
/// refusal is decided before anything is opened, so a case that created
/// its own target would be asserting the refusal it is trying to
/// measure.
fn existing_file(name: &str) -> std::path::PathBuf {
    let directory =
        std::env::temp_dir().join(format!("nsh-redir-status-{}-{name}", std::process::id()));
    std::fs::create_dir_all(&directory).expect("create the case directory");
    let path = directory.join("clobbered");
    std::fs::write(&path, b"one\n").expect("write the file noclobber refuses");
    path
}

/// The POSIX dialect answers dash's 2 for every shape, which is the half
/// the whole-corpus differential sweep observes.
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:posix:req:redir.output-noclobber/test]
#[test]
fn the_posix_dialect_answers_dashs_two() {
    for case in CASES {
        assert_eq!(
            BStr::new(&run(&script(case.failure), false)),
            BStr::new(format!("s={}\n", case.posix).as_bytes()),
            "the POSIX dialect disagrees with dash about {}",
            case.failure
        );
    }
}

/// Bash mode still answers 1, which is the number the redirection layer
/// takes for that dialect. Nothing here moved with the POSIX half.
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn bash_mode_still_answers_one() {
    for case in CASES {
        assert_eq!(
            BStr::new(&run(&script(case.failure), true)),
            BStr::new(format!("s={}\n", case.bash).as_bytes()),
            "Bash mode disagrees with Bash about {}",
            case.failure
        );
    }
}

/// A `noclobber` refusal reaches the same site as a failed open, so it
/// takes the same number. This is the row that connected the family to
/// the survey case that first exposed it.
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:posix:req:redir.output-noclobber/test]
#[test]
fn a_noclobber_refusal_takes_the_same_number() {
    for (bash, expected) in [(false, 2), (true, 1)] {
        let path = existing_file(if bash { "bash" } else { "posix" });
        let source = format!("set -C\n> {}\necho \"s=$?\"\n", path.display());
        assert_eq!(
            BStr::new(&run(&source, bash)),
            BStr::new(format!("s={expected}\n").as_bytes()),
            "a noclobber refusal answered the wrong number"
        );
        assert_eq!(
            std::fs::read(&path).expect("the refused file is still there"),
            b"one\n",
            "the refusal must not have opened the file"
        );
    }
}

/// The Bash column above is the reference's answer rather than this
/// repository's opinion.
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_recorded_bash_answers_are_the_references_own() {
    let bash = pinned_bash::path();
    for case in CASES {
        let (stdout, _) = pinned_bash::answer(&bash, &[], &script(case.failure));
        assert_eq!(
            BStr::new(&stdout),
            BStr::new(format!("s={}\n", case.bash).as_bytes()),
            "the reference disagrees with the recorded answer for {}",
            case.failure
        );
    }
}
