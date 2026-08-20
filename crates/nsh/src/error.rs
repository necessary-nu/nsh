//! Literal port of `src/error.c` / `src/error.h`.
//! Rules: `docs/spec/port/src/error.md`.
//!
//! Deviations forced by Rust, all noted inline:
//!
//! * The C variadic diagnostic entry points take a complete byte message in
//!   Rust. Callers compose typed values before crossing this boundary, so
//!   diagnostics do not need a second formatting language or a `va_list`.
//! * There is no `setjmp`/`longjmp` and no `jmp_buf`. The C's exception
//!   mechanism is gone; a failure is a value and it is returned. See
//!   `[dec:nsh:errors-are-values]` and `docs/errors-are-values.md`.

use core::sync::atomic::{Ordering, compiler_fence};
use std::io::Write;

use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;

/*
 * Types of operations (passed to the errmsg routine).
 */

pub const E_OPEN: c_int = 0o1; /* opening a file */
pub const E_CREAT: c_int = 0o2; /* creating a file */
pub const E_EXEC: c_int = 0o4; /* executing a program */

/*
 * `sig_atomic_t` is not re-exported by the `libc` crate; on every
 * platform dash targets it is `int`.
 */
pub type sig_atomic_t = c_int;

/* `jmp_buf`, `struct jmploc`, the four exception codes, `handler` and
 * `exception` were all here, along with the C's comment about saving
 * `handler` on entry to an inner scope and restoring it on exit, and an
 * `extern "C"` block for `setjmp`. All of it is gone with the mechanism:
 * there is no buffer, no handler, and no nesting discipline to observe.
 * A frame that wants to know whether what it called failed reads the
 * `Result` the call returned.
 *
 * What replaced each code is worth naming once, because the C's four
 * integers were three different things:
 *
 *   EXERROR  a diagnostic          -> `Err(Error)`
 *   EXINT    the user's interrupt  -> `Err(Error::Interrupted)`
 *   EXEND    the shell is ending   -> `Ok(Flow::END)`
 *   EXEXIT   `exit` ran            -> `Ok(Flow::exit(status))`
 *
 * `[dec:nsh:errors-are-values]` is the decision that says the middle
 * column is the right division and the last two belong in the `Ok`
 * position; `docs/api-design.md` 3.1 is where the three-way split is
 * written down. `handler` is gone because nothing arms one, and it is
 * what `[dec:nsh:no-ambient-state]` was waiting for: a pointer into a
 * live stack frame cannot be a field of a `Shell`, and there is no longer
 * a pointer. */

/* `int errlinno` was here. It is `Shell::eval.errlinno` now: it is the
 * line a diagnostic reports, six frames write it and the only reader is
 * the prefix this module builds, so it belongs to the shell that
 * reports rather than to the process. */

/*
 * These macros allow the user to suspend the handling of interrupt signals
 * over a period of time.  This is similar to SIGHOLD to or sigblock, but
 * much more efficient and portable.  (But hacking the kernel is so much
 * more fun than worrying about efficiency and portability. :-))
 */

/* `#define barrier() ({ __asm__ __volatile__ ("": : :"memory"); })` */
#[inline(always)]
pub fn barrier() {
    compiler_fence(Ordering::SeqCst);
}

/* `#define INTOFF ({ suppressint++; barrier(); 0; })` */
#[inline(always)]
pub fn INTOFF(sh: &mut crate::context::Shell) -> c_int {
    sh.interrupt_suppression += 1;
    barrier();
    0
}

/// `#define INTON ({ barrier(); if (--suppressint == 0 && intpending) onint(sh); 0; })`
///
/// The `onint(sh)` is gone and the rest is unchanged. That is step F, and
/// it is the whole of the divergence `docs/divergences.md`'s
/// `error.interrupt-delivery-point` records: the C delivers a pending
/// interrupt at the instruction where the counter reaches zero, and this
/// leaves `intpending` set for the next poll site to take.
///
/// **`INTON` stays infallible, deliberately.** §4.3 measured what making
/// it fallible costs — 44 functions enter the fixpoint, and they are the
/// shell's teardown: `popredir`, `unwindredir`, `unwindfiles`,
/// `popallfiles`, `exitreset`, `freejob`, `ifsfree`. A design in which
/// cleanup can fail while handling a failure is the wrong shape, and
/// every call site would have to decide what to do with an error raised
/// while handling an error.
#[inline(always)]
pub fn INTON(sh: &mut crate::context::Shell) -> c_int {
    barrier();
    sh.interrupt_suppression -= 1;
    0
}

