//! Literal port of `src/mknodes.c`.
//! Rules: `docs/spec/port/src/mknodes.md`.
//!
//! This program reads the nodetypes file and nodes.c.pat file.  It generates
//! the files nodes.h and nodes.c.
//!
//! It is a build-time code generator, not part of the shell.  The port keeps
//! the C's globals, `char *` buffers and pointer arithmetic, so it is
//! `unsafe` throughout.  Two unavoidable renames: `struct str` becomes `str_`
//! (a Rust `struct str` would shadow the primitive `str` for the whole
//! module) and `error` takes an already-formatted message, because Rust
//! cannot receive C varargs.
//!
//! It stays because it still describes something real: `src/Makefile.am`
//! builds `nodes.c` and `nodes.h` with `src/mknodes.c` from `src/nodetypes`,
//! and that is what the C reference the differential harness compares against
//! is built from.  What it no longer describes is `crate::nodes`.
//! [dec:nsh:owned-data] made the Rust parse tree an owned enum with `Vec`
//! children and an `Rc` for `copyfunc`; `%SIZES`, `%CALCSIZE` and `%COPY` —
//! the three things this program emits — have no counterpart there, because
//! `nodesize[]`, `calcsize` and `copynode` were consequences of allocating a
//! tree out of one block.  So `crate::nodes` is hand-written from here on,
//! and this file is a port of the C generator, not the source of that one.

use core::ptr;

use libc::{c_char, c_int, FILE};

extern "C" {
    /// glibc's `stdin`; `main` seeds `infp` with it.
    static mut stdin: *mut FILE;
}

const MAXTYPES: usize = 50; /* max number of node types */
const MAXFIELDS: usize = 20; /* max fields in a structure */
const BUFLEN: usize = 100; /* size of character buffers */

/* field types */
const T_NODE: c_int = 1; /* union node *field */
const T_NODELIST: c_int = 2; /* struct nodelist *field */
const T_STRING: c_int = 3;
const T_INT: c_int = 4; /* int field */
const T_OTHER: c_int = 5; /* other */
const T_TEMP: c_int = 6; /* don't copy this field */

// [spec:dash:def:mknodes.field]
/// a structure field
#[derive(Clone, Copy)]
struct field {
    name: *mut c_char, /* name of field */
    r#type: c_int,     /* type of field */
    decl: *mut c_char, /* declaration of field */
}

const FIELD_INIT: field = field {
    name: ptr::null_mut(),
    r#type: 0,
    decl: ptr::null_mut(),
};

// [spec:dash:def:mknodes.str]
/// struct representing a node structure (C: `struct str`)
#[derive(Clone, Copy)]
struct str_ {
    tag: *mut c_char,          /* structure tag */
    nfields: c_int,            /* number of fields in the structure */
    field: [field; MAXFIELDS], /* the fields of the structure */
    done: c_int,               /* set if fully parsed */
}

const STR_INIT: str_ = str_ {
    tag: ptr::null_mut(),
    nfields: 0,
    field: [FIELD_INIT; MAXFIELDS],
    done: 0,
};

static mut ntypes: c_int = 0; /* number of node types */
static mut nodename: [*mut c_char; MAXTYPES] = [ptr::null_mut(); MAXTYPES]; /* names of the nodes */
static mut nodestr: [*mut str_; MAXTYPES] = [ptr::null_mut(); MAXTYPES]; /* type of structure used by the node */
static mut nstr: c_int = 0; /* number of structures */
static mut str_: [str_; MAXTYPES] = [STR_INIT; MAXTYPES]; /* the structures */
static mut curstr: *mut str_ = ptr::null_mut(); /* current structure */
static mut infp: *mut FILE = ptr::null_mut();
static mut line: [c_char; 1024] = [0; 1024];
static mut linno: c_int = 0;
static mut linep: *mut c_char = ptr::null_mut();

// [spec:dash:def:mknodes.main-fn]
// [spec:dash:sem:mknodes.main-fn]
pub fn main_fn(argc: c_int, argv: Vec<String>) -> c_int {
    unsafe {
        /*
         * some versions of linux complain: initializer element is not
         * constant if this is done at compile time.
         */
        infp = stdin;

        if argc != 3 {
            error("usage: mknodes file");
        }
        let argv1 = cstring(&argv[1]);
        infp = libc::fopen(argv1.as_ptr(), c"r".as_ptr());
        if infp.is_null() {
            error(&format!("Can't open {}", argv[1]));
        }
        while readline() != 0 {
            if line[0] == b' ' as c_char || line[0] == b'\t' as c_char {
                parsefield();
            } else if line[0] != b'\0' as c_char {
                parsenode();
            }
        }
        output(&argv[2]);
        libc::exit(0);
        /* NOTREACHED */
    }
}

