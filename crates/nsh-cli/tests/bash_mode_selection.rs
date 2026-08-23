use std::ffi::OsStr;
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::process::CommandExt as _;
use std::process::{Command, Output};

fn run_shell(argument_zero: &[u8], args: &[&[u8]]) -> Output {
    let mut command = Command::new(env!("CARGO_BIN_EXE_nsh"));
    command.arg0(OsStr::from_bytes(argument_zero));
    for arg in args {
        command.arg(OsStr::from_bytes(arg));
    }
    command.env("LC_ALL", "C").output().expect("run nsh")
}

/// The `set -o` line that reports the dialect.
///
/// There are two spellings of it and which one appears is itself the
/// answer: the POSIX dialect calls the switch `bash`, and Bash mode
/// calls it what Bash calls it -- `posix`, inverted, so a Bash-mode
/// shell reports `posix off`.
fn dialect_lines(output: &Output) -> Vec<&[u8]> {
    assert!(
        output.status.success(),
        "shell failed: stdout={:?}, stderr={:?}",
        output.stdout,
        output.stderr
    );
    output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| line.starts_with(b"bash ") || line.starts_with(b"posix "))
        .collect()
}

/// What `set -o` prints for a shell in each dialect.
const POSIX_REPORT: &[u8] = b"bash            off";
const BASH_REPORT: &[u8] = b"posix           off";

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn invocation_and_options_select_bash_mode() {
    let ordinary = run_shell(b"nsh", &[b"-c", b"set -o"]);
    assert_eq!(dialect_lines(&ordinary), [POSIX_REPORT]);

    let explicit = run_shell(b"nsh", &[b"-o", b"bash", b"-c", b"set -o"]);
    assert_eq!(dialect_lines(&explicit), [BASH_REPORT]);

    let disabled = run_shell(b"nsh", &[b"+o", b"bash", b"-c", b"set -o"]);
    assert_eq!(dialect_lines(&disabled), [POSIX_REPORT]);
}

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn raw_invocation_basename_selects_mode() {
    for argument_zero in [
        b"bash".as_slice(),
        b"/opt/shells/bash",
        b"/opt/shells/-bash",
    ] {
        let output = run_shell(argument_zero, &[b"-c", b"set -o"]);
        assert_eq!(dialect_lines(&output), [BASH_REPORT], "{argument_zero:?}");
    }

    let overridden = run_shell(b"bash", &[b"+o", b"bash", b"-c", b"set -o"]);
    assert_eq!(dialect_lines(&overridden), [POSIX_REPORT]);
}

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn command_operand_does_not_select_mode() {
    let output = run_shell(b"nsh", &[b"-c", b"printf '%s\\n' \"$0\"; set -o", b"bash"]);
    assert!(output.stdout.starts_with(b"bash\n"));
    assert_eq!(dialect_lines(&output), [POSIX_REPORT]);
}

// [spec:nsh:req:compat.bash.state-isolation/test]
#[test]
fn subshell_option_changes_remain_local() {
    let output = run_shell(
        b"nsh",
        &[b"-o", b"bash", b"-c", b"(set +o bash; set -o); set -o"],
    );
    assert_eq!(dialect_lines(&output), [POSIX_REPORT, BASH_REPORT]);
}

/// Every way in and out of the dialect, and what each one selects.
///
/// The report from `set -o` is not enough on its own: it says what the
/// option table holds, not what the parser did with it. Each row is
/// therefore checked twice -- once against the reported option and once
/// against a construct only the dialect accepts.
struct Entry {
    label: &'static str,
    argument_zero: &'static [u8],
    arguments: &'static [&'static [u8]],
    enabled: bool,
}