/// `#define FORCEINTON ({ barrier(); suppressint = 0; if (intpending) onint(sh); 0; })`
///
/// Same change, same reason. This one *resets* the counter rather than
/// balancing it (§2.4), which is what makes it the top level's way of
/// discarding a leak; discarding the leak and taking delivery were one
/// operation in the C and are two now.
#[inline(always)]
pub fn FORCEINTON(sh: &mut crate::context::Shell) -> c_int {
    barrier();
    sh.interrupt_suppression = 0;
    0
}

/* `#define CLEAR_PENDING_INT intpending = 0` */
#[inline(always)]
pub fn CLEAR_PENDING_INT() {
    crate::siginbox::signals().set_interrupt_pending(false);
}

/// Take delivery of a pending interrupt, if one is due.
///
/// The question every poll site asks, in one place so that all of them
/// ask it the same way. "Due" is *pending* and *not suppressed*: an
/// `INTOFF` bracket still holds the interrupt off, exactly as it held off
/// the C's asynchronous delivery, because the bracket is what makes the
/// mutation inside it atomic against a signal.
///
/// There are five poll sites, and they are the places the shell reaches
/// on its own rather than the places a signal happens to arrive:
/// `trap::dotrap`, which `evaltree` calls before and after every command
/// and which is therefore the one that matters most; and the four `EINTR`
/// returns where a blocking syscall came back — `redir::sh_open`,
/// `input::preadfd`, `expand::expbackq`'s command-substitution read, and
/// `jobs::waitproc`'s `wait3`. `output.rs`'s `write` is deliberately not
/// one: dash collects output errors in `outerr` and checks them
/// separately rather than raising, and making the output path fallible is
/// the shape §4.3 argues against.
///
/// Returns `Some` at most once per interrupt: [`onint`] clears
/// `intpending` as it delivers.
#[inline]
pub fn poll_interrupt(sh: &crate::context::Shell) -> Option<Error> {
    if sh.interrupt_suppression == 0 && int_pending() != 0 {
        Some(onint(sh))
    } else {
        None
    }
}

/// Put a taken interrupt back, for a frame that cannot carry it out.
///
/// [`poll_interrupt`] takes delivery, which means it *clears*
/// `intpending`; a frame that then drops the value has lost the
/// interrupt, and the shell stops answering `^C`. One frame is in that
/// position and cannot be moved out of it: `parser::getprompt` is a
/// callback the line editor calls through a function pointer, so it has
/// no `Result` to return and no caller of its own to return it to.
///
/// The C's answer there was to longjmp out of the line editor, through
/// frames a C library owns — the same shape as
/// `expand::opendir_interruptible` unwinding out of `glob`, and the same
/// reason it cannot survive `panic = "abort"`. This is the honest
/// alternative: the interrupt goes back in the inbox and the next poll
/// site takes it, which is one prompt-expansion later.
pub fn rearm_interrupt(e: Error) {
    debug_assert!(
        e.is_interrupt(),
        "only an interrupt may be put back; a diagnostic has already been written"
    );
    drop(e);
    crate::siginbox::signals().set_interrupt_pending(true);
}

/* `#define int_pending() intpending` */
#[inline(always)]
pub fn int_pending() -> sig_atomic_t {
    crate::siginbox::signals().interrupt_pending() as sig_atomic_t
}

/// `#define INTOFF` — macro spelling, for call sites that keep the C shape.
#[macro_export]
macro_rules! INTOFF {
    ($sh:expr) => {
        $crate::error::INTOFF($sh)
    };
}

/// `#define INTON` — macro spelling.
#[macro_export]
macro_rules! INTON {
    ($sh:expr) => {
        $crate::error::INTON($sh)
    };
}

/// `#define FORCEINTON` — macro spelling.
#[macro_export]
macro_rules! FORCEINTON {
    ($sh:expr) => {
        $crate::error::FORCEINTON($sh)
    };
}

/// `#define SAVEINT(v) ((v) = suppressint)`
#[macro_export]
macro_rules! SAVEINT {
    ($sh:expr, $v:expr) => {
        $v = $sh.interrupt_suppression
    };
}

/// ```c
/// #define RESTOREINT(v) \
///	({ barrier(); if ((suppressint = (v)) == 0 && intpending) onint(sh); 0; })
/// ```
#[macro_export]
macro_rules! RESTOREINT {
    ($sh:expr, $v:expr) => {{
        /* The `if (... && intpending) onint(sh)` is gone with the one in
         * `INTON`; see there. */
        $crate::error::barrier();
        $sh.interrupt_suppression = $v;
        0
    }};
}

