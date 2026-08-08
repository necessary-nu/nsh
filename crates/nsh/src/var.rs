//! Literal port of `src/var.c` / `src/var.h`.
//! Rules: `docs/spec/port/src/var.md`.
//!
//! Build configuration reproduced here (matching the spec narrative and the
//! Debian build): `ATTY` undefined, `WITH_LINENO` defined, `SMALL` undefined,
//! `DEBUG` defined.  `varinit[]` order is load bearing — `var.h` addresses its
//! entries positionally through the `vifs`/`vmail`/… macros and the `*val()`
//! accessors skip the name by a hard-coded byte count.  Do not reorder.

use libc::{c_char, c_int, c_uint, c_void, intmax_t, size_t};
use core::ptr::{addr_of, addr_of_mut, null_mut};

use crate::error::{INTOFF, INTON};
use crate::memalloc::{ckfree, ckmalloc, savestr, stackstrend};
use crate::mystring::nullstr;
use crate::options::{argptr, getoptsreset, nextopt, optlist, optschanged};
use crate::output::VaArg;
use crate::shell::cstr;
use crate::system::strchrnul;

extern "C" {
    /// `MKINIT char **environ;` — the process environment (unistd.h).
    static mut environ: *mut *mut c_char;
}

/* flags */
pub const VEXPORT: c_int = 0x01; /* variable is exported */
pub const VREADONLY: c_int = 0x02; /* variable cannot be modified */
pub const VSTRFIXED: c_int = 0x04; /* variable struct is statically allocated */
pub const VTEXTFIXED: c_int = 0x08; /* text is statically allocated */
pub const VSTACK: c_int = 0x10; /* text is allocated on the stack */
pub const VUNSET: c_int = 0x20; /* the variable is not set */
pub const VNOFUNC: c_int = 0x40; /* don't call the callback function */
pub const VFULL: c_int = 0x80; /* pass value suitable for putenv */
pub const VNOSAVE: c_int = 0x100; /* when text is on the heap before setvareq */

pub const VTABSIZE: usize = 39;

// [spec:dash:def:var.var.func-fn]
// [spec:dash:sem:var.var.func-fn]
/// `void (*func)(const char *)` — the change-callback member of `struct var`.
/// Invoked whenever the variable is set or unset unless `VNOFUNC` is given;
/// the argument is the value alone, or the whole `"name=value"` when the
/// variable carries `VFULL` (see `varfunc`).  `None` means no callback.
pub type varfunc_t = Option<unsafe fn(*const c_char)>;

// [spec:dash:def:var.var]
#[repr(C)]
pub struct var {
    pub next: *mut var,      /* next entry in hash list */
    pub flags: c_int,        /* flags are defined above */
    pub text: *const c_char, /* name=value */
    pub func: varfunc_t,     /* called when the variable gets set/unset */
}

// [spec:dash:def:var.localvar]
#[repr(C)]
pub struct localvar {
    pub next: *mut localvar, /* next local variable in list */
    pub vp: *mut var,        /* the variable that was made local */
    pub flags: c_int,        /* saved flags */
    pub text: *const c_char, /* saved text */
}

// [spec:dash:def:var.localvar-list]
#[repr(C)]
pub struct localvar_list {
    pub next: *mut localvar_list,
    pub lv: *mut localvar,
}

/* MKINIT struct localvar_list *localvar_stack; */
pub static mut localvar_stack: *mut localvar_list = null_mut();

pub static defpathvar: [c_char; 66] = unsafe {
    core::mem::transmute::<[u8; 66], [c_char; 66]>(
        *b"PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\0",
    )
};
pub static mut defifsvar: [c_char; 8] =
    unsafe { core::mem::transmute::<[u8; 8], [c_char; 8]>(*b"IFS= \t\n\0") };
/* MKINIT char defoptindvar[] = "OPTIND=1"; */
pub static mut defoptindvar: [c_char; 9] =
    unsafe { core::mem::transmute::<[u8; 9], [c_char; 9]>(*b"OPTIND=1\0") };

/// `#define defifs (defifsvar + 4)`
pub unsafe fn defifs() -> *mut c_char {
    (addr_of_mut!(defifsvar) as *mut c_char).add(4)
}
/// `#define defpath (defpathvar + 36)`
pub unsafe fn defpath() -> *const c_char {
    (addr_of!(defpathvar) as *const c_char).add(36)
}

