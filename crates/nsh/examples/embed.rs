//! The intended usage of the API, written before the API was, so that the
//! example got to judge it.
//!
//! **It compiled against a sketch of `todo!()`s once. It runs now** —
//! `cargo run -p nsh --example embed` executes every line below except the
//! frontend, which ends the process and is therefore behind an argument.
//! That is the difference the `public-api` node was for, and it is worth
//! stating because the file is otherwise unchanged in shape: the design
//! was judged by writing this, and what landed had to keep it writable.
//!
//! Three programs, which are the only embedders this design was allowed to
//! consider:
//!
//!   1. run a script and capture its output
//!   2. expand a word without running a command yourself
//!   3. `nsh-cli`, which has to stay byte-for-byte dash

use std::io;
use std::sync::atomic::{AtomicUsize, Ordering};

use bstr::{BStr, BString};
use nsh::{Disposition, Error, ExitStatus, Host, Shell, Signal, SignalSink, Source, Streams};

/// Shell output is bytes, not text: an argument need not be UTF-8 and dash
/// passes such bytes through untouched. Printing is where that has to be
/// resolved, so it is resolved here rather than in the types.
fn show(b: &[u8]) -> String {
    String::from_utf8_lossy(b.trim_ascii_end()).into_owned()
}

/// An `io::Error` as a shell diagnostic.
///
/// The sketch had an `Error::Io` variant for this. It was not promoted:
/// `docs/api-design.md` §3.4's rule is to start with `Other` so every
/// raise site converts mechanically and promote the interesting variants
/// afterwards, and nothing in the shell has yet needed to *match* on an
/// I/O failure. Status 2 is what `sh_error` takes.
fn io_error(e: io::Error) -> Error {
    unsafe { Error::other(0, 2, e.to_string().as_bytes()) }
}

// ---------------------------------------------------------------------
// 1. Run a script and capture what it wrote.
// ---------------------------------------------------------------------

fn run_and_capture() -> Result<(), Error> {
    let mut sh = Shell::builder()
        .arg0(BStr::new(b"myapp"))
        .inherit_env()
        .streams(Streams::capture().map_err(io_error)?)
        .build()?;

    sh.set_var(BStr::new(b"PATH"), BStr::new(b"/usr/bin:/bin"))?;

    // Command substitution reads the child through a pipe the shell makes,
    // so an external command's output reaches the script whatever the
    // shell's own stdout is. What the *capture* holds is what the shell
    // itself wrote — see `Streams::capture`, and §10 for the per-instance
    // descriptor table that would extend it to a forked command's writes.
    let status: ExitStatus = sh.run(b"for f in /etc/hostname; do echo \"$(wc -l < \"$f\") $f\"; done")?;
    let out: BString = sh.take_captured_stdout().map_err(io_error)?;

    // Two runs compose like two lines of one script: `count` is still set.
    sh.run(b"count=$(ls /etc | wc -l)")?;
    sh.run(b"echo \"$count files\"")?;
    let counted: BString = sh.take_captured_stdout().map_err(io_error)?;

    // `var` hands back a borrow of the table, so a value that has to
    // outlive the next `run` is copied out. The borrow checker is right
    // here: an assignment can move the table under it.
    let home: Option<BString> = sh.var(BStr::new(b"HOME")).map(|v| v.to_owned());

    // Untrusted data goes in as a positional parameter, never spliced into
    // the script text. This is the whole quoting problem, gone.
    let name = BStr::new(b"a file with 'quotes' and $HOME in it");
    sh.run_command(BStr::new(b"printf '%s\\n' \"$1\""), &[BStr::new(b"myapp"), name])?;
    let echoed: BString = sh.take_captured_stdout().map_err(io_error)?;
    assert_eq!(echoed.trim_ascii_end(), &name[..]);

    println!("1. status {}, the script said {}", status.code(), show(&out));
    println!("   two runs later: {}", show(&counted));
    println!(
        "   $HOME was {}, and the hostile argument survived intact",
        home.as_deref().map_or("unset".into(), |h| show(h))
    );
    Ok(())
}

// ---------------------------------------------------------------------
// 2. Expand a word. No second process, no quoting round trip, and nothing
//    a `Command` API can offer.
// ---------------------------------------------------------------------

fn expand_a_word() -> Result<(), Error> {
    let mut sh = Shell::builder().inherit_env().build()?;

    // Unquoted: splits on $IFS, then globs. Zero, one or many fields.
    let fields: Vec<BString> = sh.expand_word(BStr::new(b"/etc/hostn*"))?;

    // As if double-quoted: exactly one, no splitting, no globbing.
    let one: BString = sh.expand_word_quoted(BStr::new(b"${EDITOR:-vi}"))?;

    println!("2. {} field(s) from the glob, and ${{EDITOR:-vi}} is {one:?}", fields.len());
    Ok(())
}

