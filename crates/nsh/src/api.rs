//! **The proposed public API. Nothing here is implemented.**
//!
//! Every body is `todo!()`. This module exists so the signatures are
//! checked by the compiler rather than by reading: the borrow shapes, the
//! object safety of [`Host`], and whether a built-in that re-enters
//! evaluation can be written at all are questions a document cannot
//! answer and a type-checked sketch can.
//!
//! The reasoning is in `docs/api-design.md`. Read that first; this is the
//! artefact it produces.
//!
//! When the real implementation lands (`public-api`, after
//! [dec:nsh:no-ambient-state] and [dec:nsh:host-owns-signals]) this module
//! is replaced by it and the re-exports move to `lib.rs`. Until then
//! nothing in the crate calls anything here.
//!
//! `#![deny(missing_docs)]` is on because the surface property
//! ([dec:nsh:public-surface]) is measured under it, and a surface designed
//! without the lint on is a surface that will not survive it.

#![deny(missing_docs)]

use core::fmt;
use std::ffi::NulError;
use std::io;
use std::os::unix::io::RawFd;
use std::path::Path;

use bstr::{BStr, BString, ByteSlice};

// =====================================================================
// The instance
// =====================================================================

/// A shell.
///
/// Owns everything the shell language can observe or change: the variable
/// table, aliases, functions and the command hash, jobs, options and the
/// positional parameters, traps, the input stack, the descriptor table,
/// and `$?`. Two `Shell` values share nothing except the C library's own
/// process globals, which are named in `docs/api-design.md` §6 and cannot
/// be separated.
///
/// `Shell` is [`Send`] and deliberately not [`Sync`]: it may be moved to
/// another thread, and every method that can observe shell state takes
/// `&mut self`, so there is no shared-reference concurrency to model.
pub struct Shell {
    // The field list IS the specification for [dec:nsh:no-ambient-state]:
    // each name below is one commit of `move-state`, and the granularity
    // is chosen so that no commit has to split a table across two fields.
    // `docs/api-design.md` §5 maps each to the statics it absorbs.
    /// `var.rs`: `vartab`, `varinit` and their backing buffers,
    /// `localvar_stack`, `lineno`.
    vars: VarTable,
    /// `alias.rs`: `atab`.
    aliases: AliasTable,
    /// `exec.rs`: `cmdtable`, `builtinloc`. Function definitions live here
    /// because dash stores them in the same hash.
    commands: CmdTable,
    /// `jobs.rs`: `jobtab`, `njobs`, `curjob` (as an index), `backgndpid`,
    /// `initialpgrp`, `ttyfd`, `jobctl`, `job_warning`.
    jobs: JobTable,
    /// `options.rs`: `optlist`, `shellparam`, `arg0`.
    options: Options,
    /// `trap.rs`: `trap`, `ptrap`, `trapcnt`, `sigmode`.
    traps: TrapTable,
    /// `input.rs`: `parsefile`, `basepf`, `basebuf`, `toppf`,
    /// `stdin_state`, `whichprompt`, `stdin_istty`; and `parser.rs`'s
    /// eleven parser globals, which are per-input-position state.
    input: InputStack,
    /// The logical-to-real descriptor map, plus `redir.rs`'s `redirlist`
    /// and `closed_redirs`. This is what makes [`Streams::from_fds`] carry
    /// to redirection and to external commands.
    fds: FdTable,
    /// `output.rs`'s `ShellIo` aggregate: buffered stdout, unbuffered stderr,
    /// and the saved stderr destination used while tracing redirections.
    io: ShellIo,
    /// `eval.rs`: `evalskip`, `skipcount`, `loopnest`, `funcline`,
    /// `commandname`, `back_exitstatus`, `savestatus`, `inps4`.
    eval: EvalState,
    /// Where the shell's own three streams come from, and the capture
    /// buffers if there are any.
    streams: Streams,
    /// What the library is not allowed to do on its own authority.
    host: Box<dyn Host>,
    /// Called for every diagnostic, including the ones dash reports and
    /// continues past, which therefore never reach a `Result`.
    on_diagnostic: Option<Box<dyn FnMut(&Error) + Send>>,
    /// The signal inbox. `Clone`d into the host at build time; polled at
    /// the same points dash checks `pending_sig`.
    signals: SignalSink,
    /// `$?`. `eval.rs`'s `exitstatus`, which stays a field rather than a
    /// return value because a dozen sites read it out of band.
    status: ExitStatus,
    /// `Some` once the script has run `exit`, or `set -e` has aborted, or
    /// end-of-input was reached under the exit-after-evaluating flag. The
    /// shell is not poisoned by it: an embedder may keep using it.
    exited: Option<ExitStatus>,
}

