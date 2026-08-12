//! Literal port of `src/var.c` / `src/var.h`.
//! Rules: `docs/spec/port/src/var.md`.
//!
//! Build configuration reproduced here (matching the spec narrative and the
//! Debian build): `ATTY` undefined, `WITH_LINENO` defined, `SMALL` undefined,
//! `DEBUG` defined.  `varinit[]` order is load bearing — `var.h` addresses its
//! entries positionally through the `vifs`/`vmail`/… macros and the `*val()`
//! accessors skip the name by a hard-coded byte count.  Do not reorder.
//!
//! One deliberate departure from the C, registered in `docs/divergences.md`:
//! `vartab` is a `BTreeMap` keyed by variable name, not 39 chained hash
//! buckets.  `listvars` therefore yields the variables in name order, and
//! since its result *is* `execve`'s `envp`, a child's environment is sorted
//! too.  dash walks the buckets, so its order is an artefact of
//! `hashval` — neither sorted nor insertion order.  POSIX constrains
//! neither, and it requires `set` to print sorted, which the container now
//! gives for free instead of a `qsort` at print time.

use crate::error::Error;
use bstr::{BStr, BString};
use core::ptr::{addr_of, addr_of_mut, null_mut};
use libc::{c_char, c_int, c_uint, intmax_t, size_t};
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::io::Write as _;

use crate::error::{INTOFF, INTON};
use crate::mystring::nullstr;
use crate::options::{NOPTS, Options, getoptsreset, optlist, optschanged};
use crate::system::strchrnul;

unsafe extern "C" {
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

// [spec:dash:def:var.var.func-fn]
// [spec:dash:sem:var.var.func-fn]
/// `void (*func)(const char *)` — the change-callback member of `struct var`.
/// Invoked whenever the variable is set or unset unless `VNOFUNC` is given;
/// the argument is the value alone, or the whole `"name=value"` when the
/// variable carries `VFULL` (see `varfunc`).  `None` means no callback.
pub type varfunc_t = Option<unsafe fn(*const c_char)>;

/// A variable's `NAME=value` bytes, and who they belong to.
///
/// The C keeps one `const char *` and answers "who frees it" from
/// `flags & (VTEXTFIXED|VSTACK)`: clear means the shell allocated it,
/// set means it is a `static` in this module or an `environ` entry the
/// process was started with. The two answers are the two variants, and
/// the flag bit is now a description of the type rather than a stand-in
/// for it — [`setvareq_text`] asserts they agree.
///
/// `Box<[u8]>` rather than a growable buffer, because the address has to
/// hold still. `listvars` hands these pointers to `execve` as `envp`, and
/// `changelocale` hands one to `putenv`, which glibc stores without
/// copying; a buffer that could reallocate would invalidate both.
enum VarText {
    Fixed(*const c_char),
    /// NUL-terminated, and the terminator is counted.
    Owned(Box<[u8]>),
}

impl VarText {
    /// `vp->text`, as the `char *` every reader wants.
    #[inline]
    fn as_ptr(&self) -> *const c_char {
        match self {
            VarText::Fixed(p) => *p,
            VarText::Owned(b) => {
                debug_assert_eq!(b.last(), Some(&0), "a variable is a C string");
                b.as_ptr() as *const c_char
            }
        }
    }
}

// [spec:dash:def:var.var]
/// The C carries a `struct var *next` here, the link in the hash chain.
/// `vartab` is an ordered map now, so the link is the map's business and the
/// field is gone; the struct is still separately allocated and never moved,
/// because `localvar.vp` holds its address across a whole function call.
pub struct var {
    pub flags: c_int,    /* flags are defined above */
    text: VarText,       /* name=value */
    pub func: varfunc_t, /* called when the variable gets set/unset */
}

// [spec:dash:def:var.localvar]
/// What one `local` declaration has to give back.
///
/// The C's `next` is gone — the chain is the `Vec` inside
/// [`localvar_list`]. What is left is three different records sharing one
/// struct, which `poplocalvars` tells apart by testing two fields in
/// order: `vp == NULL` means the saved option vector is in `text`, then
/// `flags == VUNSET` means the variable did not exist and `text` was never
/// written. Splitting them into variants means the field that must not be
/// read on a path is not there to read.
///
/// The sentinel is sound because no variable in the table can have flags
/// exactly `VUNSET`: `setvareq` stores an entry only when the incoming or
/// inherited flags carry one of `VEXPORT|VREADONLY|VSTRFIXED` as well.
enum localvar {
    /// `local -`: `optlist` copied whole. The C `ckmalloc`s
    /// `sizeof(optlist)` and keeps the copy in `text`; the array is a
    /// fixed-size `Copy` value, so it is simply held.
    Options([c_char; NOPTS]),
    /// The variable was not in the table. `mklocal` created it and
    /// `poplocalvars` takes it back out.
    Unset { vp: *mut var },
    /// The variable was in the table; these are its flags and its text
    /// from before `local` named it. The save *owns* the text — `vp`
    /// borrows it until `poplocalvars` hands it back, which is what the
    /// C's `vp->flags |= VTEXTFIXED` was saying.
    Saved {
        vp: *mut var,
        flags: c_int,
        text: VarText,
    },
}

// [spec:dash:def:var.localvar-list]
/// One function invocation's worth of saves.
///
/// `mklocal` pushes at the head of the C's list and `poplocalvars` walks
/// it from the head, so the C restores in reverse order of declaration —
/// which is what makes `local x; local x=2` end up back at the outermost
/// save. A `Vec` drained from the back does the same.
pub struct localvar_list {
    lv: Vec<localvar>,
}

/* MKINIT struct localvar_list *localvar_stack; */
/// The C's `next` chain, outermost first. A frame's index in this stack is
/// what `pushlocalvars` hands back and `unwindlocalvars` unwinds to, in
/// place of the address of the frame below it.
pub static mut localvar_stack: Vec<localvar_list> = Vec::new();

#[inline]
pub(crate) unsafe fn localvar_stack_mut() -> &'static mut Vec<localvar_list> {
    &mut *addr_of_mut!(localvar_stack)
}

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
pub static mut linenovar: [c_char; 19] =
    unsafe { core::mem::transmute::<[u8; 19], [c_char; 19]>(*b"LINENO=\0\0\0\0\0\0\0\0\0\0\0\0") };

