//! Which descriptor numbers a script may name.
//!
//! POSIX's IO_NUMBER is "a string consisting solely of digits" and
//! `[spec:posix:syn:redir.format]` calls `n` a "one or more digit decimal
//! number", so the nameable set is every decimal number the host can hold
//! -- not the ten a single digit spells. dash reads only one digit and
//! treats `10<&0` as the command `10`; this shell does not.

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

// [spec:posix:syn:redir.format/test]
// [spec:posix:syn:grammar.token-classification/test]
#[test]
fn a_descriptor_past_the_inherited_range_is_nameable() {
    for bash in [false, true] {
        let mut shell = shell(bash);
        let (status, printed, _) = run(
            &mut shell,
            b"exec 42>&1\n\
              echo forty-two >&42\n\
              exec 42>&-\n\
              echo done",
        );

        assert_eq!(printed, b"forty-two\ndone\n".to_vec(), "bash={bash}");
        assert_eq!(status, 0, "bash={bash}");
    }
}

/// A two-digit number in front of the operator is the descriptor, not a
/// word: `10<&0` is a redirection, where dash reads it as the command `10`.
// [spec:posix:syn:redir.format/test]
#[test]
fn two_digits_are_one_io_number() {
    let mut shell = shell(true);
    let (status, printed, _) = run(
        &mut shell,
        b"exec 10<&0\n\
          exec 10<&-\n\
          echo ok",
    );

    assert_eq!(printed, b"ok\n".to_vec());
    assert_eq!(status, 0);
}

/// `>&n` duplicates onto the slot `n` names, and POSIX says `n` there is
/// "one or more digits" too.
// [spec:posix:req:redir.duplicate-output/test]
#[test]
fn a_duplication_target_takes_many_digits() {
    let mut shell = shell(true);
    let (status, printed, _) = run(
        &mut shell,
        b"exec 17>&1\n\
          exec 18>&17\n\
          echo both >&18\n\
          exec 17>&- 18>&-\n\
          echo end",
    );

    assert_eq!(printed, b"both\nend\n".to_vec());
    assert_eq!(status, 0);
}

/// The table is sparse and a slot is only a map key until a child is
/// exec'd, so a number the host could never hold has to be refused where
/// it is written rather than much later, somewhere the script cannot see.
// [spec:posix:syn:redir.format/test]
#[test]
fn a_descriptor_past_the_host_limit_is_refused() {
    let mut shell = shell(true);
    let (status, printed, complained) = run(&mut shell, b"echo x >&1000000000");

    assert!(printed.is_empty(), "nothing was written");
    assert_ne!(status, 0, "the redirection failed");
    assert!(
        complained.ends_with(b"1000000000: Bad file descriptor\n"),
        "unexpected diagnostic: {}",
        BStr::new(&complained),
    );
}

/// A digit run too large to name a slot is not an IO_NUMBER, and the
/// standard says the token identifier is then TOKEN -- an ordinary word.
// [spec:posix:syn:grammar.token-classification/test]
#[test]
fn an_unrepresentable_digit_run_stays_a_word() {
    let mut shell = shell(true);
    let (_, printed, _) = run(&mut shell, b"echo 99999999999999999999");

    assert_eq!(printed, b"99999999999999999999\n".to_vec());
}