impl Shell {
    /// Start building a shell.
    pub fn builder() -> Builder {
        todo!()
    }

    /// Parse and execute `source` to its end.
    ///
    /// Two calls compose exactly as two `eval` commands do, and for the
    /// same reason: this *is* `eval`, at the top level. Variables,
    /// functions, aliases, options, traps, the working directory, jobs and
    /// `$?` all persist. What does not persist is the parse: `source` must
    /// contain complete commands, and `run(b"if true; then")` is the same
    /// syntax error that `eval 'if true; then'` is.
    ///
    /// `Err` means the run was *aborted* by a diagnostic — dash's `EXERROR`
    /// reaching the top level. Diagnostics dash reports and carries on past
    /// do not appear here at all: `run(b"nosuch; echo done")` prints
    /// `nosuch: not found`, prints `done`, and returns `Ok(0)`. Those reach
    /// an embedder only through [`Builder::on_diagnostic`].
    ///
    /// `Ok` covers a script that ran out, one that called `exit`, and one
    /// that `set -e` aborted; [`Shell::has_exited`] tells the first from
    /// the other two.
    ///
    /// This method cannot be called re-entrantly. It takes `&mut self`, and
    /// every callback the shell invokes while running already holds that
    /// borrow, so a [`Host`] method or a diagnostic hook that tried to run
    /// a script would not compile.
    pub fn run(&mut self, source: impl Into<Source>) -> Result<ExitStatus, Error> {
        todo!()
    }

    /// The `sh -c` shape: run `command`, with `args[0]` as `$0` and the
    /// rest as the positional parameters.
    ///
    /// Passing data as a positional parameter rather than interpolating it
    /// into `command` is the only way to keep it out of the parser, so this
    /// is the injection-safe form and the reason it is on the surface
    /// beside [`Shell::run`].
    ///
    /// The parameters persist afterwards, exactly as `sh -c` leaves them.
    pub fn run_command(&mut self, command: &BStr, args: &[&BStr]) -> Result<ExitStatus, Error> {
        todo!()
    }

    /// Expand one word as the shell would in command position, and return
    /// the fields it becomes.
    ///
    /// Tilde, parameter, command and arithmetic substitution, then field
    /// splitting on `$IFS`, then pathname expansion, then quote removal.
    /// One word is zero, one or many fields: `$x` with `x='a b'` is two,
    /// `$x` with `x=''` is none, `*.txt` is however many files match.
    ///
    /// **This executes.** Command substitution is part of word expansion,
    /// so `expand_word(b"$(rm -rf /)")` runs the command. There is no mode
    /// that expands without executing, because the shell language does not
    /// have one.
    pub fn expand_word(&mut self, word: &BStr) -> Result<Vec<BString>, Error> {
        todo!()
    }

    /// Expand one word as if it appeared inside double quotes: no field
    /// splitting, no pathname expansion, always exactly one result.
    ///
    /// The same caveat about command substitution applies.
    pub fn expand_word_quoted(&mut self, word: &BStr) -> Result<BString, Error> {
        todo!()
    }