// [spec:dash:def:var.changelocale-fn]
// [spec:dash:sem:var.changelocale-fn]
/// The C is `putenv(val); setlocale(LC_ALL, "")`, and the `putenv` is a
/// use-after-free in both shells.
///
/// glibc's `putenv` stores the caller's pointer in `environ` rather than
/// copying it. `varfunc` passes `vp->text`, which this crate owns — a
/// `Box<[u8]>` since [dec:nsh:owned-data] — so reassigning any of the
/// five locale variables drops the box while `environ` still points into
/// it, and the next `setlocale` reads freed memory. `setenv` copies, so
/// `environ` holds storage glibc owns and the lifetime question does not
/// arise. dash keeps the defect; we do not, per
/// [dec:nsh:we-own-the-defects].
///
/// The unset path cannot be fixed here, for a reason that is not visible
/// from inside this function. `setvareq` keeps only
/// `VSTRFIXED` when it unsets an entry (`var.c:317`), so `VFULL` is gone
/// by the time `varfunc` runs, `varfunc` takes its `varnull` branch, and
/// what arrives is the empty string — the *value* of an unset variable,
/// with no name attached. `putenv("")` fails `EINVAL`, which is why dash
/// leaves `environ` holding the old entry after `unset LC_ALL`, and why
/// the locale does not revert. Reproducing that is deliberate: the
/// observable behaviour is dash's, and changing it is a divergence for
/// the register, not something to slip into a memory-safety fix. Fixing
/// it properly means giving the callback the variable rather than a
/// pointer into its text.
unsafe fn changelocale(val: *const c_char) {
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt;

    let text = core::ffi::CStr::from_ptr(val).to_bytes();
    if let Some(i) = text.iter().position(|&b| b == b'=') {
        if i > 0 {
            unsafe {
                std::env::set_var(
                    OsStr::from_bytes(&text[..i]),
                    OsStr::from_bytes(&text[i + 1..]),
                );
            }
        }
    }
    libc::setlocale(libc::LC_ALL, addr_of!(nullstr) as *const c_char);
}

