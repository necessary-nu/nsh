//! Where `$[expression]` ends, measured against the pinned Bash 5.3.
//!
//! `$[` is Bash's older spelling of `$((`, and the two do not end at the
//! same byte: `$[` closes at a bracket and `$((` at `))`. This shell read
//! both with one scanner and one nesting counter, so a parenthesis inside
//! `$[…]` was structure rather than data -- `$[(]` never terminated, and
//! `$[))` terminated something Bash reads to end of input. A quoted run
//! was not scanned at all, so `$[']` ended an expression whose quote Bash
//! is still looking for.
//!
//! The verdicts below are the pinned reference's, and the second test
//! re-derives them from it rather than trusting this file. What is
//! asserted is accept-or-reject and not a printed form: every one of the
//! accepted expressions is a Bash *evaluation* error, so a test written
//! against output would agree with the reference for the wrong reason.

mod pinned_bash;

use bstr::BStr;
use nsh::{Shell, Streams};
use std::process::{Command, Stdio};

/// One program and the verdict the pinned Bash gives it.
const VERDICTS: &[(&[u8], bool)] = &[
    /* Refused. The two this was filed for come first: Bash ends `$[` at
     * a bracket, finds none, and reports the end of input. */
    (b": $[$(a+=)))", false),
    (b": $[])a]", false),
    (b": $[))", false),
    (b": $[(${\x92[]})))", false),
    (b": $[${x,'\"\"('}) ))", false),
    /* A quoted run Bash is still inside when the input ends. */
    (b": $['", false),
    (b": $[\"", false),
    /* Accepted. A parenthesis is one of the expression's own bytes, so
     * an unbalanced one is an arithmetic error and not a parse error. */
    (b": $[(]", true),
    (b": $[)]", true),
    (b": $[]]", true),
    (b": $[())]", true),
    (b": $[\\a]", true),
    (b": $[(1+2)*3]", true),
    (b": $[a[1]]", true),
    (b": $[${x}]", true),
    (b": $[`echo 4`+1]", true),
    /* A bracket inside a quoted run is data, and the run closes. */
    (b": $[']']", true),
    (b": $[\"]\"]", true),
    (b": $['[']", true),
    /* `$((` ends at `))` instead, and these are its side of the same
     * corpus: all six parse, and none of them is spelled `$[`. */
    (b": $((]))", true),
    (b": $(([))$()", true),
    (b": $((\\a))", true),
    (b": $((${a#'${'}))", true),
];

/// Parse `script` without running it, and say whether it parsed.
fn parses(script: &[u8]) -> bool {
    let mut shell = Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"bash"), true)
        .build()
        .expect("build shell");
    let mut source = b"set -n\n".to_vec();
    source.extend_from_slice(script);
    source.push(b'\n');
    let status = shell
        .run(source.as_slice())
        .unwrap_or_else(|error| error.status())
        .code();
    shell.take_captured_stdout().expect("capture stdout");
    shell.take_captured_stderr().expect("capture stderr");
    status == 0
}

// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn a_bracketed_expression_ends_where_bash_ends_it() {
    for (script, accepted) in VERDICTS {
        assert_eq!(
            parses(script),
            *accepted,
            "{} should {} in Bash mode",
            BStr::new(script),
            if *accepted { "parse" } else { "be refused" }
        );
    }
}

/// The table is the reference's answer, not this repository's opinion.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn the_recorded_verdicts_are_the_references_own() {
    let bash = pinned_bash::path();
    for (script, accepted) in VERDICTS {
        let mut source = b"set -n\n".to_vec();
        source.extend_from_slice(script);
        source.push(b'\n');
        let mut child = Command::new(&bash)
            .env_clear()
            .env("PATH", "/usr/bin:/bin")
            .stdin(Stdio::piped())
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .expect("start the pinned Bash");
        use std::io::Write as _;
        child
            .stdin
            .take()
            .expect("the child's standard input")
            .write_all(&source)
            .expect("write the script");
        let status = child.wait().expect("wait for the pinned Bash");
        assert_eq!(
            status.success(),
            *accepted,
            "the reference disagrees with the recorded verdict for {}",
            BStr::new(script)
        );
    }
}