    /// Read a shell variable.
    ///
    /// This is the variable table, not the language: `$?`, `$#`, `$1` and
    /// `$@` are not variables and are not here. [`Shell::status`] is `$?`,
    /// and [`Shell::expand_word`] reads the rest.
    ///
    /// The result borrows the table, so a value that has to outlive the
    /// next [`Shell::run`] must be copied out. That is not a papercut to
    /// design away: an assignment can move the table, and the borrow is
    /// what says so.
    pub fn var(&self, name: &BStr) -> Option<&BStr> {
        todo!()
    }

    /// Assign a shell variable, with the meaning `name=value` has in a
    /// script.
    ///
    /// A variable that is already exported stays exported; a new one is
    /// not, which is why `Shell::set_var(b"PATH", …)` reaches child
    /// processes and `Shell::set_var(b"MY_FLAG", …)` does not. The initial
    /// exported environment is [`Builder::env`].
    ///
    /// Fails on a name that is not a valid shell name, and on a readonly
    /// variable, which is the same diagnostic a script would get.
    pub fn set_var(&mut self, name: &BStr, value: &BStr) -> Result<(), Error> {
        todo!()
    }

    /// Unset a shell variable. Returns whether it was set.
    pub fn unset_var(&mut self, name: &BStr) -> bool {
        todo!()
    }

    /// Every variable in the table, in the order a bare `set` prints them.
    pub fn vars(&self) -> impl Iterator<Item = (&BStr, &BStr)> + '_ {
        // An anonymous return type keeps the iterator out of the surface;
        // the cost is that it cannot be named, which no embedder needs.
        core::iter::from_fn(|| todo!())
    }

    /// `$?`.
    pub fn status(&self) -> ExitStatus {
        todo!()
    }

    /// Whether the shell has run `exit`, or been stopped by `set -e`.
    ///
    /// A `Shell` in this state still works. A frontend stops; an embedder
    /// running a sequence of scripts may not want to.
    pub fn has_exited(&self) -> bool {
        todo!()
    }

    /// Take everything written to the shell's stdout since the last call.
    ///
    /// Only meaningful under [`Streams::capture`]. The capture is an
    /// unlinked temporary file rather than a pipe, so a script that writes
    /// more than a pipe buffer cannot deadlock against a host that has not
    /// got round to reading yet.
    ///
    /// Owned bytes rather than `&BStr`, which is what
    /// [dec:nsh:public-surface] records. A borrow would be tied to the
    /// `&mut self` that reads the file, so holding the output would lock
    /// the shell and `run`, look, `run` again — the reason to capture at
    /// all — would not compile. `crates/nsh/examples/embed.rs` is where
    /// that was discovered.
    pub fn take_captured_stdout(&mut self) -> io::Result<BString> {
        todo!()
    }

    /// Take everything written to the shell's stderr since the last call.
    pub fn take_captured_stderr(&mut self) -> io::Result<BString> {
        todo!()
    }
}

// =====================================================================
// Construction
// =====================================================================

/// Builds a [`Shell`].
///
/// Every setting has a default that makes the shell inert with respect to
/// the process: descriptors 0, 1 and 2, an empty environment, a host that
/// installs no signal handler and refuses to replace the process image,
/// and the current working directory.
pub struct Builder {
    _private: (),
}

impl Builder {
    /// `$0`, and the name every diagnostic is prefixed with. Defaults to
    /// `sh`, which is what dash falls back to.
    pub fn arg0(self, arg0: &BStr) -> Self {
        todo!()
    }

    /// The positional parameters `$1`, `$2`, … for scripts run with
    /// [`Shell::run`]. [`Shell::run_command`] sets its own.
    pub fn args(self, args: &[&BStr]) -> Self {
        todo!()
    }

