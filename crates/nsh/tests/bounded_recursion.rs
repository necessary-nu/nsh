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
//! about 14.9 KiB per nesting level -- so the bound's own depth would not
//! fit, and the test would fail on the harness's budget rather than on
//! the shell's. A release build spends about 1.7 KiB, which is where the
//! bound is set from: 256 levels is 0.43 MiB, and fits four times over in
//! the 2 MiB a spawned thread gets. Both figures were measured.
//!
//! What counts as a nesting level is deliberately wide, because a
//! construct that re-enters the grammar from somewhere the compound-
//! command bound cannot see spends the stack all the same: `$( )` and
//! `<( )` open a list from inside a word, `time` reaches a pipeline
//! without passing through a command, `${x:-...}` and `$(( ... ))` cost
//! their stack when the word's flat event stream is turned into a tree,
//! and `[[ ( ) ]]` is a recursive descent of its own. Every one of them
//! crashed until it was charged the same budget.

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

/// A shell that really runs what it is given, for the cases that are
/// about the evaluator rather than the parser.
fn executing_shell() -> Shell {
    Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .build()
        .expect("build shell")
}

/// The Bash dialect, for the constructs only it has.
fn bash_shell(noexec: bool) -> Shell {
    Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"bash"), true)
        .option(BStr::new(b"noexec"), noexec)
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
        (
            "command substitution",
            format!("echo {}echo hi{}", "$(".repeat(depth), ")".repeat(depth)),
        ),
        ("time", format!("{}:", "time ".repeat(depth))),
    ]
}

/// The same, for constructs whose stack is spent turning one word's flat
/// event stream into a tree rather than by the grammar around it.
///
/// A shallower depth than [`deeply_nested`] on purpose: the word is lexed
/// whole before its nesting can be read off the events, so the input is
/// what bounds the work here and a hundred thousand levels would be
/// measuring the lexer's throughput rather than the ceiling.
fn deeply_nested_words(depth: usize) -> Vec<(&'static str, String)> {
    vec![
        (
            "parameter",
            format!("echo {}y{}", "${x:-".repeat(depth), "}".repeat(depth)),
        ),
        (
            "quoted parameter",
            format!("echo {}y{}", "\"${x:-".repeat(depth), "}\"".repeat(depth)),
        ),
        (
            "arithmetic",
            format!("echo {}1{}", "$(( ".repeat(depth), " ))".repeat(depth)),
        ),
    ]
}

/// Bash-only constructs, each with a recursion of its own.
fn deeply_nested_bash(depth: usize) -> Vec<(&'static str, String)> {
    vec![
        (
            "conditional",
            format!("[[ {}1 == 1{} ]]", "( ".repeat(depth), " )".repeat(depth)),
        ),
        (
            "negated conditional",
            format!("[[ {}1 == 1 ]]", "! ".repeat(depth)),
        ),
        (
            "process substitution",
            format!("cat {}echo hi{}", "<(".repeat(depth), ")".repeat(depth)),
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

/// A word's own nesting costs its stack when the lexer's flat events are
/// turned into a tree, which is after the word is read and so out of the
/// grammar's sight. `${x:-` twenty thousand deep segfaulted.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn deep_expansions_in_one_word_are_refused() {
    with_stack(|| {
        for (name, script) in deeply_nested_words(20_000) {
            let mut shell = shell();
            let outcome = shell.run(script.as_bytes());

            assert!(
                outcome.is_err(),
                "{name} nested 20,000 deep was not refused"
            );
            let complained = shell.take_captured_stderr().expect("capture stderr");
            assert!(
                complained.contains_str("too many nested commands"),
                "{name}: unexpected diagnostic: {}",
                BStr::new(&complained),
            );
        }
    });
}

/// Bash's own recursions, each of which crashed on its own route.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn bash_only_nesting_is_refused_too() {
    with_stack(|| {
        for (name, script) in deeply_nested_bash(20_000) {
            let mut shell = bash_shell(true);
            let outcome = shell.run(script.as_bytes());

            assert!(
                outcome.is_err(),
                "{name} nested 20,000 deep was not refused"
            );
            drop(shell.take_captured_stderr());
        }
    });
}

/// A brace expression's nesting is not the parser's: braces are ordinary
/// bytes in a word and only expansion walks them, charging the words it
/// is about to build on the way back up -- too late to see the descent.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn deep_brace_expansion_is_refused() {
    with_stack(|| {
        let mut shell = bash_shell(false);
        let script = format!("echo {}b{}", "{a,".repeat(20_000), "}".repeat(20_000));

        let outcome = shell.run(script.as_bytes());

        assert!(outcome.is_err());
        let complained = shell.take_captured_stderr().expect("capture stderr");
        assert!(
            complained.contains_str("brace expansion nested too deeply"),
            "unexpected diagnostic: {}",
            BStr::new(&complained),
        );
    });
}

