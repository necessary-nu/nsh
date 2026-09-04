//! Exact witnesses for the Smoosh error, status, and diagnostic profile.

use nsh::streams::Streams;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_CASE: AtomicU64 = AtomicU64::new(0);

fn run(script: &str, interactive: bool, file_operand: bool) -> (Vec<u8>, Vec<u8>, i32) {
    let directory = std::env::temp_dir().join(format!(
        "nsh-smoosh-error-{}-{}",
        std::process::id(),
        NEXT_CASE.fetch_add(1, Ordering::Relaxed)
    ));
    std::fs::create_dir_all(&directory).expect("create isolated case directory");

    let startup = if file_operand {
        let script_path = directory.join("case.sh");
        std::fs::write(&script_path, script).expect("write case script");
        nsh::Startup::script(script_path.as_os_str().as_encoded_bytes().to_vec())
    } else {
        nsh::Startup::command(script.as_bytes().to_vec())
    };

    let (stdout_read, stdout_write) = nsh_platform::pipe().expect("create stdout pipe");
    let (stderr_read, stderr_write) = nsh_platform::pipe().expect("create stderr pipe");

    let status = nsh_platform::run_in_child(move || {
        std::env::set_current_dir(directory).expect("enter isolated case directory");
        let supplied = Streams::from_fds(std::io::stdin(), &stdout_write, &stderr_write)
            .expect("duplicate test streams");
        let mut builder = nsh::Shell::builder()
            .argument_zero(bstr::BStr::new(b"smoosh"))
            .inherit_env()
            .streams(supplied)
            .host(nsh::ProcessHost);
        if interactive {
            builder = builder
                .shell_option(nsh::ShellOption::Interactive, true)
                .shell_option(nsh::ShellOption::Monitor, true);
        }
        let mut shell = builder.build().expect("build process shell");
        let status = shell.run_to_completion(startup);
        nsh_platform::exit_immediately(status.code().into());
    })
    .expect("run shell child");

    let stdout = nsh_platform::read_to_end(&stdout_read).expect("read stdout");
    let stderr = nsh_platform::read_to_end(&stderr_read).expect("read stderr");
    (stdout, stderr, status)
}

/// `command` withdraws a special built-in's fatality, not its status.
/// The demotion is the whole of what this case is for and it still
/// holds -- the shell survives and prints the `?=` line -- while the
/// number is the dialect's 2, which is dash's answer for the same
/// script. Smoosh records 1, and that byte is a sanctioned divergence.
// [spec:nsh:req:compat.smoosh.error-contracts/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn command_demotes_readonly() {
    let (stdout, stderr, status) = run(
        "command readonly x=foo\ncommand readonly x=bar\necho ?=$?",
        false,
        false,
    );

    assert_eq!(stdout, b"?=2\n", "demoted, but still the dialect's number");
    assert_eq!(stderr, b"readonly: x: is read only\n");
    assert_eq!(status, 0);
}

/// `.` is a POSIX special built-in, so a file it cannot find ends a
/// non-interactive shell with the dialect's 2, which is dash's answer.
/// The diagnostic stays Smoosh's prefix-less spelling; only the number
/// moves, and Smoosh's 1 is a sanctioned divergence.
///
/// `source` is the control: it is not a POSIX built-in, dash has no
/// answer for it, so nothing collides with the imported 1 and it keeps
/// it. Two names, one code path, two oracles.
// [spec:nsh:req:compat.smoosh.error-contracts/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn missing_dot_is_fatal() {
    let (stdout, stderr, status) = run(". ./nonesuch", false, false);

    assert!(stdout.is_empty());
    assert_eq!(stderr, b".: ./nonesuch: not found\n");
    assert_eq!(status, 2, "the dialect boundary, not the imported 1");

    let (stdout, stderr, status) = run("source ./nonesuch", false, false);
    assert!(stdout.is_empty());
    assert_eq!(stderr, b"source: ./nonesuch: not found\n");
    assert_eq!(status, 1, "no second oracle, so the imported 1 stands");
}

/// A redirection error on a *directly invoked special built-in* ends the
/// shell with the status the redirection layer already took, which is
/// the dialect's 2 and is dash's answer byte for byte. Smoosh records 1
/// and that is a sanctioned divergence.
///
/// `exec 9&<-` is the contrast and it does not move. It parses as a
/// backgrounded `exec 9` beside a foreground `<-`, so what fails is a
/// redirection-only command with no special built-in in front of it --
/// a different clause of the same Smoosh bullet, which dash also answers
/// 2 for and which no rule this repository wrote has yet contested.
/// `bash.divergences.redirection-status-without-a-command` holds it.
// [spec:nsh:req:compat.smoosh.error-contracts/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn special_redirections_take_the_dialects_status() {
    let (stdout, _, special) = run(": 2>&9\necho unreachable", false, false);
    let (_, _, no_command) = run("exec 9&<-", false, false);

    assert!(stdout.is_empty(), "the shell ended before `echo`");
    assert_eq!(special, 2);
    assert_eq!(
        no_command, 1,
        "no built-in in front of it, so still Smoosh's"
    );
}