    /// Variables in the initial environment, exported, as `execve` would
    /// have delivered them.
    pub fn env<K, V>(self, vars: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<BString>,
        V: Into<BString>,
    {
        todo!()
    }

    /// The calling process's environment, as bytes.
    ///
    /// Separate from [`Builder::env`] because `std::env::vars_os` yields
    /// `OsString`, which is bytes on Unix but is not `Into<BString>`, and
    /// making the bound accept both would cost more than a second method.
    pub fn inherit_env(self) -> Self {
        todo!()
    }

    /// Where the shell's own three streams come from. Defaults to
    /// [`Streams::inherit`].
    pub fn streams(self, streams: Streams) -> Self {
        todo!()
    }

    /// What the library may do to the process, and who does it.
    ///
    /// Without this the shell installs no signal handler and refuses
    /// `exec`, which is the correct default for a library and is not what
    /// a shell frontend wants.
    pub fn host(self, host: impl Host + 'static) -> Self {
        todo!()
    }

    /// Set one shell option by its `set -o` long name or its letter:
    /// `option(b"errexit", true)` and `option(b"e", true)` are the same.
    ///
    /// This is the whole of the option surface. `set -eu` inside a script
    /// needs no quoting and reaches the same table, so there is nothing an
    /// embedder can express here that it could not express there.
    pub fn option(self, name: &BStr, on: bool) -> Self {
        todo!()
    }

    /// The shell's working directory.
    ///
    /// Per-instance in the sense that `$PWD` and `cd` are, and process-wide
    /// in the sense that `chdir` is; see `docs/api-design.md` §6.
    pub fn cwd(self, dir: impl AsRef<Path>) -> Self {
        todo!()
    }

    /// Observe every diagnostic the shell writes, as a value.
    ///
    /// The hook does not suppress the write — the bytes still go where dash
    /// puts them, because that ordering is under test in every differential
    /// case. It exists because most shell diagnostics never abort anything
    /// and so never reach a `Result`: `nosuch: not found` inside a loop is
    /// reported, the loop continues, and this hook is the only way to see
    /// it as structure.
    pub fn on_diagnostic(self, hook: impl FnMut(&Error) + Send + 'static) -> Self {
        todo!()
    }

    /// Build the shell.
    pub fn build(self) -> Result<Shell, Error> {
        todo!()
    }
}

// =====================================================================
// Where a script comes from
// =====================================================================

/// Where [`Shell::run`] reads commands.
///
/// Bytes or a descriptor, and nothing else. There is deliberately no
/// `impl Read` source: the input stack is descriptor-based because line
/// editing, `PS2` continuation, the stdin tee and the post-fork reset all
/// key off *which descriptor* a parse file is on, and a reader cannot be
/// handed to the line editor or shared with a child across `fork`.
pub struct Source {
    _private: (),
}

impl Source {
    /// A script held in memory. The `-c` and `eval` shape.
    pub fn bytes(text: impl Into<BString>) -> Source {
        todo!()
    }

    /// A script the shell opens by path. The `.` and `sh script` shape.
    ///
    /// A path that is not UTF-8 goes through `OsStr::from_bytes`, which is
    /// free — `Path` is bytes on Unix.
    pub fn file(path: impl AsRef<Path>) -> Source {
        todo!()
    }

    /// The shell's own standard input, from [`Streams`]. The bare `sh`
    /// shape, and the one an interactive frontend uses.
    pub fn stream() -> Source {
        todo!()
    }
}

impl From<&[u8]> for Source {
    fn from(_: &[u8]) -> Source {
        todo!()
    }
}

impl<const N: usize> From<&[u8; N]> for Source {
    fn from(_: &[u8; N]) -> Source {
        todo!()
    }
}

impl From<&BStr> for Source {
    fn from(_: &BStr) -> Source {
        todo!()
    }
}

impl From<BString> for Source {
    fn from(_: BString) -> Source {
        todo!()
    }
}

impl From<Vec<u8>> for Source {
    fn from(_: Vec<u8>) -> Source {
        todo!()
    }
}

// =====================================================================
// Streams
// =====================================================================

/// Where the shell's own three streams come from.
///
/// Under [dec:nsh:no-ambient-state] this is also the base of the shell's
/// descriptor table, so it carries further than the shell's own reads and
/// writes: redirection, pipelines and forked external commands all resolve
/// through the table and land here. The two things that do not are named
/// on [`Streams::from_fds`].
pub struct Streams {
    _private: (),
}

impl Streams {
    /// Descriptors 0, 1 and 2. What a shell started as a process uses, and
    /// the only configuration the differential harness exercises.
    pub fn inherit() -> Streams {
        todo!()
    }