/*
 * Called from trap.c when a SIGINT is received.  (If the user specifies
 * that SIGINT is to be trapped or ignored using the trap builtin, then
 * this routine is not called.)  Suppressint is nonzero when interrupts
 * are held using the INTOFF macro.  (The test for iflag is just
 * defensive programming.)
 */

/// Take delivery of a pending interrupt, as a value.
///
/// The C raises `EXINT` from here and never returns. This returns the
/// interrupt instead, and the change of shape is the whole of step F:
/// `onsig` no longer calls it from inside the signal handler, and `INTON`
/// no longer calls it when the counter reaches zero. It is called only
/// from a *poll site* — a place the shell reached on its own and that can
/// return a `Result`.
///
/// Clearing `intpending` is the delivery. After this returns, the
/// interrupt has been taken and the next poll site must not take it
/// again; that is why the poll sites call this rather than reading the
/// flag and building an `Error` themselves.
///
/// It still does not always return. When the shell is not an interactive
/// root shell it restores `SIG_DFL` and re-raises, so the process dies of
/// the signal, which is what a shell must do to report the right status
/// to its parent. That is a terminating operation in libc, not a
/// non-local jump, and `panic = "abort"` cannot break it.
/// `docs/api-design.md` §3.4 wants this half in `nsh-cli` eventually; it
/// is a frontend boundary question and not this node's.
// [spec:dash:def:error.onint-fn]
// [spec:dash:sem:error.onint-fn]
// [spec:nsh:def:idiom.shell-options]
pub fn onint(sh: &crate::context::Shell) -> Error {
    crate::siginbox::signals().set_interrupt_pending(false);
    crate::system::sigclearmask();
    /* `#define rootshell (!shlvl)` (main.h); `#define iflag optlist[3]`. */
    let rootshell: bool = sh.shell_level == 0;
    let interactive = sh.options.enabled(crate::options::ShellOption::Interactive);
    if !(rootshell && interactive) {
        nsh_platform::terminate_with_interrupt();
    }
    /* `exitstatus = SIGINT + 128` was here; `Error::status()` answers
     * exactly that for `Interrupted`, so the value already carries it. */
    Error::Interrupted {
        signal: crate::status::Signal::from(nsh_platform::interrupt_signal()),
    }
}

/* `exvwarning2` is not a separate function here. In the C it exists only
 * to accept the `va_list` that `sh_warnx`'s varargs collected, and the
 * two have the same body otherwise. A message is a `&[u8]` now, so there
 * is nothing for the inner one to accept and `sh_warnx` below carries
 * both rules. */

