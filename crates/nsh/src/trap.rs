//! Trap actions, signal dispositions, and pending delivery.
//! Rules: `docs/spec/port/src/trap.md`.
//!
//! Dispositions and pending signals are indexed by `signo - 1`, while actions
//! are indexed by `signo`, slot 0 being the `EXIT` trap.

// [spec:nsh:req:idiom.operation-modes]
// [spec:nsh:req:idiom.evaluator-control-flow]
use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;

use crate::error::Error;
use crate::eval::Flow;
use crate::options::ShellOption;
// [spec:nsh:def:idiom.shell-options]
use crate::nodes::Node;
use crate::status::Signal;

/// The active platform's signal-table width.
pub const NSIG: usize = nsh_platform::SIGNAL_COUNT;

/// What the shell should do when a condition is raised.
// [spec:nsh:def:idiom.trap-dispositions]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
pub(crate) enum TrapAction {
    /// Apply the condition's default behavior.
    #[default]
    Default,
    /// Discard the condition (`trap '' ...`).
    Ignore,
    /// Evaluate these shell bytes when the condition is delivered.
    Command(BString),
}

/// What the shell knows about one installed signal disposition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DispositionState {
    /// The host's inherited disposition has not been queried yet.
    Unknown,
    /// The shell installed and cached this disposition.
    Installed(crate::host::Disposition),
    /// The signal was ignored on entry and cannot be trapped.
    InheritedIgnore,
    /// The inherited disposition was not ignored, so the desired state
    /// must be installed even when it is `Default`.
    ResetRequired,
}

/// The trap actions, disposition cache, and counters that govern delivery.
///
/// This could not become a field until `onsig` stopped reading it. The
/// handler asked the table one question — *is a trap set for N?* — at two
/// indices, and a handler has no receiver. The answer is now a mirror in
/// the signal inbox, published by [`TrapTable::set`], which is why that is
/// the only writer of a slot and why it demands a
/// [`crate::siginbox::SignalsBlocked`] witness.
pub struct TrapTable {
    /// The action for each signal, slot 0 being the `EXIT` trap.
    ///
    /// Default, ignored, and executable actions are distinct variants. An
    /// ignored signal still counts as trapped for the signal-inbox mirror.
    action: [TrapAction; NSIG],
    /// The actions visible to a listing before this subshell executes a
    /// `trap` command with operands.  Live dispositions are still reset on
    /// entry; POSIX requires only the pre-entry commands to remain reportable.
    subshell_listing: Option<Box<[TrapAction; NSIG]>>,
    /// traps have not been fully cleared
    pub(crate) ptrap: c_int,
    /// number of non-null traps
    pub(crate) trapcnt: c_int,
    /// Current disposition knowledge, indexed by `signo - 1`.
    dispositions: [DispositionState; NSIG - 1],
    /// Cached `setinteractive` mode (`on + 1`, preserving dash's sentinel).
    interactive: c_int,
}

impl TrapTable {
    /// What the four statics were declared with, which is what a shell
    /// starts with.
    pub(crate) fn new() -> Self {
        /* The mirror is the inbox's and the inbox is the process's, so a
         * second `Shell` in one process resets the first one's bits. That
         * is api-design 6's limit rather than a bug here: one process has
         * one handler and it reports to one inbox. A fresh table has no
         * traps, so clearing is also simply correct for the only case
         * that is not that limit. */
        let sink = crate::siginbox::signals();
        for signo in 0..NSIG {
            sink.set_trapped(signo, false);
        }
        TrapTable {
            action: [const { TrapAction::Default }; NSIG],
            subshell_listing: None,
            ptrap: 0,
            trapcnt: 0,
            dispositions: [DispositionState::Unknown; NSIG - 1],
            interactive: 0,
        }
    }

    /// The action selected for `signo`.
    #[inline]
    pub(crate) fn action(&self, signo: usize) -> &TrapAction {
        &self.action[signo]
    }

    /// The action a no-operand `trap` command must report.
    pub(crate) fn listed_action(&self, signo: usize) -> &TrapAction {
        let actions = self.subshell_listing.as_deref().unwrap_or(&self.action);
        &actions[signo]
    }