/// `=~` compiles its operand into a tree of its own, one frame per group,
/// and drops it the same way. The matcher's two budgets are downstream of
/// that and never see it.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn a_deeply_grouped_regex_is_refused() {
    with_stack(|| {
        let mut shell = bash_shell(false);
        let script = format!("[[ aaa =~ {}a{} ]]", "(".repeat(20_000), ")".repeat(20_000));

        /* The refusal is `[[ ]]`'s own failure rather than the script's,
         * so it is a status the caller reads and not an `Err`; what
         * matters is that it is neither a match nor a signal. */
        let status = shell.run(script.as_bytes()).expect("the script ran");

        assert_ne!(status.code(), 0);
        let complained = shell.take_captured_stderr().expect("capture stderr");
        assert!(
            complained.contains_str("Regular expression nested too deeply"),
            "unexpected diagnostic: {}",
            BStr::new(&complained),
        );
    });
}

/// An arithmetic prefix operator recurses exactly as a parenthesis does,
/// and shares its ceiling. `$(( ---...1 ))` overflowed where
/// `$(( (((...))) ))` was already refused.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn arithmetic_prefix_operators_are_bounded() {
    with_stack(|| {
        for (name, script) in [
            ("minus", format!("echo $(( {}1 ))", "-".repeat(20_000))),
            ("not", format!("echo $(( {}1 ))", "!".repeat(20_000))),
            ("complement", format!("echo $(( {}1 ))", "~".repeat(20_000))),
        ] {
            let mut shell = executing_shell();
            let outcome = shell.run(script.as_bytes());

            assert!(outcome.is_err(), "{name} was not refused");
            let complained = shell.take_captured_stderr().expect("capture stderr");
            assert!(
                complained.contains_str("recursion level exceeded"),
                "{name}: unexpected diagnostic: {}",
                BStr::new(&complained),
            );
        }
    });
}

/// `eval` parses a string and runs it on top of the frame that asked for
/// it, so a chain of them is a call chain wearing another name. Bash and
/// dash both survive this one, and this crashed at about 3,500.
///
/// The count has a ceiling as well as a floor now, and both are the
/// point. It must pass 512 to reach the depth bound, and it must stay
/// small enough that the chain's own text does not reach the *work* bound
/// first -- which a longer one does, and which the next case is about.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn nested_eval_is_refused_not_fatal() {
    with_stack(|| {
        let mut shell = executing_shell();
        let script = format!("{}':'", "eval ".repeat(600));

        let outcome = shell.run(script.as_bytes());

        assert!(outcome.is_err());
        let complained = shell.take_captured_stderr().expect("capture stderr");
        assert!(
            complained.contains_str("Maximum recursion depth"),
            "unexpected diagnostic: {}",
            BStr::new(&complained),
        );
    });
}

/// A depth bound stops the recursion and does not stop the work: each of
/// the 512 levels it allows re-parses the whole word list that carried it,
/// so this costs 512N and was killed for memory at N = 100,000 having
/// refused nothing. The chain is legitimate at every individual level,
/// which is why no depth can see it, and it is refused for what the live
/// levels are carrying between them instead.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn a_wide_eval_chain_is_refused() {
    with_stack(|| {
        let mut shell = executing_shell();
        let script = format!("{}':'", "eval ".repeat(5_000));

        let outcome = shell.run(script.as_bytes());

        assert!(outcome.is_err());
        let complained = shell.take_captured_stderr().expect("capture stderr");
        assert!(
            complained.contains_str("Maximum evaluation size"),
            "unexpected diagnostic: {}",
            BStr::new(&complained),
        );
    });
}