/// A shell diagnostic, as a value ([dec:nsh:errors-are-values]).
///
/// Every one of these is also *written* to the shell's stderr at the point
/// it happened, in dash's bytes and dash's order — see [`report`], which is
/// the only constructor that should reach a caller. That is not redundancy:
/// `tests/harness/dscase.sh` merges stdout and stderr and compares the
/// result, so where a diagnostic lands in the stream is under test in every
/// corpus case, and a design that returned the text instead of writing it
/// would emit every diagnostic at the end of the run.
///
/// Control flow is deliberately not here. `exit`, `return`, `break`,
/// `continue` and the `set -e` abort are not errors and must not sit in the
/// `Err` position; they keep the exception codes for now and become `Flow`
/// in the `Ok` position later in this node.
///
/// One variant so far. `docs/api-design.md` §3.4 names ten and says why the
/// conversion starts with `Other` alone: every raise site can be rewritten
/// mechanically and the interesting ones promoted afterwards, instead of
/// needing the final taxonomy before the first commit.
/// Constructing one of these is not the same as raising it, and dropping it
/// on the floor is how a diagnostic gets written while the shell carries on
/// past a failure it should have abandoned. That is a silent wrong answer
/// rather than a crash, which is the failure mode
/// `docs/errors-are-values.md` §6 names as the dangerous one for this whole
/// conversion -- and it happened: `redir::sh_open_fail` stopped diverging
/// when it started returning a value, and two `goto ecreate` sites fell
/// through instead of stopping, printing the diagnostic twice and then
/// redirecting to a descriptor that was never opened. The corpus caught it.
/// This attribute is what makes the compiler catch the next one.
#[must_use = "an Error that is built and not returned reports a failure the shell then ignores"]
#[derive(Debug)]
#[non_exhaustive]
pub enum Error {
    /// The user interrupted the shell.
    ///
    /// Kept apart from every other failure because a host has to tell
    /// "your script failed" from "the user pressed ^C"
    /// (`docs/api-design.md` §3.4), and because the frames that swallow a
    /// diagnostic must not swallow this one: `evalcommand` reports a
    /// non-special built-in's error and carries on, and an interrupt
    /// arriving the same way has to keep going.
    ///
    /// It is the first variant promoted out of `Other`, which is what
    /// §3.4 said the taxonomy would do: start with `Other` so every raise
    /// site converts mechanically, promote the interesting ones after.
    Interrupted {
        /// The signal that arrived. Always `SIGINT` today — it is the
        /// only one `onsig` delivers this way.
        ///
        /// A [`Signal`](crate::status::Signal) since `public-api` step 5.
        /// `docs/api-design.md` §3.4 recorded that the variant would
        /// "carry a `c_int` until then", and this is then.
        signal: crate::status::Signal,
    },
    /// A command input source failed before it reached end-of-file.
    // [spec:posix:req:exit.unrecoverable-read-error]
    UnrecoverableRead {
        /// `errlinno` as it stood when the read failed.
        line: c_int,
        /// The already-rendered read diagnostic.
        message: BString,
    },
    /// A parameter-expansion failure that aborts a non-interactive shell
    /// but only abandons the affected command in an interactive shell.
    ///
    /// This cannot be represented by `Other`: redirection frames are
    /// deliberately allowed to turn an ordinary diagnostic into a command
    /// status, while POSIX expansion errors have to cross those same frames.
    // [spec:nsh:req:compat.smoosh.error-contracts]
    Expansion {
        /// `errlinno` as it stood when expansion failed.
        line: c_int,
        /// The diagnostic without a shell or command prefix.
        message: BString,
    },
    /// A diagnostic with no more specific variant.
    Other {
        /// `errlinno` as it stood when the diagnostic was produced.
        line: c_int,
        /// The status the shell takes from it.
        status: crate::status::ExitStatus,
        /// dash's text, without the `sh: N: cd: ` prefix.
        message: BString,
    },
}

impl Error {
    /// A diagnostic with no more specific variant, at the current line and
    /// the status the site takes.
    ///
    /// The status is a parameter rather than a read of `exitstatus`. It
    /// used to be the latter, with the comment "`sh_error` sets
    /// `exitstatus` to 2 before it reports, so reading it here captures
    /// what each site meant" -- which made the *value* depend on a global
    /// the value exists to carry. Now the raise says what it took and the
    /// frame that catches it is the frame that writes it. Same shape as
    /// [`Error::reported`], which always took it this way.
    pub fn other(line: c_int, status: impl Into<crate::status::ExitStatus>, msg: &[u8]) -> Error {
        Error::Other {
            line,
            status: status.into(),
            message: BString::from(msg),
        }
    }

    /// A failure whose diagnostic has **already been written**, with no
    /// text of its own.
    ///
    /// The C raises `EXERROR` with no message where the thing that failed
    /// wrote its own diagnostic and then returned normally — `evalcommand`'s
    /// `bail:` after a `CMDUNKNOWN`, where `find_command` reported "not
    /// found" and came back. There is no value to carry there and dash
    /// carries none; this is that, as a value, so the frame can `return
    /// Err` instead of raising.
    ///
    /// Nothing writes it: [`report`] runs at construction and this is not
    /// constructed through it. An empty message is therefore never
    /// rendered, and it must stay that way — a caller that reports one of
    /// these would emit a bare prefix and a newline dash does not.
    pub fn reported(line: c_int, status: impl Into<crate::status::ExitStatus>) -> Error {
        Error::Other {
            line,
            status: status.into(),
            message: BString::default(),
        }
    }

    /// Build a command-input read error, retaining the special-builtin
    /// treatment required for the file operand of `.`.
    pub fn unrecoverable_read(line: c_int, msg: &[u8], dot_operand: bool) -> Error {
        if dot_operand {
            Error::other(line, 2, msg)
        } else {
            Error::UnrecoverableRead {
                line,
                message: BString::from(msg),
            }
        }
    }

    /// Is this the user's interrupt rather than a diagnostic?
    ///
    /// The question every frame that swallows an error has to ask. There
    /// are four of them — `evalcommand`'s built-in arm, `redirectsafe`,
    /// `expandstr` and `exitshell`'s EXIT trap — and each used to ask it
    /// of `error::exception` after a longjmp.
    pub fn is_interrupt(&self) -> bool {
        matches!(self, Error::Interrupted { .. })
    }