    /// Preserve the listing inherited by a newly entered subshell.
    ///
    /// A nested subshell entered before any operand-bearing `trap` command
    /// inherits the same reportable list even though its parent's live table
    /// has already been reset.
    fn begin_subshell_listing(&mut self) {
        let inherited = self
            .subshell_listing
            .as_deref()
            .unwrap_or(&self.action)
            .clone();
        self.subshell_listing = Some(Box::new(inherited));
    }

    /// An operand-bearing `trap` command makes subsequent listings reflect
    /// the current subshell table.
    pub(crate) fn end_subshell_listing(&mut self) {
        self.subshell_listing = None;
    }

    /// Replace `trap[signo]`, publishing the handler's presence bit with
    /// it. The only writer of either, and it returns what was there.
    ///
    /// The `SignalsBlocked` argument is the whole point of routing every
    /// write through one function: the slot and its bit are two stores,
    /// and a handler that runs between them reads a pair dash cannot
    /// produce — its `trap[signo]` is a single pointer. Both halves of the
    /// disagreement are observable and in opposite senses, so there is no
    /// safe order to write them in and the window has to be closed rather
    /// than chosen. `siginbox::SignalsBlocked` carries the argument;
    /// `docs/api-design.md` 5.3 carries the table.
    pub(crate) fn set(
        &mut self,
        _blocked: &crate::siginbox::SignalsBlocked,
        signo: usize,
        to: TrapAction,
    ) -> TrapAction {
        let was = core::mem::replace(&mut self.action[signo], to);
        let is_trapped = !matches!(self.action[signo], TrapAction::Default);
        crate::siginbox::signals().set_trapped(signo, is_trapped);
        was
    }

    /// Take the `EXIT` action.
    ///
    /// Slot 0 is `EXIT`, which is not a signal number: `onsig` is never
    /// called with 0 and never reads the slot, so this needs neither the
    /// bracket nor the bit. Separating it is what keeps `exitshell` off
    /// the guarded path.
    pub(crate) fn take_exit_action(&mut self) -> TrapAction {
        core::mem::take(&mut self.action[0])
    }
}

// [spec:dash:def:trap.have-traps-fn]
// [spec:dash:sem:trap.have-traps-fn]
pub fn have_traps(sh: &crate::context::Shell) -> c_int {
    sh.traps.trapcnt
}

impl crate::context::Shell {
    /// Establish the child-status disposition for a newly constructed shell.
    pub(crate) fn initialize_trap_state(&mut self) {
        let child = Signal::from(nsh_platform::child_signal());
        self.traps.dispositions[(child.number() - 1) as usize] =
            DispositionState::Installed(crate::host::Disposition::Default);
        setsignal(self, child);
    }

    /// Remove parent trap actions and begin the child's independent listing.
    pub(crate) fn prepare_traps_for_child(&mut self, command: Option<&Node>) {
        self.traps.begin_subshell_listing();
        clear_traps(self, command);
    }
}

/*
 * The trap builtin.
 */

/*
 * Clear traps on a fork.
 */

/// Clear the traps a fork inherited, and put back the dispositions that
/// go with having none.
///
/// **Its `setsignal` is child-side at every reachable call site**, which
/// is why it takes [`setsignal_in_child`] and needs no split. The seam was
/// recorded as "on both paths"; counted through, it is not:
///
/// * `prepare_traps_for_child` ← `prepare_fork_child` ← `jobs::forkchild` is the
///   child.
/// * `prepare_fork_child`'s other caller is `evalsubshell`'s no-fork arm,
///   which runs in the shell's own process — but it is guarded by
///   `have_traps(sh) == 0`, and `trapcnt` counts exactly the slots with a
///   non-empty action, which is exactly what the loop below skips. The
///   loop body is unreachable from there. (It is also `EV_EXIT`-only, so
///   `Shell::run` cannot reach it at all.)
/// * `builtins::trap::trapcmd` calls it under `ptrap != 0`, and only this
///   function ever writes `ptrap`, from `simplecmd` — which is non-zero
///   only when a fork was made *for* a `trap` command. So that `trapcmd`
///   is running in that child, and the parent's `ptrap` stays 0.
// [spec:dash:def:trap.clear-traps-fn]
// [spec:dash:sem:trap.clear-traps-fn]
// [spec:posix:req:builtin.trap.persistence]
// [spec:posix:req:builtin.trap.subshell-reset]
// [spec:posix:req:builtin.trap.subshell-lexical-check]
pub fn clear_traps(sh: &mut crate::context::Shell, n: Option<&Node>) {
    let simplecmd: c_int;

    simplecmd = crate::parser::issimplecmd(n, BStr::new(crate::builtins::TRAPCMD.name.to_bytes()));

    crate::error::with_interrupts_deferred(sh, |sh| {
        /* One guard for the whole loop rather than one per slot. The
         * `simplecmd` arm clears a slot and puts it back with a disposition
         * update in between, so the whole transition is one scope. */
        let blocked = crate::siginbox::SignalsBlocked::new();
        for signo in 0..NSIG {
            if !matches!(sh.traps.action(signo), TrapAction::Command(_)) {
                continue;
            }
            let previous = sh.traps.set(&blocked, signo, TrapAction::Default);
            if signo != 0 {
                let signal = Signal::from_number(signo as i32)
                    .expect("nonzero trap slots are positive signals");
                setsignal_in_child(sh, signal);
            }

            if simplecmd != 0 {
                drop(sh.traps.set(&blocked, signo, previous));
            }
            /* The C leaks the previous action in the non-simple-command arm.
             * This owned value drops it after the last possible restore. */
        }
        sh.traps.trapcnt = 0;
        sh.traps.ptrap = simplecmd;
        drop(blocked);
    });
}