    /// Three descriptors the caller owns and keeps open for the shell's
    /// lifetime.
    ///
    /// The shell's descriptor table starts as `0 -> stdin`, `1 -> stdout`,
    /// `2 -> stderr`, and every descriptor number in the script resolves
    /// through it; a forked child materialises the table with `dup2` before
    /// `execve`, so external commands agree. Two things do not:
    ///
    /// * `/dev/stdout`, `/dev/fd/N` and `/proc/self/fd/N` name the kernel's
    ///   table, not the shell's, and open the process's descriptors.
    /// * `exec cmd`, which replaces the process image and so has to ask
    ///   [`Host::may_replace_process`]; a host that says yes gets its own
    ///   descriptors replaced along with everything else.
    pub fn from_fds(stdin: RawFd, stdout: RawFd, stderr: RawFd) -> Streams {
        todo!()
    }

    /// Empty stdin; stdout and stderr collected into unlinked temporary
    /// files, readable with [`Shell::captured_stdout`].
    ///
    /// A temporary file rather than a pipe: a pipe with no concurrent
    /// reader blocks the shell as soon as the script writes more than the
    /// pipe buffer, and a capture API that deadlocks on large output is
    /// worse than none.
    pub fn capture() -> io::Result<Streams> {
        todo!()
    }
}

// =====================================================================
// The host seam
// =====================================================================

/// What the library will not do on its own authority.
///
/// Implemented by whoever owns the process. `crates/nsh-cli` implements it
/// by doing exactly what dash does; an embedder that implements nothing
/// gets a shell that installs no handler and refuses to `exec`.
///
/// This is [dec:nsh:host-owns-signals] as a type. No method takes a
/// [`Shell`], and that is load bearing rather than an omission: it makes
/// the host a leaf, so `self.host.set_signal(…)` is a field-disjoint borrow
/// inside a `&mut self` method, and it makes re-entering the shell from a
/// host callback a compile error instead of a documented hazard.
pub trait Host: Send {
    /// Take the shell's signal inbox. Called once, from
    /// [`Builder::build`].
    ///
    /// A host that installs [`Disposition::Catch`] must keep this and must
    /// have its handler do nothing but [`SignalSink::raise`] — the handler
    /// runs in signal context, where nothing else here is safe.
    fn attach(&mut self, sink: SignalSink);

    /// What is installed for `signal` right now.
    ///
    /// The shell needs this, not only `set_signal`: a signal that was
    /// already ignored when the shell started stays ignored and cannot be
    /// trapped (`trap.rs:258-266`, dash's `S_HARD_IGN`), and that rule
    /// cannot be reproduced without reading the inherited disposition.
    fn signal(&mut self, signal: Signal) -> io::Result<Disposition>;

    /// Install a disposition the shell has asked for.
    ///
    /// The shell decides *which*; the host performs it. To be dash the
    /// host must install with all signals blocked in the handler's mask and
    /// no flags — `sigfillset` on `sa_mask`, `sa_flags = 0`
    /// (`trap.rs:284-287`).
    fn set_signal(&mut self, signal: Signal, to: Disposition) -> io::Result<()>;