pub static mut lineno: c_int = 0;
/* char linenovar[sizeof("LINENO=") + sizeof(int) * CHAR_BIT / 3 + 1] = "LINENO="; */
pub static mut linenovar: [c_char; 19] = unsafe {
    core::mem::transmute::<[u8; 19], [c_char; 19]>(*b"LINENO=\0\0\0\0\0\0\0\0\0\0\0\0")
};

// [spec:dash:def:var.changelocale-fn]
// [spec:dash:sem:var.changelocale-fn]
unsafe fn changelocale(val: *const c_char) {
    libc::putenv(val as *mut c_char);
    libc::setlocale(libc::LC_ALL, addr_of!(nullstr) as *const c_char);
}

/* Some macros in var.h depend on the order, add new variables to the end. */
pub static mut varinit: [var; 16] = [
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED,
        text: addr_of!(defifsvar) as *const c_char,
        func: Some(crate::expand::changeifs),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VUNSET,
        text: b"MAIL\0\0".as_ptr() as *const c_char,
        func: Some(crate::mail::changemail),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VUNSET,
        text: b"MAILPATH\0\0".as_ptr() as *const c_char,
        func: Some(crate::mail::changemail),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED,
        text: addr_of!(defpathvar) as *const c_char,
        func: Some(crate::exec::changepath),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED,
        text: b"PS1=$ \0".as_ptr() as *const c_char,
        func: None,
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED,
        text: b"PS2=> \0".as_ptr() as *const c_char,
        func: None,
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED,
        text: b"PS4=+ \0".as_ptr() as *const c_char,
        func: None,
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VNOFUNC,
        text: addr_of!(defoptindvar) as *const c_char,
        func: Some(getoptsreset),
    },
    /* #ifdef WITH_LINENO */
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED,
        text: addr_of!(linenovar) as *const c_char,
        func: None,
    },
    /* #ifndef SMALL */
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VUNSET,
        text: b"TERM\0\0".as_ptr() as *const c_char,
        func: None,
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VUNSET,
        text: b"HISTSIZE\0\0".as_ptr() as *const c_char,
        func: Some(crate::histedit::sethistsize),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: b"LC_ALL\0\0".as_ptr() as *const c_char,
        func: Some(changelocale),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: b"LC_COLLATE\0\0".as_ptr() as *const c_char,
        func: Some(changelocale),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: b"LC_CTYPE\0\0".as_ptr() as *const c_char,
        func: Some(changelocale),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: b"LC_NUMERIC\0\0".as_ptr() as *const c_char,
        func: Some(changelocale),
    },
    var {
        next: null_mut(),
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: b"LANG\0\0".as_ptr() as *const c_char,
        func: Some(changelocale),
    },
];

/*
 * Positional accessors into varinit[].  These reproduce the var.h macros
 * literally: `#define vifs varinit[0]`, `#define vmail (&vifs)[1]`, ...
 */
pub unsafe fn vifs() -> *mut var {
    addr_of_mut!(varinit) as *mut var
}
pub unsafe fn vmail() -> *mut var {
    vifs().add(1)
}
pub unsafe fn vmpath() -> *mut var {
    vmail().add(1)
}
pub unsafe fn vpath() -> *mut var {
    vmpath().add(1)
}
pub unsafe fn vps1() -> *mut var {
    vpath().add(1)
}
pub unsafe fn vps2() -> *mut var {
    vps1().add(1)
}
pub unsafe fn vps4() -> *mut var {
    vps2().add(1)
}
pub unsafe fn voptind() -> *mut var {
    vps4().add(1)
}
pub unsafe fn vlineno() -> *mut var {
    voptind().add(1)
}
pub unsafe fn vterm() -> *mut var {
    vlineno().add(1)
}
pub unsafe fn vhistsize() -> *mut var {
    vterm().add(1)
}

/*
 * The following accessors reproduce the var.h value macros.  They have to
 * skip over the name, by a hard-coded byte count.
 */
