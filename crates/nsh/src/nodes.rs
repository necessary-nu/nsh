//! The shell's parse tree.
//!
//! `src/nodes.c` and `src/nodes.h` are generated at build time by
//! `src/mknodes.c` from `src/nodetypes` and `src/nodes.c.pat`, and the first
//! port of this module was a transcription of that output: a `#[repr(C)]
//! union node` whose arms shared an `int type` first member, a `nodesize[]`
//! table indexed by that member, and a `copyfunc` that measured a tree with
//! `calcsize` and then laid a copy of it out inside one `ckmalloc`'d block,
//! refcounted through `struct funcnode { int count; union node n; }`.
//!
//! Every part of that existed because the tree lived in the stack allocator
//! and C has neither destructors nor a discriminated union.
//! [dec:nsh:owned-data] takes the allocator away, so what is here now is an
//! owned Rust tree: children are `Option<Box<Node>>`, the `next`-linked
//! sibling lists are `Vec<Node>`, and the whole deep-copy apparatus is
//! a derived `Clone`.
//!
//! Two things the C's layout did that an enum cannot, and how they are
//! spelled here instead:
//!
//!   * **Several node types share one arm.** `NREDIR`/`NBACKGND`/`NSUBSHELL`
//!     are all `struct nredir`, and `list()` *relabels* a node from one to
//!     another in place. The arms that cover more than one type therefore
//!     keep the `type` field the C had, and [`Node::node_type`] recovers the
//!     number every `switch (n->type)` in the shell still switches on.
//!   * **Two fields are written at run time, not at parse time.**
//!     `nfile.expfname` (the C marks it `temp`, so `copynode` never copied
//!     it) and `ndup.dupfd` are filled in by `expredir` on a tree that may
//!     be a shared function definition. They are `Cell`s.
//!
//! A word's text is an owned [`NodeText`], which is a `BString` carrying the
//! parser's trailing NUL ([dec:nsh:bytes-not-text]). The C kept a `char *`
//! into the stack allocator and `copyfunc` called `nodesavestr` on every one
//! of them; owning the bytes at parse time makes that copy the ordinary
//! `Clone` and removes the distinction between a tree the allocator still
//! backs and a tree that outlived its mark.
//!
//! `src/mknodes.c` still generates the `nodes.c`/`nodes.h` that the C
//! reference is built from, but nothing it emits — `nodesize[]`, `calcsize`,
//! `copynode` — has a counterpart here, so the port of that generator went
//! with the layout it described.

use core::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex, PoisonError};

use bstr::{BStr, BString};
use core::ffi::c_int;

// ---- node types (positional in src/nodetypes) ------------------------

/// a simple command
pub const NCMD: c_int = 0;
/// a pipeline
pub const NPIPE: c_int = 1;
/// redirection (of a complex command)
pub const NREDIR: c_int = 2;
/// run command in background
pub const NBACKGND: c_int = 3;
/// run command in a subshell
pub const NSUBSHELL: c_int = 4;
/// the && operator
pub const NAND: c_int = 5;
/// the || operator
pub const NOR: c_int = 6;
/// two commands separated by a semicolon
pub const NSEMI: c_int = 7;
/// the if statement
pub const NIF: c_int = 8;
/// the while statement
pub const NWHILE: c_int = 9;
/// the until statement
pub const NUNTIL: c_int = 10;
/// the for statement
pub const NFOR: c_int = 11;
/// a case statement
pub const NCASE: c_int = 12;
/// a case
pub const NCLIST: c_int = 13;
/// a function
pub const NDEFUN: c_int = 14;
/// represents a word
pub const NARG: c_int = 15;
/// fd> fname
pub const NTO: c_int = 16;
/// fd>| fname
pub const NCLOBBER: c_int = 17;
/// fd< fname
pub const NFROM: c_int = 18;
/// fd<> fname
pub const NFROMTO: c_int = 19;
/// fd>> fname
pub const NAPPEND: c_int = 20;
/// fd<&dupfd
pub const NTOFD: c_int = 21;
/// fd>&dupfd
pub const NFROMFD: c_int = 22;
/// fd<<\! (unexpanded here-document)
pub const NHERE: c_int = 23;
/// fd<<! (expanded here-document)
pub const NXHERE: c_int = 24;
/// ! command  (actually pipeline)
pub const NNOT: c_int = 25;

// ---- the per-tag structs --------------------------------------------

