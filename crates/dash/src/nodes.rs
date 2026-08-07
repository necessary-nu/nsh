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
//! `Rc::new` of a derived `Clone`.
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
//! Strings are still the C's `char *` into the stack allocator, wrapped in
//! [`NodeText`] only so that `copyfunc` can take the copy the C took with
//! `nodesavestr`. Converting them properly is a later slice of
//! [dec:nsh:owned-data], and [dec:nsh:bytes-not-text] says what they become.
//!
//! `crate::gen::mknodes` is untouched: it is the port of the generator that
//! still emits the C the reference build compiles. It no longer describes
//! this file.

use core::cell::{Cell, OnceCell, RefCell};
use core::ptr;
use std::rc::Rc;

use libc::{c_char, c_int};

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
/// `struct heredoc` hold one handle each. `OnceCell` rather than `RefCell`
/// because the write happens exactly once and readers must be able to hold a
/// plain `&Node` across an expansion that can re-enter the tree.
pub type heredoc_body = Rc<OnceCell<Node>>;

/// The text of a word, a `for` variable or a function name.
///
/// The C stores a bare `char *` that points into the stack allocator, and
/// that is what a freshly parsed tree still holds here. It works because the
/// tree and the text die together, at the `popstackmark` that ends the
/// command.
///
/// `copyfunc` is the one place where they do not die together: a function
/// definition outlives the mark its text was parsed under. So `copynode` did
/// not copy the pointer, it copied the *bytes* — that is what
/// `funcstringsize` measured and `nodesavestr` wrote. Cloning a node
/// therefore has to take ownership of the text, and this is the type that
/// says so.
///
/// The owned arm becomes a `BString` when [dec:nsh:bytes-not-text]'s slice
/// lands; the borrowed arm goes when the stack allocator does.
pub enum NodeText {
    /// into the stack allocator, valid until the enclosing mark pops
    Borrowed(*mut c_char),
    /// a copy of the bytes, NUL included — what `nodesavestr` produced
    Owned(Box<[u8]>),
}

impl NodeText {
    /// The C's `char *`. Callers only read through it; nothing in the shell
    /// writes a word's text after the parser has built the node.
    pub fn as_ptr(&self) -> *mut c_char {
        match self {
            NodeText::Borrowed(p) => *p,
            NodeText::Owned(b) => b.as_ptr() as *mut c_char,
        }
    }
}

impl Clone for NodeText {
    /// `nodesavestr`: copy the bytes, never the pointer.
    fn clone(&self) -> NodeText {
        let p = self.as_ptr();
        unsafe {
            let len = libc::strlen(p);
            let mut v: Vec<u8> = Vec::with_capacity(len + 1);
            ptr::copy_nonoverlapping(p as *const u8, v.as_mut_ptr(), len + 1);
            v.set_len(len + 1);
            NodeText::Owned(v.into_boxed_slice())
        }
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
    pub expfname: Cell<*mut c_char>,
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
            expfname: Cell::new(ptr::null_mut()),
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
        let doc: heredoc_body = Rc::new(OnceCell::new());
        if let Some(n) = self.doc.get() {
            let _ = doc.set(n.clone());
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

/// The C's `struct funcnode { int count; union node n; }`.
///
/// `Rc` *is* `count`: the C starts a fresh copy at `count = 0` meaning one
/// owner and frees at `count < 0`, which is `Rc`'s strong count offset by
/// one. The command table entry that holds it (`exec::tblentry`) is still a
/// `ckmalloc`'d C struct with a flexible array member, so what it stores is
/// the raw form of the `Rc` rather than the `Rc` itself; that goes when
/// `memalloc` does.
pub type funcnode = Node;

/// Make a copy of a parse tree.
///
/// The C measured the tree with `calcsize`, allocated one block for the
/// nodes and the strings together, and laid the copy out inside it with
/// `copynode`/`copystring`. There is nothing left of that: an owned tree
/// clones itself, and one allocation for the whole tree was only ever a
/// consequence of having to free it in one `ckfree`.
pub unsafe fn copyfunc(n: &Node) -> *const funcnode {
    Rc::into_raw(Rc::new(n.clone()))
}

/// `f->count++` — take a second reference to a function that is about to
/// run, so redefining it mid-execution does not pull the body out from under
/// the evaluator.
pub unsafe fn reffunc(f: *const funcnode) {
    if !f.is_null() {
        Rc::increment_strong_count(f);
    }
}

/// Free a parse tree.
pub unsafe fn freefunc(f: *const funcnode) {
    if !f.is_null() {
        Rc::decrement_strong_count(f);
    }
}