pub unsafe fn ifsval() -> *const c_char {
    (*vifs()).text.add(4)
}
pub unsafe fn ifsset() -> c_int {
    (((*vifs()).flags & VUNSET) == 0) as c_int
}
pub unsafe fn mailval() -> *const c_char {
    (*vmail()).text.add(5)
}
pub unsafe fn mpathval() -> *const c_char {
    (*vmpath()).text.add(9)
}
pub unsafe fn pathval() -> *const c_char {
    (*vpath()).text.add(5)
}
pub unsafe fn ps1val() -> *const c_char {
    (*vps1()).text.add(4)
}
pub unsafe fn ps2val() -> *const c_char {
    (*vps2()).text.add(4)
}
pub unsafe fn ps4val() -> *const c_char {
    (*vps4()).text.add(4)
}
pub unsafe fn optindval() -> *const c_char {
    (*voptind()).text.add(7)
}
pub unsafe fn linenoval() -> *const c_char {
    (*vlineno()).text.add(7)
}
pub unsafe fn histsizeval() -> *const c_char {
    (*vhistsize()).text.add(9)
}
pub unsafe fn termval() -> *const c_char {
    (*vterm()).text.add(5)
}
pub unsafe fn mpathset() -> c_int {
    (((*vmpath()).flags & VUNSET) == 0) as c_int
}

/// `#define environment() listvars(VEXPORT, VUNSET, 0)`
pub unsafe fn environment() -> *mut *mut c_char {
    listvars(VEXPORT, VUNSET, null_mut())
}

static mut vartab: [*mut var; VTABSIZE] = [null_mut(); VTABSIZE];

// [spec:dash:def:var.hashval-fn]
// [spec:dash:sem:var.hashval-fn]
pub unsafe fn hashval(mut p: *const c_char) -> c_uint {
    let mut hashval: c_uint;

    hashval = ((*p as u8) as c_uint) << 4;
    while *p != 0 {
        hashval = hashval.wrapping_add((*p as u8) as c_uint);
        p = p.add(1);
        if *p == b'=' as c_char {
            break;
        }
    }

    hashval
}

// [spec:dash:def:var.varequal-fn]
// [spec:dash:sem:var.varequal-fn]
pub unsafe fn varequal(a: *const c_char, b: *const c_char) -> c_int {
    (varcmp(a, b) == 0) as c_int
}

/*
 * Search the environment of a builtin command.
 */

// [spec:dash:def:var.bltinlookup-fn]
// [spec:dash:sem:var.bltinlookup-fn]
pub unsafe fn bltinlookup(name: *const c_char) -> *mut c_char {
    lookupvar(name)
}

/*
 * Initialize the varable symbol tables and import the environment
 */