    /// Whether POSIX requires this error to end even an interactive shell.
    pub fn is_unrecoverable_read(&self) -> bool {
        matches!(self, Error::UnrecoverableRead { .. })
    }

    /// Whether this failure has the interactive/non-interactive expansion
    /// consequences specified by the shell language.
    pub fn is_expansion(&self) -> bool {
        matches!(self, Error::Expansion { .. })
    }

    /// The exit status the shell takes from this error.
    pub fn status(&self) -> crate::status::ExitStatus {
        match self {
            /* `onint` sets `exitstatus` to this before it returns, as the
             * C does before it raises. */
            Error::Interrupted { signal } => signal.as_status(),
            // [spec:posix:req:sh.exit-status-values]
            Error::UnrecoverableRead { .. } => crate::status::ExitStatus::UNRECOVERABLE_READ,
            Error::Expansion { .. } => crate::status::ExitStatus::FAILURE,
            Error::Other { status, .. } => *status,
        }
    }

    /// dash's text for this error, byte for byte, **without** the
    /// `sh: 1: cd: ` prefix.
    ///
    /// The prefix is `$0`, `eval.errlinno` and the running command's
    /// name, which are shell state and not error state, so an `Error` on
    /// its own cannot render them. `Shell::sh_warnx` adds them when it
    /// writes.
    pub fn message(&self) -> &BStr {
        match self {
            /* dash prints nothing for an interrupt. `main`'s handler
             * writes a bare newline and that is the whole of it. */
            Error::Interrupted { .. } => BStr::new(b""),
            Error::UnrecoverableRead { message, .. }
            | Error::Expansion { message, .. }
            | Error::Other { message, .. } => message.as_bstr(),
        }
    }

    /// The line the error was reported at.
    pub fn line(&self) -> c_int {
        match self {
            /* No line: an interrupt did not happen *at* a line the way a
             * diagnostic did, and reading `eval.errlinno` here would report
             * whichever line last failed. */
            Error::Interrupted { .. } => 0,
            Error::UnrecoverableRead { line, .. }
            | Error::Expansion { line, .. }
            | Error::Other { line, .. } => *line,
        }
    }
}

/// The diagnostic spine, threaded.
///
/// `move-state` transferred `arg0`, `errlinno` and `commandname` here as a
/// choice between two options — thread the spine, or give the diagnostic
/// its own sink so the spine never needs a receiver — and recommended the
/// second in order to avoid the first. **They were never alternatives.**
///
/// `docs/api-design.md` §3.2 already fixes `report` as a `&mut self`
/// method, and it requires the write to happen *at the raise point*: the
/// interleaving of diagnostics with command output is what `dscase.sh`'s
/// `2>&1` merge puts under test in every corpus case. A sink that
/// assembled the prefix elsewhere would have to defer the write to
/// wherever it is polled, which is the one thing §3.2 forbids. And the
/// write needs `&mut` on the stderr `Output`, so even a sink owning the
/// prefix would not free `report` from a receiver. What the sink genuinely
/// replaces is not the receiver — it is the three *statics*, which become
/// fields of the shell that reports.
///
/// The threading is also much cheaper than it was costed, because
/// `thread-context` had already put a `&mut Shell` on every execution
/// path: of the 66 call sites outside this module, 45 already had a
/// receiver in scope and the 21 that did not were leaf helpers whose
/// callers did.
impl crate::context::Shell {
    /// Write a diagnostic where dash writes it, and hand it back as a
    /// value.
    ///
    /// This is [`exverror`] with the raise removed, and it is the funnel
    /// every diagnostic goes through: the bytes on the stream are rendered
    /// from the same `Error` that is returned, so the two cannot drift.
    ///
    /// Two details of dash's write are load-bearing and are preserved by
    /// doing nothing more than the C does. `errout` is unbuffered, so the
    /// message is three raw `write(2)`s and needs no flush of its own; and
    /// `flushall()` runs *after* the message, so a built-in that filled
    /// the stdout buffer and then failed produces its diagnostic before
    /// its own output in the merged stream. Both are pinned by the corpus.
    // [spec:posix:req:xcu.defaults.stderr-diagnostics-only]
    // [spec:posix:req:xcu.stderr.terminal-background]
    // [spec:posix:req:xcu.stderr.message-language]
    // [spec:posix:req:xcu.stderr.env-independence]
    // [spec:posix:req:xcu.errors.failure-reasons-unspecified]
    // [spec:posix:req:xcu.errors.operand-failure-continues]
    // [spec:posix:req:xcu.errors.option-failure]
    // [spec:posix:req:xcu.errors.unrecoverable-exit-status]
    // [spec:posix:req:xcu.errors.diagnostic-message-required]
    pub fn report(&mut self, e: Error) -> Error {
        self.sh_warnx(e.message());

        self.io.flushall();
        e
    }