    /// May the shell replace the process image?
    ///
    /// `exec cmd` `execve`s in place. In a frontend that is the point; in a
    /// library it destroys the host. A host that refuses gets the same
    /// diagnostic and status a failed `exec` produces.
    fn may_replace_process(&mut self) -> bool;
}

/// What a signal does.
#[derive(Copy, Clone, Debug, PartialEq, Eq, Hash)]
pub enum Disposition {
    /// `SIG_DFL`.
    Default,
    /// `SIG_IGN`.
    Ignore,
    /// Deliver to this shell, through the [`SignalSink`] it was given.
    Catch,
}

/// The shell's signal inbox.
///
/// Cheap to clone, safe to hold across threads, and safe to touch from a
/// signal handler. A shell polls it where dash reads `pending_sig`.
#[derive(Clone)]
pub struct SignalSink {
    _private: (),
}

impl SignalSink {
    /// Record that `signal` was delivered.
    ///
    /// The only method a signal handler may call: one relaxed atomic store,
    /// no allocation, no lock, no reentrancy.
    pub fn raise(&self, signal: Signal) {
        todo!()
    }
}

// =====================================================================
// Values
// =====================================================================

/// A shell exit status: `$?`.
///
/// A `u8`, because that is the range `$?` has — `exit 300` leaves 44, in
/// dash and in this port.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExitStatus(u8);

impl ExitStatus {
    /// Zero.
    pub const SUCCESS: ExitStatus = ExitStatus(0);

    /// The status as a number.
    pub fn code(self) -> u8 {
        self.0
    }

    /// Whether the status is zero.
    pub fn success(self) -> bool {
        self.0 == 0
    }

    /// The signal a command died from, under the shell's `128 + n`
    /// convention. A command that merely exited 130 is indistinguishable
    /// from one killed by SIGINT, because in a shell it is.
    pub fn signal(self) -> Option<Signal> {
        todo!()
    }
}

/// A signal number.
///
/// A newtype over the number rather than an enum: signal numbers are
/// platform-dependent, the shell has to carry ones it does not know a name
/// for, and an enum would need an `Other(i32)` arm anyway.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Signal(i32);

impl Signal {
    /// From a raw number.
    pub fn from_raw(number: i32) -> Signal {
        Signal(number)
    }

    /// The raw number.
    pub fn number(self) -> i32 {
        self.0
    }

    /// The name without the `SIG` prefix — `INT`, `TERM` — as `trap -l`
    /// prints it, or `None` for a number this platform has no name for.
    pub fn name(self) -> Option<&'static BStr> {
        todo!()
    }
}

// =====================================================================
// Errors
// =====================================================================

