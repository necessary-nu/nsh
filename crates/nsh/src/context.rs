//! The shell instance, and the parameter that carries it.
//!
//! Implements the first half of [dec:nsh:no-ambient-state]: shell state
//! belongs to a shell instance rather than to the process. That decision
//! lands in two steps and this module is the seam between them.
//!
//! `thread-context` — this step — gives every function that touches shell
//! state a `&mut Shell` to reach it through. `move-state` then moves the
//! tables onto this type one at a time, and every function that will need
//! a receiver already has one, so each table moves without a second sweep
//! through the crate's signatures.
//!
//! ## Why the type is empty
//!
//! It has no fields yet, and that is the point rather than an oversight.
//! The state is still in the `static mut`s the literal port inherited from
//! the C; `docs/api-design.md` §5 lists, field by field, what `move-state`
//! will put here. Threading the parameter first and moving the state
//! second keeps each commit small enough to gate against the differential
//! corpus, which a single commit doing both would not be.
//!
//! A function that has been given the context but whose state has not
//! moved yet names it `_sh`. The underscore is the marker for "carries the
//! context, does not read it yet", and it disappears when `move-state`
//! rewrites the body to read a field.
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
/// Empty by construction until `move-state` fills it; see the module
/// documentation for why.
pub struct Shell {
    /// Keeps the type from being constructible outside this module, so
    /// that every instance comes from [`Shell::new`] or
    /// [`Shell::detached`] and the two are countable.
    _private: (),
}

impl Shell {
    /// The shell the process runs as.
    ///
    /// There is one, made at the entry point, and it is threaded down
    /// from there. When `move-state` gives this type fields, this is
    /// where their initial values go — which is what makes it the one
    /// constructor that survives.
    pub(crate) fn new() -> Self {
        Shell { _private: () }
    }

    /// A context for a call site that has not been threaded yet.
    ///
    /// **Transitional.** Every one of these is a remaining edge in the
    /// threading graph: a function that needs the context, called from a
    /// function that does not have one to give it. The count is this
    /// node's progress metric, it only goes down, and the node is
    /// finished when it reaches zero.
    ///
    /// It is sound only because the type is empty. Two `&mut Shell` that
    /// alias would be a bug the moment either one names a field, so this
    /// must be gone *before* `move-state` gives the type any — which is
    /// exactly the order the two nodes are sequenced in.
    pub(crate) fn detached() -> Self {
        Shell { _private: () }
    }
}