/* Some macros in var.h depend on the order, add new variables to the end. */
pub static mut varinit: [var; 16] = [
    var {
        flags: VSTRFIXED | VTEXTFIXED,
        text: VarText::Fixed(addr_of!(defifsvar) as *const c_char),
        func: Some(crate::expand::changeifs),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VUNSET,
        text: VarText::Fixed(b"MAIL\0\0".as_ptr() as *const c_char),
        func: Some(crate::mail::changemail),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VUNSET,
        text: VarText::Fixed(b"MAILPATH\0\0".as_ptr() as *const c_char),
        func: Some(crate::mail::changemail),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED,
        text: VarText::Fixed(addr_of!(defpathvar) as *const c_char),
        func: Some(crate::exec::changepath),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED,
        text: VarText::Fixed(b"PS1=$ \0".as_ptr() as *const c_char),
        func: None,
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED,
        text: VarText::Fixed(b"PS2=> \0".as_ptr() as *const c_char),
        func: None,
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED,
        text: VarText::Fixed(b"PS4=+ \0".as_ptr() as *const c_char),
        func: None,
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VNOFUNC,
        text: VarText::Fixed(addr_of!(defoptindvar) as *const c_char),
        func: Some(getoptsreset),
    },
    /* #ifdef WITH_LINENO */
    var {
        flags: VSTRFIXED | VTEXTFIXED,
        text: VarText::Fixed(addr_of!(linenovar) as *const c_char),
        func: None,
    },
    /* #ifndef SMALL */
    var {
        flags: VSTRFIXED | VTEXTFIXED | VUNSET,
        text: VarText::Fixed(b"TERM\0\0".as_ptr() as *const c_char),
        func: None,
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VUNSET,
        text: VarText::Fixed(b"HISTSIZE\0\0".as_ptr() as *const c_char),
        func: Some(crate::histedit::sethistsize),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: VarText::Fixed(b"LC_ALL\0\0".as_ptr() as *const c_char),
        func: Some(changelocale),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: VarText::Fixed(b"LC_COLLATE\0\0".as_ptr() as *const c_char),
        func: Some(changelocale),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: VarText::Fixed(b"LC_CTYPE\0\0".as_ptr() as *const c_char),
        func: Some(changelocale),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: VarText::Fixed(b"LC_NUMERIC\0\0".as_ptr() as *const c_char),
        func: Some(changelocale),
    },
    var {
        flags: VSTRFIXED | VTEXTFIXED | VFULL | VUNSET,
        text: VarText::Fixed(b"LANG\0\0".as_ptr() as *const c_char),
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
    (*vifs()).text.as_ptr().add(4)
}
pub unsafe fn ifsset() -> c_int {
    (((*vifs()).flags & VUNSET) == 0) as c_int
}
pub unsafe fn mailval() -> *const c_char {
    (*vmail()).text.as_ptr().add(5)
}
pub unsafe fn mpathval() -> *const c_char {
    (*vmpath()).text.as_ptr().add(9)
}
pub unsafe fn pathval() -> *const c_char {
    (*vpath()).text.as_ptr().add(5)
}
pub unsafe fn ps1val() -> *const c_char {
    (*vps1()).text.as_ptr().add(4)
}
pub unsafe fn ps2val() -> *const c_char {
    (*vps2()).text.as_ptr().add(4)
}
pub unsafe fn ps4val() -> *const c_char {
    (*vps4()).text.as_ptr().add(4)
}
pub unsafe fn optindval() -> *const c_char {
    (*voptind()).text.as_ptr().add(7)
}
pub unsafe fn linenoval() -> *const c_char {
    (*vlineno()).text.as_ptr().add(7)
}
pub unsafe fn histsizeval() -> *const c_char {
    (*vhistsize()).text.as_ptr().add(9)
}
pub unsafe fn termval() -> *const c_char {
    (*vterm()).text.as_ptr().add(5)
}
pub unsafe fn mpathset() -> c_int {
    (((*vmpath()).flags & VUNSET) == 0) as c_int
}

/// `#define environment() listvars(VEXPORT, VUNSET, 0)`
///
/// Returns the array `execve` wants: the `text` of every exported, set
/// variable, in name order, with the terminating NULL. The caller owns it
/// and must keep it alive across the `execve`.
pub unsafe fn environment() -> Vec<*mut c_char> {
    let mut envp = listvars(VEXPORT, VUNSET);
    envp.push(null_mut());
    envp
}

/// One entry of `vartab`: one of `varinit`'s sixteen, which the table
/// borrows and never drops, or a `var` the table owns.
///
/// The `Box` is not decoration. `localvar::Saved` holds the entry's
/// address across a whole function invocation, and `setvareq` may file or
/// remove other names in between — which a `BTreeMap` answers by moving
/// the values it holds. Boxing keeps the address the C's `ckmalloc` gave.
enum VarSlot {
    Builtin(*mut var),
    Owned(Box<var>),
}

impl VarSlot {
    #[inline]
    unsafe fn as_ptr(&mut self) -> *mut var {
        match self {
            VarSlot::Builtin(p) => *p,
            VarSlot::Owned(b) => &mut **b as *mut var,
        }
    }
}

/// Every variable, by name. dash's `vartab` is `struct var *[39]` walked
/// through `hashval`; this is the same set of separately allocated `var`s
/// filed in an order that means something. See the module comment.
static mut vartab: BTreeMap<BString, VarSlot> = BTreeMap::new();

#[inline]
unsafe fn vartab_mut() -> &'static mut BTreeMap<BString, VarSlot> {
    &mut *addr_of_mut!(vartab)
}