/// The slot a here-document body lands in.
///
/// A here-document is read *after* its redirection node is already buried in
/// a command: `parseredir` builds the node, and `parseheredoc` — which runs
/// at the next newline — supplies the text. The C did that with a back
/// pointer out of `struct heredoc` into the tree (`here->here->nhere.doc =
/// n`). A back pointer into an owned tree is the one thing Rust will not
/// give you, so the *slot* is shared instead: the node and the pending
/// `struct heredoc` hold one handle each. The body is filled exactly once,
/// then readers take an owned snapshot before an expansion can re-enter the
/// tree. The mutex makes sharing the delayed write safe without preventing a
/// complete [`Shell`](crate::context::Shell) from moving to another thread.
#[allow(non_camel_case_types)]
#[derive(Clone)]
pub struct heredoc_body(Arc<Mutex<Option<Node>>>);

impl heredoc_body {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    pub fn fill(&self, node: Node) {
        let mut body = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(body.is_none(), "a here-document body is filled only once");
        *body = Some(node);
    }

    pub fn snapshot(&self) -> Option<Node> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

/// The text of a word, a `for` variable or a function name.
///
/// The C stores a bare `char *` into the stack allocator, which works only
/// because the tree and the text die together at the `popstackmark` that ends
/// the command. `copyfunc` is where they do not: a function definition
/// outlives the mark its text was parsed under, so `copynode` copied the
/// *bytes* — `funcstringsize` measured them and `nodesavestr` wrote them.
///
/// Here the bytes are owned from the moment the parser produces them, so both
/// cases are the same case and `Clone` is derived.
///
/// The trailing NUL the parser wrote is *part of the value*; `as_bstr`
/// hides it from readers that operate on a length.
#[derive(Clone)]
pub struct NodeText(BString);

impl NodeText {
    /// Take ownership of a word's bytes. `text` ends in the NUL the parser
    /// wrote, which is part of the value.
    pub fn new(text: BString) -> NodeText {
        NodeText(text)
    }

    /* The `from_ptr` constructor is gone with the `strlen` it was built
     * around. Its one caller in the shell is `parser.rs` handing `for` an
     * implicit `"$@"`, and `dolatstr` is a fixed seven bytes ending in
     * the NUL, so the walk was answering a question the static had
     * already answered. */

    /// The text without its terminating NUL.
    pub fn as_bstr(&self) -> &BStr {
        BStr::new(&self.0[..self.0.len() - 1])
    }