/// A shell diagnostic, as a value.
///
/// Every one of these was also *written* to the shell's stderr at the point
/// it happened, in dash's bytes and dash's order. That is not redundancy:
/// `tests/harness/dscase.sh:64-71` merges stdout and stderr and compares
/// the result, so where a diagnostic lands in the stream is under test in
/// all 61,498 cases, and a design that returned the text instead of writing
/// it would emit every diagnostic at the end of the run.
///
/// Control flow is not here. `exit`, `return`, `break`, `continue` and the
/// `set -e` abort are ordinary results, not errors — see
/// [dec:nsh:errors-are-values].
///
/// `#[non_exhaustive]`, and [`Error::Other`] is where a diagnostic with no
/// specific variant goes. Its `message` is not stable: a variant promoted
/// out of `Other` is a change to text nobody should have been matching on.
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The parser rejected the input. Status 2.
    Syntax {
        /// Line within the current input.
        line: u32,
        /// dash's text, without the `sh: N: ` prefix.
        message: BString,
    },

    /// Word expansion failed: a bad substitution, `${x?}` on an unset
    /// variable, an arithmetic error. Status 2.
    Expansion {
        /// Line within the current input.
        line: u32,
        /// The word that failed, with the parser's in-band markers already
        /// removed.
        word: BString,
        /// dash's text.
        message: BString,
    },

    /// A redirection could not be performed. Status 2.
    Redirect {
        /// Line within the current input.
        line: u32,
        /// The descriptor number in the script.
        fd: RawFd,
        /// What the underlying call reported.
        source: io::Error,
    },

    /// No such command on `$PATH`. Status 127.
    ///
    /// Usually **not** an aborting error: dash reports it and carries on,
    /// so it reaches [`Builder::on_diagnostic`] and not a `Result`. It
    /// aborts only where dash's does — inside the forked child that was
    /// about to `execve`, and after a special built-in.
    NotFound {
        /// Line within the current input.
        line: u32,
        /// The name as written.
        name: BString,
    },

    /// The command exists but could not be executed. Status 126.
    NotExecutable {
        /// Line within the current input.
        line: u32,
        /// The name as written.
        name: BString,
        /// What `execve` reported.
        source: io::Error,
    },

    /// A built-in reported an error.
    Builtin {
        /// Which built-in.
        name: &'static BStr,
        /// The status it asked for.
        status: ExitStatus,
        /// dash's text.
        message: BString,
    },

    /// The shell was interrupted.
    ///
    /// Distinct from every other variant because the host has to be able to
    /// tell "your script failed" from "the user pressed ^C". Dying by the
    /// signal, which is what dash does when it is not an interactive root
    /// shell, is the frontend's act and not the library's.
    Interrupted(Signal),

    /// A shell value reaching a system call contained a NUL byte.
    ///
    /// The one invariant `BString` stops enforcing that `*mut c_char`
    /// enforced by construction; `CString::new` is where it is re-checked
    /// ([dec:nsh:bytes-not-text]).
    Nul(NulError),

    /// An I/O error with no more specific variant.
    Io(io::Error),

    /// A diagnostic with no specific variant.
    ///
    /// Around a hundred `sh_error` sites produce text that is worth
    /// reporting and not worth a type. This is also what makes the
    /// conversion tractable: `errors-are-values` can turn every raise site
    /// into `Other` mechanically and promote the interesting ones after,
    /// rather than needing the final taxonomy before the first commit.
    Other {
        /// Line within the current input.
        line: u32,
        /// The status the shell takes from it — 2 unless the site says
        /// otherwise.
        status: ExitStatus,
        /// dash's text.
        message: BString,
    },
}

impl Error {
    /// The exit status the shell takes from this error.
    pub fn status(&self) -> ExitStatus {
        todo!()
    }

    /// dash's text for this error, byte for byte, **without** the
    /// `sh: 1: cd: ` prefix.
    ///
    /// The prefix is `$0`, `$LINENO` and the running command's name
    /// (`error.rs:267-295`), which are shell state and not error state, so
    /// an `Error` on its own cannot render them. The shell adds them when
    /// it writes; an embedder that wants the prefixed form reads the
    /// stderr stream, which is where it already is.
    pub fn message(&self) -> BString {
        todo!()
    }

    /// The line the error was reported at, where there is one.
    pub fn line(&self) -> Option<u32> {
        todo!()
    }
}

impl fmt::Display for Error {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // Lossy UTF-8, because `Display` cannot be anything else. The
        // byte-exact form is `message`.
        fmt::Display::fmt(self.message().as_bstr(), f)
    }
}

impl std::error::Error for Error {}

// =====================================================================
// Internal shapes the surface constrains
// =====================================================================
//
// Not public. Here because they are the parts of the design that the
// public signatures decide, and getting them wrong is the expensive
// mistake `public-api-design` exists to avoid.