// [spec:dash:def:mknodes.parsenode-fn]
// [spec:dash:sem:mknodes.parsenode-fn]
unsafe fn parsenode() {
    let mut name: [c_char; BUFLEN] = [0; BUFLEN];
    let mut tag: [c_char; BUFLEN] = [0; BUFLEN];
    let mut sp: *mut str_;

    if !curstr.is_null() && (*curstr).nfields > 0 {
        (*curstr).done = 1;
    }
    nextfield(name.as_mut_ptr());
    if nextfield(tag.as_mut_ptr()) == 0 {
        error("Tag expected");
    }
    if *linep != b'\0' as c_char {
        error("Garbage at end of line");
    }
    nodename[ntypes as usize] = savestr(name.as_ptr());
    sp = ptr::addr_of_mut!(str_) as *mut str_;
    while sp < (ptr::addr_of_mut!(str_) as *mut str_).add(nstr as usize) {
        if libc::strcmp((*sp).tag, tag.as_ptr()) == 0 {
            break;
        }
        sp = sp.add(1);
    }
    if sp >= (ptr::addr_of_mut!(str_) as *mut str_).add(nstr as usize) {
        (*sp).tag = savestr(tag.as_ptr());
        (*sp).nfields = 0;
        curstr = sp;
        nstr += 1;
    }
    nodestr[ntypes as usize] = sp;
    ntypes += 1;
}

// [spec:dash:def:mknodes.parsefield-fn]
// [spec:dash:sem:mknodes.parsefield-fn]
unsafe fn parsefield() {
    let mut name: [c_char; BUFLEN] = [0; BUFLEN];
    let mut r#type: [c_char; BUFLEN] = [0; BUFLEN];
    let mut decl: [c_char; 2 * BUFLEN] = [0; 2 * BUFLEN];
    let fp: *mut field;

    if curstr.is_null() || (*curstr).done != 0 {
        error("No current structure to add field to");
    }
    if nextfield(name.as_mut_ptr()) == 0 {
        error("No field name");
    }
    if nextfield(r#type.as_mut_ptr()) == 0 {
        error("No field type");
    }
    fp = &mut (*curstr).field[(*curstr).nfields as usize];
    (*fp).name = savestr(name.as_ptr());
    if libc::strcmp(r#type.as_ptr(), c"nodeptr".as_ptr()) == 0 {
        (*fp).r#type = T_NODE;
        libc::snprintf(
            decl.as_mut_ptr(),
            decl.len(),
            c"union node *%s".as_ptr(),
            name.as_ptr(),
        );
    } else if libc::strcmp(r#type.as_ptr(), c"nodelist".as_ptr()) == 0 {
        (*fp).r#type = T_NODELIST;
        libc::snprintf(
            decl.as_mut_ptr(),
            decl.len(),
            c"struct nodelist *%s".as_ptr(),
            name.as_ptr(),
        );
    } else if libc::strcmp(r#type.as_ptr(), c"string".as_ptr()) == 0 {
        (*fp).r#type = T_STRING;
        libc::snprintf(
            decl.as_mut_ptr(),
            decl.len(),
            c"char *%s".as_ptr(),
            name.as_ptr(),
        );
    } else if libc::strcmp(r#type.as_ptr(), c"int".as_ptr()) == 0 {
        (*fp).r#type = T_INT;
        libc::snprintf(
            decl.as_mut_ptr(),
            decl.len(),
            c"int %s".as_ptr(),
            name.as_ptr(),
        );
    } else if libc::strcmp(r#type.as_ptr(), c"other".as_ptr()) == 0 {
        (*fp).r#type = T_OTHER;
    } else if libc::strcmp(r#type.as_ptr(), c"temp".as_ptr()) == 0 {
        (*fp).r#type = T_TEMP;
    } else {
        error(&format!(
            "Unknown type {}",
            core::ffi::CStr::from_ptr(r#type.as_ptr()).to_string_lossy()
        ));
    }
    if (*fp).r#type == T_OTHER || (*fp).r#type == T_TEMP {
        skipbl();
        (*fp).decl = savestr(linep);
    } else {
        if *linep != 0 {
            error("Garbage at end of line");
        }
        (*fp).decl = savestr(decl.as_ptr());
    }
    (*curstr).nfields += 1;
}