const ENTRIES: &[Entry] = &[
    Entry {
        label: "plain nsh",
        argument_zero: b"nsh",
        arguments: &[],
        enabled: false,
    },
    Entry {
        label: "nsh -o bash",
        argument_zero: b"nsh",
        arguments: &[b"-o", b"bash"],
        enabled: true,
    },
    Entry {
        label: "nsh +o bash",
        argument_zero: b"nsh",
        arguments: &[b"+o", b"bash"],
        enabled: false,
    },
    Entry {
        label: "argv[0] bash",
        argument_zero: b"bash",
        arguments: &[],
        enabled: true,
    },
    Entry {
        label: "argv[0] -bash (login)",
        argument_zero: b"-bash",
        arguments: &[],
        enabled: true,
    },
    Entry {
        label: "argv[0] /opt/shells/bash",
        argument_zero: b"/opt/shells/bash",
        arguments: &[],
        enabled: true,
    },
    Entry {
        label: "argv[0] /opt/shells/-bash",
        argument_zero: b"/opt/shells/-bash",
        arguments: &[],
        enabled: true,
    },
    Entry {
        label: "argv[0] bash with +o bash",
        argument_zero: b"bash",
        arguments: &[b"+o", b"bash"],
        enabled: false,
    },
    Entry {
        label: "argv[0] bashful",
        argument_zero: b"bashful",
        arguments: &[],
        enabled: false,
    },
    Entry {
        label: "argv[0] nsh-bash",
        argument_zero: b"nsh-bash",
        arguments: &[],
        enabled: false,
    },
];

/// Report the option table, then let the parser answer the same question
/// with a construct only the dialect accepts.
const REPORT: &[u8] = b"set -o; [[ a == a ]] && printf 'dialect on\\n'";

/// `dialect_lines` insists the shell succeeded, which a deliberately
/// refused construct does not. This reads the same report without that
/// demand.
fn reported_dialect(output: &Output) -> bool {
    let lines: Vec<&[u8]> = output
        .stdout
        .split(|byte| *byte == b'\n')
        .filter(|line| line.starts_with(b"bash ") || line.starts_with(b"posix "))
        .collect();
    match lines.as_slice() {
        [line] if *line == BASH_REPORT => true,
        [line] if *line == POSIX_REPORT => false,
        other => panic!("unreadable set -o report: {other:?}"),
    }
}

// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn entry_forms_select_the_named_dialect() {
    for entry in ENTRIES {
        let mut arguments = entry.arguments.to_vec();
        arguments.push(b"-c");
        arguments.push(REPORT);
        let output = run_shell(entry.argument_zero, &arguments);
        assert_eq!(
            reported_dialect(&output),
            entry.enabled,
            "{}: set -o reports the wrong dialect",
            entry.label
        );
        let accepted = output.stdout.ends_with(b"dialect on\n");
        assert_eq!(
            accepted, entry.enabled,
            "{}: the parser disagrees with the reported option",
            entry.label
        );
    }
}

/// `set -o bash` and `set +o bash` inside a running script are the same
/// two entries, taken at a parser boundary rather than at startup.
// [spec:nsh:req:compat.bash.selection/test]
// [spec:nsh:req:compat.bash.parse-boundary/test]
#[test]
fn set_o_bash_switches_at_runtime() {
    let script: &[u8] = b"probe() { eval '[[ a == a ]]' 2>/dev/null; printf '%s\\n' \"$?\"; }\n\
         probe\n\
         set -o bash\n\
         probe\n\
         set +o bash\n\
         probe\n";
    let output = run_shell(b"nsh", &[b"-c", script]);
    assert_eq!(output.stdout, b"127\n0\n127\n", "{:?}", output.stdout);
}

/// A command operand is `$0`, not the invocation name, so `bash` there
/// must not infer the dialect.
// [spec:nsh:req:compat.bash.selection/test]
#[test]
fn command_operand_bash_stays_posix() {
    let output = run_shell(
        b"nsh",
        &[b"-c", b"printf '%s\\n' \"$0\"; [[ a == a ]]", b"bash"],
    );
    assert!(output.stdout.starts_with(b"bash\n"));
    assert_ne!(output.status.code(), Some(0));
}
