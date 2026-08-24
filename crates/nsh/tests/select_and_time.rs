//! `select` and `time`, the two reserved words this shell was missing.
//!
//! `time` is POSIX's (XCU 2.4) and is grammar in both dialects; `select`
//! is Bash's, and reserving it in the POSIX dialect would change what a
//! script that names a command `select` means. Both are script
//! constructs: a script containing either does not misbehave without it,
//! it fails to parse.

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

/// The menu and the prompt go to standard error; the body's output does
/// not. A reply that names a choice sets the variable, and `REPLY` always
/// carries what was actually typed.
// [spec:nsh:req:compat.bash.select-time-grammar/test]
#[test]
fn select_offers_a_menu_and_reads_a_choice() {
    let mut shell = shell(true);
    let (status, printed, menu) = run(
        &mut shell,
        b"select x in alpha beta; do echo \"got=[$x] reply=[$REPLY]\"; break; done <<EOF\n\
          2\n\
          EOF\n",
    );

    assert_eq!(printed, b"got=[beta] reply=[2]\n".to_vec());
    assert_eq!(menu, b"1) alpha\n2) beta\n#? ".to_vec());
    assert_eq!(status, 0);
}

/// A reply that names nothing leaves the variable empty and still runs
/// the body -- `REPLY` is how a script sees what was typed. A blank line
/// reprints the menu and runs nothing at all.
// [spec:nsh:req:compat.bash.select-time-grammar/test]
#[test]
fn a_reply_outside_the_menu_empties_the_name() {
    let mut shell = shell(true);
    let (status, printed, menu) = run(
        &mut shell,
        b"select x in only; do echo \"[$x][$REPLY]\"; done <<EOF\n\
          9\n\
          \n\
          zzz\n\
          EOF\n",
    );

    assert_eq!(printed, b"[][9]\n[][zzz]\n".to_vec());
    // The blank line reprints the menu; end of input closes the prompt.
    assert_eq!(menu, b"1) only\n#? #? 1) only\n#? #? \n".to_vec());
    // End of input answers 1 whatever the body last did.
    assert_eq!(status, 1);
}

/// Nothing to choose from is not an empty menu, it is no loop at all.
// [spec:nsh:req:compat.bash.select-time-grammar/test]
#[test]
fn select_over_nothing_runs_nothing() {
    let mut shell = shell(true);
    let (status, printed, menu) = run(&mut shell, b"select x in; do echo body; done");

    assert!(printed.is_empty());
    assert!(menu.is_empty());
    assert_eq!(status, 0);
}

/// A POSIX script may name a command `select`, so the word is not
/// reserved there.
// [spec:nsh:req:compat.bash.select-time-grammar/test]
#[test]
fn select_is_an_ordinary_name_in_posix() {
    let mut shell = shell(false);
    let (status, printed, _) = run(&mut shell, b"select() { echo function; }\nselect");

    assert_eq!(printed, b"function\n".to_vec());
    assert_eq!(status, 0);
}

/// `time` reports on standard error and answers with the pipeline's own
/// status. It prefixes the whole pipeline, `!` included.
// [spec:posix:req:token.reserved-word-time/test]
#[test]
fn time_reports_and_keeps_the_pipeline_status() {
    for bash in [false, true] {
        let mut shell = shell(bash);
        let (status, printed, report) = run(&mut shell, b"time false");
        assert!(printed.is_empty(), "the report is not standard output");
        assert!(
            report.starts_with(b"\nreal\t0m") && report.ends_with(b"s\n"),
            "unexpected report: {}",
            BStr::new(&report),
        );
        assert_eq!(status, 1, "the pipeline's status, not the report's");

        let (negated, _, _) = run(&mut shell, b"time ! true");
        assert_eq!(negated, 1, "`time` prefixes the `!` (bash={bash})");
    }
}

/// `-p` asks for the POSIX report format, and a bare `time` has nothing
/// to time.
// [spec:posix:req:token.reserved-word-time/test]
#[test]
fn time_p_uses_the_posix_format() {
    let mut shell = shell(true);
    let (status, printed, report) = run(&mut shell, b"time -p true");

    assert!(printed.is_empty());
    assert!(
        report.starts_with(b"real 0.0") && report.contains(&b'\n'),
        "unexpected report: {}",
        BStr::new(&report),
    );
    assert!(!report.starts_with(b"\n"), "no leading blank line under -p");
    assert_eq!(status, 0);

    let (bare, _, bare_report) = run(&mut shell, b"time");
    assert_eq!(bare, 0);
    assert!(bare_report.starts_with(b"\nreal\t0m0."));
}

/// `time` prefixes a built-in and a function, which an external
/// `time(1)` cannot see at all -- the reason it has to be a reserved word.
// [spec:posix:req:token.reserved-word-time/test]
#[test]
fn time_prefixes_a_function() {
    let mut shell = shell(true);
    let (status, printed, report) = run(&mut shell, b"f() { echo inside; return 3; }\ntime f");

    assert_eq!(printed, b"inside\n".to_vec());
    assert!(report.starts_with(b"\nreal\t"));
    assert_eq!(status, 3);
}

/// POSIX's grammar takes one `!` -- `pipeline: Bang pipe_sequence` --
/// and dash refuses a second. Bash repeats it and each one negates.
/// A Bash script that writes it has to run here, so the dialect decides.
/// Found by the `differential` fuzz target.
// [spec:nsh:req:compat.bash.select-time-grammar/test]
#[test]
fn repeated_negation_is_bash_only() {
    let mut bash_mode = shell(true);
    for (script, expected) in [
        ("! true; echo $?", "1\n"),
        ("! ! true; echo $?", "0\n"),
        ("! ! false; echo $?", "1\n"),
        ("! ! ! true; echo $?", "1\n"),
        ("! ! ! ! false; echo $?", "1\n"),
    ] {
        let (status, printed, _) = run(&mut bash_mode, script.as_bytes());
        assert_eq!(printed, expected.as_bytes(), "{script}");
        assert_eq!(status, 0, "{script}");
    }

    /* The POSIX dialect keeps refusing what the POSIX grammar refuses,
     * and a syntax error is an `Err` rather than a status, so this cannot
     * go through `run`. */
    let mut posix = shell(false);
    let refused = posix.run(b"! ! true; echo reached".as_slice());
    drop(posix.take_captured_stdout());
    drop(posix.take_captured_stderr());
    assert!(refused.is_err(), "POSIX mode accepted a second `!`");
}