    /// The text **with** its terminating NUL, which is what a reader that
    /// walks it as the C does needs: the terminator is the stop condition,
    /// and `argstr` counts it into the run it appends.
    pub fn as_cbytes(&self) -> &[u8] {
        &self.0
    }
}

/// `NCMD`
#[derive(Clone)]
pub struct ncmd {
    pub linno: c_int,
    /// variable assignments (C: a `narg.next`-linked list)
    pub assign: Vec<Node>,
    /// the arguments (C: a `narg.next`-linked list)
    pub args: Vec<Node>,
    /// list of file redirections (C: an `nfile.next`-linked list)
    pub redirect: Vec<Node>,
}

/// `NPIPE`
#[derive(Clone)]
pub struct npipe {
    pub backgnd: c_int,
    /// the commands in the pipeline (C: `struct nodelist *`)
    pub cmdlist: Vec<Node>,
}

/// `NREDIR`, `NBACKGND`, `NSUBSHELL`
#[derive(Clone)]
pub struct nredir {
    pub r#type: c_int,
    pub linno: c_int,
    pub n: Option<Box<Node>>,
    pub redirect: Vec<Node>,
}

/// `NAND`, `NOR`, `NSEMI`, `NWHILE`, `NUNTIL`
#[derive(Clone)]
pub struct nbinary {
    pub r#type: c_int,
    pub ch1: Option<Box<Node>>,
    pub ch2: Option<Box<Node>>,
}

/// `NIF`
#[derive(Clone)]
pub struct nif {
    pub test: Option<Box<Node>>,
    pub ifpart: Option<Box<Node>>,
    pub elsepart: Option<Box<Node>>,
}

/// `NFOR`
#[derive(Clone)]
pub struct nfor {
    pub linno: c_int,
    pub args: Vec<Node>,
    pub body: Option<Box<Node>>,
    pub var: NodeText,
}

/// `NCASE`
#[derive(Clone)]
pub struct ncase {
    pub linno: c_int,
    pub expr: Option<Box<Node>>,
    /// the list of cases (C: an `nclist.next`-linked list of NCLIST nodes)
    pub cases: Vec<Node>,
}

/// `NCLIST`
#[derive(Clone)]
pub struct nclist {
    /// list of patterns for this case (C: a `narg.next`-linked list)
    pub pattern: Vec<Node>,
    pub body: Option<Box<Node>>,
}

/// `NDEFUN`
#[derive(Clone)]
pub struct ndefun {
    pub linno: c_int,
    pub text: NodeText,
    pub body: Option<Box<Node>>,
}

/// `NARG`
#[derive(Clone)]
pub struct narg {
    pub text: NodeText,
    /// list of commands in back quotes (C: `struct nodelist *`).  An entry is
    /// `None` where the C stored a null `n`, which is what `$( )` parses to.
    pub backquote: Vec<Option<Node>>,
}

/// `NTO`, `NCLOBBER`, `NFROM`, `NFROMTO`, `NAPPEND`
pub struct nfile {
    pub r#type: c_int,
    /// file descriptor being redirected
    pub fd: c_int,
    /// file name, in a NARG node
    pub fname: Option<Box<Node>>,
    /// actual file name — the C's `temp` field: written by `expredir` before
    /// every use and never copied by `copynode`, so it is interior-mutable
    /// here rather than part of the tree's value.
    ///
    /// The C stores `fn.list->text`, a pointer into the region that stays
    /// valid until `evalcommand`'s `popstackmark`. The node owns the bytes
    /// instead, which is the same lifetime said without the allocator:
    /// `redirect` runs while the node is alive, and nothing between
    /// `expredir` and it can free the word. `None` is the C's null — the
    /// value the field has before `expredir` has ever written it, which is
    /// not the same as an empty file name (`> ""` is a real redirection).
    pub expfname: RefCell<Option<BString>>,
}

impl nfile {
    /// The expanded redirection target as owned shell bytes, without its
    /// storage terminator. Ownership lets opening the path re-enter shell
    /// code without retaining a `RefCell` borrow or a raw pointer.
    pub fn expanded_filename(&self) -> BString {
        let mut name = self
            .expfname
            .borrow()
            .as_ref()
            .expect("expredir fills every file redirection target")
            .clone();
        debug_assert_eq!(name.last(), Some(&0), "expfname is a C string");
        name.pop();
        name
    }

}

impl Clone for nfile {
    /// `expfname` is the C's `temp` field: `copynode` skips it, so a copied
    /// node inherits whatever was in the block. `expredir` writes it before
    /// every use, so what it starts as does not matter; null is the value an
    /// owned node can state.
    fn clone(&self) -> nfile {
        nfile {
            r#type: self.r#type,
            fd: self.fd,
            fname: self.fname.clone(),
            expfname: RefCell::new(None),
        }
    }
}

/// `NTOFD`, `NFROMFD`
#[derive(Clone)]
pub struct ndup {
    pub r#type: c_int,
    /// file descriptor being redirected
    pub fd: c_int,
    /// file descriptor to duplicate — rewritten by `expredir`/`fixredir`
    /// each time the redirection is performed, so interior-mutable.
    pub dupfd: Cell<c_int>,
    /// file name if `fd>&$var`; `fixredir` clears it at parse time.
    pub vname: RefCell<Option<Box<Node>>>,
}

/// `NHERE`, `NXHERE`
pub struct nhere {
    pub r#type: c_int,
    /// file descriptor being redirected
    pub fd: c_int,
    /// input to command (NARG node), filled in by `parseheredoc`
    pub doc: heredoc_body,
}

impl Clone for nhere {
    /// `new->nhere.doc = copynode(n->nhere.doc)`. The slot is shared with a
    /// `struct heredoc` only so `parseheredoc` can reach it; a *copy* of the
    /// node needs its own body, not a second handle on this one — otherwise
    /// the copy would keep pointing at text in the stack allocator.
    fn clone(&self) -> nhere {
        let doc = heredoc_body::new();
        if let Some(n) = self.doc.snapshot() {
            doc.fill(n);
        }
        nhere {
            r#type: self.r#type,
            fd: self.fd,
            doc,
        }
    }
}

/// `NNOT`
#[derive(Clone)]
pub struct nnot {
    pub com: Option<Box<Node>>,
}

/// The C's `union node`.
#[derive(Clone)]
pub enum Node {
    Cmd(ncmd),
    Pipe(npipe),
    Redir(nredir),
    Binary(nbinary),
    If(nif),
    For(nfor),
    Case(ncase),
    Clist(nclist),
    Defun(ndefun),
    Arg(narg),
    File(nfile),
    Dup(ndup),
    Here(nhere),
    Not(nnot),
}

/* The C spells this `union node`; ported modules refer to it by both names. */
pub type node = Node;

/// Names a `union node` arm that the node is not.
///
/// Reached only where the C would have read one arm's fields through
/// another's — a type pun on a node type that cannot occur at that point.
/// Every such site in the shell is a `switch` default that C lets fall into
/// the next `case`; see the comments there.
#[cold]
#[inline(never)]
fn wrong_arm(want: &str) -> ! {
    panic!("node is not a {want}");
}

impl Node {
    /// The `int type` first member every arm of the C's union shared.
    pub fn node_type(&self) -> c_int {
        match self {
            Node::Cmd(_) => NCMD,
            Node::Pipe(_) => NPIPE,
            Node::Redir(n) => n.r#type,
            Node::Binary(n) => n.r#type,
            Node::If(_) => NIF,
            Node::For(_) => NFOR,
            Node::Case(_) => NCASE,
            Node::Clist(_) => NCLIST,
            Node::Defun(_) => NDEFUN,
            Node::Arg(_) => NARG,
            Node::File(n) => n.r#type,
            Node::Dup(n) => n.r#type,
            Node::Here(n) => n.r#type,
            Node::Not(_) => NNOT,
        }
    }