/// Control flow, which is not an error.
///
/// dash's `evalskip` bitmask and its `EXEND`/`EXEXIT` exceptions, together,
/// in the `Ok` position. The status that goes with it is in
/// [`Shell::status`], as dash keeps it in `exitstatus`, because a dozen
/// sites read it out of band and turning it into a return value would be a
/// second refactor riding on this one.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub(crate) enum Flow {
    /// Carry on.
    Normal,
    /// `break n`.
    Break(u32),
    /// `continue n`.
    Continue(u32),
    /// `return`, and the function-definition unwind `exitshell` uses.
    Return,
    /// `exit`, the `set -e` abort, and end-of-input under the
    /// exit-after-evaluating flag. dash's `EXEXIT` and `EXEND`, which
    /// differ only in which status is taken.
    Exit,
}

/// What every evaluation step returns.
pub(crate) type Eval = Result<Flow, Error>;

/// The syntactic context of an evaluation. dash's `EV_*` flags.
#[derive(Copy, Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct EvalCtx {
    /// The enclosing syntax consumes this command's status — a `while` or
    /// `if` condition, a `!`, the left operand of `&&` or `||` — so
    /// `set -e` must not act on it. dash's `EV_TESTED`.
    ///
    /// This stays a property of the *call* and never becomes a property of
    /// an [`Error`], because most `set -e` aborts have no error in flight:
    /// `set -e; false` exits the shell, and `false` produced no diagnostic,
    /// no `Err`, and nothing but a status.
    pub tested: bool,
    /// This is the last thing the shell will do, so a simple command may
    /// `execve` in place instead of forking. dash's `EV_EXIT`.
    pub exit: bool,
}

impl Shell {
    /// Write `e` where dash writes it, hand it to the hook, and give it
    /// back so a raise site can `return Err(sh.report(e))`.
    ///
    /// The single funnel for [dec:nsh:errors-are-values]'s
    /// write-*and*-return rule. Rendering goes through [`Error::message`]
    /// so the bytes on the stream and the bytes in the value cannot drift.
    pub(crate) fn report(&mut self, e: Error) -> Error {
        todo!()
    }

    /// The shell asks the host for a disposition.
    ///
    /// This compiles, and that is the point of it being here: `host` is one
    /// field, so borrowing it does not borrow the tables, and a `Host`
    /// method that took `&mut Shell` would make this line impossible.
    pub(crate) fn ask_for_signal(&mut self, signal: Signal, to: Disposition) -> io::Result<()> {
        self.host.set_signal(signal, to)
    }
}

/// A built-in.
///
/// `&mut Shell` and a borrowed argument vector — and the vector must not
/// borrow from the shell, which is the whole constraint. Ten built-ins
/// re-enter evaluation (`.`, `eval`, `command`, `fc`, `trap`'s handler),
/// so they have to hand `&mut Shell` straight back; if `args` pointed into
/// a `Shell` field they could not.
pub(crate) type Builtin = fn(&mut Shell, &[&BStr]) -> Result<ExitStatus, Error>;

/// `.` — the case that proves the type above is writable.
pub(crate) fn dot_builtin(sh: &mut Shell, args: &[&BStr]) -> Result<ExitStatus, Error> {
    use std::os::unix::ffi::OsStrExt;

    let name = match args.get(1) {
        Some(n) => *n,
        None => {
            return Err(sh.report(Error::Other {
                line: 0,
                status: ExitStatus(2),
                message: BString::from(&b"filename argument required"[..]),
            }));
        }
    };
    let path = std::ffi::OsStr::from_bytes(name.as_bytes());
    // Re-entry. `args` is borrowed from the caller's storage, not from
    // `sh`, so `sh` is free to be reborrowed here.
    sh.run(Source::file(path))?;
    Ok(sh.status())
}

// ---------------------------------------------------------------------
// Placeholders for the state `Shell` owns. Each is one commit of
// `move-state`; `docs/api-design.md` §5 says which statics each absorbs.
// ---------------------------------------------------------------------

struct VarTable;
struct AliasTable;
struct CmdTable;
struct JobTable;
struct Options;
struct TrapTable;
struct InputStack;
struct FdTable;
struct ShellIo;
struct EvalState;
