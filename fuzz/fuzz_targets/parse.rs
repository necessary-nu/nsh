//! Arbitrary bytes through the parser, which must survive all of them.
//!
//! Script text is the shell's untrusted input. The parser has to answer
//! every byte sequence with a tree or an `Err`, and never with a panic, a
//! blown stack or a hang -- `[dec:nsh:shell-as-library]` makes that a
//! promise to an embedder rather than a nicety, because a process that
//! dies here takes the host with it and there is nothing to catch.
//!
//! Two crashes of exactly this shape were found by hand before any of
//! this existed: nesting past about 1,700 levels overflowed the stack,
//! and it did so under `noexec`, so a syntax check on a hostile file was
//! enough. Both are bounded now. This target is what looks for the next
//! one.
//!
//! `noexec` is what keeps the target pure. The parser runs and nothing it
//! parses is executed, so a corpus entry cannot fork, write a file or
//! spawn anything, and the fuzzer stays fast enough to be worth running.
//! Run it under `scripts/sandboxed` regardless -- see `fuzz/README.md`
//! for why that is not optional.

#![no_main]

use libfuzzer_sys::fuzz_target;
use nsh::{Shell, Streams};

fuzz_target!(|data: &[u8]| {
    /* A fresh shell per input: option state, aliases, functions and the
     * variable table all outlive one `run`, so a shared one would make
     * every finding depend on the inputs before it and none of them
     * reproducible from the artifact alone. */
    let Ok(streams) = Streams::capture() else {
        return;
    };
    let Ok(mut shell) = Shell::builder()
        .streams(streams)
        .option(bstr::BStr::new(b"noexec"), true)
        .build()
    else {
        return;
    };

    /* The result is deliberately ignored. A syntax error is a correct
     * answer to most of what a fuzzer produces; what is being asserted is
     * that the call *returns at all*, which a panic, an overflow or a
     * hang would each break in a way no assertion here could report. */
    let _ = shell.run(data);
    drop(shell.take_captured_stdout());
    drop(shell.take_captured_stderr());
});