/* mkinit INIT fragment from src/var.c:136-162. */
pub unsafe fn mkinit_init() {
    let mut envp: *mut *mut c_char;
    static mut ppid: [c_char; 32] = unsafe {
        core::mem::transmute::<[u8; 32], [c_char; 32]>(
            *b"PPID=\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        )
    };
    let mut p: *const c_char;
    let mut st1: libc::stat64 = core::mem::zeroed();
    let mut st2: libc::stat64 = core::mem::zeroed();

    initvar();
    envp = environ;
    while !(*envp).is_null() {
        p = crate::parser::endofname(*envp);
        if p != *envp && *p == b'=' as c_char {
            setvareq(*envp, VEXPORT | VTEXTFIXED);
        }
        envp = envp.add(1);
    }

    setvareq(addr_of_mut!(defifsvar) as *mut c_char, VTEXTFIXED);
    setvareq(addr_of_mut!(defoptindvar) as *mut c_char, VTEXTFIXED);

    crate::output::fmtstr(
        (addr_of_mut!(ppid) as *mut c_char).add(5),
        (32 - 5) as size_t,
        cstr(b"%ld\0"),
        &[VaArg::Long(libc::getppid() as libc::c_long)],
    );
    setvareq(addr_of_mut!(ppid) as *mut c_char, VTEXTFIXED);

    p = lookupvar(b"PWD\0".as_ptr() as *const c_char);
    if !p.is_null() {
        if *p != b'/' as c_char
            || libc::stat64(p, &mut st1) != 0
            || libc::stat64(addr_of!(crate::mystring::dotdir) as *const c_char, &mut st2) != 0
            || st1.st_dev != st2.st_dev
            || st1.st_ino != st2.st_ino
        {
            p = null_mut();
        }
    }
    crate::cd::setpwd(p, 0);
}

/* mkinit RESET fragment from src/var.c:164-166. */
pub unsafe fn mkinit_reset() {
    unwindlocalvars(null_mut());
}

// [spec:dash:def:var.varnull-fn]
// [spec:dash:sem:var.varnull-fn]
unsafe fn varnull(s: *const c_char) -> *mut c_char {
    /* Unset variables always end with two NUL chars. */
    strchrnul(s, b'=' as c_int).add(1)
}

// [spec:dash:def:var.varfunc-fn]
// [spec:dash:sem:var.varfunc-fn]
unsafe fn varfunc(vp: *mut var) {
    let mut s: *const c_char;

    if (*vp).func.is_none() {
        return;
    }

    s = (*vp).text;
    if ((*vp).flags & VFULL) == 0 {
        s = varnull(s);
    }
    ((*vp).func.unwrap())(s);
}

/*
 * This routine initializes the builtin variables.  It is called when the
 * shell is initialized.
 */

// [spec:dash:def:var.initvar-fn]
// [spec:dash:sem:var.initvar-fn]
pub unsafe fn initvar() {
    let mut vp: *mut var;
    let end: *mut var;
    let mut vpp: *mut *mut var;

    vp = addr_of_mut!(varinit) as *mut var;
    end = vp.add(16);
    loop {
        vpp = hashvar((*vp).text);
        (*vp).next = *vpp;
        *vpp = vp;
        vp = vp.add(1);
        if !(vp < end) {
            break;
        }
    }
    /*
     * PS1 depends on uid
     */
    if libc::geteuid() == 0 {
        (*vps1()).text = b"PS1=# \0".as_ptr() as *const c_char;
    }
}

/*
 * Set the value of a variable.  The flags argument is ored with the
 * flags of the variable.  If val is NULL, the variable is unset.
 */

// [spec:dash:def:var.setvar-fn]
// [spec:dash:sem:var.setvar-fn]
pub unsafe fn setvar(name: *const c_char, val: *const c_char, mut flags: c_int) -> *mut var {
    let mut p: *mut c_char;
    let q: *mut c_char;
    let namelen: size_t;
    let nameeq: *mut c_char;
    let mut vallen: size_t;
    let vp: *mut var;

    q = crate::parser::endofname(name);
    p = strchrnul(q, b'=' as c_int);
    namelen = (p as usize - name as usize) as size_t;
    if namelen == 0 || p != q {
        /* NB: `namelen` is a size_t handed to a `%.*s` precision, which
         * expects an int.  Reproduced verbatim (src/var.c:231). */
        crate::error::sh_error(
            cstr(b"%.*s: bad variable name\0"),
            &[VaArg::Int(namelen as c_int), VaArg::Str(name)],
        );
    }
    vallen = 0;
    if val.is_null() {
        flags |= VUNSET;
    } else {
        vallen = libc::strlen(val) as size_t;
    }
    INTOFF();
    nameeq = ckmalloc(namelen + vallen + 2) as *mut c_char;
    p = crate::system::mempcpy(nameeq as *mut c_void, name as *const c_void, namelen + 1)
        as *mut c_char;
    if !val.is_null() {
        *p.offset(-1) = b'=' as c_char;
        p = crate::system::mempcpy(p as *mut c_void, val as *const c_void, vallen) as *mut c_char;
    }
    *p = b'\0' as c_char;
    vp = setvareq(nameeq, flags | VNOSAVE);
    INTON();

    vp
}

/*
 * Set the given integer as the value of a variable.  The flags argument is
 * ored with the flags of the variable.
 */

// [spec:dash:def:var.setvarint-fn]
// [spec:dash:sem:var.setvarint-fn]
pub unsafe fn setvarint(name: *const c_char, val: intmax_t, flags: c_int) -> intmax_t {
    let len = crate::shell::max_int_length(core::mem::size_of_val(&val) as c_int);
    /* C declares a VLA `char buf[len]`; max_int_length(8) is 32. */
    let mut buf = [0 as c_char; 32];

    crate::output::fmtstr(
        buf.as_mut_ptr(),
        len as size_t,
        cstr(b"%jd\0"),
        &[VaArg::Intmax(val)],
    );
    setvar(name, buf.as_ptr(), flags);
    val
}

/*
 * Same as setvar except that the variable and value are passed in
 * the first argument as name=value.  Since the first argument will
 * be actually stored in the table, it should not be a string that
 * will go away.
 * Called with interrupts off.
 */

// [spec:dash:def:var.setvareq-fn]
// [spec:dash:sem:var.setvareq-fn]
pub unsafe fn setvareq(mut s: *mut c_char, mut flags: c_int) -> *mut var {
    let mut vp: *mut var;
    let vpp: *mut *mut var;

    flags |= VEXPORT & (((1 - crate::options::optlist[crate::options::aflag] as c_int) as c_uint).wrapping_sub(1)) as c_int;
    vpp = findvar(s);
    vp = *vpp;
    if !vp.is_null() {
        let bits: c_uint;

        if ((*vp).flags & VREADONLY) != 0 {
            let n: *const c_char;

            if (flags & VNOSAVE) != 0 {
                libc::free(s as *mut c_void);
            }
            n = (*vp).text;
            crate::error::sh_error(
                cstr(b"%.*s: is read only\0"),
                &[
                    VaArg::Int((strchrnul(n, b'=' as c_int) as usize - n as usize) as c_int),
                    VaArg::Str(n),
                ],
            );
        }

        if ((*vp).flags & (VTEXTFIXED | VSTACK)) == 0 {
            ckfree((*vp).text as *mut c_void);
        }

        if (flags & (VEXPORT | VREADONLY | VSTRFIXED | VUNSET)) != VUNSET {
            bits = !((VTEXTFIXED | VSTACK | VNOSAVE | VUNSET) as c_uint);
        } else if ((*vp).flags & VSTRFIXED) != 0 {
            bits = VSTRFIXED as c_uint;
        } else {
            *vpp = (*vp).next;
            ckfree(vp as *mut c_void);
            /* out_free: */
            if (flags & (VTEXTFIXED | VSTACK | VNOSAVE)) == VNOSAVE {
                ckfree(s as *mut c_void);
            }
            /* goto out — NB `vp` has just been freed and is returned
             * dangling, exactly as the C does (src/var.c:304-309, 331). */
            return vp;
        }

        flags |= (*vp).flags & bits as c_int;
    } else {
        if (flags & (VEXPORT | VREADONLY | VSTRFIXED | VUNSET)) == VUNSET {
            /* goto out_free */
            if (flags & (VTEXTFIXED | VSTACK | VNOSAVE)) == VNOSAVE {
                ckfree(s as *mut c_void);
            }
            return vp;
        }
        /* not found */
        vp = ckmalloc(core::mem::size_of::<var>() as size_t) as *mut var;
        (*vp).next = *vpp;
        (*vp).func = None;
        *vpp = vp;
    }
    if (flags & (VTEXTFIXED | VSTACK | VNOSAVE)) == 0 {
        s = savestr(s);
    }
    (*vp).text = s;
    (*vp).flags = flags;

    if (flags & VNOFUNC) == 0 {
        varfunc(vp);
    }

    vp
}

/*
 * Find the value of a variable.  Returns NULL if not set.
 */

// [spec:dash:def:var.lookupvar-fn]
// [spec:dash:sem:var.lookupvar-fn]
pub unsafe fn lookupvar(name: *const c_char) -> *mut c_char {
    let v: *mut var;

    v = *findvar(name);
    if !v.is_null() && ((*v).flags & VUNSET) == 0 {
        /* #ifdef WITH_LINENO */
        if v == vlineno() && (*v).text == addr_of!(linenovar) as *const c_char {
            crate::output::fmtstr(
                (addr_of_mut!(linenovar) as *mut c_char).add(7),
                (19 - 7) as size_t,
                cstr(b"%d\0"),
                &[VaArg::Int(lineno)],
            );
        }
        return strchrnul((*v).text, b'=' as c_int).add(1);
    }
    null_mut()
}

// [spec:dash:def:var.lookupvarint-fn]
// [spec:dash:sem:var.lookupvarint-fn]
pub unsafe fn lookupvarint(name: *const c_char) -> intmax_t {
    let p = lookupvar(name);
    crate::mystring::atomax(
        if !p.is_null() {
            p as *const c_char
        } else {
            addr_of!(nullstr) as *const c_char
        },
        0,
    )
}

/*
 * Generate a list of variables satisfying the given conditions.
 */

// [spec:dash:def:var.listvars-fn]
// [spec:dash:sem:var.listvars-fn]
pub unsafe fn listvars(on: c_int, off: c_int, end: *mut *mut *mut c_char) -> *mut *mut c_char {
    let mut vpp: *mut *mut var;
    let mut vp: *mut var;
    let mut ep: *mut *mut c_char;
    let mask: c_int;

    crate::STARTSTACKSTR!(ep);
    vpp = addr_of_mut!(vartab) as *mut *mut var;
    mask = on | off;
    loop {
        vp = *vpp;
        while !vp.is_null() {
            if ((*vp).flags & mask) == on {
                if ep as *mut c_void == stackstrend() {
                    ep = crate::memalloc::growstackstr() as *mut *mut c_char;
                }
                *ep = (*vp).text as *mut c_char;
                ep = ep.add(1);
            }
            vp = (*vp).next;
        }
        vpp = vpp.add(1);
        if !(vpp < (addr_of_mut!(vartab) as *mut *mut var).add(VTABSIZE)) {
            break;
        }
    }
    if ep as *mut c_void == stackstrend() {
        ep = crate::memalloc::growstackstr() as *mut *mut c_char;
    }
    if !end.is_null() {
        *end = ep;
    }
    *ep = null_mut();
    ep = ep.add(1);
    crate::memalloc::grabstackstr(ep as *mut c_void) as *mut *mut c_char
}

/*
 * POSIX requires that 'set' (but not export or readonly) output the
 * variables in lexicographic order - by the locale's collating order (sigh).
 * Maybe we could keep them in an ordered balanced binary tree
 * instead of hashed lists.
 * For now just roll 'em through qsort for printing...
 */

// [spec:dash:def:var.showvars-fn]
// [spec:dash:sem:var.showvars-fn]
pub unsafe fn showvars(prefix: *const c_char, on: c_int, off: c_int) -> c_int {
    let sep: *const c_char;
    let mut ep: *mut *mut c_char;
    let mut epend: *mut *mut c_char = null_mut();

    ep = listvars(on, off, &mut epend);
    libc::qsort(
        ep as *mut c_void,
        ((epend as usize - ep as usize) / core::mem::size_of::<*mut c_char>()) as size_t,
        core::mem::size_of::<*mut c_char>() as size_t,
        Some(vpcmp),
    );

    sep = if *prefix != 0 {
        addr_of!(crate::mystring::spcstr) as *const c_char
    } else {
        prefix
    };

    while ep < epend {
        let mut p: *const c_char;
        let mut q: *const c_char;

        p = strchrnul(*ep, b'=' as c_int);
        q = addr_of!(nullstr) as *const c_char;
        if *p != 0 {
            p = p.add(1);
            q = crate::mystring::single_quote(p);
        }

        crate::output::out1fmt(
            cstr(b"%s%s%.*s%s\n\0"),
            &[
                VaArg::Str(prefix),
                VaArg::Str(sep),
                VaArg::Int((p as usize - *ep as usize) as c_int),
                VaArg::Str(*ep),
                VaArg::Str(q),
            ],
        );
        ep = ep.add(1);
    }

    0
}

/*
 * The export and readonly commands.
 */

// [spec:dash:def:var.exportcmd-fn]
// [spec:dash:sem:var.exportcmd-fn]
pub unsafe fn exportcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut vp: *mut var;
    let mut name: *mut c_char;
    let mut p: *const c_char;
    let mut aptr: *mut *mut c_char;
    let flag: c_int = if **argv.offset(0) == b'r' as c_char {
        VREADONLY
    } else {
        VEXPORT
    };
    let notp: c_int;

    notp = nextopt(b"p\0".as_ptr() as *const c_char) - b'p' as c_int;
    aptr = argptr;
    name = *aptr;
    if notp != 0 && !name.is_null() {
        loop {
            p = libc::strchr(name, b'=' as c_int);
            if !p.is_null() {
                p = p.add(1);
            } else {
                vp = *findvar(name);
                if !vp.is_null() {
                    (*vp).flags |= flag;
                    /* continue */
                    aptr = aptr.add(1);
                    name = *aptr;
                    if name.is_null() {
                        break;
                    }
                    continue;
                }
            }
            setvar(name, p, flag);
            aptr = aptr.add(1);
            name = *aptr;
            if name.is_null() {
                break;
            }
        }
    } else {
        showvars(*argv.offset(0), flag, 0);
    }
    0
}