    pub fn ncmd(&self) -> &ncmd {
        match self {
            Node::Cmd(n) => n,
            _ => wrong_arm("ncmd"),
        }
    }

    pub fn npipe(&self) -> &npipe {
        match self {
            Node::Pipe(n) => n,
            _ => wrong_arm("npipe"),
        }
    }

    pub fn npipe_mut(&mut self) -> &mut npipe {
        match self {
            Node::Pipe(n) => n,
            _ => wrong_arm("npipe"),
        }
    }

    pub fn nredir(&self) -> &nredir {
        match self {
            Node::Redir(n) => n,
            _ => wrong_arm("nredir"),
        }
    }

    pub fn nredir_mut(&mut self) -> &mut nredir {
        match self {
            Node::Redir(n) => n,
            _ => wrong_arm("nredir"),
        }
    }

    pub fn nbinary(&self) -> &nbinary {
        match self {
            Node::Binary(n) => n,
            _ => wrong_arm("nbinary"),
        }
    }

    pub fn nif(&self) -> &nif {
        match self {
            Node::If(n) => n,
            _ => wrong_arm("nif"),
        }
    }

    pub fn nfor(&self) -> &nfor {
        match self {
            Node::For(n) => n,
            _ => wrong_arm("nfor"),
        }
    }

    pub fn ncase(&self) -> &ncase {
        match self {
            Node::Case(n) => n,
            _ => wrong_arm("ncase"),
        }
    }

    pub fn nclist(&self) -> &nclist {
        match self {
            Node::Clist(n) => n,
            _ => wrong_arm("nclist"),
        }
    }

    pub fn ndefun(&self) -> &ndefun {
        match self {
            Node::Defun(n) => n,
            _ => wrong_arm("ndefun"),
        }
    }

    pub fn narg(&self) -> &narg {
        match self {
            Node::Arg(n) => n,
            _ => wrong_arm("narg"),
        }
    }

    /// Consume the node for its `narg`. `simplecmd` needs this where the C
    /// relabelled an NARG node NDEFUN in place and kept its `text`.
    pub fn into_narg(self) -> narg {
        match self {
            Node::Arg(n) => n,
            _ => wrong_arm("narg"),
        }
    }

    /// The `nfile` view. Where the C reads `n->nfile.fd` on a redirection
    /// that is not one — `fd` sits at the same offset in `nfile`, `ndup` and
    /// `nhere` — use [`Node::redir_fd`] instead.
    pub fn nfile(&self) -> &nfile {
        match self {
            Node::File(n) => n,
            _ => wrong_arm("nfile"),
        }
    }

    pub fn nfile_mut(&mut self) -> &mut nfile {
        match self {
            Node::File(n) => n,
            _ => wrong_arm("nfile"),
        }
    }

    pub fn ndup(&self) -> &ndup {
        match self {
            Node::Dup(n) => n,
            _ => wrong_arm("ndup"),
        }
    }

    pub fn nhere(&self) -> &nhere {
        match self {
            Node::Here(n) => n,
            _ => wrong_arm("nhere"),
        }
    }

    pub fn nhere_mut(&mut self) -> &mut nhere {
        match self {
            Node::Here(n) => n,
            _ => wrong_arm("nhere"),
        }
    }

    pub fn nnot(&self) -> &nnot {
        match self {
            Node::Not(n) => n,
            _ => wrong_arm("nnot"),
        }
    }

