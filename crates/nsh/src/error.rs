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

/// Operation whose failure is being described.
// [spec:nsh:req:idiom.no-abi-scalars-core]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum Operation {
    Open,
    Create,
    Execute,
}

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

/// Prevent compiler reordering across an interrupt-deferral transition.
#[inline(always)]
pub fn barrier() {
    compiler_fence(Ordering::SeqCst);
}

/// Nesting depth of shell interrupt deferral.
// [spec:nsh:sem:idiom.interrupt-deferral]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InterruptDeferral {
    depth: usize,
}

impl InterruptDeferral {
    pub(crate) const fn new() -> Self {
        Self { depth: 0 }
    }

    /// Mutate one explicitly supplied subsystem while delivery is deferred.
    /// The callback cannot reach any sibling shell state.
    pub(crate) fn run_with<S, T>(&mut self, state: &mut S, body: impl FnOnce(&mut S) -> T) -> T {
        let previous = self.depth;
        self.depth = previous + 1;
        barrier();
        let outcome = body(state);
        barrier();
        self.depth = previous;
        outcome
    }
}

/// Run one operation while interrupt delivery is deferred.
///
/// The previous depth is restored after any ordinary return, including an
/// error or evaluator control value. Reaching depth zero does not deliver a
/// pending interrupt; delivery remains the responsibility of
/// [`poll_interrupt`] at a documented polling boundary.
pub(crate) fn with_interrupts_deferred<T>(
    shell: &mut crate::context::Shell,
    body: impl FnOnce(&mut crate::context::Shell) -> T,
) -> T {
    let previous = shell.interrupt_deferral.depth;
    shell.interrupt_deferral.depth = previous + 1;
    barrier();
    let outcome = body(shell);
    barrier();
    shell.interrupt_deferral.depth = previous;
    outcome
}

/// Reset legacy deferral at a top-level recovery boundary.
pub(crate) fn clear_interrupt_deferral(deferral: &mut InterruptDeferral) {
    barrier();
    deferral.depth = 0;
}

#[cfg(test)]
#[inline(always)]
pub fn clear_pending_interrupt() {
    crate::signal_inbox::signals().set_interrupt_pending(false);
}

/// Take delivery of a pending interrupt, if one is due.
///
/// The question every poll site asks, in one place so that all of them
/// ask it the same way. "Due" is *pending* and *not suppressed*: an
/// An active deferral scope still holds the interrupt off, exactly as the
/// translated counter did, because the scope makes the mutation inside it
/// atomic against delivery.
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
#[derive(Clone, Copy)]
pub(crate) struct InterruptContext {
    deferred: bool,
    interactive_root: bool,
}

impl crate::context::Shell {
    pub(crate) fn interrupt_context(&self) -> InterruptContext {
        InterruptContext {
            deferred: self.interrupt_deferral.depth != 0,
            interactive_root: self.shell_level == 0
                && self
                    .options
                    .enabled(crate::options::ShellOption::Interactive),
        }
    }
}