/// The budget is on what the live re-entries carry, not on how many there
/// are, so a recursion far shallower than the depth bound reaches it: this
/// one is refused around thirty levels down. A single `eval` of the same
/// text is ordinary and must still run, which is what the second half
/// asserts -- the charge is against re-entry, not against size.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn shallow_evaluation_is_charged_for_size() {
    with_stack(|| {
        let padding = "x".repeat(256 << 10);
        let mut shell = executing_shell();

        let outcome = shell.run(format!("f() {{ eval '# {padding}\nf'; }}\nf").as_bytes());

        assert!(outcome.is_err());
        let complained = shell.take_captured_stderr().expect("capture stderr");
        assert!(
            complained.contains_str("Maximum evaluation size"),
            "unexpected diagnostic: {}",
            BStr::new(&complained),
        );

        let mut shell = executing_shell();
        let outcome = shell.run(format!("eval '# {padding}\necho ran'").as_bytes());

        assert!(outcome.is_ok(), "one large evaluation was refused");
        let printed = shell.take_captured_stdout().expect("capture stdout");
        assert_eq!(BStr::new(&printed), BStr::new(b"ran\n"));
    });
}

/// Calls and `eval` share one ceiling because they compose: a body that
/// evals its own name spends one of each per turn, and two ceilings would
/// let it reach a depth neither of them names.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn a_call_that_evals_itself_is_refused() {
    with_stack(|| {
        let mut shell = executing_shell();

        let outcome = shell.run(b"f() { eval f; }\nf");

        assert!(outcome.is_err());
        let complained = shell.take_captured_stderr().expect("capture stderr");
        assert!(
            complained.contains_str("Maximum function recursion depth"),
            "unexpected diagnostic: {}",
            BStr::new(&complained),
        );
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
        let posix = deeply_nested(50).into_iter().chain(deeply_nested_words(50));
        for (name, script) in posix {
            let mut shell = shell();
            let status = shell
                .run(script.as_bytes())
                .unwrap_or_else(|_| panic!("{name} nested 50 deep should parse"));

            assert_eq!(status.code(), 0, "{name}");
        }
        for (name, script) in deeply_nested_bash(50) {
            let mut shell = bash_shell(true);
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

/// A here-document delimiter is recorded literally so the body can be
/// matched against it, and the input can end in the middle of one.
///
/// `<<${e` is the whole file, five bytes, and it panicked: the recorder
/// asked an end-of-input item for a byte it does not carry. Found by the
/// `parse` fuzz target. `check_here_document_end` says the delimiter is
/// being recorded and says nothing about whether the cursor is on a
/// byte, so every site that confused the two is the same defect --
/// `record_delimiter_byte` is now the only way to record one.
// [spec:nsh:req:idiom.bounded-recursion/test]
#[test]
fn a_truncated_here_delimiter_is_an_error() {
    with_stack(|| {
        for script in [
            "<<${e",
            "<<${",
            "<<`",
            "<<$((",
            "<<$(",
            "<<${x:",
            "<<${x:-",
            "<<${x:=",
            "<<${x:?",
            "<<${x:+",
            "<<${x#",
            "<<${x##",
            "<<${x%",
            "<<${x%%",
            "<<${x/",
            "<<${x//",
            "<<${x^",
            "<<${x@",
            "<<${x[",
            "<<${#",
            "<<${!",
            "<<${x-",
            "<<${x=",
            "<<${x+",
            "<<${x:1",
            "cat <<${x",
            "<<\"",
            "<<'",
            "<<${x:-${y",
        ] {
            let mut shell = shell();
            // The assertion is that this returns at all.
            let _ = shell.run(script.as_bytes());
            drop(shell.take_captured_stderr());
        }
    });
}
