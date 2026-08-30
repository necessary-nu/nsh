//! `time` and `select`, measured against the pinned Bash 5.3.
//!
//! `!` and `time` are flags on a pipeline command in Bash's grammar rather
//! than wrappers around it, which is why they take either order and any
//! number: `time` is idempotent, `-p` written anywhere in the run selects
//! the POSIX report for all of it, and `!` toggles. This shell read `time`
//! only *before* the `!`, so `if ! time cmd` -- the commoner spelling --
//! was a syntax error, and it wrapped a second `time` instead of
//! collapsing it, so `time time :` reported twice.
//!
//! The report's numbers are a clock, so every run of digits is replaced by
//! `N` before comparing. What survives that is the layout, which is the
//! part this shell chooses: three tab-separated lines with minutes, or
//! `time -p`'s two-decimal seconds. `TIMEFORMAT` is deliberately not read
//! -- see `docs/divergences.md` -- so no row varies it.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// One script, run under both shells, compared on stdout and stderr with
/// the digits flattened.
const CASES: &[&str] = &[
    /* `time` on its own, with and without something to time. */
    "time :\n",
    "time\n",
    "time -p :\n",
    "time echo hi | cat\n",
    "time { :; }\n",
    "time for i in 1 2; do :; done\n",
    /* Either order, any number. */
    "! time false; echo st=$?\n",
    "! time -p false; echo st=$?\n",
    "time ! false; echo st=$?\n",
    "! ! time false; echo st=$?\n",
    "! time ! false; echo st=$?\n",
    "time ! ! false; echo st=$?\n",
    "if ! time false; then echo yes; fi\n",
    /* A repeated `time` is one flag, and `-p` anywhere wins. */
    "time time :\n",
    "time time echo hi\n",
    "time -p time :\n",
    "time time -p :\n",
    "time -p time -p :\n",
    /* Both words are keywords to `type`, in the dialect that parses them. */
    "type -t time; type -t select\n",
    /* `select`: the menu and prompt go to standard error, the reply is
     * `REPLY`, an out-of-range reply selects nothing, an empty reply
     * reprints the menu, and no `in` list reads the positional
     * parameters. */
    "PS3=\"pick: \"; select x in a b; do echo \"[$x][$REPLY]\"; break; done < /dev/null\n",
    "select x in a b; do break; done < /dev/null; echo st=$?\n",
    "printf '2\\n' | { select x in a b c; do echo \"got=$x rep=$REPLY\"; break; done; }\n",
    "printf '9\\n1\\n' | { select x in a b; do echo \"got=[$x]\"; break; done; }\n",
    "printf '\\n1\\n' | { select x in a; do echo \"got=$x\"; break; done; }\n",
    "set -- p q; printf '1\\n' | { select x; do echo \"got=$x\"; break; done; }\n",
    "printf '1\\n' | { select x in; do echo \"got=$x\"; break; done; }\n",
];

/// Every run of digits becomes `N`, so a report is compared as a layout.
fn flatten(text: &[u8]) -> String {
    let mut out = String::new();
    let mut in_digits = false;
    for byte in text {
        if byte.is_ascii_digit() {
            if !in_digits {
                out.push('N');
                in_digits = true;
            }
        } else {
            in_digits = false;
            out.push(*byte as char);
        }
    }
    out
}

/// Run one script and return its stdout and stderr, flattened.
fn output(shell: &Path, dialect: &[&str], script: &str) -> String {
    let mut child = Command::new(shell)
        .args(dialect)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    child
        .stdin
        .take()
        .expect("the child's standard input")
        .write_all(script.as_bytes())
        .expect("write the script");
    let output = child.wait_with_output().expect("wait for the shell");
    format!(
        "out:{}err:{}",
        flatten(&output.stdout),
        flatten(&output.stderr)
    )
}

// [spec:nsh:req:compat.bash.select-time-grammar/test]
#[test]
fn time_and_select_answer_as_bash_answers() {
    let bash = pinned_bash::path();
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for script in CASES {
        assert_eq!(
            output(nsh, &["-o", "bash"], script),
            output(&bash, &[], script),
            "for\n{script}"
        );
    }
}

/// `select` is Bash's and stays out of the POSIX dialect, where reserving
/// it would change what a script naming a command `select` means. `time`
/// is POSIX's reserved word and is grammar in both.
// [spec:nsh:req:compat.bash.default-isolation/test]
#[test]
fn only_time_is_grammar_in_the_posix_dialect() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    assert!(output(nsh, &[], "time :\n").contains("real"));
    assert!(output(nsh, &[], "! time false\n").contains("real"));
    assert!(
        output(nsh, &[], "select x in a; do break; done\n").contains("Syntax error"),
        "the POSIX dialect parsed `select`"
    );
}