/*
 * The "local" command.
 */

// [spec:dash:def:var.localcmd-fn]
// [spec:dash:sem:var.localcmd-fn]
pub unsafe fn localcmd(argc: c_int, mut argv: *mut *mut c_char) -> c_int {
    let mut name: *mut c_char;

    if localvar_stack.is_null() {
        crate::error::sh_error(cstr(b"not in a function\0"), &[]);
    }

    argv = argptr;
    loop {
        name = *argv;
        argv = argv.add(1);
        if name.is_null() {
            break;
        }
        mklocal(name, 0);
    }
    0
}

/*
 * Make a variable a local variable.  When a variable is made local, it's
 * value and flags are saved in a localvar structure.  The saved values
 * will be restored when the shell function returns.  We handle the name
 * "-" as a special case.
 */

// [spec:dash:def:var.mklocal-fn]
// [spec:dash:sem:var.mklocal-fn]
pub unsafe fn mklocal(name: *mut c_char, flags: c_int) {
    let lvp: *mut localvar;
    let vp: *mut var;

    INTOFF();
    lvp = ckmalloc(core::mem::size_of::<localvar>() as size_t) as *mut localvar;
    if *name.offset(0) == b'-' as c_char && *name.offset(1) == b'\0' as c_char {
        let p: *mut c_char;
        p = ckmalloc(core::mem::size_of_val(&optlist) as size_t) as *mut c_char;
        (*lvp).text = libc::memcpy(
            p as *mut c_void,
            addr_of!(optlist) as *const c_void,
            core::mem::size_of_val(&optlist) as size_t,
        ) as *const c_char;
        vp = null_mut();
    } else {
        let eq: *mut c_char;
        let found: *mut var;

        found = *findvar(name);
        eq = libc::strchr(name, b'=' as c_int);
        if found.is_null() {
            if !eq.is_null() {
                vp = setvareq(name, VSTRFIXED | flags);
            } else {
                vp = setvar(name, null_mut(), VSTRFIXED | flags);
            }
            /* NB: lvp->text is left uninitialised on this path — safe only
             * because lvp->flags == VUNSET makes poplocalvars ignore it. */
            (*lvp).flags = VUNSET;
        } else {
            vp = found;
            (*lvp).text = (*vp).text;
            (*lvp).flags = (*vp).flags;
            (*vp).flags |= VSTRFIXED | VTEXTFIXED;
            if !eq.is_null() {
                setvareq(name, flags);
            }
        }
    }
    (*lvp).vp = vp;
    (*lvp).next = (*localvar_stack).lv;
    (*localvar_stack).lv = lvp;
    INTON();
}

