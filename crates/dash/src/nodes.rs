//! Literal port of `src/nodes.c` / `src/nodes.h`.
//!
//! Both files are *generated* at build time by `src/mknodes.c` (see
//! `crate::gen::mknodes`, `docs/spec/port/src/mknodes.md`) from
//! `src/nodetypes` and `src/nodes.c.pat`.  They are not checked-in C source,
//! so nothing here carries `[spec:dash:…]` annotations; only the generator
//! does.
//!
//! The contents below are a transcription of the real generator's output on
//! this tree: the node numbering is positional in `src/nodetypes`, the
//! per-tag structs keep their field order, and `calcsize`/`copynode` walk the
//! tree in exactly the order `mknodes` emits (fields from last down to index
//! 1, skipping field 0 — the `type` field, which `copynode` assigns last).
//!
//! `union node` is modelled as a `#[repr(C)] union`, the direct analogue of
//! the C: the shell relies on every arm sharing an `int type` first member
//! and on `nodesize[]` being the *aligned size of the arm actually in use*,
//! so a Rust enum would change both the layout and the allocation
//! arithmetic in `copyfunc`.

use core::mem::{offset_of, size_of};
use core::ptr;

use libc::{c_char, c_int, c_short, size_t};

use crate::memalloc::{ckfree, ckmalloc};
use crate::shell::pointer;

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

/// Number of node types; the length of `nodesize[]`.
pub const NODE_TYPES: usize = 26;