/// Which side of a `fork` a disposition change is being made on, and so
/// what performs it.
///
/// The call site chooses, and that is the design rather than a shortcut:
/// whether a caller runs in a forked child is a static property of the
/// *path*, not a dynamic property of the shell — a child's `Shell` is
/// bit-for-bit the one that forked it, so there is nothing in shell state
/// a flag could have been read from.
#[derive(Copy, Clone, PartialEq, Eq)]
enum Via {
    /// The parent, where the process belongs to whoever linked the library
    /// in. [dec:nsh:host-owns-signals]: the shell decides *which*
    /// disposition, and the host is what installs it.
    Host,
    /// A forked child, which goes through the platform boundary directly.
    /// Two reasons, each
    /// sufficient on its own:
    ///
    /// * Routing it would be an indirect call into embedder code made in
    ///   a forked child, which
    ///   [dec:nsh:fork-child-is-a-terminus] forbids.
    /// * Under [`crate::host::NoHost`] a routed call installs nothing, so
    ///   a background job would go on taking `^C` from the terminal
    ///   because its `ignoresig(SIGINT)` had been quietly dropped. The
    ///   child *is* the whole process, so there is no third party for the
    ///   host to be protecting.
    Platform,
}

/// The `struct sigaction` a query filled in, read as a [`Disposition`].
///
/// dash asks only "is this `SIG_IGN`", so `Default` and `Catch` are one
/// answer as far as [`setsignal`] is concerned. They are kept apart
/// because [`crate::host::Host`] is what an embedder implements, and an
/// embedder can tell them apart.
pub(crate) fn disposition_of(action: nsh_platform::SignalAction) -> crate::host::Disposition {
    match action {
        nsh_platform::SignalAction::Ignore => crate::host::Disposition::Ignore,
        nsh_platform::SignalAction::Default => crate::host::Disposition::Default,
        nsh_platform::SignalAction::Catch => crate::host::Disposition::Catch,
    }
}

/// What is installed for `signo` right now, or `Err` if it cannot be read.
fn current_disposition(
    sh: &mut crate::context::Shell,
    signal: Signal,
    via: Via,
) -> std::io::Result<crate::host::Disposition> {
    match via {
        Via::Host => sh.host.signal(signal),
        Via::Platform => nsh_platform::signal_action(signal.platform()).map(disposition_of),
    }
}