static writer: &core::ffi::CStr =
    c"/*\n * This file was generated by the mknodes program.\n */\n\n";

// [spec:dash:def:mknodes.output-fn]
// [spec:dash:sem:mknodes.output-fn]
unsafe fn output(file: &str) {
    let hfile: *mut FILE;
    let cfile: *mut FILE;
    let patfile: *mut FILE;
    let mut i: c_int;
    let mut sp: *mut str_;
    let mut fp: *mut field;
    let mut p: *mut c_char;

    let cfilename = cstring(file);
    patfile = libc::fopen(cfilename.as_ptr(), c"r".as_ptr());
    if patfile.is_null() {
        error(&format!("Can't open {}", file));
    }
    hfile = libc::fopen(c"nodes.h".as_ptr(), c"w".as_ptr());
    if hfile.is_null() {
        error("Can't create nodes.h");
    }
    cfile = libc::fopen(c"nodes.c".as_ptr(), c"w".as_ptr());
    if cfile.is_null() {
        error("Can't create nodes.c");
    }
    libc::fputs(writer.as_ptr(), hfile);
    i = 0;
    while i < ntypes {
        libc::fprintf(hfile, c"#define %s %d\n".as_ptr(), nodename[i as usize], i);
        i += 1;
    }
    libc::fputs(c"\n\n\n".as_ptr(), hfile);
    sp = ptr::addr_of_mut!(str_) as *mut str_;
    while sp < (ptr::addr_of_mut!(str_) as *mut str_).add(nstr as usize) {
        libc::fprintf(hfile, c"struct %s {\n".as_ptr(), (*sp).tag);
        i = (*sp).nfields;
        fp = (*sp).field.as_mut_ptr();
        loop {
            i -= 1;
            if i < 0 {
                break;
            }
            libc::fprintf(hfile, c"      %s;\n".as_ptr(), (*fp).decl);
            fp = fp.add(1);
        }
        libc::fputs(c"};\n\n\n".as_ptr(), hfile);
        sp = sp.add(1);
    }
    libc::fputs(c"union node {\n".as_ptr(), hfile);
    libc::fprintf(hfile, c"      int type;\n".as_ptr());
    sp = ptr::addr_of_mut!(str_) as *mut str_;
    while sp < (ptr::addr_of_mut!(str_) as *mut str_).add(nstr as usize) {
        libc::fprintf(
            hfile,
            c"      struct %s %s;\n".as_ptr(),
            (*sp).tag,
            (*sp).tag,
        );
        sp = sp.add(1);
    }
    libc::fputs(c"};\n\n\n".as_ptr(), hfile);
    libc::fputs(c"struct nodelist {\n".as_ptr(), hfile);
    libc::fputs(c"\tstruct nodelist *next;\n".as_ptr(), hfile);
    libc::fputs(c"\tunion node *n;\n".as_ptr(), hfile);
    libc::fputs(c"};\n\n\n".as_ptr(), hfile);
    libc::fputs(c"struct funcnode {\n".as_ptr(), hfile);
    libc::fputs(c"\tint count;\n".as_ptr(), hfile);
    libc::fputs(c"\tunion node n;\n".as_ptr(), hfile);
    libc::fputs(c"};\n\n\n".as_ptr(), hfile);
    libc::fputs(c"struct funcnode *copyfunc(union node *);\n".as_ptr(), hfile);
    libc::fputs(c"void freefunc(struct funcnode *);\n".as_ptr(), hfile);

    libc::fputs(writer.as_ptr(), cfile);
    while !libc::fgets(ptr::addr_of_mut!(line) as *mut c_char, 1024, patfile).is_null() {
        p = ptr::addr_of_mut!(line) as *mut c_char;
        while *p == b' ' as c_char || *p == b'\t' as c_char {
            p = p.add(1);
        }
        if libc::strcmp(p, c"%SIZES\n".as_ptr()) == 0 {
            outsizes(cfile);
        } else if libc::strcmp(p, c"%CALCSIZE\n".as_ptr()) == 0 {
            outfunc(cfile, 1);
        } else if libc::strcmp(p, c"%COPY\n".as_ptr()) == 0 {
            outfunc(cfile, 0);
        } else {
            libc::fputs(ptr::addr_of!(line) as *const c_char, cfile);
        }
    }
}