    /// `sh_error`'s value half: take the status dash takes, write the
    /// diagnostic where dash writes it, and **return** the error rather
    /// than raising it.
    ///
    /// This is what a converted raise site calls —
    /// `return Err(sh.sh_error_value(&msg))` — and it is the same three
    /// writes in the same order as the diverging form, because both are
    /// this function. When the last caller of `sh_error` is gone this one
    /// takes its name.
    pub fn sh_error_value(&mut self, msg: &[u8]) -> Error {
        /* `exitstatus = 2` was here. It is the returned value's `status`
         * instead: the error carries what it took and the frame that
         * catches it writes it. That is why the *status* needed no
         * receiver even before this method had one. */
        let e = Error::other(self.eval.errlinno, 2, msg);
        self.report(e)
    }

    /// Report a parameter-expansion error without the implementation's
    /// shell/line prefix and retain its distinct control-flow class.
    // [spec:nsh:req:compat.smoosh.error-contracts]
    pub fn expansion_error_value(&mut self, msg: &[u8]) -> Error {
        let e = Error::Expansion {
            line: self.eval.errlinno,
            message: BString::from(msg),
        };
        let _ = self.io.stderr().write_all(msg);
        let _ = self.io.stderr().write_all(b"\n");
        self.io.flushall();
        e
    }

    /// Report `command: message` and return a diagnostic already written.
    /// Builtin-defined failures use this form rather than the parser's
    /// `$0: line: command:` diagnostic spine.
    // [spec:nsh:req:compat.smoosh.error-contracts]
    pub fn builtin_error_value(
        &mut self,
        status: impl Into<crate::status::ExitStatus>,
        msg: &[u8],
    ) -> Error {
        let name = self
            .eval
            .commandname
            .clone()
            .unwrap_or_else(|| BString::from(&b"sh"[..]));
        let errors = self.io.stderr();
        let _ = errors.write_all(&name);
        let _ = errors.write_all(b": ");
        let _ = errors.write_all(msg);
        let _ = errors.write_all(b"\n");
        self.io.flushall();
        Error::reported(self.eval.errlinno, status)
    }

    /// Write `$0: command: message` for an output failure detected after a
    /// builtin returns. Unlike `sh_warnx`, this contract has no line field.
    // [spec:nsh:req:compat.smoosh.error-contracts]
    pub fn command_warnx(&mut self, msg: &[u8]) {
        let shell_name = self
            .options
            .invocation_name
            .as_ref()
            .map(|name| BStr::new(&name[..name.len() - 1]))
            .unwrap_or(BStr::new(b"sh"))
            .to_owned();
        let command_name = self.eval.commandname.clone();
        let errors = self.io.stderr();
        let _ = errors.write_all(&shell_name);
        let _ = errors.write_all(b": ");
        if let Some(command_name) = command_name {
            let _ = errors.write_all(&command_name);
            let _ = errors.write_all(b": ");
        }
        let _ = errors.write_all(msg);
        let _ = errors.write_all(b"\n");
    }

    /*
     * error/warning routines for external builtins
     */

    // [spec:dash:def:error.sh-warnx-fn]
    // [spec:dash:sem:error.sh-warnx-fn]
    /// Write a diagnostic with the `sh: 17: cd: ` prefix the shell puts on
    /// one, to the shell's own unbuffered stderr.
    // [spec:dash:def:error.exvwarning2-fn]
    // [spec:dash:sem:error.exvwarning2-fn]
    pub fn sh_warnx(&mut self, msg: &[u8]) {
        let name = self.options.arg0().unwrap_or(BStr::new(b"sh"));

        /* The prefix is assembled here from the reporting shell. */
        let mut prefix = Vec::new();
        prefix.extend_from_slice(name);
        prefix.extend_from_slice(b": ");
        let line = self.eval.errlinno;
        write!(&mut prefix, "{line}").expect("writing to a Vec cannot fail");
        prefix.extend_from_slice(b": ");
        if let Some(name) = &self.eval.commandname {
            prefix.extend_from_slice(name);
            prefix.extend_from_slice(b": ");
        }

        /* stderr is unbuffered. Keep the C's three output operations
         * visible: prefix, complete message body, then newline. */
        let errs = self.io.get(crate::output::Dest::Stderr);
        let _ = errs.write_all(&prefix);
        let _ = errs.write_all(msg);
        let _ = errs.write_all(b"\n");
    }
}

