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
pub struct Shell {
    /// Every alias, by name. `alias.rs` owns the shape; this owns the
    /// value.
    pub(crate) aliases: crate::alias::AliasTable,
    /// The command hash and `builtinloc`. Function definitions live
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
    pub(crate) backgndpid: libc::pid_t,
    /// The saved-descriptor stack and the closed-descriptor bitmap.
    /// `redir.rs` owns the shape; this owns the value.
    pub(crate) redirs: crate::redir::RedirStack,
    /// Every variable, the sixteen the shell is born with, the `LINENO`
    /// buffer and line, and the `local` save stack. `var.rs` owns the
    /// shape; this owns the value.
    pub(crate) vars: crate::var::VarTable,
}

impl Shell {
    /// The shell the process runs as.
    ///
    /// There is one, made at the entry point, and it is threaded down
    /// from there. As tables move onto this type, this is where their
    /// initial values go — which is what makes it the one constructor.
    /// Each field starts at what the `static mut` it replaces was
    /// declared with, so the shell a process begins with is the one the
    /// C began with.
    pub(crate) fn new() -> Self {
        Shell {
            aliases: crate::alias::AliasTable::new(),
            backgndpid: 0,
            commands: crate::exec::CmdTable::new(),
            eval: crate::eval::EvalState::new(),
            jobs: crate::jobs::JobTable::new(),
            options: crate::options::ShellOptions::new(),
            redirs: crate::redir::RedirStack::new(),
            vars: crate::var::VarTable::new(),
        }
    }
}