#[inline]
pub(crate) fn poll_interrupt(context: InterruptContext) -> Option<Error> {
    if !context.deferred && interrupt_pending() {
        Some(interrupt_error(context))
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
pub fn rearm_interrupt(error: Error) {
    debug_assert!(
        error.is_interrupt(),
        "only an interrupt may be put back; a diagnostic has already been written"
    );
    drop(error);
    crate::signal_inbox::signals().set_interrupt_pending(true);
}

#[inline(always)]
pub fn interrupt_pending() -> bool {
    crate::signal_inbox::signals().interrupt_pending()
}

/*
 * Called from trap.c when a SIGINT is received.  (If the user specifies
 * that SIGINT is to be trapped or ignored using the trap builtin, then
 * this routine is not called.)  Suppressint is nonzero when interrupts
 * are held using the interrupt-deferral state. (The test for iflag is just
 * defensive programming.)
 */

/// Take delivery of a pending interrupt, as a value.
///
/// The C raises `EXINT` from here and never returns. This returns the
/// interrupt instead, and the change of shape is the whole of step F:
/// `onsig` no longer calls it from inside the signal handler, and leaving a
/// deferral scope is not itself delivery. It is called only from a *poll
/// site* — a place the shell reached on its own and that can return a
/// `Result`.
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
// [spec:dash:sem:error.onint-fn]
// [spec:nsh:def:idiom.shell-options]
// [spec:dash:sem:system.sigclearmask-fn]
fn interrupt_error(context: InterruptContext) -> Error {
    crate::signal_inbox::signals().set_interrupt_pending(false);
    if nsh_platform::unblock_all_signals().is_err() {
        // Interrupt delivery still has to proceed when mask restoration fails.
    }
    if !context.interactive_root {
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
        /// "carry a plain integer until then", and this is then.
        signal: crate::status::Signal,
    },
    /// A command input source failed before it reached end-of-file.
    // [spec:posix:req:exit.unrecoverable-read-error]
    UnrecoverableRead {
        /// `errlinno` as it stood when the read failed.
        line: i32,
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
        line: i32,
        /// The status the shell takes from it, which is the dialect's:
        /// 2 where XCU 2.8.1 ends a non-interactive shell and dash
        /// answers 2, 1 where Bash mode reports and reads on. A field
        /// rather than a constant because the frame that finally takes
        /// it is several `?` returns away from the dialect that wrote
        /// the diagnostic.
        // [spec:nsh:req:compat.bash.error-boundary]
        status: crate::status::ExitStatus,
        /// The diagnostic without a shell or command prefix.
        message: BString,
    },
    /// A failure the Bash dialect reports and then recovers from.
    ///
    /// Bash reports a read-only assignment, a bad substitution, a bad
    /// array subscript and an arithmetic error, abandons the input record
    /// the failure was raised in, and reads the next one --
    /// `exp_jump_to_top_level(DISCARD)`. Status is 1 and the shell lives.
    ///
    /// The POSIX dialect never builds one. There the same failures are
    /// [`Diagnostics::shell_error`], which takes status 2 and ends a
    /// non-interactive shell as XCU 2.8.1 requires, and that is why the
    /// frames which recover from this variant need no dialect test of
    /// their own: the variant cannot exist outside Bash mode.
    // [spec:nsh:req:compat.bash.error-boundary]
    Abandoned {
        /// `errlinno` as it stood when the failure was reported.
        line: i32,
        /// The diagnostic, already written, or empty where the site that
        /// found the fault wrote its own.
        message: BString,
        /// Whether the failure was computing the value an assignment was
        /// about to store, which decides what a built-in frame does with
        /// it. Bash reaches those two outcomes by two mechanisms rather
        /// than one: a declaration utility that cannot evaluate the value
        /// it was handed calls `jump_to_top_level(DISCARD)` from inside
        /// itself, so the record goes with it, while a utility that
        /// merely *refuses* an operand -- a read-only name, a bad
        /// identifier, an option it does not have -- returns
        /// `EXECUTION_FAILURE` and the list runs on. Both are this
        /// variant here, so the frame that catches a built-in's failure
        /// needs the distinction carried to it rather than inferred from
        /// the variant or the built-in's identity, neither of which
        /// separates `declare -i x=1+` from `local x=1` outside a
        /// function.
        // [spec:nsh:req:compat.bash.error-boundary]
        from_assignment: bool,
        /// Whether an arithmetic evaluation raised it, which decides
        /// whether `errexit` can reach it: the reference resumes at the
        /// next record after one of these even under `set -e`, and ends
        /// the shell after a refusal. Both classes abandon their record,
        /// so the abandonment cannot separate them and the raise has to
        /// say which it was.
        ///
        /// `(( 1+ ))` and `let x=1+` are neither. The arithmetic is the
        /// command there, so its failure becomes that command's status
        /// instead of an abandonment and ordinary `errexit` acts on it;
        /// they never reach this variant and so need no arm.
        ///
        /// Set by `crate::arithmetic`, the only frame that knows the
        /// failure was arithmetic, and read by
        /// [`crate::evaluation::evaluate_record`].
        /// `crates/nsh-cli/tests/bash_errexit_over_an_assignment_error.rs`
        /// measures both classes through both shells.
        // [spec:nsh:req:compat.bash.error-boundary]
        from_arithmetic: bool,
        /// Whether only the shell's own outermost input loop recovers it,
        /// every nested frame passing it on.
        ///
        /// True for the arithmetic the variable machinery evaluates itself
        /// -- a declaration's integer value and an indexed subscript --
        /// and false for the arithmetic that reaches it through an
        /// expansion, `$(( ))` and a slice bound. The reference separates
        /// the two by where the failure is raised rather than by what
        /// failed: `-c 'declare -i x=1+ ; echo A'` runs nothing, while
        /// `-c 'x=$((1+)); echo A'` abandons that record and reads on.
        ///
        /// So an `eval`, a `.` script and a `-c` string are not recovery
        /// points for this class, and a subshell still contains it because
        /// it contains everything.
        /// `crates/nsh-cli/tests/bash_assignment_error_frames.rs` measures
        /// each frame through both shells.
        // [spec:nsh:req:compat.bash.error-boundary]
        unwinds_to_the_input_loop: bool,
    },
    /// A diagnostic with no more specific variant.
    Other {
        /// `errlinno` as it stood when the diagnostic was produced.
        line: i32,
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
    pub fn other(line: i32, status: impl Into<crate::status::ExitStatus>, msg: &[u8]) -> Error {
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
    pub fn reported(line: i32, status: impl Into<crate::status::ExitStatus>) -> Error {
        Error::Other {
            line,
            status: status.into(),
            message: BString::default(),
        }
    }

    /// A Bash-dialect failure whose diagnostic has **already been
    /// written**, with no text of its own.
    ///
    /// [`Error::reported`] with the Bash error boundary rather than the
    /// POSIX one. The one site that needs it is an assignment through a
    /// subscript that named no element: the subscript resolver reported
    /// the fault where it found it, because a *read* through the same
    /// subscript is reported and then carries on, and only the assignment
    /// has an error to raise afterwards.
    // [spec:nsh:req:compat.bash.error-boundary]
    pub fn abandoned(line: i32) -> Error {
        Error::Abandoned {
            line,
            message: BString::default(),
            from_assignment: false,
            from_arithmetic: false,
            unwinds_to_the_input_loop: false,
        }
    }

    /// Build a command-input read error, retaining the special-builtin
    /// treatment required for the file operand of `.`.
    pub fn unrecoverable_read(line: i32, msg: &[u8], dot_operand: bool) -> Error {
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

    /// Whether the Bash dialect recovers from this failure at the input
    /// record that raised it.
    ///
    /// The question the two frames that resume from one have to ask:
    /// [`crate::evaluation::evaluate_record`], which abandons the rest of
    /// the record and reads the next, and `evalcommand`'s special
    /// built-in arm, which must not escalate a failure the dialect has
    /// already decided is survivable.
    // [spec:nsh:req:compat.bash.error-boundary]
    pub fn is_abandoned(&self) -> bool {
        matches!(self, Error::Abandoned { .. })
    }

    /// The exit status the shell takes from this error.
    pub fn status(&self) -> crate::status::ExitStatus {
        match self {
            /* `onint` sets `exitstatus` to this before it returns, as the
             * C does before it raises. */
            Error::Interrupted { signal } => signal.as_status(),
            // [spec:posix:req:sh.exit-status-values]
            Error::UnrecoverableRead { .. } => crate::status::ExitStatus::UNRECOVERABLE_READ,
            Error::Expansion { status, .. } => *status,
            Error::Abandoned { .. } => crate::status::ExitStatus::FAILURE,
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
            | Error::Abandoned { message, .. }
            | Error::Other { message, .. } => message.as_bstr(),
        }
    }

    /// The line the error was reported at.
    pub fn line(&self) -> i32 {
        match self {
            /* No line: an interrupt did not happen *at* a line the way a
             * diagnostic did, and reading `eval.errlinno` here would report
             * whichever line last failed. */
            Error::Interrupted { .. } => 0,
            Error::UnrecoverableRead { line, .. }
            | Error::Expansion { line, .. }
            | Error::Abandoned { line, .. }
            | Error::Other { line, .. } => *line,
        }
    }
}

/// The narrow capability required to render and flush a shell diagnostic.
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
/// fields of the shell that reports. Borrowing only those fields prevents a
/// diagnostic helper from reaching unrelated parser, job, variable, or process
/// state just because its caller owns a [`crate::context::Shell`].
///
/// The threading is also much cheaper than it was costed, because
/// `thread-context` had already put a `&mut Shell` on every execution
/// path: of the 66 call sites outside this module, 45 already had a
/// receiver in scope and the 21 that did not were leaf helpers whose
/// callers did.
// [spec:nsh:req:idiom.narrow-shell-context]
pub(crate) struct Diagnostics<'a> {
    argument_zero: Option<&'a BStr>,
    invocation_name: Option<&'a BString>,
    command_name: Option<&'a BString>,
    line: i32,
    /// Which error boundary the shell reporting is under. A diagnostic is
    /// the point at which the two dialects part company, so the sink that
    /// writes one is the narrowest place the dialect can be read from.
    // [spec:nsh:req:compat.bash.error-boundary]
    dialect: crate::options::Dialect,
    io: &'a mut crate::output::ShellIo,
}

impl crate::context::Shell {
    pub(crate) fn diagnostics(&mut self) -> Diagnostics<'_> {
        Diagnostics {
            argument_zero: self.options.argument_zero(),
            invocation_name: self.options.invocation_name.as_ref(),
            command_name: self.evaluation.command_name.as_ref(),
            line: self.evaluation.diagnostic_line,
            dialect: self.options.dialect(),
            io: &mut self.io,
        }
    }
}

impl Diagnostics<'_> {
    /// A diagnostic write cannot report its own failure through the same
    /// broken stream. Observe the result here so callers never grow a hidden
    /// output-error channel or recursively attempt another diagnostic.
    fn write_diagnostic(&mut self, record: &[u8]) {
        if self.io.stderr().write_all(record).is_err() {
            // The diagnostic stream is the final reporting boundary.
        }
    }

    /// Preserve the original diagnostic when flushing earlier stdout fails.
    fn flush_after_diagnostic(&mut self) {
        if self.io.flush_all().is_err() {
            // The error already being reported takes precedence over flush.
        }
    }

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
    pub fn report(&mut self, error: Error) -> Error {
        self.shell_warning(error.message());
        self.flush_after_diagnostic();
        error
    }

    /// `sh_error`'s value half: take the status dash takes, write the
    /// diagnostic where dash writes it, and **return** the error rather
    /// than raising it.
    ///
    /// This is what a converted raise site calls —
    /// `return Err(sh.diagnostics().sh_error_value(&msg))` — and it is the same three
    /// writes in the same order as the diverging form, because both are
    /// this function. When the last caller of `sh_error` is gone this one
    /// takes its name.
    pub fn shell_error(&mut self, msg: &[u8]) -> Error {
        /* `exitstatus = 2` was here. It is the returned value's `status`
         * instead: the error carries what it took and the frame that
         * catches it writes it. That is why the *status* needed no
         * receiver even before this method had one. */
        let error = Error::other(self.line, 2, msg);
        self.report(error)
    }

    /// [`Self::shell_error`] where the dialect decides the boundary.
    ///
    /// The text, the prefix and the three writes are the same either way;
    /// what the dialect chooses is what happens *after* the write. In the
    /// POSIX dialect this is `sh_error` unchanged -- status 2, and a
    /// non-interactive shell ends, which is XCU 2.8.1 and what the
    /// conformance harness is built on. In Bash mode it is status 1 and
    /// the shell abandons the input record rather than itself, which is
    /// what a Bash script that assigns to a read-only name and keeps
    /// going expects. `set -o posix` leaves the dialect, so it restores
    /// the fatal boundary with no further test here.
    // [spec:nsh:req:compat.bash.error-boundary]
    pub fn dialect_error(&mut self, msg: &[u8]) -> Error {
        if self.dialect != crate::options::Dialect::Bash {
            return self.shell_error(msg);
        }
        let error = Error::Abandoned {
            line: self.line,
            message: BString::from(msg),
            from_assignment: false,
            from_arithmetic: false,
            unwinds_to_the_input_loop: false,
        };
        self.report(error)
    }

    /// [`Self::expansion_error_value`] where the dialect decides the
    /// boundary.
    ///
    /// The diagnostic is the same either way, and deliberately keeps the
    /// prefix-less shape `[spec:nsh:req:compat.smoosh.error-contracts]`
    /// asks for -- what the dialect chooses is only what happens after it
    /// is written. Bash mode abandons the record, which is what a script
    /// that expands `${!bad}` and reads on expects.
    ///
    /// This belongs only where the refusal is genuinely terminal for the
    /// expansion. `Error::Expansion` is not merely a status class: the
    /// expander raises and *catches* it to decide `${x+word}` under
    /// `set -u`, `${!z:=foo}` and slice arithmetic, so widening this to
    /// every parameter refusal replaces those recoveries with an
    /// abandoned record and loses three cases that pass today.
    // [spec:nsh:req:compat.bash.error-boundary]
    pub fn dialect_expansion_error(&mut self, msg: &[u8]) -> Error {
        if self.dialect != crate::options::Dialect::Bash {
            return self.expansion_error_value(msg);
        }
        let mut record = BString::from(msg);
        record.push(b'\n');
        self.write_diagnostic(&record);
        self.flush_after_diagnostic();
        Error::Abandoned {
            line: self.line,
            message: BString::new(Vec::new()),
            from_assignment: false,
            from_arithmetic: false,
            unwinds_to_the_input_loop: false,
        }
    }

    /// Report a parameter-expansion error without the implementation's
    /// shell/line prefix and retain its distinct control-flow class.
    // [spec:nsh:req:compat.smoosh.error-contracts]
    pub fn expansion_error_value(&mut self, msg: &[u8]) -> Error {
        let error = Error::Expansion {
            line: self.line,
            status: self.dialect.refusal_status(),
            message: BString::from(msg),
        };
        let mut record = BString::from(msg);
        record.push(b'\n');
        self.write_diagnostic(&record);
        self.flush_after_diagnostic();
        error
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
        self.builtin_warning(msg);
        Error::reported(self.line, status)
    }

    /// Write that same `command: message` where the failure is not a
    /// control-flow value.
    ///
    /// A built-in may report an operand and carry on to the next: Bash's
    /// `unset a x b` reports the read-only `x` and still unsets `b`, so
    /// its operand loop must be left standing rather than handed an
    /// [`Error`] to return.
    // [spec:nsh:req:compat.bash.error-boundary]
    pub fn builtin_warning(&mut self, msg: &[u8]) {
        let name = self
            .command_name
            .map_or(BStr::new(b"sh"), |name| name.as_bstr());
        let mut record = BString::from(name);
        record.extend_from_slice(b": ");
        record.extend_from_slice(msg);
        record.push(b'\n');
        self.write_diagnostic(&record);
        self.flush_after_diagnostic();
    }

    /// Write `$0: command: message` for an output failure detected after a
    /// builtin returns. Unlike `sh_warnx`, this contract has no line field.
    // [spec:nsh:req:compat.smoosh.error-contracts]
    pub fn command_warning(&mut self, msg: &[u8]) {
        let shell_name = self
            .invocation_name
            .map(|name| BStr::new(name.as_slice()))
            .unwrap_or(BStr::new(b"sh"));
        let mut record = BString::from(shell_name);
        record.extend_from_slice(b": ");
        if let Some(command_name) = self.command_name {
            record.extend_from_slice(command_name);
            record.extend_from_slice(b": ");
        }
        record.extend_from_slice(msg);
        record.push(b'\n');
        self.write_diagnostic(&record);
    }

    /*
     * error/warning routines for external builtins
     */

    // [spec:dash:sem:error.sh-warnx-fn]
    /// Write a diagnostic with the `sh: 17: cd: ` prefix the shell puts on
    /// one, to the shell's own unbuffered stderr.
    // [spec:dash:sem:error.exvwarning2-fn]
    pub fn shell_warning(&mut self, msg: &[u8]) {
        let name = self.argument_zero.unwrap_or(BStr::new(b"sh"));

        /* The prefix is assembled here from the reporting shell. */
        let mut prefix = Vec::new();
        prefix.extend_from_slice(name);
        prefix.extend_from_slice(b": ");
        let line = self.line;
        write!(&mut prefix, "{line}").expect("writing to a Vec cannot fail");
        prefix.extend_from_slice(b": ");
        if let Some(name) = self.command_name {
            prefix.extend_from_slice(name);
            prefix.extend_from_slice(b": ");
        }

        /* stderr is unbuffered. Keep the C's three output operations
         * visible: prefix, complete message body, then newline. */
        prefix.extend_from_slice(msg);
        prefix.push(b'\n');
        self.write_diagnostic(&prefix);
    }
}

/*
 * Return a string describing an error.  The returned string may be a
 * pointer to a static buffer that will be overwritten on the next call.
 * Action describes the operation that got the error.
 */

// [spec:dash:sem:error.errmsg-fn]
// [spec:nsh:req:idiom.platform-errors]
pub fn error_message(
    locale: &nsh_platform::Locale,
    error: &std::io::Error,
    operation: Operation,
) -> bstr::BString {
    if !nsh_platform::is_path_error(error, nsh_platform::PathErrorKind::NotFound) {
        return bstr::BString::from(locale.error_message(error));
    }

    match operation {
        Operation::Open => bstr::BString::from("No such file"),
        Operation::Create => bstr::BString::from("Directory nonexistent"),
        Operation::Execute => bstr::BString::from("not found"),
    }
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
        let _g = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        let error = shell.diagnostics().shell_error(b"a diagnostic");

        /* The value carries what the site took, so propagation
         * through any number of `?` cannot lose it. */
        assert_eq!(error.status(), crate::status::ExitStatus::ERROR);
        assert_eq!(error.message().to_vec(), b"a diagnostic".to_vec());

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
        let _g = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        shell.evaluation.diagnostic_line = 17;
        let error = Error::other(shell.evaluation.diagnostic_line, 2, b"cd: bad directory");
        let error = shell.diagnostics().report(error);

        /* The `sh: 17: ` prefix is `arg0`, `errlinno` and the running
         * command's name -- shell state, not error state -- so
         * `sh_warnx` adds it on the way out and the value does not
         * carry it. */
        assert_eq!(error.message().to_vec(), b"cd: bad directory".to_vec());
        assert_eq!(error.line(), 17);
    }

    #[test]
    fn exend_keeps_its_own_status() {
        let _g = crate::test_support::lock();
        /* `shellexec` reports its text and takes 127 or 126, then
         * raises EXEND. The status travels with the value even though
         * the code that goes with it does not. */
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        let error = Error::other(
            shell.evaluation.diagnostic_line,
            127,
            b"nosuchcmd: not found",
        );
        let error = shell.diagnostics().report(error);

        assert_eq!(error.status(), crate::status::ExitStatus::NOT_FOUND);
    }

    /// Arrange for `onint` to be able to *return*.
    ///
    /// It restores `SIG_DFL` and re-raises unless the shell is an
    /// interactive root shell, which in a test process means the test
    /// dies of SIGINT. That branch is dash's and is deliberate; these
    /// cases are about the other one.
    fn as_interactive_root(shell: &mut crate::context::Shell) {
        shell
            .options
            .set(crate::options::ShellOption::Interactive, true);
        /* Copied out: a shared reference to a mutable static is what the
         * lint forbids, and `assert_eq!` takes one. */
        let lvl = shell.shell_level;
        assert_eq!(lvl, 0, "a test process is a root shell");
    }

    /// An interrupt is a value, it knows it is one, and it carries dash's
    /// status.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn an_interrupt_is_a_value() {
        let _g = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        as_interactive_root(shell);
        clear_pending_interrupt();

        let error = interrupt_error(shell.interrupt_context());

        assert!(error.is_interrupt());
        assert_eq!(
            error.status(),
            crate::status::Signal::from(nsh_platform::interrupt_signal()).as_status()
        );
        /* `onint` used to write this to `exitstatus` as well. It does
         * not any more -- and it could not: it takes `&Shell`, a
         * shared receiver, so the type says it reads the shell and
         * does not write it. `Error::status()` answers `signal + 128`
         * for `Interrupted`, and the frame that catches it writes. */
        assert_eq!(
            shell.status,
            crate::status::ExitStatus::SUCCESS,
            "the raise path writes no shell state"
        );
        assert!(error.message().is_empty(), "dash prints nothing for a ^C");
    }

    /// `poll_interrupt` takes delivery once and only once: `onint` clears
    /// the flag as it hands the value over, so a second poll finds
    /// nothing. A frame that drops the value has lost the user's ^C,
    /// which is what `rearm_interrupt` exists for.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn delivery_happens_once() {
        let _g = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        as_interactive_root(shell);
        shell.interrupt_deferral.depth = 0;
        crate::signal_inbox::signals().set_interrupt_pending(true);

        assert!(
            poll_interrupt(shell.interrupt_context()).is_some(),
            "one pending interrupt, one delivery"
        );
        assert!(
            poll_interrupt(shell.interrupt_context()).is_none(),
            "and not a second time"
        );
    }

    /// An interrupt that arrives inside a deferral scope is pending but
    /// not *due*, and no poll site may take it there -- the bracket is
    /// what makes the mutation inside it atomic against a signal. This is
    /// the one property that distinguishes "delivery moved to a poll
    /// site" from "delivery moved anywhere at all".
    // [spec:nsh:sem:idiom.interrupt-deferral/test]
    #[test]
    fn nested_deferral_restores_depth() {
        let _g = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        as_interactive_root(shell);
        shell.interrupt_deferral.depth = 0;
        crate::signal_inbox::signals().set_interrupt_pending(true);

        with_interrupts_deferred(shell, |shell| {
            assert_eq!(shell.interrupt_deferral.depth, 1);
            with_interrupts_deferred(shell, |shell| {
                assert_eq!(shell.interrupt_deferral.depth, 2);
                assert!(
                    poll_interrupt(shell.interrupt_context()).is_none(),
                    "suppressed: not due"
                );
            });
            assert_eq!(shell.interrupt_deferral.depth, 1);
        });
        assert_eq!(shell.interrupt_deferral.depth, 0);
        let failed: Result<(), ()> = with_interrupts_deferred(shell, |shell| {
            assert_eq!(shell.interrupt_deferral.depth, 1);
            Err(())
        });
        assert_eq!(failed, Err(()));
        assert_eq!(shell.interrupt_deferral.depth, 0);
        assert!(
            interrupt_pending(),
            "still pending, waiting for a poll site"
        );
        assert!(
            poll_interrupt(shell.interrupt_context()).is_some(),
            "and due again once unsuppressed"
        );
    }

    /// A frame that cannot carry the interrupt out puts it back rather
    /// than losing it.
    // [spec:dash:sem:error.onint-fn/test]
    #[test]
    fn a_rearmed_interrupt_is_taken_later() {
        let _g = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        as_interactive_root(shell);
        shell.interrupt_deferral.depth = 0;
        clear_pending_interrupt();

        rearm_interrupt(Error::Interrupted {
            signal: crate::status::Signal::from(nsh_platform::interrupt_signal()),
        });
        assert!(interrupt_pending());
        assert!(
            poll_interrupt(shell.interrupt_context()).is_some(),
            "the next poll site takes it"
        );
    }
}