/*
 * Called after a function returns.
 * Interrupts must be off.
 */

// [spec:dash:def:var.poplocalvars-fn]
// [spec:dash:sem:var.poplocalvars-fn]
unsafe fn poplocalvars() {
    let ll: *mut localvar_list;
    let mut lvp: *mut localvar;
    let mut next: *mut localvar;
    let mut vp: *mut var;

    INTOFF();
    ll = localvar_stack;
    localvar_stack = (*ll).next;

    next = (*ll).lv;
    ckfree(ll as *mut c_void);

    loop {
        lvp = next;
        if lvp.is_null() {
            break;
        }
        next = (*lvp).next;
        vp = (*lvp).vp;
        /* `TRACE(("poplocalvar %s\n", vp ? vp->text : "-"));` — `#ifdef
         * DEBUG` in `shell.h`, and the dash build does not define it. */
        if vp.is_null() {
            /* $- saved */
            libc::memcpy(
                addr_of_mut!(optlist) as *mut c_void,
                (*lvp).text as *const c_void,
                core::mem::size_of_val(&optlist) as size_t,
            );
            ckfree((*lvp).text as *mut c_void);
            optschanged();
        } else if (*lvp).flags == VUNSET {
            (*vp).flags &= !(VSTRFIXED | VREADONLY);
            unsetvar((*vp).text);
        } else {
            if ((*vp).flags & (VTEXTFIXED | VSTACK)) == 0 {
                ckfree((*vp).text as *mut c_void);
            }
            (*vp).flags = (*lvp).flags;
            (*vp).text = (*lvp).text;
            if ((*vp).flags & VNOFUNC) == 0 {
                varfunc(vp);
            }
        }
        ckfree(lvp as *mut c_void);
    }
    INTON();
}

