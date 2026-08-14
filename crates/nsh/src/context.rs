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
/// Still empty; `docs/api-design.md` §5 is the list it fills from, one
/// table per commit.
pub struct Shell {
    /// Keeps the type from being constructible outside this module, so
    /// that every instance comes from [`Shell::new`] — which is what
    /// makes "one shell per process" checkable rather than hoped for.
    _private: (),
}

impl Shell {
    /// The shell the process runs as.
    ///
    /// There is one, made at the entry point, and it is threaded down
    /// from there. As tables move onto this type, this is where their
    /// initial values go — which is what makes it the one constructor.
    pub(crate) fn new() -> Self {
        Shell { _private: () }
    }
}