/// Install `to` for `signo`.
///
/// The `struct sigaction` the query filled in is not carried into the
/// install the way the C carries it, and nothing observable is lost:
/// every field the C reused is overwritten before the second call —
/// `sa_sigaction` by the choice below, `sa_flags` by the `0`, `sa_mask` by
/// `sigfillset` — and `sa_restorer` never reaches the kernel at all,
/// because glibc's `sigaction` writes its own trampoline into the
/// kernel-facing copy unconditionally. What is left is the signal number
/// and the disposition, which is exactly what the host is asked for.
fn install_disposition(
    sh: &mut crate::context::Shell,
    signal: Signal,
    to: crate::host::Disposition,
    via: Via,
) {
    match via {
        Via::Host => {
            /* The C ignores `sigaction`'s return value here, so a shell
             * that cannot install a disposition carries on with the one it
             * has; a host that refuses reads the same way. */
            let _ = sh.host.set_signal(signal, to);
        }
        Via::Platform => {
            let action = match to {
                crate::host::Disposition::Catch => nsh_platform::SignalAction::Catch,
                crate::host::Disposition::Ignore => nsh_platform::SignalAction::Ignore,
                crate::host::Disposition::Default => nsh_platform::SignalAction::Default,
            };
            let _ = nsh_platform::install_signal_action(signal.platform(), action, onsig);
        }
    }
}

/*
 * Set the signal handler for the specified signal.  The routine figures
 * out what it should be set to.
 */

/// The parent's entry point: the host installs what the shell decided.
// [spec:dash:def:trap.setsignal-fn]
// [spec:dash:sem:trap.setsignal-fn]
// [spec:posix:req:builtin.trap.signals-ignored-on-entry]
// [spec:posix:req:sh.signals-standard-action]
// [spec:posix:req:sh.interactive-sigint]
// [spec:posix:req:sh.interactive-sigquit-sigterm]
// [spec:posix:req:sh.interactive-stop-signals]
// [spec:posix:req:sh.signal-actions-overridable]
// [spec:posix:req:xcu.defaults.asynchronous-events-default]
// [spec:posix:req:xcu.async.may-catch-and-resignal]
pub fn setsignal(sh: &mut crate::context::Shell, signal: Signal) {
    setsignal_via(sh, signal, Via::Host)
}

/// The forked child's entry point: identical policy, installed directly.
///
/// See [`Via::Platform`] for why this is a second entry point rather than an
/// argument the shell could have answered for itself.
pub fn setsignal_in_child(sh: &mut crate::context::Shell, signal: Signal) {
    setsignal_via(sh, signal, Via::Platform)
}

fn setsignal_via(sh: &mut crate::context::Shell, signal: Signal, via: Via) {
    let signo = signal.number();

    let mut desired = match sh.traps.action(signo as usize) {
        TrapAction::Default => crate::host::Disposition::Default,
        TrapAction::Ignore => crate::host::Disposition::Ignore,
        TrapAction::Command(_) => crate::host::Disposition::Catch,
    };
    if crate::shellmain::rootshell(sh) != 0 && desired == crate::host::Disposition::Default {
        match signal {
            signal if signal == Signal::from(nsh_platform::interrupt_signal()) => {
                if sh.options.enabled(ShellOption::Interactive)
                    || sh.options.minusc.is_some()
                    || !sh.options.enabled(ShellOption::Stdin)
                {
                    desired = crate::host::Disposition::Catch;
                }
            }
            signal if signal == Signal::from(nsh_platform::quit_signal()) => {
                if sh.options.enabled(ShellOption::Interactive) {
                    desired = crate::host::Disposition::Ignore;
                }
            }
            signal if signal == Signal::from(nsh_platform::termination_signal()) => {
                if sh.options.enabled(ShellOption::Interactive) {
                    desired = crate::host::Disposition::Ignore;
                }
            }
            signal
                if signal == Signal::from(nsh_platform::terminal_stop_signal())
                    || signal == Signal::from(nsh_platform::terminal_output_signal()) =>
            {
                if sh.options.enabled(ShellOption::Monitor) {
                    desired = crate::host::Disposition::Ignore;
                }
            }
            _ => {}
        }
    }

    if signal == Signal::from(nsh_platform::child_signal()) {
        desired = crate::host::Disposition::Catch;
    }

    let index = (signo - 1) as usize;
    let mut state = sh.traps.dispositions[index];
    if state == DispositionState::Unknown {
        let current = match current_disposition(sh, signal, via) {
            Ok(d) => d,
            Err(_) => {
                // Leave the state unknown so a later call retries the query.
                return;
            }
        };
        /* This test is the whole reason `Host` has a `signal` as well as
         * a `set_signal`: a signal already ignored when the shell started
         * is hard-ignored and can never be trapped, and that rule cannot
         * be reproduced without reading the inherited disposition. */
        if current == crate::host::Disposition::Ignore {
            if sh.options.enabled(ShellOption::Monitor)
                && (signal == Signal::from(nsh_platform::terminal_stop_signal())
                    || signal == Signal::from(nsh_platform::terminal_input_signal())
                    || signal == Signal::from(nsh_platform::terminal_output_signal()))
            {
                state = DispositionState::Installed(crate::host::Disposition::Ignore);
            } else {
                state = DispositionState::InheritedIgnore;
            }
        } else {
            state = DispositionState::ResetRequired;
        }
    }
    if state == DispositionState::InheritedIgnore || state == DispositionState::Installed(desired) {
        return;
    }
    sh.traps.dispositions[index] = DispositionState::Installed(desired);
    install_disposition(sh, signal, desired, via);
}