/*
 * Create a new localvar environment.
 */

// [spec:dash:def:var.pushlocalvars-fn]
// [spec:dash:sem:var.pushlocalvars-fn]
pub unsafe fn pushlocalvars(push: c_int) -> *mut localvar_list {
    let ll: *mut localvar_list;
    let top: *mut localvar_list;

    top = localvar_stack;
    if push == 0 {
        return top; /* goto out */
    }

    INTOFF();
    ll = ckmalloc(core::mem::size_of::<localvar_list>() as size_t) as *mut localvar_list;
    (*ll).lv = null_mut();
    (*ll).next = top;
    localvar_stack = ll;
    INTON();

    top
}

// [spec:dash:def:var.unwindlocalvars-fn]
// [spec:dash:sem:var.unwindlocalvars-fn]
pub unsafe fn unwindlocalvars(stop: *mut localvar_list) {
    while localvar_stack != stop {
        poplocalvars();
    }
}

/*
 * The unset builtin command.  We unset the function before we unset the
 * variable to allow a function to be unset when there is a readonly variable
 * with the same name.
 */

// [spec:dash:def:var.unsetcmd-fn]
// [spec:dash:sem:var.unsetcmd-fn]
pub unsafe fn unsetcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut ap: *mut *mut c_char;
    let mut i: c_int;
    let mut flag: c_int = 0;

    loop {
        i = nextopt(b"vf\0".as_ptr() as *const c_char);
        if i == b'\0' as c_int {
            break;
        }
        flag = i;
    }

    ap = argptr;
    while !(*ap).is_null() {
        if flag != b'f' as c_int {
            unsetvar(*ap);
            ap = ap.add(1);
            continue;
        }
        if flag != b'v' as c_int {
            crate::exec::unsetfunc(*ap);
        }
        ap = ap.add(1);
    }
    0
}