/*
 * Return a string describing an error.  The returned string may be a
 * pointer to a static buffer that will be overwritten on the next call.
 * Action describes the operation that got the error.
 */

// [spec:dash:def:error.errmsg-fn]
// [spec:dash:sem:error.errmsg-fn]
// [spec:nsh:req:idiom.platform-errors]
pub fn errmsg(
    locale: &nsh_platform::Locale,
    error: &std::io::Error,
    action: c_int,
) -> bstr::BString {
    if !nsh_platform::is_path_error(error, nsh_platform::PathErrorKind::NotFound) {
        return bstr::BString::from(locale.error_message(error));
    }

    if action & E_OPEN != 0 {
        bstr::BString::from("No such file")
    } else if action & E_CREAT != 0 {
        bstr::BString::from("Directory nonexistent")
    } else {
        bstr::BString::from("not found")
    }
}

/*
 * `#ifdef REALLY_SMALL` — out-of-line body of INTON.  REALLY_SMALL is
 * not defined in the shipped build, so this is never called; it is kept
 * so the symbol has a home and stays in step with `INTON` above.
 */
// [spec:dash:def:error.inton-fn]
// [spec:dash:sem:error.inton-fn]
pub fn __inton(sh: &mut crate::context::Shell) {
    /* In step with `INTON` above, including the `onint(sh)` it no longer
     * makes. */
    sh.interrupt_suppression -= 1;
}

/* There is no setjmp/longjmp here, no stand-in for one, and no FFI
 * declaration of either — and now no `catch_unwind` and no `panic_any`
 * either. The last of it went with `errors-are-values`.
 *
 * The port never used libc's `longjmp`: a `jmploc` armed by
 * `eval::setjmp_catch` was a `catch_unwind`, not a real jump buffer, and
 * handing one to `longjmp` is undefined and in practice segfaulted. That
 * was a real bug here on every fork and exit path. Reintroducing a shim
 * would make it easy to recreate, and there is nothing left that would
 * want one: a failure is a value, and it is returned.
 *
 * `panic = "abort"` is therefore sound for this crate, which is the
 * consequence `[dec:nsh:errors-are-values]` exists to deliver. */

#[cfg(test)]
mod tests {
    use super::*;

    /* The funnel itself. `docs/errors-are-values.md` §5 lists the error
     * value first among what the differential harness cannot see: it
     * compares bytes on a stream, and the value never reaches it. These
     * assert the half the corpus cannot, and they are deliberately about
     * `sh_error_value` rather than `sh_error`, because the diverging form
     * is now defined as the value form plus a jump. */

    #[test]
    fn reported_error_carries_its_status() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        let e = sh.sh_error_value(b"a diagnostic");

        /* The value carries what the site took, so propagation
         * through any number of `?` cannot lose it. */
        assert_eq!(e.status(), crate::status::ExitStatus::ERROR);
        assert_eq!(e.message().to_vec(), b"a diagnostic".to_vec());