// [spec:dash:def:mknodes.outsizes-fn]
// [spec:dash:sem:mknodes.outsizes-fn]
unsafe fn outsizes(cfile: *mut FILE) {
    let mut i: c_int;

    libc::fprintf(
        cfile,
        c"static const short nodesize[%d] = {\n".as_ptr(),
        ntypes,
    );
    i = 0;
    while i < ntypes {
        libc::fprintf(
            cfile,
            c"      SHELL_ALIGN(sizeof (struct %s)),\n".as_ptr(),
            (*nodestr[i as usize]).tag,
        );
        i += 1;
    }
    libc::fprintf(cfile, c"};\n".as_ptr());
}

// [spec:dash:def:mknodes.outfunc-fn]
// [spec:dash:sem:mknodes.outfunc-fn]
unsafe fn outfunc(cfile: *mut FILE, calcsize: c_int) {
    let mut sp: *mut str_;
    let mut fp: *mut field;
    let mut i: c_int;

    libc::fputs(c"      if (n == NULL)\n".as_ptr(), cfile);
    if calcsize != 0 {
        libc::fputs(c"\t    return;\n".as_ptr(), cfile);
    } else {
        libc::fputs(c"\t    return NULL;\n".as_ptr(), cfile);
    }
    if calcsize != 0 {
        libc::fputs(c"      funcblocksize += nodesize[n->type];\n".as_ptr(), cfile);
    } else {
        libc::fputs(c"      new = funcblock;\n".as_ptr(), cfile);
        libc::fputs(
            c"      funcblock = (char *) funcblock + nodesize[n->type];\n".as_ptr(),
            cfile,
        );
    }
    libc::fputs(c"      switch (n->type) {\n".as_ptr(), cfile);
    sp = ptr::addr_of_mut!(str_) as *mut str_;
    while sp < (ptr::addr_of_mut!(str_) as *mut str_).add(nstr as usize) {
        i = 0;
        while i < ntypes {
            if nodestr[i as usize] == sp {
                libc::fprintf(cfile, c"      case %s:\n".as_ptr(), nodename[i as usize]);
            }
            i += 1;
        }
        i = (*sp).nfields;
        loop {
            i -= 1;
            if i < 1 {
                break;
            }
            fp = &mut (*sp).field[i as usize];
            match (*fp).r#type {
                T_NODE => {
                    if calcsize != 0 {
                        indent(12, cfile);
                        libc::fprintf(
                            cfile,
                            c"calcsize(n->%s.%s);\n".as_ptr(),
                            (*sp).tag,
                            (*fp).name,
                        );
                    } else {
                        indent(12, cfile);
                        libc::fprintf(
                            cfile,
                            c"new->%s.%s = copynode(n->%s.%s);\n".as_ptr(),
                            (*sp).tag,
                            (*fp).name,
                            (*sp).tag,
                            (*fp).name,
                        );
                    }
                }
                T_NODELIST => {
                    if calcsize != 0 {
                        indent(12, cfile);
                        libc::fprintf(
                            cfile,
                            c"sizenodelist(n->%s.%s);\n".as_ptr(),
                            (*sp).tag,
                            (*fp).name,
                        );
                    } else {
                        indent(12, cfile);
                        libc::fprintf(
                            cfile,
                            c"new->%s.%s = copynodelist(n->%s.%s);\n".as_ptr(),
                            (*sp).tag,
                            (*fp).name,
                            (*sp).tag,
                            (*fp).name,
                        );
                    }
                }
                T_STRING => {
                    if calcsize != 0 {
                        indent(12, cfile);
                        libc::fprintf(
                            cfile,
                            c"funcstringsize += strlen(n->%s.%s) + 1;\n".as_ptr(),
                            (*sp).tag,
                            (*fp).name,
                        );
                    } else {
                        indent(12, cfile);
                        libc::fprintf(
                            cfile,
                            c"new->%s.%s = nodesavestr(n->%s.%s);\n".as_ptr(),
                            (*sp).tag,
                            (*fp).name,
                            (*sp).tag,
                            (*fp).name,
                        );
                    }
                }
                T_INT | T_OTHER => {
                    if calcsize == 0 {
                        indent(12, cfile);
                        libc::fprintf(
                            cfile,
                            c"new->%s.%s = n->%s.%s;\n".as_ptr(),
                            (*sp).tag,
                            (*fp).name,
                            (*sp).tag,
                            (*fp).name,
                        );
                    }
                }
                _ => {}
            }
        }
        indent(12, cfile);
        libc::fputs(c"break;\n".as_ptr(), cfile);
        sp = sp.add(1);
    }
    libc::fputs(c"      };\n".as_ptr(), cfile);
    if calcsize == 0 {
        libc::fputs(c"      new->type = n->type;\n".as_ptr(), cfile);
    }
}

