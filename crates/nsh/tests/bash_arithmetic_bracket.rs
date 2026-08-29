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

use bstr::BStr;
use nsh::{Shell, Streams};
use std::path::PathBuf;
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

/// The pinned Bash whose verdicts the table above records.
///
/// Named by `NSH_FUZZ_BASH` or found beside the build tree, and checked
/// against the version `calibrate-bash-5-3-oracle` pinned -- the ambient
/// `/usr/bin/bash` is 5.2 on this machine and is not an answer here.
///
/// A reference that is not there is a failure and not a pass. This used
/// to report and return `None` for the caller to skip on, which made the
/// one test that re-derives the table from the reference incapable of
/// disagreeing with it on any machine that had not built the oracle
/// ([`spec:nsh:req:oracle.cannot-measure-is-a-failure`]).
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
fn reference_bash() -> PathBuf {
    let path = std::env::var_os("NSH_FUZZ_BASH").map_or_else(
        || {
            PathBuf::from(concat!(
                env!("CARGO_MANIFEST_DIR"),
                "/../../target/bash-reference/bash"
            ))
        },
        PathBuf::from,
    );
    assert!(
        path.exists(),
        "no pinned Bash at {}, so the verdicts below cannot be checked against \
         the reference that produced them\n\
         build it and name it to the run:\n\
         \x20   cargo run -p nsh-survey -- build-bash-reference\n\
         \x20   (or point NSH_FUZZ_BASH at an existing pinned build)",
        path.display()
    );
    /* The pin itself is read out of the survey's calibration record, by
     * the string search `nsh::fuzzing::reference` uses, so the two
     * cannot drift apart. That module is behind a feature this test
     * does not turn on. */
    let record = std::fs::read_to_string(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/surveys/oils/BASH_REFERENCE_CASES.json"
    ))
    .expect("read the Bash calibration record");
    let at = record
        .find("\"oracle_version\"")
        .expect("the record names an oracle_version");
    let tail = &record[at..];
    let open = tail[16..].find('"').expect("a quoted oracle_version") + 17;
    let close = tail[open..].find('"').expect("a terminated oracle_version");
    let pinned = &tail[open..open + close];

    let reported = Command::new(&path)
        .arg("--version")
        .output()
        .expect("run the pinned Bash");
    let reported = String::from_utf8_lossy(&reported.stdout);
    let first = reported.lines().next().unwrap_or_default();
    assert!(
        first.contains(pinned),
        "{} reports {first:?}, which is not the pinned {pinned:?}",
        path.display()
    );
    path
}

/// The table is the reference's answer, not this repository's opinion.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn the_recorded_verdicts_are_the_references_own() {
    let bash = reference_bash();
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