/// The name `s` is filed under: its bytes up to the first `=`, or all of
/// them if there is none.
///
/// This is what dash's `hashval`/`varequal` pair did implicitly — both stop
/// at the `=`, so `"PATH"` and `"PATH=/bin"` reach the same entry. Made
/// explicit, it is the map key, and the two functions go away with the
/// hash. Borrowed from `s`, so it must not outlive the buffer.
pub(crate) unsafe fn varname<'a>(s: *const c_char) -> &'a BStr {
    let len = strchrnul(s, b'=' as c_int) as usize - s as usize;
    BStr::new(core::slice::from_raw_parts(s as *const u8, len))
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

    let ppid_text = format!("{}", libc::getppid());
    crate::mystring::copy_ascii_cstr(
        (addr_of_mut!(ppid) as *mut c_char).add(5),
        32 - 5,
        &ppid_text,
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
    unwindlocalvars(0);
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

    s = (*vp).text.as_ptr();
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

    vp = addr_of_mut!(varinit) as *mut var;
    end = vp.add(16);
    loop {
        /* The 16 entries stay a static array: `vifs`/`vps1`/… address them
         * positionally, `lookupvar` compares against `vlineno()` by
         * address, and their `text` is `VTEXTFIXED`. Only the link into the
         * table changes — the map holds the address, it does not own the
         * `var`. */
        vartab_mut().insert(
            varname((*vp).text.as_ptr()).to_owned(),
            VarSlot::Builtin(vp),
        );
        vp = vp.add(1);
        if !(vp < end) {
            break;
        }
    }
    /*
     * PS1 depends on uid
     */
    if libc::geteuid() == 0 {
        (*vps1()).text = VarText::Fixed(b"PS1=# \0".as_ptr() as *const c_char);
    }
}

/*
 * Set the value of a variable.  The flags argument is ored with the
 * flags of the variable.  If val is NULL, the variable is unset.
 */

// [spec:dash:def:var.setvar-fn]
// [spec:dash:sem:var.setvar-fn]
pub unsafe fn setvar(name: *const c_char, val: *const c_char, mut flags: c_int) -> *mut var {
    let p: *mut c_char;
    let q: *mut c_char;
    let namelen: size_t;
    let mut nameeq: Vec<u8>;
    let mut vallen: size_t;
    let vp: *mut var;

    q = crate::parser::endofname(name);
    p = strchrnul(q, b'=' as c_int);
    namelen = (p as usize - name as usize) as size_t;
    if namelen == 0 || p != q {
        let mut message = Vec::new();
        message.extend_from_slice(core::slice::from_raw_parts(name as *const u8, namelen));
        message.extend_from_slice(b": bad variable name");
        crate::error::sh_error(&message);
    }
    vallen = 0;
    if val.is_null() {
        flags |= VUNSET;
    } else {
        vallen = CStr::from_ptr(val).count_bytes();
    }
    INTOFF();
    /* `ckmalloc(namelen + vallen + 2)` filled by two `mempcpy`s.  The
     * first copies `namelen + 1` bytes -- the name *and the byte after
     * it*, which is the `=` or the NUL that ended it -- and the `=` is
     * written back over that byte only when there is a value.  So an
     * unset variable's buffer is `NAME\0\0`, and the second NUL is the
     * one `varnull` returns a pointer to. */
    nameeq = Vec::with_capacity(namelen + vallen + 2);
    nameeq.extend_from_slice(core::slice::from_raw_parts(name as *const u8, namelen + 1));
    if !val.is_null() {
        nameeq[namelen] = b'=';
        nameeq.extend_from_slice(core::slice::from_raw_parts(val as *const u8, vallen));
    }
    nameeq.push(b'\0');
    vp = setvareq_text(VarText::Owned(nameeq.into_boxed_slice()), flags | VNOSAVE);
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

    let value = format!("{val}");
    crate::mystring::copy_ascii_cstr(buf.as_mut_ptr(), len as usize, &value);
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
/// The C takes a `char *` plus flag bits saying who owns it: `VTEXTFIXED`
/// or `VSTACK` for a buffer it must not free, `VNOSAVE` for one handed
/// over outright, neither for one to `savestr`.  Only `setvar` ever passed
/// `VNOSAVE`, and it now hands its buffer to [`setvareq_text`] directly,
/// so what is left here is the two cases an outside caller can mean.
pub unsafe fn setvareq(s: *mut c_char, flags: c_int) -> *mut var {
    debug_assert_eq!(
        flags & VNOSAVE,
        0,
        "VNOSAVE means the callee adopts the caller's allocation, which this signature cannot express"
    );
    let text = if (flags & (VTEXTFIXED | VSTACK)) != 0 {
        VarText::Fixed(s)
    } else {
        /* `savestr(s)`.  The C copies at the far end of the function, one
         * statement before the store; the only path between here and there
         * that does not reach the store is the read-only `sh_error`, where
         * the copy is dropped by the unwind. */
        VarText::Owned(CStr::from_ptr(s).to_bytes_with_nul().into())
    };
    setvareq_text(text, flags)
}

/// The body of `setvareq`, over a text whose owner is already settled.
unsafe fn setvareq_text(text: VarText, mut flags: c_int) -> *mut var {
    let mut vp: *mut var;
    let s: *const c_char = text.as_ptr();

    flags |= VEXPORT
        & (((1 - crate::options::optlist[crate::options::aflag] as c_int) as c_uint)
            .wrapping_sub(1)) as c_int;
    vp = findvar(s);
    if !vp.is_null() {
        let bits: c_uint;

        if ((*vp).flags & VREADONLY) != 0 {
            let n: *const c_char;

            /* The C's `if (flags & VNOSAVE) free(s)`: `text` is a local,
             * so the unwind out of `sh_error` drops it. */
            n = (*vp).text.as_ptr();
            let name_len = strchrnul(n, b'=' as c_int) as usize - n as usize;
            let mut message = Vec::new();
            message.extend_from_slice(core::slice::from_raw_parts(n as *const u8, name_len));
            message.extend_from_slice(b": is read only");
            crate::error::sh_error(&message);
        }

        /* The name this entry is filed under, for the removal path below. */
        let key = varname(s).to_owned();

        /* `if ((vp->flags & (VTEXTFIXED|VSTACK)) == 0) ckfree(vp->text);`
         * belongs here, and is instead the drop of the old value at the
         * store below — so the field stays readable in between rather than
         * dangling for the width of the flag arithmetic. */

        if (flags & (VEXPORT | VREADONLY | VSTRFIXED | VUNSET)) != VUNSET {
            bits = !((VTEXTFIXED | VSTACK | VNOSAVE | VUNSET) as c_uint);
        } else if ((*vp).flags & VSTRFIXED) != 0 {
            bits = VSTRFIXED as c_uint;
        } else {
            /* The C unlinks the node, `ckfree`s it and then `ckfree`s `s`;
             * taking the entry out of the map drops the `Box` that is the
             * node and the text inside it, and `text` goes out of scope. */
            vartab_mut().remove(&key);
            /* out_free, then goto out — NB `vp` has just been dropped and
             * is returned dangling, exactly as the C does
             * (src/var.c:304-309, 331). */
            return vp;
        }

        flags |= (*vp).flags & bits as c_int;
    } else {
        if (flags & (VEXPORT | VREADONLY | VSTRFIXED | VUNSET)) == VUNSET {
            /* goto out_free */
            return vp;
        }
        /* not found */
        /* The C leaves `flags` and `text` uninitialised here and fills
         * them in below, which every path from this point reaches. */
        vp = vartab_mut()
            .entry(varname(s).to_owned())
            .or_insert(VarSlot::Owned(Box::new(var {
                flags: 0,
                text: VarText::Fixed(null_mut()),
                func: None,
            })))
            .as_ptr();
    }
    (*vp).text = text;
    (*vp).flags = flags;
    debug_assert_eq!(
        matches!((*vp).text, VarText::Owned(_)),
        ((*vp).flags & (VTEXTFIXED | VSTACK)) == 0,
        "who owns vp->text and what its flags say must agree"
    );

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

    v = findvar(name);
    if !v.is_null() && ((*v).flags & VUNSET) == 0 {
        /* #ifdef WITH_LINENO */
        if v == vlineno() && (*v).text.as_ptr() == addr_of!(linenovar) as *const c_char {
            let current_lineno = lineno;
            let value = format!("{current_lineno}");
            crate::mystring::copy_ascii_cstr(
                (addr_of_mut!(linenovar) as *mut c_char).add(7),
                19 - 7,
                &value,
            );
        }
        return strchrnul((*v).text.as_ptr(), b'=' as c_int).add(1);
    }
    null_mut()
}

// [spec:dash:def:var.lookupvarint-fn]
// [spec:dash:sem:var.lookupvarint-fn]
pub unsafe fn lookupvarint(name: *const c_char) -> Result<intmax_t, Error> {
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
/// The `text` of every variable whose flags match — every bit in `on` set
/// and every bit in `off` clear — in name order.
///
/// The C accumulates into the stack allocator and returns a NULL-terminated
/// `char **` plus, through `end`, the position of the terminator so the
/// caller gets a count without a second scan. An owned `Vec` carries its own
/// length, so both go: `environment` appends the NULL that `execve` needs,
/// and `showvars` just iterates.
pub unsafe fn listvars(on: c_int, off: c_int) -> Vec<*mut c_char> {
    let mask = on | off;
    let mut ep = Vec::new();

    for slot in vartab_mut().values_mut() {
        let vp = slot.as_ptr();
        if ((*vp).flags & mask) == on {
            ep.push((*vp).text.as_ptr() as *mut c_char);
        }
    }

    ep
}

/*
 * POSIX requires that 'set' (but not export or readonly) output the
 * variables in lexicographic order - by the locale's collating order (sigh).
 * The C's comment here wishes for "an ordered balanced binary tree instead
 * of hashed lists" and settles for rolling them through qsort at print
 * time.  `vartab` is that tree, so the sort and its `vpcmp` comparator are
 * gone and the order comes from the container.
 *
 * Neither shell honours the locale: dash's `varcmp` compares bytes, and so
 * does a `BStr` key.  The parenthesised sigh is still owed.
 */

// [spec:dash:def:var.showvars-fn]
// [spec:dash:sem:var.showvars-fn]
pub unsafe fn showvars(prefix: *const c_char, on: c_int, off: c_int) -> c_int {
    let sep: *const c_char;

    sep = if *prefix != 0 {
        addr_of!(crate::mystring::spcstr) as *const c_char
    } else {
        prefix
    };

    for &e in listvars(on, off).iter() {
        let mut p: *const c_char;
        let mut q: *const c_char;

        p = strchrnul(e, b'=' as c_int);
        q = addr_of!(nullstr) as *const c_char;
        if *p != 0 {
            p = p.add(1);
            q = crate::mystring::single_quote(p);
        }

        let mut record = Vec::new();
        record.extend_from_slice(CStr::from_ptr(prefix).to_bytes());
        record.extend_from_slice(CStr::from_ptr(sep).to_bytes());
        record.extend_from_slice(core::slice::from_raw_parts(
            e as *const u8,
            p as usize - e as usize,
        ));
        record.extend_from_slice(CStr::from_ptr(q).to_bytes());
        record.push(b'\n');
        let _ = (&mut *crate::output::stdout()).write_all(&record);
    }

    0
}

/*
 * The export and readonly commands.
 */

/*
 * The "local" command.
 */

/*
 * Make a variable a local variable.  When a variable is made local, it's
 * value and flags are saved in a localvar structure.  The saved values
 * will be restored when the shell function returns.  We handle the name
 * "-" as a special case.
 */

// [spec:dash:def:var.mklocal-fn]
// [spec:dash:sem:var.mklocal-fn]
pub unsafe fn mklocal(name: *mut c_char, flags: c_int) {
    INTOFF();
    if *name.offset(0) == b'-' as c_char && *name.offset(1) == b'\0' as c_char {
        pushlocal(localvar::Options(optlist));
    } else {
        let found: *mut var;

        found = findvar(name);
        /* The C keeps `strchr`'s pointer and only ever asks whether it is
         * NULL: `setvareq` finds the `=` again for itself. */
        let eq = CStr::from_ptr(name).to_bytes().contains(&b'=');
        if found.is_null() {
            let vp: *mut var;
            if eq {
                vp = setvareq(name, VSTRFIXED | flags);
            } else {
                vp = setvar(name, null_mut(), VSTRFIXED | flags);
            }
            pushlocal(localvar::Unset { vp });
        } else {
            let vp: *mut var = found;
            let saved: c_int = (*vp).flags;
            /* The C leaves two pointers to one buffer -- `lvp->text` and
             * `vp->text` -- and says who frees it by setting VTEXTFIXED on
             * the variable.  The save takes the buffer and the variable is
             * left borrowing it, which is the same arrangement with the
             * ownership in the type. */
            let p = (*vp).text.as_ptr();
            let text = core::mem::replace(&mut (*vp).text, VarText::Fixed(p));
            (*vp).flags |= VSTRFIXED | VTEXTFIXED;
            /* Pushed before `setvareq`, which raises on a read-only
             * variable: the save now holds the only copy of what `vp` is
             * borrowing, so it has to be somewhere an unwind will not drop
             * it.  The C leaks the `localvar` on that path instead, and
             * leaves the variable VSTRFIXED|VTEXTFIXED for good. */
            pushlocal(localvar::Saved {
                vp,
                flags: saved,
                text,
            });
            if eq {
                setvareq(name, flags);
            }
        }
    }
    INTON();
}

/// Add a save to the innermost frame.
unsafe fn pushlocal(lvp: localvar) {
    localvar_stack_mut()
        .last_mut()
        .expect("mklocal runs inside a function")
        .lv
        .push(lvp);
}

/*
 * Called after a function returns.
 * Interrupts must be off.
 */

// [spec:dash:def:var.poplocalvars-fn]
// [spec:dash:sem:var.poplocalvars-fn]
unsafe fn poplocalvars() {
    let mut ll: localvar_list;

    INTOFF();
    ll = localvar_stack_mut()
        .pop()
        .expect("poplocalvars runs on a pushed frame");

    /* The C walks the chain from the head, which is the most recent
     * `local`; draining from the back of the `Vec` is the same order. */
    while let Some(lvp) = ll.lv.pop() {
        /* `TRACE(("poplocalvar %s\n", vp ? vp->text : "-"));` — `#ifdef
         * DEBUG` in `shell.h`, and the dash build does not define it. */
        match lvp {
            localvar::Options(saved) => {
                optlist = saved;
                optschanged();
            }
            localvar::Unset { vp } => {
                (*vp).flags &= !(VSTRFIXED | VREADONLY);
                /* `setvar` copies the name out before `setvareq_text` can
                 * drop the buffer it was read from. */
                unsetvar((*vp).text.as_ptr());
            }
            localvar::Saved { vp, flags, text } => {
                /* The C frees `vp->text` first when the flags say the
                 * variable owns it; the assignment is what does that.
                 * When nothing was assigned to the variable while it was
                 * local, what it drops is the `Fixed` borrow `mklocal`
                 * left in place of the buffer it took. */
                (*vp).flags = flags;
                (*vp).text = text;
                debug_assert_eq!(
                    matches!((*vp).text, VarText::Owned(_)),
                    ((*vp).flags & (VTEXTFIXED | VSTACK)) == 0,
                    "who owns vp->text and what its flags say must agree"
                );
                if ((*vp).flags & VNOFUNC) == 0 {
                    varfunc(vp);
                }
            }
        }
    }
    INTON();
}

/*
 * Create a new localvar environment.
 */

// [spec:dash:def:var.pushlocalvars-fn]
// [spec:dash:sem:var.pushlocalvars-fn]
/// The C returns the `localvar_list *` that was on top, which the caller
/// hands back to `unwindlocalvars`; with the stack owned, that address is
/// the frame's depth.
pub unsafe fn pushlocalvars(push: c_int) -> usize {
    let top: usize;

    top = localvar_stack_mut().len();
    if push == 0 {
        return top; /* goto out */
    }

    INTOFF();
    localvar_stack_mut().push(localvar_list { lv: Vec::new() });
    INTON();

    top
}

// [spec:dash:def:var.unwindlocalvars-fn]
// [spec:dash:sem:var.unwindlocalvars-fn]
/// The C's loop is `while (localvar_stack != stop)`, which runs off the
/// bottom of the stack if `stop` was never on it; `>` is total, and the
/// only state it declines to reproduce is a NULL dereference.
pub unsafe fn unwindlocalvars(stop: usize) {
    while localvar_stack_mut().len() > stop {
        poplocalvars();
    }
}

/*
 * The unset builtin command.  We unset the function before we unset the
 * variable to allow a function to be unset when there is a readonly variable
 * with the same name.
 */

/*
 * Unset the specified variable.
 */

// [spec:dash:def:var.unsetvar-fn]
// [spec:dash:sem:var.unsetvar-fn]
pub unsafe fn unsetvar(s: *const c_char) {
    setvar(s, null_mut(), 0);
}

/*
 * Find the entry for a name, which may be given bare or as "name=value".
 */

// [spec:dash:def:var.findvar-fn]
// [spec:dash:sem:var.findvar-fn]
/// The C returns the address of the *link* holding the entry — never NULL,
/// so callers test `*result` — because that is what lets `setvareq` unlink
/// without a second traversal. A map removes by key, so this returns the
/// entry itself and NULL when there is none.
pub(crate) unsafe fn findvar(name: *const c_char) -> *mut var {
    match vartab_mut().get_mut(varname(name)) {
        Some(slot) => slot.as_ptr(),
        None => null_mut(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{CStr0, lock, s};

    /// The whole buffer `vp->text` points at, `len` bytes of it.
    unsafe fn text_bytes(name: &str, len: usize) -> Vec<u8> {
        let n = CStr0::new(name);
        let vp = findvar(n.p());
        assert!(!vp.is_null(), "{name} is not in the table");
        core::slice::from_raw_parts((*vp).text.as_ptr() as *const u8, len).to_vec()
    }

    // [spec:dash:sem:var.setvar-fn/test]
    // [spec:dash:sem:var.varnull-fn/test]
    /// `setvar` builds one `NAME=value` buffer, and an unset variable's
    /// buffer ends in two NULs -- which is not decoration: `varnull`
    /// returns the byte after the first one and every reader of an unset
    /// variable's value stops there.
    #[test]
    fn setvar_files_a_name_equals_value() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("Tsetvar");
            let val = CStr0::new("hello");

            setvar(name.p(), val.p(), 0);
            assert_eq!(text_bytes("Tsetvar", 14), b"Tsetvar=hello\0".to_vec());
            assert_eq!(s(lookupvar(name.p())), "hello");

            /* VSTRFIXED so the entry survives being unset and can be read. */
            setvar(name.p(), null_mut(), VSTRFIXED);
            assert_eq!(text_bytes("Tsetvar", 9), b"Tsetvar\0\0".to_vec());
            let vp = findvar(name.p());
            assert_eq!(s(varnull((*vp).text.as_ptr())), "");
            assert!(lookupvar(name.p()).is_null());
        }
    }

    // [spec:dash:sem:var.poplocalvars-fn/test]
    /// A frame restores in reverse order of declaration, so two `local`s
    /// on one name leave the outermost value behind, not the middle one.
    #[test]
    fn a_frame_restores_in_reverse_order() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("Tframe");
            let two = CStr0::new("Tframe=two");
            let three = CStr0::new("Tframe=three");

            setvar(name.p(), CStr0::new("one").p(), 0);
            let stop = pushlocalvars(1);
            mklocal(two.p() as *mut c_char, 0);
            mklocal(three.p() as *mut c_char, 0);
            assert_eq!(s(lookupvar(name.p())), "three");

            unwindlocalvars(stop);
            assert_eq!(s(lookupvar(name.p())), "one");
            unsetvar(name.p());
        }
    }

    // [spec:dash:sem:var.mklocal-fn/test]
    /// A save holds the variable's address for the whole invocation, so an
    /// entry must not move while it is in the table -- and filing another
    /// two hundred names that sort below it is what would move it.
    #[test]
    fn a_saved_entry_does_not_move() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("Tchurn");
            let local = CStr0::new("Tchurn=inner");

            setvar(name.p(), CStr0::new("outer").p(), 0);
            let stop = pushlocalvars(1);
            mklocal(local.p() as *mut c_char, 0);
            let entry = findvar(name.p());

            let filler: Vec<CStr0> = (0..200).map(|i| CStr0::new(&format!("Ta{i:04}"))).collect();
            for f in &filler {
                setvar(f.p(), CStr0::new("x").p(), 0);
            }
            assert_eq!(findvar(name.p()), entry, "the entry moved under the save");

            unwindlocalvars(stop);
            assert_eq!(s(lookupvar(name.p())), "outer");
            for f in &filler {
                unsetvar(f.p());
            }
            unsetvar(name.p());
        }
    }

    // [spec:dash:sem:var.changelocale-fn/test]
    /// `environ` must own its bytes, not borrow the shell's.
    ///
    /// glibc's `putenv` files the caller's pointer; `setenv` copies. Under
    /// `putenv` the entry `getenv` hands back *is* the tail of `vp->text`,
    /// so the next assignment drops that `Box` out from under `setlocale`
    /// — valgrind reports it as an invalid read inside `getenv`, reached
    /// from `unsetvar`. The assertion is that the two addresses differ.
    ///
    /// Mutation-checked: restoring `libc::putenv(val)` makes them equal
    /// and the test fails.
    #[test]
    fn environ_owns_the_locale_bytes() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("LC_COLLATE");
            let saved = libc::getenv(name.p());
            let saved = if saved.is_null() {
                None
            } else {
                Some(core::ffi::CStr::from_ptr(saved).to_bytes().to_vec())
            };

            let text: &[u8] = b"LC_COLLATE=C\0";
            changelocale(text.as_ptr() as *const c_char);

            let filed = libc::getenv(name.p());
            assert!(!filed.is_null(), "changelocale reached the environment");
            assert_eq!(core::ffi::CStr::from_ptr(filed).to_bytes(), b"C");
            assert_ne!(
                filed as usize,
                text.as_ptr().add("LC_COLLATE=".len()) as usize,
                "environ borrowed the caller's buffer instead of copying it"
            );

            // An unset arrives as the bare value with no name attached,
            // because `setvareq` drops VFULL; it must not panic and must
            // not disturb what is filed. See the function's own comment.
            changelocale(b"\0".as_ptr() as *const c_char);
            assert_eq!(
                core::ffi::CStr::from_ptr(libc::getenv(name.p())).to_bytes(),
                b"C"
            );

            match saved {
                None => std::env::remove_var("LC_COLLATE"),
                Some(v) => {
                    use std::os::unix::ffi::OsStrExt;
                    std::env::set_var("LC_COLLATE", std::ffi::OsStr::from_bytes(&v))
                }
            }
        }
    }
}
