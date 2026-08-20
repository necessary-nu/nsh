//! The shell instance, and the parameter that carries it.
//!
//! Implements [dec:nsh:no-ambient-state]: shell state belongs to a shell
//! instance rather than to the process. That decision lands in two steps
//! and this module is where both of them arrive.
//!
//! `thread-context` gave every execution path a `&mut Shell` to reach the
//! state through. `move-state` — this step — moves the tables onto this
//! type one at a time, and the functions that read a table take the
//! receiver in the commit that moves it, so no signature is edited twice.
//!
//! ## Every instance is the shell
//!
//! There is exactly one constructor and one call to it, at the entry
//! point. That is not a convention, it is the invariant the type now
//! depends on: a second `Shell` made at a call site would carry a second,
//! empty set of tables, and every field added here makes that a wrong
//! answer rather than a harmless one. `Shell::detached()` was the
//! transitional constructor for call sites the threading had not reached;
//! it is gone, and the last site it served — `parser::getprompt`, called
//! from the line editor's prompt request — takes the receiver by
//! parameter instead.
//!
//! `docs/api-design.md` §5 lists, field by field, what moves here; §5.1
//! and §5.2 list what does not, and the one shape that still cannot take
//! a receiver is the signal handler, which has no frame to thread through
//! and gets a shared inbox instead.
//!
//! A function that has been given the context but whose state has not
//! moved yet names it `_sh`. The underscore is the marker for "carries the
//! context, does not read it yet", and it disappears when the commit that
//! moves its table rewrites the body to read a field.
//!
//! ## What it is not
//!
//! It is not the public `Shell` of `docs/api-design.md` §2. That type is
//! `public-api`'s, and it grows out of this one: the builder, the host,
//! the streams and the `run` surface are all that node's. What this type
//! settles now is only the receiver — `[dec:nsh:public-surface]` records
//! the destination as `fn(&mut Shell, &[&BStr]) -> Result<ExitStatus,
//! Error>`, and the receiver in that signature is this.

