//! The pattern matcher and the regular-expression engine, driven with
//! arbitrary patterns against arbitrary subjects.
//!
//! Both carry budgets rather than answers: the glob matcher memoizes on
//! `(pattern node, offset)` and the ERE engine answers `no match` at a
//! step and depth budget instead of running unbounded. Those budgets are
//! a *semantic commitment* under `[dec:nsh:safety-trumps-compatibility]`
//! -- "the budget cannot be raised or removed as a performance tweak" --
//! and a budget nobody attacks is a budget nobody has checked. The engine
//! segfaulted on `[[ $s =~ (a)+ ]]` over a long subject once already.
//!
//! Neither byte string is ever spliced into script text. The scripts here
//! are fixed and the fuzzer's bytes reach them through the *environment*,
//! so a pattern cannot quote its way out into a command -- which also
//! means this target never forks, and runs at parser speed rather than
//! process speed.
//!
//! A hang is as much a finding as a crash: libFuzzer's `-timeout` reports
//! one, and an unbounded backtrack is exactly what the budgets exist to
//! prevent.
//!
//! # What it is evidence for
//!
//! `idiom.bounded-recursion`, which `crates/nsh/src/regex.rs` cites for
//! the group-nesting ceiling this target attacks: an arbitrary pattern is
//! mostly parentheses, and building that tree and dropping it each
//! recurse once per one.
//!
//! It is evidence about survival and not about answers. Neither shell is
//! consulted, so a matcher that never crashed and always reported no
//! match would pass every input here. The budgets themselves are the
//! other half of what this file is about and no rule states them: they
//! are a commitment under `[dec:nsh:safety-trumps-compatibility]`, and a
//! decision is not something an annotation can cite.
// [spec:nsh:req:idiom.bounded-recursion/test]

#![no_main]

use libfuzzer_sys::fuzz_target;
use nsh::{Shell, Streams};

/// `case` applies the glob matcher; `[[ =~ ]]` applies the ERE engine.
/// `$P` is unquoted in both, which is what makes it a pattern rather than
/// a literal, and `$S` is quoted so the subject stays one word.
const GLOB: &[u8] = b"case \"$S\" in $P) ;; *) ;; esac";
const REGEX: &[u8] = b"[[ $S =~ $P ]]";
const EXTGLOB: &[u8] = b"shopt -s extglob\ncase \"$S\" in $P) ;; *) ;; esac";

fuzz_target!(|data: &[u8]| {
    /* One NUL splits pattern from subject. A shell variable cannot hold a
     * NUL anyway, so neither half can contain one and the split is
     * unambiguous. Inputs without a separator exercise an empty subject,
     * which is a case worth reaching too. */
    let (pattern, subject) = match data.iter().position(|byte| *byte == 0) {
        Some(at) => (&data[..at], &data[at + 1..]),
        None => (data, &b""[..]),
    };
    if subject.contains(&0) {
        return;
    }

    for script in [GLOB, REGEX, EXTGLOB] {
        let Ok(streams) = Streams::capture() else {
            return;
        };
        let Ok(mut shell) = Shell::builder()
            .streams(streams)
            /* `[[ ]]`, `=~` and `extglob` are all Bash's. */
            .option(bstr::BStr::new(b"bash"), true)
            .env([
                (b"P".to_vec(), pattern.to_vec()),
                (b"S".to_vec(), subject.to_vec()),
            ])
            .build()
        else {
            return;
        };

        /* Ignored deliberately: no match, a bad pattern and a refused
         * regex are all correct answers. What is asserted is that the
         * call returns -- which a panic, an overflow or an unbounded
         * backtrack would each break in a way no assertion could report.
         */
        let _ = shell.run(script);
        drop(shell.take_captured_stdout());
        drop(shell.take_captured_stderr());
    }
});