// ---------------------------------------------------------------------
// 3. nsh-cli. The host is the whole difference between a library and a
//    shell, and it is about forty lines.
// ---------------------------------------------------------------------

/// Where the handler finds the inbox.
///
/// A signal handler is reached with a signal number and nothing else, so
/// the sink it reports to cannot be a field it reaches through `self`.
/// This is why [`Host::attach`] exists, and why the sink is a `&'static`
/// rather than something a handler would have to clone.
static SINK: AtomicUsize = AtomicUsize::new(0);

extern "C" fn on_signal(signo: libc::c_int) {
    let p = SINK.load(Ordering::Relaxed);
    if p != 0 {
        // The only thing a handler may do, and the whole of what it may
        // do. Everything behind it is two atomics and three stores.
        unsafe { (*(p as *const nsh::siginbox::SignalSink)).raise(signo) };
    }
}

/// The frontend's host: it owns the process, so it may do all of this.
///
/// `nsh::ProcessHost` is this, in the library, and a frontend should use
/// it. Written out here because the point of the example is to show that
/// an *embedder* can write one — everything it needs is on the surface.
struct FrontendHost;

impl Host for FrontendHost {
    fn attach(&mut self, sink: SignalSink) {
        // Stored where the `extern "C"` handler can reach it. The handler
        // does nothing but `sink.raise(signo)`.
        SINK.store(sink as *const _ as usize, Ordering::Relaxed);
    }

    fn signal(&mut self, signal: Signal) -> io::Result<Disposition> {
        // `sigaction(signo, NULL, &old)`. The shell needs the inherited
        // value to reproduce dash's "ignored on entry stays ignored".
        unsafe {
            let mut act: libc::sigaction = std::mem::zeroed();
            if libc::sigaction(signal.number(), std::ptr::null(), &mut act) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(if act.sa_sigaction == libc::SIG_IGN {
                Disposition::Ignore
            } else if act.sa_sigaction == libc::SIG_DFL {
                Disposition::Default
            } else {
                Disposition::Catch
            })
        }
    }

    fn set_signal(&mut self, signal: Signal, to: Disposition) -> io::Result<()> {
        // `sigaction` with `sigfillset(&sa_mask)` and `sa_flags = 0`. Both
        // are part of the contract: no SA_RESTART is why an interrupted
        // syscall returns EINTR and the shell always has a poll site.
        unsafe {
            let mut act: libc::sigaction = std::mem::zeroed();
            act.sa_sigaction = match to {
                Disposition::Catch => on_signal as *const () as usize,
                Disposition::Ignore => libc::SIG_IGN,
                Disposition::Default => libc::SIG_DFL,
            };
            act.sa_flags = 0;
            libc::sigfillset(&mut act.sa_mask);
            if libc::sigaction(signal.number(), &act, std::ptr::null_mut()) == -1 {
                return Err(io::Error::last_os_error());
            }
            Ok(())
        }
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
        .host(FrontendHost)
        .option(BStr::new(b"interactive"), interactive)
        .build()
    {
        Ok(sh) => sh,
        Err(e) => {
            // The diagnostic is already on stderr; this is only the status.
            std::process::exit(e.status().into());
        }
    };

    // A shell with no operand reads its own standard input, prompting if
    // it is interactive. This is dash's `cmdloop(1)`.
    let status = match sh.run(Source::stream()) {
        Ok(st) => st,
        Err(e) => ExitStatus::from_raw(e.status()),
    };

    std::process::exit(status.code().into());
}

/// The host, exercised without ending the process.
///
/// `frontend` cannot be part of a demo run — it reads standard input and
/// then exits — so this is the part of it that can be: a shell built with
/// an embedder-written host, which therefore installs real handlers.
fn a_shell_with_a_host() -> Result<(), Error> {
    let mut sh = Shell::builder()
        .arg0(BStr::new(b"sh"))
        .inherit_env()
        .host(FrontendHost)
        .streams(Streams::capture().map_err(io_error)?)
        .build()?;
    sh.run(b"trap 'echo caught' INT; echo host installed the handler")?;
    let out = sh.take_captured_stdout().map_err(io_error)?;
    println!("3. {}, and $? is {}", show(&out), sh.status().code());
    Ok(())
}

fn main() {
    let argv: Vec<BString> = std::env::args().map(|a| BString::from(a.into_bytes())).collect();
    if argv.iter().any(|a| a.as_slice() == b"--frontend") {
        frontend(&argv);
    }
    run_and_capture().unwrap();
    expand_a_word().unwrap();
    a_shell_with_a_host().unwrap();
}