/*
 * Ignore a signal.
 */

/// Ignore a signal, in a forked child, directly.
///
/// There is no parent-side twin, because there is no parent-side caller:
/// both call sites are `forkchild`'s `FORK_BG` arm, where the child must
/// genuinely stop taking `^C` from the terminal. [`Via::Platform`] carries the
/// argument; a parent-side caller appearing later needs a twin routed
/// through the host, and the name here is what should make that obvious.
///
/// `signal` rather than `sigaction` is dash's spelling and is kept, which
/// costs nothing: `SIG_IGN` runs no handler, so the flags and mask the two
/// calls disagree about have nothing to apply to.
// [spec:dash:def:trap.ignoresig-fn]
// [spec:dash:sem:trap.ignoresig-fn]
pub fn ignoresig_in_child(sh: &mut crate::context::Shell, signal: Signal) {
    let signo = signal.number();
    let index = (signo - 1) as usize;
    let state = sh.traps.dispositions[index];
    if state == DispositionState::Installed(crate::host::Disposition::Ignore)
        || state == DispositionState::InheritedIgnore
    {
        return;
    }
    let _ = nsh_platform::ignore_signal(signal.platform());
    sh.traps.dispositions[index] = DispositionState::Installed(crate::host::Disposition::Ignore);
}

/*
 * Signal handler.
 */

// [spec:dash:def:trap.onsig-fn]
// [spec:dash:sem:trap.onsig-fn]
/* The platform crate owns the C-ABI trampoline and hands this callback a
 * validated signal. Delivery records atomics and returns; it never unwinds
 * through the signal frame. */
pub fn onsig(signal: nsh_platform::Signal) {
    let signal = Signal::from_number(signal.number())
        .expect("the platform callback supplies a positive signal");
    crate::siginbox::signals().raise(signal);
}

/*
 * Called to execute a trap.  Perhaps we should avoid entering new trap
 * handlers while we are executing a trap handler.
 */

// [spec:dash:def:trap.dotrap-fn]
// [spec:dash:sem:trap.dotrap-fn]
// [spec:posix:req:builtin.trap.action-overrides-and-exit-status]
// [spec:posix:req:builtin.trap.action-executed-as-eval]
// [spec:posix:sem:signal.pending-trap-order]
pub fn dotrap(sh: &mut crate::context::Shell) -> Result<Flow, Error> {
    let mut i: c_int;
    let status: crate::status::ExitStatus;

    /* The poll site the shell reaches most often: `evaltree` calls
     * `dotrap` before every command and again at its `out:`, so an
     * interrupt taken anywhere the shell was not looking is delivered at
     * the next command boundary at the latest. It is tested before
     * `pending_sig`, because an *untrapped* SIGINT sets `intpending` and
     * has no trap action for the loop below to run. */
    if let Some(e) = crate::error::poll_interrupt(sh) {
        return Err(e);
    }

    let signals = crate::siginbox::signals();
    if signals.pending_signal().is_none() {
        return Ok(Flow::Done((0).into()));
    }

    /* Each invocation owns the status it interrupted. In particular, a
     * signal delivered while an EXIT action is running must save that
     * action's current status, not reuse the status that entered EXIT. */
    // [spec:nsh:req:compat.smoosh.trap-status]
    status = sh.status;
    signals.set_pending_signal(None);
    crate::error::barrier();

    i = 0;
    while i < NSIG as c_int - 1 {
        let signal = Signal::from_number(i + 1).expect("trap loop visits positive signals");
        if !signals.signal_pending(signal) {
            i += 1;
            continue;
        }

        signals.set_signal_pending(signal, false);

        /* The action is copied out because `evalstring` parses from the
         * buffer it is handed and the action it runs may `trap` over this
         * very slot; the C passes the slot's own pointer and keeps reading
         * it after `trapcmd` has freed it. */
        let command = match sh.traps.action(signal.number() as usize) {
            TrapAction::Command(command) => command.clone(),
            TrapAction::Default | TrapAction::Ignore => {
                i += 1;
                continue;
            }
        };
        /* A signal action is an evaluation catch boundary. The depth lets
         * `evalcommand` turn a special-builtin command failure into its
         * ordinary status, so the command performs its own redirection,
         * input, and local-variable cleanup before returning here. Syntax
         * and interrupt errors still arrive as `Err` and propagate. */
        let outer_trap_status = sh.eval.trap_default_exit_status.replace(status);
        sh.eval.signal_trap_depth += 1;
        let outcome =
            crate::eval::evalstring(sh, command.as_bstr(), crate::eval::EvalContext::DEFAULT);
        sh.eval.signal_trap_depth -= 1;
        sh.eval.trap_default_exit_status = outer_trap_status;
        match outcome? {
            Flow::Done(_) => sh.status = status,
            control @ Flow::Return { explicit: true, .. } => return Ok(control),
            control @ Flow::Return {
                explicit: false, ..
            }
            | control @ Flow::Break { .. }
            | control @ Flow::Continue { .. } => {
                sh.status = status;
                return Ok(control.with_status(status));
            }
            exit @ Flow::Exit { .. } => return Ok(exit),
        }
        i += 1;
    }

    Ok(Flow::Done((sh.status).into()))
}