/// The shell, as an instance rather than as a process.
///
/// `docs/api-design.md` §5 is the list this fills from, one table per
/// commit; the fields here are the ones that have arrived.
// [spec:posix:def:shenv.components]
pub struct Shell {
    /// The immutable POSIX locale object selected by this shell's locale
    /// variables. Raw handles and temporary thread selection stay inside
    /// `nsh-platform`.
    // [spec:nsh:req:shell-locale.instance-isolation]
    pub(crate) locale: nsh_platform::Locale,
    /// Every alias, by name. `alias.rs` owns the shape; this owns the
    /// value.
    pub(crate) aliases: crate::alias::AliasTable,
    /// The command hash and `%builtin` location. Function definitions live
    /// here too, because dash stores them in the same table.
    pub(crate) commands: crate::exec::CmdTable,
    /// The shell's option flags. `options.rs` owns the shape; the
    /// array behind it is private, so the flags are set through named
    /// accessors rather than by indexing from anywhere.
    pub(crate) options: crate::options::ShellOptions,
    /// Where the evaluator is: what it is skipping, how deep, and the
    /// buffers it must not re-enter. `eval.rs` owns the shape.
    pub(crate) eval: crate::eval::EvalState,
    /// The jobs, and the terminal state job control needs. `jobs.rs`
    /// owns the shape.
    pub(crate) jobs: crate::jobs::JobTable,
    /// `$!` — the process id of the last command the shell put in the
    /// background.
    ///
    /// The rest of `jobs.rs`'s table has not moved. This member could go
    /// on its own because it is not part of it in any way that matters:
    /// one function writes it (`jobs::forkparent`) and one reads it
    /// (`expand::varvalue`, for `$!`), and neither reaches the job table
    /// to do so.
    pub(crate) backgndpid: Option<nsh_platform::ProcessId>,
    /// PID of the process that created this shell instance.
    pub(crate) root_pid: nsh_platform::ProcessId,
    /// Zero in the root shell and incremented in forked shell children.
    pub(crate) shell_level: usize,
    /// Nesting depth of regions that defer delivery of SIGINT.
    pub(crate) interrupt_deferral: crate::error::InterruptDeferral,
    /// The shell-language descriptor namespace. Open entries own hidden
    /// close-on-exec backing descriptors; redirection changes this table,
    /// never the host process table.
    pub(crate) fds: crate::fd::FdTable,
    /// The stack of logical descriptor values saved for restoration.
    /// `redir.rs` owns the shape; this owns the value.
    pub(crate) redirs: crate::redir::RedirStack,
    /// Every variable, the sixteen the shell is born with, the `LINENO`
    /// buffer and line, and the `local` save stack. `var.rs` owns the
    /// shape; this owns the value.
    pub(crate) vars: crate::var::VarTable,
    /// Who owns the process, and therefore what this shell may do to it.
    ///
    /// A `Box<dyn Host>` rather than a type parameter because `Shell`
    /// appears in hundreds of signatures and a parameter would spread to
    /// every one of them, for a choice made once at construction.
    pub(crate) host: Box<dyn crate::host::Host>,
    /// Where the shell is reading from, and what it has read.
    /// `input.rs` owns the shape; this owns the value.
    pub(crate) input: crate::input::InputStack,
    /// Where the shell thinks it is: the logical and physical working
    /// directories. `cd.rs` owns the shape.
    pub(crate) cwd: crate::cd::Cwd,
    /// What `$MAILPATH` checking remembers between prompts. `mail.rs`
    /// owns the shape.
    pub(crate) mail: crate::mail::MailState,
    /// `IFS` in the forms field splitting wants it, rebuilt by the
    /// variable hook. `expand.rs` owns the shape.
    pub(crate) ifs: crate::expand::IfsCache,
    /// Scratch owned by one expansion frame. Top-level expansion moves it
    /// out while it runs so command substitutions receive an independent
    /// nested frame.
    pub(crate) expand: crate::expand::ExpandState,
    /// `fc -l`: list the history rather than re-running it.
    pub(crate) displayhist: bool,
    /// Interactive history, the line editor, and `fc` recursion state.
    /// `histedit.rs` owns the shape; keeping it here makes two shell
    /// instances independent instead of sharing a process-global editor.
    pub(crate) histedit: crate::histedit::HistEditState,
    /// The trap actions, the disposition cache and their two counters.
    /// `trap.rs` owns the shape; this owns the value.
    ///
    /// The last table `move-state` could not take, and not for want of
    /// effort: `onsig` read it, and a signal handler has no receiver. It
    /// moves here because the one question the handler asked it — *is a
    /// trap set for N?* — is now answered by a mirror in the signal
    /// inbox. `docs/api-design.md` §5.3 has the design and
    /// `[dec:nsh:host-owns-signals]` the argument.
    pub(crate) traps: crate::trap::TrapTable,
    /// The shell's three writers: buffered stdout, unbuffered stderr,
    /// and the stderr saved across a redirection that `set -x` traces to.
    /// `output.rs` owns the shape; this owns the value.
    ///
    /// The other half of `move-state`'s blocked group, and it moved for
    /// the same reason `streams` did — see that field.
    pub(crate) io: crate::output::ShellIo,
    /// Where the shell's own three streams come from.
    ///
    /// [dec:nsh:host-owns-streams] as a field rather than as a process
    /// global. The builder takes a `Streams` value; the constructor snapshots
    /// it into `fds`, and `io` retains stable references to its logical
    /// output slots. No process-global stream installation step remains.
    pub(crate) streams: crate::streams::Streams,
    /// `$?` — the exit status of the last command.
    ///
    /// Its own field rather than a member of `eval`, because
    /// `docs/api-design.md` §5 gives it its own row: it is the shell's
    /// answer to the outside world, so it has the same `ExitStatus` type
    /// the public API returns rather than borrowing the evaluator's shape.
    ///
    /// It could only move once the raise path stopped writing it. That
    /// is what the commit before this one did: an error carries the
    /// status it took and the frame that catches it writes it here, so
    /// `sh_error_value`'s 56 call sites need no receiver.
    pub(crate) status: crate::status::ExitStatus,
    /// The status the shell exited with, once it has.
    ///
    /// `docs/api-design.md` §5's last row, and what it says there is what
    /// it is for: it replaces the `EXEND`/`EXEXIT` unwind reaching `main`.
    /// A process shell answered "has it exited?" by not existing any more;
    /// a library shell is still a value afterwards, so the fact has to be
    /// somewhere, and [`Shell::has_exited`] is where an embedder reads it.
    pub(crate) exited: Option<crate::status::ExitStatus>,
}