        /* Nothing here asserts that the raise leaves `$?` alone.
         * The signature says it: `sh_error_value` takes no receiver
         * and `$?` is a field of one, so there is no shell in scope
         * for it to write and no way for a test to observe
         * otherwise. */
    }

    // [spec:posix:req:exit.unrecoverable-read-error/test]
    // [spec:posix:req:exit.shell-error-consequences/test]
    // [spec:posix:req:sh.exit-status-values/test]
    #[test]
    fn read_error_classifies_dot_operand() {
        let input = Error::unrecoverable_read(7, b"read failed", false);
        assert!(input.is_unrecoverable_read());
        assert_eq!(input.line(), 7);
        assert_eq!(
            input.status(),
            crate::status::ExitStatus::UNRECOVERABLE_READ
        );
        assert_eq!(input.message(), BStr::new(b"read failed"));

        let dot = Error::unrecoverable_read(8, b"dot read failed", true);
        assert!(!dot.is_unrecoverable_read());
        assert_eq!(dot.line(), 8);
        assert_eq!(dot.status(), crate::status::ExitStatus::ERROR);
        assert_eq!(dot.message(), BStr::new(b"dot read failed"));
    }

    #[test]
    fn message_drops_the_prefix() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        sh.eval.errlinno = 17;
        let e = Error::other(sh.eval.errlinno, 2, b"cd: bad directory");
        let e = sh.report(e);

        /* The `sh: 17: ` prefix is `arg0`, `errlinno` and the running
         * command's name -- shell state, not error state -- so
         * `sh_warnx` adds it on the way out and the value does not
         * carry it. */
        assert_eq!(e.message().to_vec(), b"cd: bad directory".to_vec());
        assert_eq!(e.line(), 17);
    }

    #[test]
    fn exend_keeps_its_own_status() {
        let _g = crate::testutil::lock();
        /* `shellexec` reports its text and takes 127 or 126, then
         * raises EXEND. The status travels with the value even though
         * the code that goes with it does not. */
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        let e = Error::other(sh.eval.errlinno, 127, b"nosuchcmd: not found");
        let e = sh.report(e);

        assert_eq!(e.status(), crate::status::ExitStatus::NOT_FOUND);
    }

    /// Arrange for `onint` to be able to *return*.
    ///
    /// It restores `SIG_DFL` and re-raises unless the shell is an
    /// interactive root shell, which in a test process means the test
    /// dies of SIGINT. That branch is dash's and is deliberate; these
    /// cases are about the other one.
    fn as_interactive_root(sh: &mut crate::context::Shell) {
        sh.options
            .set(crate::options::ShellOption::Interactive, true);
        /* Copied out: a shared reference to a mutable static is what the
         * lint forbids, and `assert_eq!` takes one. */
        let lvl = sh.shell_level;
        assert_eq!(lvl, 0, "a test process is a root shell");
    }

    /// An interrupt is a value, it knows it is one, and it carries dash's
    /// status.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn an_interrupt_is_a_value() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        as_interactive_root(sh);
        CLEAR_PENDING_INT();

        let e = onint(sh);

        assert!(e.is_interrupt());
        assert_eq!(
            e.status(),
            crate::status::Signal::from(nsh_platform::interrupt_signal()).as_status()
        );
        /* `onint` used to write this to `exitstatus` as well. It does
         * not any more -- and it could not: it takes `&Shell`, a
         * shared receiver, so the type says it reads the shell and
         * does not write it. `Error::status()` answers `signal + 128`
         * for `Interrupted`, and the frame that catches it writes. */
        assert_eq!(
            sh.status,
            crate::status::ExitStatus::SUCCESS,
            "the raise path writes no shell state"
        );
        assert!(e.message().is_empty(), "dash prints nothing for a ^C");
    }

    /// `poll_interrupt` takes delivery once and only once: `onint` clears
    /// the flag as it hands the value over, so a second poll finds
    /// nothing. A frame that drops the value has lost the user's ^C,
    /// which is what `rearm_interrupt` exists for.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn delivery_happens_once() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        as_interactive_root(sh);
        sh.interrupt_suppression = 0;
        crate::siginbox::signals().set_interrupt_pending(true);

        assert!(
            poll_interrupt(sh).is_some(),
            "one pending interrupt, one delivery"
        );
        assert!(poll_interrupt(sh).is_none(), "and not a second time");
    }

    /// **The INTOFF discipline, which the polling must not break.** An
    /// interrupt that arrives inside an `INTOFF` bracket is pending but
    /// not *due*, and no poll site may take it there -- the bracket is
    /// what makes the mutation inside it atomic against a signal. This is
    /// the one property that distinguishes "delivery moved to a poll
    /// site" from "delivery moved anywhere at all".
    // [spec:dash:sem:error.inton-fn/test]
    #[test]
    fn intoff_still_holds_it_off() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        as_interactive_root(sh);
        sh.interrupt_suppression = 0;
        crate::siginbox::signals().set_interrupt_pending(true);

        INTOFF(sh);
        assert!(poll_interrupt(sh).is_none(), "suppressed: not due");
        /* And `INTON` does not deliver it either -- that is the
         * divergence, and it is why the counter reaching zero is no
         * longer a delivery point. */
        INTON(sh);
        assert_eq!(int_pending(), 1, "still pending, waiting for a poll site");
        assert!(
            poll_interrupt(sh).is_some(),
            "and due again once unsuppressed"
        );
    }

    /// A frame that cannot carry the interrupt out puts it back rather
    /// than losing it.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn a_rearmed_interrupt_is_taken_later() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        as_interactive_root(sh);
        sh.interrupt_suppression = 0;
        CLEAR_PENDING_INT();

        rearm_interrupt(Error::Interrupted {
            signal: crate::status::Signal::from(nsh_platform::interrupt_signal()),
        });
        assert_eq!(int_pending(), 1);
        assert!(poll_interrupt(sh).is_some(), "the next poll site takes it");
    }
}