/*
 * Controls whether the shell is interactive or not.
 */

// [spec:dash:def:trap.setinteractive-fn]
// [spec:dash:sem:trap.setinteractive-fn]
pub fn setinteractive(sh: &mut crate::context::Shell, on: c_int) {
    let on = on + 1;
    if on == sh.traps.interactive {
        return;
    }
    sh.traps.interactive = on;
    setsignal(sh, nsh_platform::interrupt_signal().into());
    setsignal(sh, nsh_platform::quit_signal().into());
    setsignal(sh, nsh_platform::termination_signal().into());
}

/*
 * Called to exit the shell.
 */

// [spec:dash:def:trap.exitshell-fn]
// [spec:dash:sem:trap.exitshell-fn]
// [spec:posix:req:builtin.trap.exit-condition]
// [spec:posix:req:builtin.trap.exit-action-environment]
// [spec:nsh:req:compat.smoosh.trap-status]
/// Run the EXIT trap, tear job control down, and **return** the status
/// the shell leaves with.
///
/// It used to end in `_exit`, and that was the one `_exit` in the crate
/// that ended the *host's* process rather than a child the library
/// forked. `[dec:nsh:host-owns-the-process]` puts ending the process
/// outside what a library may do on its own authority, and answers it
/// with an absence rather than a grant: there is no `Host` method for it
/// because after this there is nothing to grant — the status is returned,
/// and whoever owns the process decides what to do with it. `nsh-cli`
/// calls `std::process::exit`.
///
/// The other three `_exit`s stay, and are correct: `shellmain`'s
/// `exit_from_child`, `jobs`' `forkchild_fatal` and `redir.rs:483` all
/// end a child the library forked, which `[dec:nsh:fork-child-is-a-terminus]`
/// says is a terminus rather than a frame.
pub fn exitshell(
    sh: &mut crate::context::Shell,
    explicit_status: Option<crate::status::ExitStatus>,
) -> crate::status::ExitStatus {
    if let Some(status) = explicit_status {
        sh.status = status;
    }
    'out: {
        /* `trap[0] = NULL` with no free: the C leaks the EXIT action on
         * purpose so `evalstring` can still read it.  Taking it keeps the
         * action alive for exactly as long and gives the buffer back. */
        let action = sh.traps.take_exit_action();
        if let TrapAction::Command(command) = action {
            if sh.traps.ptrap != 0 {
                break 'out;
            }
            /* An error in the EXIT trap is reported and dropped -- the
             * shell is already exiting, and the C's `longjmp` landed at
             * `out:` with nothing left to inspect it. What must not be
             * dropped is an `exit` *inside* the trap, because it names the
             * status the shell leaves with. */
            let trap_entry_status = sh.status;
            let outer_trap_status = sh.eval.trap_default_exit_status.replace(trap_entry_status);
            let outcome =
                crate::eval::evalstring(sh, command.as_bstr(), crate::eval::EvalContext::DEFAULT);
            sh.eval.trap_default_exit_status = outer_trap_status;
            match outcome {
                Ok(crate::eval::Flow::Exit {
                    status: Some(status),
                }) => {
                    sh.status = status;
                    break 'out;
                }
                Ok(crate::eval::Flow::Exit { status: None }) => {
                    if let Some(status) = explicit_status {
                        sh.status = status;
                    }
                    break 'out;
                }
                Ok(crate::eval::Flow::Done(status)) => {
                    sh.status = explicit_status.unwrap_or(status);
                }
                Ok(control) => {
                    sh.status = explicit_status
                        .or_else(|| control.status())
                        .unwrap_or(sh.status);
                }
                Err(e) => {
                    /* The EXIT trap failed. An explicit outer `exit n`
                     * still names the status; implicit shutdown instead
                     * uses the action error. Write the selected value here
                     * because raising the error no longer writes it. */
                    sh.status = explicit_status.unwrap_or_else(|| e.status());
                    drop(e);
                    break 'out;
                }
            }
        }
    }
    /* out: */
    crate::histedit::save_history(sh);
    sh.clear_evaluation_resources();
    sh.flush_input();
    /*
     * Disable job control so that whoever had the foreground before we
     * started can get it back.
     */
    /* The C wraps this in a second `setjmp(loc.loc)` for one reason: a
     * raise inside the job-control teardown must not prevent the `_exit`
     * below. Dropping the diagnostic is that frame, exactly -- it caught
     * and went on -- and it is why the frame itself can go. */
    drop(crate::jobs::setjobctl(sh, 0));
    sh.io.flushall();
    crate::shell::flush_coverage();
    sh.status
}

