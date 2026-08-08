//! The intended usage of the proposed API, written before the API was, so
//! that the example got to judge it.
//!
//! **This compiles and must not be run.** Every body behind it is
//! `todo!()` — see `crates/nsh/src/api.rs` and `docs/api-design.md`. There
//! are no doctests here deliberately: a doctest is executed.
//!
//! Three programs, which are the only embedders this design was allowed to
//! consider:
//!
//!   1. run a script and capture its output
//!   2. expand a word without running a command yourself
//!   3. `nsh-cli`, which has to stay byte-for-byte dash

use std::io;

use bstr::{BStr, BString};
use nsh::api::{Disposition, Error, ExitStatus, Host, Shell, Signal, SignalSink, Source, Streams};

// ---------------------------------------------------------------------
// 1. Run a script and capture what it wrote.
// ---------------------------------------------------------------------

fn run_and_capture() -> Result<(), Error> {
    let mut sh = Shell::builder()
        .arg0(BStr::new(b"myapp"))
        .inherit_env()
        .streams(Streams::capture().map_err(Error::Io)?)
        .build()?;

    sh.set_var(BStr::new(b"PATH"), BStr::new(b"/usr/bin:/bin"))?;

    let status: ExitStatus = sh.run(b"for f in *.txt; do wc -l \"$f\"; done")?;
    let out: BString = sh.take_captured_stdout().map_err(Error::Io)?;

    // Two runs compose like two lines of one script: `count` is still set.
    sh.run(b"count=$(ls | wc -l)")?;
    sh.run(b"echo \"$count files\"")?;

    // `var` hands back a borrow of the table, so a value that has to
    // outlive the next `run` is copied out. The borrow checker is right
    // here: an assignment can move the table under it.
    let home: Option<BString> = sh.var(BStr::new(b"HOME")).map(|v| v.to_owned());

    // Untrusted data goes in as a positional parameter, never spliced into
    // the script text. This is the whole quoting problem, gone.
    let name = BStr::new(b"a file with 'quotes' and $HOME in it");
    sh.run_command(BStr::new(b"printf '%s\\n' \"$1\""), &[BStr::new(b"myapp"), name])?;

    let _ = (status, out, home);
    Ok(())
}

// ---------------------------------------------------------------------
// 2. Expand a word. No second process, no quoting round trip, and nothing
//    a `Command` API can offer.
// ---------------------------------------------------------------------

fn expand_a_word() -> Result<(), Error> {
    let mut sh = Shell::builder().inherit_env().build()?;

    // Unquoted: splits on $IFS, then globs. Zero, one or many fields.
    let fields: Vec<BString> = sh.expand_word(BStr::new(b"~/src/*.rs"))?;

    // As if double-quoted: exactly one, no splitting, no globbing.
    let one: BString = sh.expand_word_quoted(BStr::new(b"${EDITOR:-vi}"))?;

    let _ = (fields, one);
    Ok(())
}

// ---------------------------------------------------------------------
// 3. nsh-cli. The host is the whole difference between a library and a
//    shell, and it is about forty lines.
// ---------------------------------------------------------------------

/// The frontend's host: it owns the process, so it may do all of this.
struct ProcessHost {
    sink: Option<SignalSink>,
}

impl Host for ProcessHost {
    fn attach(&mut self, sink: SignalSink) {
        // Stored where the `extern "C"` handler can reach it. The handler
        // does nothing but `sink.raise(signal)`.
        self.sink = Some(sink);
    }

    fn signal(&mut self, _signal: Signal) -> io::Result<Disposition> {
        // `sigaction(signo, NULL, &old)`. The shell needs the inherited
        // value to reproduce dash's "ignored on entry stays ignored".
        todo!()
    }

    fn set_signal(&mut self, _signal: Signal, _to: Disposition) -> io::Result<()> {
        // `sigaction` with `sigfillset(&sa_mask)` and `sa_flags = 0`.
        todo!()
    }

    fn may_replace_process(&mut self) -> bool {
        // `exec cmd` is the point of a shell frontend.
        true
    }
}

fn frontend(argv: &[BString]) -> ! {
    // argv parsing lives here, not in the library: dash's `sh` command line
    // is the frontend's business, and every differential case runs through
    // it, so the oracle covers the move.
    let interactive = argv.iter().any(|a| a.as_slice() == b"-i");

    let mut sh = match Shell::builder()
        .arg0(BStr::new(b"sh"))
        .inherit_env()
        .streams(Streams::inherit())
        .host(ProcessHost { sink: None })
        .option(BStr::new(b"interactive"), interactive)
        .build()
    {
        Ok(sh) => sh,
        Err(e) => {
            // The diagnostic is already on stderr; this is only the status.
            std::process::exit(e.status().code() as i32);
        }
    };

    // A shell with no operand reads its own standard input, prompting if
    // it is interactive. This is dash's `cmdloop(1)`.
    let status = match sh.run(Source::stream()) {
        Ok(st) => st,
        Err(e) => e.status(),
    };

    std::process::exit(status.code() as i32);
}

fn main() {
    // Nothing here is callable yet.
    if std::env::args().len() == usize::MAX {
        let _ = run_and_capture();
        let _ = expand_a_word();
        frontend(&[]);
    }
}