    /// `n->nfile.fd` for any redirection node — the C reads it through the
    /// `nfile` arm whatever the redirection actually is, because `fd` sits
    /// at the same offset in `nfile`, `ndup` and `nhere`.
    pub fn redir_fd(&self) -> c_int {
        match self {
            Node::File(n) => n.fd,
            Node::Dup(n) => n.fd,
            Node::Here(n) => n.fd,
            _ => wrong_arm("redirection"),
        }
    }
}

// ---- nodes.c ---------------------------------------------------------

/// Compatibility name for a stored function body. The command table owns the
/// node directly; there is no separate allocation header.
pub type funcnode = Node;

// ---------------------------------------------------------------------
// A word's bytes are not text, and the trailing NUL is part of the value.
// Both of those are load-bearing for every reader that is still C-shaped,
// and neither is visible to the differential harness as anything other
// than "the shell changed".
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::parser::{CTLENDVAR, CTLQUOTEMARK, CTLVAR};

    /// The bytes `"$x"` leaves the parser as: they are invalid UTF-8 by
    /// construction, so nothing on this path may validate them as text.
    fn quoted_var() -> BString {
        BString::from(vec![
            CTLQUOTEMARK as u8,
            CTLVAR as u8,
            b'x',
            CTLENDVAR as u8,
            CTLQUOTEMARK as u8,
            0,
        ])
    }

    #[test]
    fn node_text_keeps_the_in_band_control_bytes() {
        let t = NodeText::new(quoted_var());
        assert_eq!(t.as_bstr(), &quoted_var()[..5]);
        assert!(core::str::from_utf8(t.as_bstr()).is_err());
        assert_eq!(t.as_cbytes().len(), 6);
        assert_eq!(t.as_cbytes()[0], CTLQUOTEMARK as u8);
    }

    #[test]
    fn node_text_keeps_its_terminator() {
        // `as_bstr` stops one short of the end, so a value built from
        // bytes that already carry their NUL reads back without it -- and
        // the storage still carries one.
        let src = quoted_var();
        let t = NodeText::new(BString::from(&src[..]));
        assert_eq!(t.as_bstr(), &src[..5]);
        assert_eq!(t.as_cbytes()[5], 0);
    }

    #[test]
    fn cloning_a_node_copies_the_bytes_rather_than_sharing_them() {
        // `copyfunc` is why this matters: a function definition outlives
        // the text it was parsed from, so `copynode` called `nodesavestr`.
        let n = Node::Arg(narg {
            text: NodeText::new(quoted_var()),
            backquote: Vec::new(),
        });
        let copy = n.clone();
        assert_eq!(copy.narg().text.as_bstr(), n.narg().text.as_bstr());
        assert_ne!(copy.narg().text.as_cbytes().as_ptr(), n.narg().text.as_cbytes().as_ptr());
    }

    #[test]
    fn a_word_may_contain_a_nul_the_terminator_does_not_hide() {
        // A raw NUL byte reaches the parser from the input, so the value is
        // its bytes and not what a C reader makes of them.
        let t = NodeText::new(BString::from(vec![b'a', 0, b'b', 0]));
        assert_eq!(t.as_bstr(), b"a\0b".as_slice());
        assert_eq!(t.as_cbytes().iter().position(|byte| *byte == 0), Some(1));
    }

    /// `SHELL_ALIGN(sizeof(union node))` for every node struct, so the
    /// figures can be diffed against the C.
    ///
    /// This was `examples/nodesizes.rs`, whose own first line called it a
    /// temporary check and not part of the shell. It could not stay: an
    /// example is a separate crate, so it needed `nodes` to be `pub`, and
    /// the surface closure is exactly the commit that stops an internal
    /// measurement from holding a module open. Run it for the numbers with
    /// `cargo test -p nsh --lib node_sizes -- --nocapture`.
    #[test]
    fn node_sizes_are_printable_for_a_diff_against_the_c() {
        const fn align(n: usize) -> usize {
            (n + 7) & !7
        }
        macro_rules! p {
            ($t:ty, $n:expr) => {
                println!("{} {}", $n, align(core::mem::size_of::<$t>()))
            };
        }
        p!(ncmd, "ncmd");
        p!(npipe, "npipe");
        p!(nredir, "nredir");
        p!(nbinary, "nbinary");
        p!(nif, "nif");
        p!(nfor, "nfor");
        p!(ncase, "ncase");
        p!(nclist, "nclist");
        p!(narg, "narg");
        p!(nfile, "nfile");
        p!(ndup, "ndup");
        p!(nhere, "nhere");
        p!(nnot, "nnot");
        println!("node {}", core::mem::size_of::<Node>());
    }
}