// ---- the per-tag structs --------------------------------------------

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ncmd {
    pub r#type: c_int,
    pub linno: c_int,
    pub assign: *mut node,
    pub args: *mut node,
    pub redirect: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct npipe {
    pub r#type: c_int,
    pub backgnd: c_int,
    pub cmdlist: *mut nodelist,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nredir {
    pub r#type: c_int,
    pub linno: c_int,
    pub n: *mut node,
    pub redirect: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nbinary {
    pub r#type: c_int,
    pub ch1: *mut node,
    pub ch2: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nif {
    pub r#type: c_int,
    pub test: *mut node,
    pub ifpart: *mut node,
    pub elsepart: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nfor {
    pub r#type: c_int,
    pub linno: c_int,
    pub args: *mut node,
    pub body: *mut node,
    pub var: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ncase {
    pub r#type: c_int,
    pub linno: c_int,
    pub expr: *mut node,
    pub cases: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nclist {
    pub r#type: c_int,
    pub next: *mut node,
    pub pattern: *mut node,
    pub body: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ndefun {
    pub r#type: c_int,
    pub linno: c_int,
    pub text: *mut c_char,
    pub body: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct narg {
    pub r#type: c_int,
    pub next: *mut node,
    pub text: *mut c_char,
    pub backquote: *mut nodelist,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nfile {
    pub r#type: c_int,
    pub next: *mut node,
    pub fd: c_int,
    pub fname: *mut node,
    /// `temp` field: filled in at run time, never copied by `copynode`
    pub expfname: *mut c_char,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct ndup {
    pub r#type: c_int,
    pub next: *mut node,
    pub fd: c_int,
    pub dupfd: c_int,
    pub vname: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nhere {
    pub r#type: c_int,
    pub next: *mut node,
    pub fd: c_int,
    pub doc: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nnot {
    pub r#type: c_int,
    pub com: *mut node,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub union node {
    pub r#type: c_int,
    pub ncmd: ncmd,
    pub npipe: npipe,
    pub nredir: nredir,
    pub nbinary: nbinary,
    pub nif: nif,
    pub nfor: nfor,
    pub ncase: ncase,
    pub nclist: nclist,
    pub ndefun: ndefun,
    pub narg: narg,
    pub nfile: nfile,
    pub ndup: ndup,
    pub nhere: nhere,
    pub nnot: nnot,
}

#[repr(C)]
#[derive(Clone, Copy)]
pub struct nodelist {
    pub next: *mut nodelist,
    pub n: *mut node,
}

#[repr(C)]
pub struct funcnode {
    pub count: c_int,
    pub n: node,
}

// ---- nodes.c ---------------------------------------------------------

/// size of structures in function
pub static mut funcblocksize: c_int = 0;
/// size of strings in node
pub static mut funcstringsize: c_int = 0;
/// block to allocate function from
pub static mut funcblock: pointer = ptr::null_mut();
/// block to allocate strings from
pub static mut funcstring: *mut c_char = ptr::null_mut();

/// `machdep.h`: `SHELL_SIZE` is `sizeof(union {int i; char *cp; double d;}) - 1`.
const SHELL_SIZE: usize = 8 - 1;

/// `machdep.h`: `#define SHELL_ALIGN(nbytes) (((nbytes) + SHELL_SIZE) & ~SHELL_SIZE)`
const fn SHELL_ALIGN(nbytes: usize) -> usize {
    (nbytes + SHELL_SIZE) & !SHELL_SIZE
}

static nodesize: [c_short; NODE_TYPES] = [
    SHELL_ALIGN(size_of::<ncmd>()) as c_short,
    SHELL_ALIGN(size_of::<npipe>()) as c_short,
    SHELL_ALIGN(size_of::<nredir>()) as c_short,
    SHELL_ALIGN(size_of::<nredir>()) as c_short,
    SHELL_ALIGN(size_of::<nredir>()) as c_short,
    SHELL_ALIGN(size_of::<nbinary>()) as c_short,
    SHELL_ALIGN(size_of::<nbinary>()) as c_short,
    SHELL_ALIGN(size_of::<nbinary>()) as c_short,
    SHELL_ALIGN(size_of::<nif>()) as c_short,
    SHELL_ALIGN(size_of::<nbinary>()) as c_short,
    SHELL_ALIGN(size_of::<nbinary>()) as c_short,
    SHELL_ALIGN(size_of::<nfor>()) as c_short,
    SHELL_ALIGN(size_of::<ncase>()) as c_short,
    SHELL_ALIGN(size_of::<nclist>()) as c_short,
    SHELL_ALIGN(size_of::<ndefun>()) as c_short,
    SHELL_ALIGN(size_of::<narg>()) as c_short,
    SHELL_ALIGN(size_of::<nfile>()) as c_short,
    SHELL_ALIGN(size_of::<nfile>()) as c_short,
    SHELL_ALIGN(size_of::<nfile>()) as c_short,
    SHELL_ALIGN(size_of::<nfile>()) as c_short,
    SHELL_ALIGN(size_of::<nfile>()) as c_short,
    SHELL_ALIGN(size_of::<ndup>()) as c_short,
    SHELL_ALIGN(size_of::<ndup>()) as c_short,
    SHELL_ALIGN(size_of::<nhere>()) as c_short,
    SHELL_ALIGN(size_of::<nhere>()) as c_short,
    SHELL_ALIGN(size_of::<nnot>()) as c_short,
];

/*
 * Make a copy of a parse tree.
 */

pub unsafe fn copyfunc(n: *mut node) -> *mut funcnode {
    let f: *mut funcnode;
    let blocksize: size_t;

    funcblocksize = offset_of!(funcnode, n) as c_int;
    funcstringsize = 0;
    calcsize(n);
    blocksize = funcblocksize as size_t;
    f = ckmalloc(blocksize + funcstringsize as size_t) as *mut funcnode;
    funcblock = (f as *mut c_char).offset(offset_of!(funcnode, n) as isize) as pointer;
    funcstring = (f as *mut c_char).offset(blocksize as isize);
    copynode(n);
    (*f).count = 0;
    f
}

unsafe fn calcsize(n: *mut node) {
    if n.is_null() {
        return;
    }
    funcblocksize += nodesize[(*n).r#type as usize] as c_int;
    match (*n).r#type {
        NCMD => {
            calcsize((*n).ncmd.redirect);
            calcsize((*n).ncmd.args);
            calcsize((*n).ncmd.assign);
        }
        NPIPE => {
            sizenodelist((*n).npipe.cmdlist);
        }
        NREDIR | NBACKGND | NSUBSHELL => {
            calcsize((*n).nredir.redirect);
            calcsize((*n).nredir.n);
        }
        NAND | NOR | NSEMI | NWHILE | NUNTIL => {
            calcsize((*n).nbinary.ch2);
            calcsize((*n).nbinary.ch1);
        }
        NIF => {
            calcsize((*n).nif.elsepart);
            calcsize((*n).nif.ifpart);
            calcsize((*n).nif.test);
        }
        NFOR => {
            funcstringsize += libc::strlen((*n).nfor.var) as c_int + 1;
            calcsize((*n).nfor.body);
            calcsize((*n).nfor.args);
        }
        NCASE => {
            calcsize((*n).ncase.cases);
            calcsize((*n).ncase.expr);
        }
        NCLIST => {
            calcsize((*n).nclist.body);
            calcsize((*n).nclist.pattern);
            calcsize((*n).nclist.next);
        }
        NDEFUN => {
            calcsize((*n).ndefun.body);
            funcstringsize += libc::strlen((*n).ndefun.text) as c_int + 1;
        }
        NARG => {
            sizenodelist((*n).narg.backquote);
            funcstringsize += libc::strlen((*n).narg.text) as c_int + 1;
            calcsize((*n).narg.next);
        }
        NTO | NCLOBBER | NFROM | NFROMTO | NAPPEND => {
            calcsize((*n).nfile.fname);
            calcsize((*n).nfile.next);
        }
        NTOFD | NFROMFD => {
            calcsize((*n).ndup.vname);
            calcsize((*n).ndup.next);
        }
        NHERE | NXHERE => {
            calcsize((*n).nhere.doc);
            calcsize((*n).nhere.next);
        }
        NNOT => {
            calcsize((*n).nnot.com);
        }
        _ => {}
    }
}

unsafe fn sizenodelist(mut lp: *mut nodelist) {
    while !lp.is_null() {
        funcblocksize += SHELL_ALIGN(size_of::<nodelist>()) as c_int;
        calcsize((*lp).n);
        lp = (*lp).next;
    }
}

unsafe fn copynode(n: *mut node) -> *mut node {
    let new: *mut node;

    if n.is_null() {
        return ptr::null_mut();
    }
    new = funcblock as *mut node;
    funcblock =
        (funcblock as *mut c_char).offset(nodesize[(*n).r#type as usize] as isize) as pointer;
    match (*n).r#type {
        NCMD => {
            (*new).ncmd.redirect = copynode((*n).ncmd.redirect);
            (*new).ncmd.args = copynode((*n).ncmd.args);
            (*new).ncmd.assign = copynode((*n).ncmd.assign);
            (*new).ncmd.linno = (*n).ncmd.linno;
        }
        NPIPE => {
            (*new).npipe.cmdlist = copynodelist((*n).npipe.cmdlist);
            (*new).npipe.backgnd = (*n).npipe.backgnd;
        }
        NREDIR | NBACKGND | NSUBSHELL => {
            (*new).nredir.redirect = copynode((*n).nredir.redirect);
            (*new).nredir.n = copynode((*n).nredir.n);
            (*new).nredir.linno = (*n).nredir.linno;
        }
        NAND | NOR | NSEMI | NWHILE | NUNTIL => {
            (*new).nbinary.ch2 = copynode((*n).nbinary.ch2);
            (*new).nbinary.ch1 = copynode((*n).nbinary.ch1);
        }
        NIF => {
            (*new).nif.elsepart = copynode((*n).nif.elsepart);
            (*new).nif.ifpart = copynode((*n).nif.ifpart);
            (*new).nif.test = copynode((*n).nif.test);
        }
        NFOR => {
            (*new).nfor.var = nodesavestr((*n).nfor.var);
            (*new).nfor.body = copynode((*n).nfor.body);
            (*new).nfor.args = copynode((*n).nfor.args);
            (*new).nfor.linno = (*n).nfor.linno;
        }
        NCASE => {
            (*new).ncase.cases = copynode((*n).ncase.cases);
            (*new).ncase.expr = copynode((*n).ncase.expr);
            (*new).ncase.linno = (*n).ncase.linno;
        }
        NCLIST => {
            (*new).nclist.body = copynode((*n).nclist.body);
            (*new).nclist.pattern = copynode((*n).nclist.pattern);
            (*new).nclist.next = copynode((*n).nclist.next);
        }
        NDEFUN => {
            (*new).ndefun.body = copynode((*n).ndefun.body);
            (*new).ndefun.text = nodesavestr((*n).ndefun.text);
            (*new).ndefun.linno = (*n).ndefun.linno;
        }
        NARG => {
            (*new).narg.backquote = copynodelist((*n).narg.backquote);
            (*new).narg.text = nodesavestr((*n).narg.text);
            (*new).narg.next = copynode((*n).narg.next);
        }
        NTO | NCLOBBER | NFROM | NFROMTO | NAPPEND => {
            (*new).nfile.fname = copynode((*n).nfile.fname);
            (*new).nfile.fd = (*n).nfile.fd;
            (*new).nfile.next = copynode((*n).nfile.next);
        }
        NTOFD | NFROMFD => {
            (*new).ndup.vname = copynode((*n).ndup.vname);
            (*new).ndup.dupfd = (*n).ndup.dupfd;
            (*new).ndup.fd = (*n).ndup.fd;
            (*new).ndup.next = copynode((*n).ndup.next);
        }
        NHERE | NXHERE => {
            (*new).nhere.doc = copynode((*n).nhere.doc);
            (*new).nhere.fd = (*n).nhere.fd;
            (*new).nhere.next = copynode((*n).nhere.next);
        }
        NNOT => {
            (*new).nnot.com = copynode((*n).nnot.com);
        }
        _ => {}
    }
    (*new).r#type = (*n).r#type;
    new
}

unsafe fn copynodelist(mut lp: *mut nodelist) -> *mut nodelist {
    let mut start: *mut nodelist = ptr::null_mut();
    let mut lpp: *mut *mut nodelist;

    lpp = &mut start;
    while !lp.is_null() {
        *lpp = funcblock as *mut nodelist;
        funcblock = (funcblock as *mut c_char)
            .offset(SHELL_ALIGN(size_of::<nodelist>()) as isize) as pointer;
        (**lpp).n = copynode((*lp).n);
        lp = (*lp).next;
        lpp = &mut (**lpp).next;
    }
    *lpp = ptr::null_mut();
    start
}

unsafe fn nodesavestr(s: *mut c_char) -> *mut c_char {
    let rtn: *mut c_char = funcstring;

    funcstring = libc::stpcpy(funcstring, s).offset(1);
    rtn
}

/*
 * Free a parse tree.
 */

pub unsafe fn freefunc(f: *mut funcnode) {
    if !f.is_null() && {
        (*f).count -= 1;
        (*f).count < 0
    } {
        ckfree(f as pointer);
    }
}

/* The C spells this `union node`; some ported modules refer to it by the
 * Rust-conventional `Node`. Alias rather than rename, so the literal C
 * name stays canonical. */
pub use self::node as Node;