impl Shell {
    /// The shell the process runs as, reading and writing `streams`.
    ///
    /// There is one, made at the entry point, and it is threaded down
    /// from there. As tables move onto this type, this is where their
    /// initial values go — which is what makes it the one constructor.
    /// Each field starts at what the `static mut` it replaces was
    /// declared with, so the shell a process begins with is the one the
    /// C began with.
    ///
    /// `streams` is the one argument, and it is here rather than on a
    /// setter because the alternative is a process-global that two
    /// shells would share: `docs/api-design.md` §7 and
    /// [dec:nsh:host-owns-streams]. `Streams::INHERIT` snapshots the
    /// process's shell-language descriptor range, which is what a shell
    /// started as a process uses. Explicit streams seed only logical
    /// descriptors 0, 1 and 2.
    /// A [`crate::builder::Builder`] with every setting at its default.
    ///
    /// The public way to make a shell. `new` stays `pub(crate)` and stays
    /// the one place field initial values live; the builder grows on top
    /// of it rather than around it, so there is still exactly one list of
    /// what a fresh shell contains.
    pub fn builder() -> crate::builder::Builder {
        crate::builder::Builder::new()
    }

    #[cfg(test)]
    pub(crate) fn new(streams: crate::streams::Streams) -> Self {
        Self::try_new(streams).expect("test shell snapshots its standard streams")
    }

    pub(crate) fn try_new(streams: crate::streams::Streams) -> std::io::Result<Self> {
        let fds = crate::fd::FdTable::from_streams(&streams)?;
        // [spec:nsh:def:idiom.logical-descriptors]
        let io = crate::output::ShellIo::new(
            fds.slot(crate::fd::LogicalDescriptor::STDOUT),
            fds.slot(crate::fd::LogicalDescriptor::STDERR),
        );
        let locale = nsh_platform::Locale::c()?;
        let root_pid = nsh_platform::current_process_id();
        Ok(Shell {
            locale,
            io,
            streams,
            fds,
            aliases: crate::alias::AliasTable::new(),
            backgndpid: None,
            root_pid,
            shell_level: 0,
            interrupt_deferral: crate::error::InterruptDeferral::new(),
            commands: crate::exec::CmdTable::new(),
            eval: crate::eval::EvalState::new(),
            jobs: crate::jobs::JobTable::new(),
            options: crate::options::ShellOptions::new(),
            redirs: crate::redir::RedirStack::new(),
            cwd: crate::cd::Cwd::new(),
            mail: crate::mail::MailState::new(),
            ifs: crate::expand::IfsCache::new(),
            expand: crate::expand::ExpandState::new(),
            displayhist: false,
            histedit: crate::histedit::HistEditState::new(),
            traps: crate::trap::TrapTable::new(),
            input: crate::input::InputStack::new(),
            status: crate::status::ExitStatus::SUCCESS,
            exited: None,
            vars: crate::var::VarTable::new(),
            /* [dec:nsh:host-owns-signals]: a shell that was not told who
             * owns the process assumes nobody did, and touches nothing
             * outside itself. `Builder::host` replaces this. */
            host: Box::new(crate::host::NoHost),
        })
    }
}