/// A declaration utility's refusal of a read-only name is the sentence
/// `[spec:nsh:req:compat.bash.error-boundary]` is written about, so the
/// default dialect takes 2 -- which is also what a plain `a=c` on the
/// same name has always answered here, and what dash answers for both.
/// Smoosh's 1 is a sanctioned divergence.
// [spec:nsh:req:compat.smoosh.error-contracts/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn readonly_assignment_is_fatal() {
    let (stdout, stderr, status) = run("readonly a=b\nexport a=c\necho unreachable", false, false);

    assert!(stdout.is_empty());
    assert_eq!(stderr, b"export: a: is read only\n");
    assert_eq!(status, 2);

    let (_, stderr, status) = run("readonly a=b\nreadonly a=c\necho unreachable", false, false);
    assert_eq!(stderr, b"readonly: a: is read only\n");
    assert_eq!(status, 2, "one site, two names, one answer");
}

/// The one imported Smoosh result this profile declines, and the only
/// thing about the case that moves. Smoosh records status 1 for a refused
/// `unset`; `[spec:nsh:req:compat.bash.error-boundary]` writes 2 down for
/// the default dialect and dash answers 2, so the imported byte loses to
/// the rule this repository wrote about its own boundary. The stdout, the
/// diagnostic and the shell ending there are still Smoosh's, and the
/// second run is what says "ending there" rather than inferring it from
/// the status.
// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn unset_readonly_ends_the_shell_with_two() {
    let smoosh_case = "readonly x=foo\ny=bar\nunset y\necho ${y-unset}\necho ${x-error}\nunset y\necho ${y-unset}\nunset x";
    let (stdout, stderr, status) = run(smoosh_case, false, false);

    assert_eq!(stdout, b"unset\nfoo\nunset\n");
    assert_eq!(stderr, b"unset: x is read-only\n");
    assert_eq!(status, 2, "the dialect boundary, not the imported 1");

    let (stdout, _, status) = run(&format!("{smoosh_case}\necho reached"), false, false);
    assert_eq!(stdout, b"unset\nfoo\nunset\n");
    assert_eq!(status, 2);
}

/// The interactive and non-interactive halves of an unset-parameter `?`
/// expansion. Only the non-interactive status moves: dash answers 2 and
/// `[spec:nsh:req:compat.bash.error-boundary]` names "a failed
/// expansion" for the default dialect. Neither reference answers 1 --
/// the pinned Bash 5.3.15 answers 127 in both its modes -- so the
/// imported 1 was nobody's but Smoosh's. It is a sanctioned divergence.
// [spec:nsh:req:compat.smoosh.error-contracts/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn expansion_modes_diverge() {
    let (stdout, stderr, status) = run("unset x; echo ${x?z}; echo unreachable", false, false);
    assert!(stdout.is_empty());
    assert_eq!(stderr, b"x: z\n");
    assert_eq!(status, 2);

    let (stdout, _, status) = run("echo ${x?alas, poor yorick}; echo hello; exit", true, false);
    assert_eq!(stdout, b"hello\n");
    assert_eq!(status, 0);
}

// [spec:nsh:req:compat.smoosh.error-contracts/test]
#[test]
fn times_write_failure_is_two() {
    let script = "exec 3>&1\n(\ntrap \"\" PIPE\nsleep 1\ncommand times\necho ?=$? >&3\n) | true";
    let (stdout, stderr, status) = run(script, false, true);

    assert_eq!(stdout, b"?=2\n");
    assert_eq!(stderr, b"smoosh: times: I/O error\n");
    assert_eq!(status, 0);
}

/// The restored close is what this case exists for and it is unchanged:
/// the duplication fails and nothing is printed. Its status is the
/// previous test's, because `: <&8` is a redirection error on a directly
/// invoked special built-in -- 2, and byte-identical to dash. Smoosh's 1
/// is the same sanctioned divergence.
// [spec:nsh:req:compat.smoosh.error-contracts/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn closed_descriptor_stays_closed() {
    let script = "{ exec 8</dev/null; } 8<&-; : <&8 && echo 'oops, still open'";
    let (stdout, _, status) = run(script, false, false);

    assert!(stdout.is_empty());
    assert_eq!(status, 2);
}