/*
 * Unset the specified variable.
 */

// [spec:dash:def:var.unsetvar-fn]
// [spec:dash:sem:var.unsetvar-fn]
pub unsafe fn unsetvar(s: *const c_char) {
    setvar(s, null_mut(), 0);
}

/*
 * Find the appropriate entry in the hash table from the name.
 */

// [spec:dash:def:var.hashvar-fn]
// [spec:dash:sem:var.hashvar-fn]
unsafe fn hashvar(p: *const c_char) -> *mut *mut var {
    (addr_of_mut!(vartab) as *mut *mut var).add((hashval(p) % VTABSIZE as c_uint) as usize)
}

/*
 * Compares two strings up to the first = or '\0'.  The first
 * string must be terminated by '='; the second may be terminated by
 * either '=' or '\0'.
 */

// [spec:dash:def:var.varcmp-fn]
// [spec:dash:sem:var.varcmp-fn]
pub unsafe fn varcmp(mut p: *const c_char, mut q: *const c_char) -> c_int {
    let mut c: c_int = *p as c_int;
    let mut d: c_int = *q as c_int;
    while c == d {
        if c == 0 {
            break;
        }
        p = p.add(1);
        q = q.add(1);
        c = *p as c_int;
        d = *q as c_int;
        if c == b'=' as c_int {
            c = b'\0' as c_int;
        }
        if d == b'=' as c_int {
            d = b'\0' as c_int;
        }
    }
    c - d
}

// [spec:dash:def:var.vpcmp-fn]
// [spec:dash:sem:var.vpcmp-fn]
unsafe extern "C" fn vpcmp(a: *const c_void, b: *const c_void) -> c_int {
    varcmp(*(a as *const *const c_char), *(b as *const *const c_char))
}

// [spec:dash:def:var.findvar-fn]
// [spec:dash:sem:var.findvar-fn]
unsafe fn findvar(name: *const c_char) -> *mut *mut var {
    let mut vpp: *mut *mut var;

    vpp = hashvar(name);
    while !(*vpp).is_null() {
        if varequal((**vpp).text, name) != 0 {
            break;
        }
        vpp = addr_of_mut!((**vpp).next);
    }
    vpp
}
