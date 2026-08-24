//! Script text is untrusted input, and the parser must survive it.
//!
//! These run in this process on purpose. A depth bound that does not hold
//! shows up as the test binary dying on a stack overflow rather than as a
//! failed assertion, which is exactly the failure being guarded: there is
//! no unwind out of a blown stack, so an embedder calling `Shell::run` on
//! a string it did not write would get a dead process where
//! [`dec:nsh:shell-as-library`] promised an `Err`.
//!
//! Nothing here executes a command. `-n`/`noexec` reaching the crash is
//! what makes it reachable from a syntax check on a hostile file.
//!
//! Each case runs on a thread that names its own stack size, because the
//! test harness gives a spawned thread 2 MiB and a *debug* parser spends
//! about 41 KiB per nesting level -- so the bound's own depth would not
//! fit, and the test would fail on the harness's budget rather than on
//! the shell's. A release build spends about 5.2 KiB, which is where the
//! bound is set from: 100 levels is half a mebibyte, and fits four times
//! over in the 2 MiB a spawned thread gets. Both figures were measured.

use bstr::{BStr, ByteSlice as _};
use nsh::{Shell, Streams};

/// Run one case with room for a debug build's frames; see the module note.
fn with_stack(case: impl FnOnce() + Send + 'static) {
    std::thread::Builder::new()
        .stack_size(16 << 20)
        .spawn(case)
        .expect("spawn a case thread")
        .join()
        .expect("case thread finished");
}

/// Parsing only. `noexec` is what makes these parser tests rather than
/// evaluator ones -- and it is the threat model too, since a syntax check
/// on a hostile file is the cheapest way to reach the crash. It also
/// keeps the nested `while true; do ... done` cases from being what they
/// literally say: an infinite loop.
fn shell() -> Shell {
    Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"noexec"), true)
        .build()
        .expect("build shell")
}

/// A shell that really runs what it is given, for the one case that is
/// about length rather than depth.
fn executing_shell() -> Shell {
    Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .build()
        .expect("build shell")
}

/// Every construct that nests a command inside another, at a depth no
/// stack could hold, one frame per level.
fn deeply_nested(depth: usize) -> Vec<(&'static str, String)> {
    vec![
        (
            "subshell",
            format!("{}:{}", "(".repeat(depth), ")".repeat(depth)),
        ),
        (
            "brace",
            format!("{}:{}", "{ ".repeat(depth), "; }".repeat(depth)),
        ),
        (
            "if",
            format!(
                "{}:{}",
                "if true; then ".repeat(depth),
                "; fi".repeat(depth)
            ),
        ),
        (
            "while",
            format!(
                "{}:{}",
                "while true; do ".repeat(depth),
                "; done".repeat(depth)
            ),
        ),
        (
            "case",
            format!(
                "{}:{}",
                "case x in x) ".repeat(depth),
                ";; esac".repeat(depth)
            ),
        ),
    ]
}

// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn nesting_past_the_bound_is_refused_not_fatal() {
    with_stack(|| {
        for (name, script) in deeply_nested(100_000) {
            let mut shell = shell();
            let outcome = shell.run(script.as_bytes());

            assert!(
                outcome.is_err(),
                "{name} nested 100,000 deep parsed instead of being refused",
            );
            drop(shell.take_captured_stderr());
        }
    });
}

/// The bound is on nesting, not on length: a long flat list is ordinary
/// input and must still parse.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn a_long_flat_list_is_not_deep() {
    with_stack(|| {
        let mut shell = executing_shell();
        let script = ":\n".repeat(20_000);

        let status = shell.run(script.as_bytes()).expect("a flat list parses");

        assert_eq!(status.code(), 0);
    });
}

/// And ordinary nesting is unaffected -- far past anything a written
/// script reaches.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn ordinary_nesting_still_parses() {
    with_stack(|| {
        for (name, script) in deeply_nested(50) {
            let mut shell = shell();
            let status = shell
                .run(script.as_bytes())
                .unwrap_or_else(|_| panic!("{name} nested 50 deep should parse"));

            assert_eq!(status.code(), 0, "{name}");
        }
    });
}

/// The refusal is a syntax error the caller can read, not a signal.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn the_refusal_names_what_it_refused() {
    with_stack(|| {
        let mut shell = shell();
        let script = format!("{}:{}", "(".repeat(5_000), ")".repeat(5_000));

        let outcome = shell.run(script.as_bytes());

        assert!(outcome.is_err());
        let complained = shell.take_captured_stderr().expect("capture stderr");
        assert!(
            complained.contains_str("too many nested commands"),
            "unexpected diagnostic: {}",
            BStr::new(&complained),
        );
    });
}

/// A call spends a stack frame, and a script can recurse without meaning
/// to. Bash leaves this unbounded and segfaults; dash refuses, and so
/// does this.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn runaway_calls_are_refused_not_fatal() {
    with_stack(|| {
        for (name, script) in [
            ("direct", "f() { f; }\nf"),
            ("mutual", "a() { b; }\nb() { a; }\na"),
        ] {
            let mut shell = executing_shell();
            let outcome = shell.run(script.as_bytes());

            assert!(outcome.is_err(), "{name} recursion was not refused");
            let complained = shell.take_captured_stderr().expect("capture stderr");
            assert!(
                complained.contains_str("Maximum function recursion depth"),
                "{name}: unexpected diagnostic: {}",
                BStr::new(&complained),
            );
        }
    });
}

/// Calls a script can actually mean are unaffected.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn ordinary_call_depth_is_unaffected() {
    with_stack(|| {
        let mut shell = executing_shell();
        let status = shell
            .run(b"n=0\nf() { n=$((n+1)); [ $n -ge 100 ] && return 0; f; }\nf\necho $n")
            .expect("100 nested calls run");

        assert_eq!(status.code(), 0);
        assert_eq!(
            shell.take_captured_stdout().expect("capture stdout"),
            b"100\n".to_vec(),
        );
    });
}
