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

    /// A context for a call site that cannot be handed one.
    ///
    /// **Transitional.** Each of these is an edge the threading did not
    /// cross: a function that needs the context, called from one that has
    /// none to give it. The count is this node's progress metric, but it
    /// is not monotonic and expecting it to fall every commit is a
    /// misreading — a boundary moves *outward* as the frontier grows, so
    /// threading a subsystem removes its own sites and can put new ones in
    /// the callers it just reached. It falls to zero as the frontier meets
    /// the edges of the call graph.
    ///
    /// **One kind of site cannot be removed by threading at all**, and it
    /// is the kind that is left: a callback the shell is invoked *through*
    /// rather than called *by*. `parser::getprompt` is handed to the line
    /// editor as a `fn(*mut c_void)`, and a fixed signature has nowhere to
    /// put a receiver however much of the crate is threaded. This is the
    /// same shape as the signal handler `docs/api-design.md` §5.1 excludes
    /// from the state that moves, and it wants the same kind of answer —
    /// a handle the callback carries — which belongs to `public-api`.
    ///
    /// It is sound only because the type is empty. Two `&mut Shell` that
    /// alias would be a bug the moment either one names a field, so the
    /// call sites must be gone — and the callback one answered — *before*
    /// `move-state` gives the type any.
    pub(crate) fn detached() -> Self {
        Shell { _private: () }
    }
}