// [spec:dash:def:mknodes.indent-fn]
// [spec:dash:sem:mknodes.indent-fn]
unsafe fn indent(mut amount: c_int, fp: *mut FILE) {
    while amount >= 8 {
        libc::fputc(b'\t' as c_int, fp);
        amount -= 8;
    }
    loop {
        amount -= 1;
        if amount < 0 {
            break;
        }
        libc::fputc(b' ' as c_int, fp);
    }
}

// [spec:dash:def:mknodes.nextfield-fn]
// [spec:dash:sem:mknodes.nextfield-fn]
unsafe fn nextfield(buf: *mut c_char) -> c_int {
    let mut p: *mut c_char;
    let mut q: *mut c_char;

    p = linep;
    while *p == b' ' as c_char || *p == b'\t' as c_char {
        p = p.add(1);
    }
    q = buf;
    while *p != b' ' as c_char && *p != b'\t' as c_char && *p != b'\0' as c_char {
        *q = *p;
        q = q.add(1);
        p = p.add(1);
    }
    *q = b'\0' as c_char;
    linep = p;
    (q > buf) as c_int
}

// [spec:dash:def:mknodes.skipbl-fn]
// [spec:dash:sem:mknodes.skipbl-fn]
unsafe fn skipbl() {
    while *linep == b' ' as c_char || *linep == b'\t' as c_char {
        linep = linep.add(1);
    }
}

// [spec:dash:def:mknodes.readline-fn]
// [spec:dash:sem:mknodes.readline-fn]
unsafe fn readline() -> c_int {
    let mut p: *mut c_char;

    if libc::fgets(ptr::addr_of_mut!(line) as *mut c_char, 1024, infp).is_null() {
        return 0;
    }
    p = ptr::addr_of_mut!(line) as *mut c_char;
    while *p != b'#' as c_char && *p != b'\n' as c_char && *p != b'\0' as c_char {
        p = p.add(1);
    }
    while p > ptr::addr_of_mut!(line) as *mut c_char
        && (*p.offset(-1) == b' ' as c_char || *p.offset(-1) == b'\t' as c_char)
    {
        p = p.offset(-1);
    }
    *p = b'\0' as c_char;
    linep = ptr::addr_of_mut!(line) as *mut c_char;
    linno += 1;
    if p.offset_from(ptr::addr_of!(line) as *const c_char) > BUFLEN as isize {
        error("Line too long");
    }
    1
}

// [spec:dash:def:mknodes.error-fn]
// [spec:dash:sem:mknodes.error-fn]
/// The C is `error(const char *msg, ...)`; Rust cannot receive C varargs, so
/// callers format the message themselves.
fn error(msg: &str) -> ! {
    unsafe {
        eprint!("line {}: ", ptr::addr_of!(linno).read());
    }
    eprint!("{}", msg);
    eprintln!();

    unsafe { libc::exit(2) }
    /* NOTREACHED */
}

// [spec:dash:def:mknodes.savestr-fn]
// [spec:dash:sem:mknodes.savestr-fn]
unsafe fn savestr(s: *const c_char) -> *mut c_char {
    let p: *mut c_char;

    p = libc::malloc(libc::strlen(s) + 1) as *mut c_char;
    if p.is_null() {
        error("Out of space");
    }
    libc::strcpy(p, s);
    p
}

/// Adapter: the C works in `char *` throughout, so `argv` strings have to be
/// NUL-terminated before they can be handed to `fopen`.
fn cstring(s: &str) -> std::ffi::CString {
    std::ffi::CString::new(s).unwrap()
}