/// A signal-like condition accepted by `trap` and `kill`. `EXIT` is the
/// shell's pseudo-condition and never reaches the operating system.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SignalSpec {
    Exit,
    Signal(Signal),
}

impl SignalSpec {
    pub(crate) fn index(self) -> usize {
        match self {
            SignalSpec::Exit => 0,
            SignalSpec::Signal(signal) => signal.number() as usize,
        }
    }
}

// [spec:dash:def:trap.decode-signum-fn]
// [spec:dash:sem:trap.decode-signum-fn]
pub(crate) fn decode_signum(string: &BStr) -> Option<SignalSpec> {
    let number = crate::mystring::decimal_digits(string)?;
    if number >= NSIG as u64 {
        return None;
    }
    if number == 0 {
        Some(SignalSpec::Exit)
    } else {
        Signal::from_number(number as i32).map(SignalSpec::Signal)
    }
}

// [spec:dash:def:trap.decode-signal-fn]
// [spec:dash:sem:trap.decode-signal-fn]
pub(crate) fn decode_signal(string: &BStr, include_exit_name: bool) -> Option<SignalSpec> {
    if let Some(signal) = decode_signum(string) {
        return Some(signal);
    }

    let first = usize::from(!include_exit_name);
    for index in first..NSIG {
        if string.eq_ignore_ascii_case(crate::signames::signal_names[index].to_bytes()) {
            return if index == 0 {
                Some(SignalSpec::Exit)
            } else {
                Signal::from_number(index as i32).map(SignalSpec::Signal)
            };
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::siginbox::{SignalsBlocked, signals};

    /// **The mirror is the table, or it is a bug.** `onsig` cannot reach
    /// `sh.traps`, so the bit it reads instead has to say the same thing
    /// the slot does. Every direction of that is a pty case
    /// (`tests/harness/ptydiff.py`, the job-control block); this states
    /// the same property where a mutation of `TrapTable::set` fails it in
    /// milliseconds rather than in a terminal.
    #[test]
    fn mirror_follows_the_slot() {
        let _g = crate::testutil::lock();
        let interrupt = nsh_platform::interrupt_signal();
        let mut t = TrapTable::new();
        assert!(
            !signals().is_trapped(interrupt.into()),
            "a new table has no traps"
        );

        let b = SignalsBlocked::new();
        drop(t.set(
            &b,
            interrupt.number() as usize,
            TrapAction::Command(BString::from("echo hi")),
        ));
        assert!(
            signals().is_trapped(interrupt.into()),
            "set an action, set the bit"
        );

        drop(t.set(&b, interrupt.number() as usize, TrapAction::Default));
        assert!(
            !signals().is_trapped(interrupt.into()),
            "clear the action, clear the bit"
        );
        drop(b);
    }

    /// **The predicate is `is_some()`, not "has an action".** The C's
    /// three states are `NULL`, `""` and an action, and `onsig` tests
    /// `trap[signo] != NULL` — so `trap '' INT`, which *ignores* the
    /// signal, still reads as trapped. A mirror keyed on emptiness passes
    /// every other test here and gets that one case backwards.
    #[test]
    fn ignored_signal_counts_as_trapped() {
        let _g = crate::testutil::lock();
        let interrupt = nsh_platform::interrupt_signal();
        let mut t = TrapTable::new();
        let b = SignalsBlocked::new();
        drop(t.set(&b, interrupt.number() as usize, TrapAction::Ignore));
        assert!(
            signals().is_trapped(interrupt.into()),
            "`trap '' INT` is a trap as far as the handler is concerned"
        );
        drop(t.set(&b, interrupt.number() as usize, TrapAction::Default));
        drop(b);
    }

    /// A fresh table starts the mirror fresh with it. This is also where
    /// `docs/api-design.md` 6's limit bites: the inbox is the process's,
    /// so a second `Shell` in one process resets the first one's bits.
    /// Stated as a test so it is a known property rather than a surprise.
    #[test]
    fn a_new_table_clears_the_mirror() {
        let _g = crate::testutil::lock();
        let child = nsh_platform::child_signal();
        let mut t = TrapTable::new();
        let b = SignalsBlocked::new();
        drop(t.set(
            &b,
            child.number() as usize,
            TrapAction::Command(BString::from("echo chld")),
        ));
        drop(b);
        assert!(signals().is_trapped(child.into()));

        let _fresh = TrapTable::new();
        assert!(
            !signals().is_trapped(child.into()),
            "a new table, a clear mirror"
        );
    }

    /// **The guard blocks, and puts the mask back.** Without the `Drop`
    /// the shell runs on with every signal blocked — which no test above
    /// would notice, and which would make it stop answering anything.
    #[test]
    fn the_guard_blocks_and_restores() {
        let _g = crate::testutil::lock();
        crate::system::sigclearmask();
        let interrupt = nsh_platform::interrupt_signal();
        let child = nsh_platform::child_signal();
        assert!(
            !nsh_platform::signal_is_blocked(interrupt).unwrap(),
            "start unblocked"
        );

        {
            let _b = SignalsBlocked::new();
            assert!(
                nsh_platform::signal_is_blocked(interrupt).unwrap(),
                "blocked inside"
            );
            assert!(
                nsh_platform::signal_is_blocked(child).unwrap(),
                "all of them"
            );
        }

        assert!(
            !nsh_platform::signal_is_blocked(interrupt).unwrap(),
            "restored on drop"
        );
    }

    // [spec:nsh:def:idiom.trap-dispositions/test]
    #[test]
    fn trap_and_disposition_states_are_distinct() {
        let actions = [
            TrapAction::Default,
            TrapAction::Ignore,
            TrapAction::Command(BString::from("echo caught")),
        ];
        assert!(matches!(&actions[0], TrapAction::Default));
        assert!(matches!(&actions[1], TrapAction::Ignore));
        assert!(matches!(&actions[2], TrapAction::Command(_)));

        let states = [
            DispositionState::Unknown,
            DispositionState::Installed(crate::host::Disposition::Default),
            DispositionState::Installed(crate::host::Disposition::Catch),
            DispositionState::Installed(crate::host::Disposition::Ignore),
            DispositionState::InheritedIgnore,
            DispositionState::ResetRequired,
        ];
        assert_eq!(states.len(), 6);

        let source = include_str!("trap.rs");
        for parts in [
            ("c_", "char"),
            ("S_", "DFL"),
            ("S_", "CATCH"),
            ("S_HARD_", "IGN"),
            ("S_", "RESET"),
        ] {
            let fragment = format!("{}{}", parts.0, parts.1);
            assert!(
                !source.contains(&fragment),
                "found numeric trap mode {fragment}"
            );
        }
    }
}
