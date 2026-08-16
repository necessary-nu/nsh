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

use crate::context::Shell;
use crate::error::{INTOFF, INTON};
use crate::mystring::nullstr;
use crate::options::{NOPTS, Options, getoptsreset, optschanged};
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
///
/// The receiver is not in the C and is not decoration. Three of the six
/// hooks reach state that is moving onto the shell — `changepath` writes
/// `builtinloc` and clears the command table, `getoptsreset` writes the
/// option state, `changeifs` writes the IFS cache — so a hook that could
/// not be handed a `&mut Shell` would pin those tables as `static mut`
/// however much of the crate was threaded. Widening the pointer is what
/// unblocks them, which is why it lands on its own rather than inside
/// whichever table move discovers it.
pub type varfunc_t = Option<unsafe fn(&mut Shell, *const c_char)>;

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

/// Every variable, the sixteen built-in entries, the `LINENO` buffer, the
/// current line and the `local` save stack — the whole of `docs/api-design.md`
/// §5's `vars` row, less the constant text it says moves with them.
///
/// The fields are private because this module owns an invariant across
/// them that nothing outside can be trusted with: `tab`'s `VarSlot::Builtin`
/// names an entry of `init` **by index**, and `init[VLINENO].text` points
/// into `linenobuf`. Each is resolved in one place -- the index by
/// [`VarSlot::ptr`] and [`VarTable::builtin`], the buffer's address by
/// [`VarTable::new`] -- which is what makes them checkable.
///
/// ## Why the `LINENO` buffer is boxed
///
/// `Shell::new(crate::streams::Streams::INHERIT)` returns by value, so the struct moves exactly once and
/// anything pointing *into* it is left behind. `init[VLINENO].text` is a
/// `VarText::Fixed` into the `LINENO=` buffer, and it has to stay valid
/// across that move — so the buffer is a `Box`, whose address is on the
/// heap and does not move when the struct does.
///
/// This is the same answer, for the same reason, that `VarText::Owned` and
/// `VarSlot::Owned` already gave: "the address has to hold still". The
/// alternative the scoping note proposed — a third `VarText` arm meaning
/// "the LINENO buffer" — would need a `&Shell` at every `text.as_ptr()` in
/// the module to resolve it, which is a far wider change than the one
/// self-reference it removes.
pub struct VarTable {
    /// Every variable, by name. dash's `vartab` is `struct var *[39]`
    /// walked through `hashval`; this is the same set of separately
    /// allocated `var`s filed in an order that means something. See the
    /// module comment.
    tab: BTreeMap<BString, VarSlot>,
    /// `varinit`: the sixteen the shell is born with. `var.h`'s
    /// `#define vifs varinit[0]` / `#define vmail (&vifs)[1]` chain is the
    /// `V*` index constants below.
    init: [var; 16],
    /// `linenovar`, boxed. See the type comment.
    linenobuf: Box<[c_char; 19]>,
    /// The line number the parser is on, which `$LINENO` reports.
    pub(crate) lineno: c_int,
    /// `MKINIT struct localvar_list *localvar_stack;` — the C's `next`
    /// chain, outermost first. A frame's index in this stack is what
    /// `pushlocalvars` hands back and `unwindlocalvars` unwinds to, in
    /// place of the address of the frame below it.
    locals: Vec<localvar_list>,
}

pub static defpathvar: [c_char; 66] = unsafe {
    core::mem::transmute::<[u8; 66], [c_char; 66]>(
        *b"PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin\0",
    )
};
/// Constant text, and `static` rather than `static mut` on purpose.
///
/// The C declares it `char[]` because C has no better way to say "text
/// a `char *` may point at"; nothing in either shell writes it. Making
/// that explicit is what takes it out of `move-state`'s way: a
/// `VarText::Fixed` pointing into an immutable static is valid forever,
/// so this buffer never has to move onto the shell however the rest of
/// the variable table is arranged. Same for `defoptindvar` below, and
/// `defpathvar` above was already this.
pub static defifsvar: [c_char; 8] =
    unsafe { core::mem::transmute::<[u8; 8], [c_char; 8]>(*b"IFS= \t\n\0") };
/* MKINIT char defoptindvar[] = "OPTIND=1"; */
/// Constant text; see `defifsvar`.
pub static defoptindvar: [c_char; 9] =
    unsafe { core::mem::transmute::<[u8; 9], [c_char; 9]>(*b"OPTIND=1\0") };

/// `#define defifs (defifsvar + 4)`
pub unsafe fn defifs() -> *mut c_char {
    (addr_of!(defifsvar) as *mut c_char).add(4)
}
/// `#define defpath (defpathvar + 36)`
pub unsafe fn defpath() -> *const c_char {
    (addr_of!(defpathvar) as *const c_char).add(36)
}

/* int lineno; */
/* char linenovar[sizeof("LINENO=") + sizeof(int) * CHAR_BIT / 3 + 1] = "LINENO="; */
/// Both are `VarTable` fields now — `lineno` and `linenobuf`. The buffer's
/// declared contents live in [`VarTable::new`].
const LINENOVAR_INIT: [c_char; 19] =
    unsafe { core::mem::transmute::<[u8; 19], [c_char; 19]>(*b"LINENO=\0\0\0\0\0\0\0\0\0\0\0\0") };
/// `strlen("LINENO=")` — where the digits `lookupvar` writes begin.
const LINENO_TEXT: usize = 7;

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
unsafe fn changelocale(_sh: &mut Shell, val: *const c_char) {
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
/// `varinit`: the sixteen entries a shell is born with.
///
/// No longer a `static` — it is [`VarTable::init`], and this builds the
/// value the C declared. `lineno_text` is the address of the table's own
/// `LINENO=` buffer, passed in rather than taken here because the buffer
/// belongs to the table this array is going into, and taking it here would
/// be the self-reference the boxing exists to avoid.
fn varinit(lineno_text: *const c_char) -> [var; 16] {
    [
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
            text: VarText::Fixed(lineno_text),
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
    ]
}

/*
 * `var.h` addresses `varinit`'s entries positionally: `#define vifs
 * varinit[0]`, `#define vmail (&vifs)[1]`, and so on down the array. The
 * chain of `+ 1`s is these constants, which say the same thing without
 * pointer arithmetic and can be checked by reading them.
 */
const VIFS: usize = 0;
const VMAIL: usize = 1;
const VMPATH: usize = 2;
const VPATH: usize = 3;
const VPS1: usize = 4;
const VPS2: usize = 5;
const VPS4: usize = 6;
const VOPTIND: usize = 7;
const VLINENO: usize = 8;
const VTERM: usize = 9;
const VHISTSIZE: usize = 10;

impl VarTable {
    /// The table a shell is born with: `varinit`'s sixteen filed under
    /// their own names, an empty `local` stack, and line zero.
    ///
    /// `initvar` used to do the filing at run time from a `static mut`;
    /// it still exists, because it also asks the effective uid what `PS1`
    /// should be, which a constructor cannot know for a shell that has
    /// not started.
    pub(crate) fn new() -> Self {
        let linenobuf = Box::new(LINENOVAR_INIT);
        /* Taken before the box moves into the struct, which does not move
         * what the box points at -- that is the whole reason it is a box.
         * See the type comment. */
        let lineno_text = linenobuf.as_ptr() as *const c_char;
        VarTable {
            tab: BTreeMap::new(),
            init: varinit(lineno_text),
            linenobuf,
            lineno: 0,
            locals: Vec::new(),
        }
    }

    /// `&varinit[i]`. The one place that knows where the sixteen live, and
    /// therefore the only one that changes if they move again.
    #[inline]
    fn builtin(&self, i: usize) -> *const var {
        &self.init[i] as *const var
    }

    #[inline]
    fn builtin_mut(&mut self, i: usize) -> *mut var {
        &mut self.init[i] as *mut var
    }

    /// `varinit[i]`'s value: its text past the `NAME=`, which the C's
    /// `*val()` macros skip by a hard-coded byte count.
    #[inline]
    unsafe fn builtin_val(&self, i: usize, skip: usize) -> *const c_char {
        (*self.builtin(i)).text.as_ptr().add(skip)
    }

    /// `!(varinit[i].flags & VUNSET)` — the C's `ifsset()`/`mpathset()`.
    #[inline]
    fn builtin_isset(&self, i: usize) -> c_int {
        ((self.init[i].flags & VUNSET) == 0) as c_int
    }

    /// Whether there is a frame for a `local` to record itself in --
    /// the C's `localvar_stack == NULL` test at the head of `localcmd`.
    #[inline]
    pub(crate) fn in_function(&self) -> bool {
        !self.locals.is_empty()
    }

}


/*
 * The following accessors reproduce the var.h value macros.  They have to
 * skip over the name, by a hard-coded byte count.
 */
/*
 * These read and do not write, so they take a shared receiver. That is
 * the standing idiom for a reason worth restating here, because this is
 * the family it pays off on: `pathval` is read in an argument list that
 * also passes the shell at five sites, and a shared borrow is the only
 * kind that can compose beside another borrow without restructuring.
 */
pub unsafe fn ifsval(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VIFS, 4)
}
pub unsafe fn ifsset(sh: &Shell) -> c_int {
    sh.vars.builtin_isset(VIFS)
}
pub unsafe fn mailval(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VMAIL, 5)
}
pub unsafe fn mpathval(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VMPATH, 9)
}
pub unsafe fn pathval(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VPATH, 5)
}
pub unsafe fn ps1val(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VPS1, 4)
}
pub unsafe fn ps2val(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VPS2, 4)
}
pub unsafe fn ps4val(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VPS4, 4)
}
pub unsafe fn optindval(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VOPTIND, 7)
}
pub unsafe fn linenoval(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VLINENO, LINENO_TEXT)
}
pub unsafe fn histsizeval(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VHISTSIZE, 9)
}
pub unsafe fn termval(sh: &Shell) -> *const c_char {
    sh.vars.builtin_val(VTERM, 5)
}
pub unsafe fn mpathset(sh: &Shell) -> c_int {
    sh.vars.builtin_isset(VMPATH)
}

/// `#define environment() listvars(VEXPORT, VUNSET, 0)`
///
/// Returns the array `execve` wants: the `text` of every exported, set
/// variable, in name order, with the terminating NULL. The caller owns it
/// and must keep it alive across the `execve`.
pub unsafe fn environment(sh: &mut Shell) -> Vec<*mut c_char> {
    let mut envp = listvars(sh, VEXPORT, VUNSET);
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
    /// One of `varinit`'s sixteen, **by index and not by address**.
    ///
    /// The C files a `struct var *`, and so did this until `move-state`
    /// needed the table and `varinit` to be able to live on the same
    /// `Shell`. A stored pointer into a sibling field is a
    /// self-reference: `Shell::new` returns by value, so the struct
    /// moves once, and every such pointer would be left behind. An
    /// index survives the move because it does not name a location.
    ///
    /// This is the same answer `owned-jobs` gave for the job table ("a
    /// job is named by its index") and `owned-input` for the parse-file
    /// stack. It is resolved fresh at every use by [`VarTable::find`],
    /// which is the one place that has to know where `varinit` lives --
    /// and therefore the one place that changed when it moved onto the
    /// shell.
    Builtin(usize),
    Owned(Box<var>),
}

impl VarSlot {
    /// The entry this slot names, given where `varinit` lives.
    ///
    /// The base is a parameter rather than something this reads, because
    /// every caller is walking or indexing `VarTable::tab` and cannot
    /// also borrow `VarTable::init` while it does. Taking the base out
    /// first is the "copy the scalar out before the walk" technique the
    /// command table settled on, with a pointer in place of a flag.
    #[inline]
    unsafe fn ptr(&mut self, init: *mut var) -> *mut var {
        match self {
            VarSlot::Builtin(i) => init.add(*i),
            VarSlot::Owned(b) => &mut **b as *mut var,
        }
    }
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
pub unsafe fn bltinlookup(sh: &mut Shell, name: *const c_char) -> *mut c_char {
    lookupvar(sh, name)
}

/*
 * Initialize the varable symbol tables and import the environment
 */

/// Where a new shell's exported variables come from.
///
/// The C had no such choice: `environ` is the only source there is when
/// the shell *is* the process. A shell built by [`crate::builder::Builder`]
/// is not, and [dec:nsh:host-owns-the-process] makes reading the host's
/// environment something the caller asks for rather than something the
/// library takes.
pub(crate) enum EnvSource<'a> {
    /// The process's own `environ`, borrowed rather than copied
    /// (`VTEXTFIXED`), exactly as `execve` delivered it. What a shell
    /// started as a process uses.
    Process,
    /// Explicit `name`/`value` pairs, copied. The empty slice is the
    /// library default: a shell that inherits nothing.
    Explicit(&'a [(BString, BString)]),
}

/* mkinit INIT fragment from src/var.c:136-162. */
pub unsafe fn mkinit_init(sh: &mut Shell) -> Result<(), Error> {
    mkinit_init_from(sh, EnvSource::Process)
}

/// `mkinit_init` with the environment's source chosen by the caller.
///
/// Everything other than the import loop is identical and stays in one
/// place, because the order it runs in is load bearing: `initvar` first or
/// `setvareq` files `PATH` as a fresh entry instead of updating
/// `varinit[VPATH]`, and the `PWD` validation last because it reads the
/// environment that was just imported.
pub(crate) unsafe fn mkinit_init_from(sh: &mut Shell, env: EnvSource<'_>) -> Result<(), Error> {
    static mut ppid: [c_char; 32] = unsafe {
        core::mem::transmute::<[u8; 32], [c_char; 32]>(
            *b"PPID=\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0\0",
        )
    };
    let mut p: *const c_char;
    let mut st1: libc::stat64 = core::mem::zeroed();
    let mut st2: libc::stat64 = core::mem::zeroed();

    initvar(sh);
    match env {
        EnvSource::Process => {
            let mut envp: *mut *mut c_char = environ;
            while !(*envp).is_null() {
                p = crate::parser::endofname(*envp);
                if p != *envp && *p == b'=' as c_char {
                    setvareq(sh, *envp, VEXPORT | VTEXTFIXED)?;
                }
                envp = envp.add(1);
            }
        }
        EnvSource::Explicit(pairs) => mkinit_env_pairs(sh, pairs)?,
    }

    setvareq(sh, addr_of!(defifsvar) as *mut c_char, VTEXTFIXED)?;
    setvareq(sh, addr_of!(defoptindvar) as *mut c_char, VTEXTFIXED)?;

    let ppid_text = format!("{}", libc::getppid());
    crate::mystring::copy_ascii_cstr(
        (addr_of_mut!(ppid) as *mut c_char).add(5),
        32 - 5,
        &ppid_text,
    );
    setvareq(sh, addr_of_mut!(ppid) as *mut c_char, VTEXTFIXED)?;

    p = lookupvar(sh, b"PWD\0".as_ptr() as *const c_char);
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
    crate::cd::setpwd(sh, p, 0)
}

/// File explicit `name`/`value` pairs into the variable table, exported.
///
/// The copying half of the environment import. `setvareq` is given no
/// `VTEXTFIXED`, so it takes its copying branch and each buffer dies at
/// the end of its iteration -- which is the difference from the `environ`
/// walk, where the shell borrows the process's bytes forever.
///
/// Its own function because two callers need it: a shell built with no
/// `inherit_env` gets only these, and one built with both gets these on
/// top of the borrowed import.
pub(crate) unsafe fn mkinit_env_pairs(
    sh: &mut Shell,
    pairs: &[(BString, BString)],
) -> Result<(), Error> {
    for (name, value) in pairs {
        /* `NAME=value\0`, which is the shape `setvareq` reads. */
        let mut entry: Vec<u8> = Vec::with_capacity(name.len() + value.len() + 2);
        entry.extend_from_slice(&name[..]);
        entry.push(b'=');
        entry.extend_from_slice(&value[..]);
        entry.push(0);
        let ptr = entry.as_mut_ptr() as *mut c_char;
        /* The same filter the `environ` walk applies, and for the same
         * reason: a name the shell cannot express is one no script could
         * have read back. A pair that fails it is dropped rather than
         * reported, because that is what `execve` delivery does with the
         * same bytes. */
        let p = crate::parser::endofname(ptr);
        if p != ptr && *p == b'=' as c_char {
            setvareq(sh, ptr, VEXPORT)?;
        }
    }
    Ok(())
}

/* mkinit RESET fragment from src/var.c:164-166. */
pub unsafe fn mkinit_reset(sh: &mut Shell) {
    unwindlocalvars(sh, 0);
}

// [spec:dash:def:var.varnull-fn]
// [spec:dash:sem:var.varnull-fn]
unsafe fn varnull(s: *const c_char) -> *mut c_char {
    /* Unset variables always end with two NUL chars. */
    strchrnul(s, b'=' as c_int).add(1)
}

// [spec:dash:def:var.varfunc-fn]
// [spec:dash:sem:var.varfunc-fn]
unsafe fn varfunc(sh: &mut Shell, vp: *mut var) {
    let mut s: *const c_char;

    if (*vp).func.is_none() {
        return;
    }

    s = (*vp).text.as_ptr();
    if ((*vp).flags & VFULL) == 0 {
        s = varnull(s);
    }
    ((*vp).func.unwrap())(sh, s);
}

/*
 * This routine initializes the builtin variables.  It is called when the
 * shell is initialized.
 */

// [spec:dash:def:var.initvar-fn]
// [spec:dash:sem:var.initvar-fn]
pub unsafe fn initvar(sh: &mut Shell) {
    for i in 0..16usize {
        /* The 16 entries stay one array: `vifs`/`vps1`/… address them
         * positionally and their `text` is `VTEXTFIXED`. Only the link
         * into the table is here — the map records *which* of the
         * sixteen, and does not own the `var`.
         *
         * The C walks a `struct var *` and files the pointer; this walks
         * the same array and files the index, because the pointer would
         * be a self-reference once both the table and `varinit` live on
         * one `Shell`. See `VarSlot::Builtin`. */
        let name = varname((*sh.vars.builtin(i)).text.as_ptr()).to_owned();
        sh.vars.tab.insert(name, VarSlot::Builtin(i));
    }
    /*
     * PS1 depends on uid
     */
    if libc::geteuid() == 0 {
        (*sh.vars.builtin_mut(VPS1)).text =
            VarText::Fixed(b"PS1=# \0".as_ptr() as *const c_char);
    }
}

/*
 * Set the value of a variable.  The flags argument is ored with the
 * flags of the variable.  If val is NULL, the variable is unset.
 */

// [spec:dash:def:var.setvar-fn]
// [spec:dash:sem:var.setvar-fn]
pub unsafe fn setvar(sh: &mut Shell, name: *const c_char, val: *const c_char, mut flags: c_int) -> Result<*mut var, Error> {
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
        return Err(sh.sh_error_value(&message));
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
    vp = setvareq_text(sh, VarText::Owned(nameeq.into_boxed_slice()), flags | VNOSAVE)?;
    INTON();

    Ok(vp)
}

/*
 * Set the given integer as the value of a variable.  The flags argument is
 * ored with the flags of the variable.
 */

// [spec:dash:def:var.setvarint-fn]
// [spec:dash:sem:var.setvarint-fn]
pub unsafe fn setvarint(
    sh: &mut Shell,
    name: *const c_char,
    val: intmax_t,
    flags: c_int,
) -> Result<intmax_t, Error> {
    let len = crate::shell::max_int_length(core::mem::size_of_val(&val) as c_int);
    /* C declares a VLA `char buf[len]`; max_int_length(8) is 32. */
    let mut buf = [0 as c_char; 32];

    let value = format!("{val}");
    crate::mystring::copy_ascii_cstr(buf.as_mut_ptr(), len as usize, &value);
    setvar(sh, name, buf.as_ptr(), flags)?;
    Ok(val)
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
pub unsafe fn setvareq(sh: &mut Shell, s: *mut c_char, flags: c_int) -> Result<*mut var, Error> {
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
    setvareq_text(sh, text, flags)
}

/// The body of `setvareq`, over a text whose owner is already settled.
unsafe fn setvareq_text(sh: &mut Shell, text: VarText, mut flags: c_int) -> Result<*mut var, Error> {
    let mut vp: *mut var;
    let s: *const c_char = text.as_ptr();

    flags |= VEXPORT
        & (((1 - sh.options.flag(crate::options::aflag) as c_int) as c_uint)
            .wrapping_sub(1)) as c_int;
    vp = findvar(sh, s);
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
            return Err(sh.sh_error_value(&message));
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
            sh.vars.tab.remove(&key);
            /* out_free, then goto out — NB `vp` has just been dropped and
             * is returned dangling, exactly as the C does
             * (src/var.c:304-309, 331). */
            return Ok(vp);
        }

        flags |= (*vp).flags & bits as c_int;
    } else {
        if (flags & (VEXPORT | VREADONLY | VSTRFIXED | VUNSET)) == VUNSET {
            /* goto out_free */
            return Ok(vp);
        }
        /* not found */
        /* The C leaves `flags` and `text` uninitialised here and fills
         * them in below, which every path from this point reaches. */
        let init = sh.vars.init.as_mut_ptr();
        vp = sh
            .vars
            .tab
            .entry(varname(s).to_owned())
            .or_insert(VarSlot::Owned(Box::new(var {
                flags: 0,
                text: VarText::Fixed(null_mut()),
                func: None,
            })))
            .ptr(init);
    }
    (*vp).text = text;
    (*vp).flags = flags;
    debug_assert_eq!(
        matches!((*vp).text, VarText::Owned(_)),
        ((*vp).flags & (VTEXTFIXED | VSTACK)) == 0,
        "who owns vp->text and what its flags say must agree"
    );

    if (flags & VNOFUNC) == 0 {
        varfunc(sh, vp);
    }

    Ok(vp)
}

/*
 * Find the value of a variable.  Returns NULL if not set.
 */

// [spec:dash:def:var.lookupvar-fn]
// [spec:dash:sem:var.lookupvar-fn]
pub unsafe fn lookupvar(sh: &mut Shell, name: *const c_char) -> *mut c_char {
    let v: *mut var;

    v = findvar(sh, name);
    if !v.is_null() && ((*v).flags & VUNSET) == 0 {
        /* #ifdef WITH_LINENO */
        /* Both halves of the C's condition ask "is this the LINENO entry,
         * still holding its own buffer" -- the user may have assigned to
         * LINENO, which replaces the text with an owned string and stops
         * the refresh. The addresses are the table's now rather than two
         * statics', and the test is otherwise the C's. */
        if v == sh.vars.builtin_mut(VLINENO)
            && (*v).text.as_ptr() == sh.vars.linenobuf.as_ptr() as *const c_char
        {
            let current_lineno = sh.vars.lineno;
            let value = format!("{current_lineno}");
            let buf = sh.vars.linenobuf.as_mut_ptr() as *mut c_char;
            crate::mystring::copy_ascii_cstr(
                buf.add(LINENO_TEXT),
                19 - LINENO_TEXT,
                &value,
            );
        }
        return strchrnul((*v).text.as_ptr(), b'=' as c_int).add(1);
    }
    null_mut()
}

// [spec:dash:def:var.lookupvarint-fn]
// [spec:dash:sem:var.lookupvarint-fn]
pub unsafe fn lookupvarint(sh: &mut Shell, name: *const c_char) -> Result<intmax_t, Error> {
    let p = lookupvar(sh, name);
    crate::mystring::atomax(sh, 
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
pub unsafe fn listvars(sh: &mut Shell, on: c_int, off: c_int) -> Vec<*mut c_char> {
    let mask = on | off;
    let mut ep = Vec::new();

    /* Where `varinit` lives, taken before the walk: a `Builtin` slot
     * names its entry by index, and resolving it inside the loop would
     * mean borrowing the table while the walk holds it. */
    let init = sh.vars.init.as_mut_ptr();
    for slot in sh.vars.tab.values_mut() {
        let vp = slot.ptr(init);
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
pub unsafe fn showvars(sh: &mut Shell, prefix: *const c_char, on: c_int, off: c_int) -> c_int {
    let sep: *const c_char;

    sep = if *prefix != 0 {
        addr_of!(crate::mystring::spcstr) as *const c_char
    } else {
        prefix
    };

    for &e in listvars(sh, on, off).iter() {
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
        let _ = sh.io.stdout().write_all(&record);
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
pub unsafe fn mklocal(sh: &mut Shell, name: *mut c_char, flags: c_int) -> Result<(), Error> {
    INTOFF();
    if *name.offset(0) == b'-' as c_char && *name.offset(1) == b'\0' as c_char {
/* The snapshot is copied out before the stack is touched: this
         * is the one place two *moved* tables meet, and
         * `sh.vars.pushlocal(..sh.options.snapshot()..)` would borrow
         * `sh` twice in one expression. `[c_char; NOPTS]` is `Copy`, so
         * the local is what the call was going to make anyway. */
        let saved = sh.options.snapshot();
        sh.vars.pushlocal(localvar::Options(saved));
    } else {
        let found: *mut var;

        found = findvar(sh, name);
        /* The C keeps `strchr`'s pointer and only ever asks whether it is
         * NULL: `setvareq` finds the `=` again for itself. */
        let eq = CStr::from_ptr(name).to_bytes().contains(&b'=');
        if found.is_null() {
            let vp: *mut var;
            if eq {
                vp = setvareq(sh, name, VSTRFIXED | flags)?;
            } else {
                vp = setvar(sh, name, null_mut(), VSTRFIXED | flags)?;
            }
            sh.vars.pushlocal(localvar::Unset { vp });
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
            sh.vars.pushlocal(localvar::Saved {
                vp,
                flags: saved,
                text,
            });
            if eq {
                /* The `?` returns between the INTOFF above and the INTON
                 * below, leaking the interrupt counter exactly as the
                 * longjmp out of `sh_error` did. Not a bracket to pair;
                 * see docs/errors-are-values.md 2.4. */
                setvareq(sh, name, flags)?;
            }
        }
    }
    INTON();
    Ok(())
}

/// Add a save to the innermost frame.
/// A second `impl` block on purpose: `pushlocal` sits here, between
/// `mklocal` and `poplocalvars`, because that is where `var.c` puts it and
/// this file follows the C's order.
impl VarTable {
    unsafe fn pushlocal(&mut self, lvp: localvar) {
        self.locals
            .last_mut()
            .expect("mklocal runs inside a function")
            .lv
            .push(lvp);
    }
}

/*
 * Called after a function returns.
 * Interrupts must be off.
 */

// [spec:dash:def:var.poplocalvars-fn]
// [spec:dash:sem:var.poplocalvars-fn]
unsafe fn poplocalvars(sh: &mut Shell) {
    let mut ll: localvar_list;

    INTOFF();
    ll = sh
        .vars
        .locals
        .pop()
        .expect("poplocalvars runs on a pushed frame");

    /* The C walks the chain from the head, which is the most recent
     * `local`; draining from the back of the `Vec` is the same order. */
    while let Some(lvp) = ll.lv.pop() {
        /* `TRACE(("poplocalvar %s\n", vp ? vp->text : "-"));` — `#ifdef
         * DEBUG` in `shell.h`, and the dash build does not define it. */
        match lvp {
            localvar::Options(saved) => {
                sh.options.restore(saved);
                /* Teardown, and 4.3's rule is that teardown does not
                 * become fallible: cleanup that can fail while handling a
                 * failure would make every unwind path decide what to do
                 * with an error raised while handling an error. The
                 * diagnostic has already been written by the time it gets
                 * here; what the C added was a longjmp out of the middle
                 * of restoring a `local -` option set, and that is what
                 * goes. */
                /* Teardown, so the result is dropped rather than
                 * propagated -- but the status it took is still the
                 * shell's, and the raise no longer writes it. */
                let changed = optschanged(sh);
                if let Err(e) = &changed {
                    sh.status = e.status();
                }
                drop(changed);
            }
            localvar::Unset { vp } => {
                (*vp).flags &= !(VSTRFIXED | VREADONLY);
                /* `setvar` copies the name out before `setvareq_text` can
                 * drop the buffer it was read from. */
                /* The C longjmps out of `poplocalvars` when this raises,
                 * and it cannot raise. `unsetvar` reaches `setvar`, which
                 * has exactly two failure paths: a malformed name, which
                 * this text cannot have because it came out of the
                 * variable table; and the read-only test in
                 * `setvareq_text`, which reads the flags of the entry
                 * `findvar` returns -- and that is this very `vp`, whose
                 * VREADONLY was cleared on the line above. The assertion
                 * is the claim; the drop is what happens if it is ever
                 * wrong, and it keeps the rest of the teardown running
                 * rather than abandoning it. */
                let unset = unsetvar(sh, (*vp).text.as_ptr());
                debug_assert!(
                    unset.is_ok(),
                    "poplocalvars cleared VREADONLY on the entry unsetvar will find"
                );
                if let Err(e) = &unset {
                    sh.status = e.status();
                }
                drop(unset);
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
                    varfunc(sh, vp);
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
pub unsafe fn pushlocalvars(sh: &mut Shell, push: c_int) -> usize {
    let top: usize;

    top = sh.vars.locals.len();
    if push == 0 {
        return top; /* goto out */
    }

    INTOFF();
    sh.vars.locals.push(localvar_list { lv: Vec::new() });
    INTON();

    top
}

// [spec:dash:def:var.unwindlocalvars-fn]
// [spec:dash:sem:var.unwindlocalvars-fn]
/// The C's loop is `while (localvar_stack != stop)`, which runs off the
/// bottom of the stack if `stop` was never on it; `>` is total, and the
/// only state it declines to reproduce is a NULL dereference.
pub unsafe fn unwindlocalvars(sh: &mut Shell, stop: usize) {
    while sh.vars.locals.len() > stop {
        poplocalvars(sh);
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
pub unsafe fn unsetvar(sh: &mut Shell, s: *const c_char) -> Result<(), Error> {
    setvar(sh, s, null_mut(), 0)?;
    Ok(())
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
pub(crate) unsafe fn findvar(sh: &mut Shell, name: *const c_char) -> *mut var {
    /* Where `varinit` lives, taken before the lookup borrows the map: a
     * `Builtin` slot names its entry by index, and the two are sibling
     * fields of one table. */
    let init = sh.vars.init.as_mut_ptr();
    match sh.vars.tab.get_mut(varname(name)) {
        Some(slot) => slot.ptr(init),
        None => null_mut(),
    }
}

/// The variable table, as an embedder reaches it.
///
/// `docs/api-design.md` §2. These are the table and not the *language*:
/// `$?`, `$#`, `$1` and `$@` are not variables and are not here.
/// [`Shell::status`] is `$?`, and [`Shell::expand_word`] reads the rest.
impl Shell {
    /// Read a shell variable.
    ///
    /// **`&mut self` where the sketch had `&self`**, and dash is the
    /// reason rather than an implementation detail: `$LINENO` is computed
    /// on read. `lookupvar` rewrites the `LINENO` entry's buffer from the
    /// parser's current line before returning it, so *reading* a variable
    /// writes one. A `&self` signature would have been a lie about that,
    /// and would have needed either a second lookup path or interior
    /// mutability to keep.
    ///
    /// The result borrows the table, so a value that has to outlive the
    /// next [`Shell::run`] must be copied out. That is not a papercut to
    /// design away: an assignment can move the table, and the borrow is
    /// what says so.
    pub fn var(&mut self, name: &bstr::BStr) -> Option<&bstr::BStr> {
        unsafe {
            let c = crate::shell::cstring(name);
            let p = lookupvar(self, c.as_ptr());
            if p.is_null() {
                return None;
            }
            /* The pointer is into the entry's own `text`, which the table
             * owns and which `&mut self` is borrowed for. */
            Some(bstr::BStr::new(CStr::from_ptr(p).to_bytes()))
        }
    }

    /// Assign a shell variable, with the meaning `name=value` has in a
    /// script.
    ///
    /// A variable that is already exported stays exported; a new one is
    /// not, which is why `set_var(b"PATH", …)` reaches child processes and
    /// `set_var(b"MY_FLAG", …)` does not. The initial exported environment
    /// is [`crate::builder::Builder::env`].
    ///
    /// # Errors
    ///
    /// On a name that is not a valid shell name, and on a readonly
    /// variable — the same diagnostic a script would get, because it is
    /// the same code path.
    pub fn set_var(&mut self, name: &bstr::BStr, value: &bstr::BStr) -> Result<(), Error> {
        unsafe {
            let n = crate::shell::cstring(name);
            let v = crate::shell::cstring(value);
            setvar(self, n.as_ptr(), v.as_ptr(), 0)?;
            Ok(())
        }
    }

    /// Unset a shell variable, and say whether it had been set.
    ///
    /// # Errors
    ///
    /// **A `Result` where the sketch had a bare `bool`.** Unsetting a
    /// readonly variable is a shell error, and swallowing it here would
    /// have made this the one place in the surface where the library
    /// silently declines what a script is told about.
    pub fn unset_var(&mut self, name: &bstr::BStr) -> Result<bool, Error> {
        unsafe {
            let c = crate::shell::cstring(name);
            let was_set = !lookupvar(self, c.as_ptr()).is_null();
            unsetvar(self, c.as_ptr())?;
            Ok(was_set)
        }
    }

    /// Every variable that is set, as `(name, value)`, in the order a bare
    /// `set` prints them.
    ///
    /// **Owned pairs where the sketch had a borrowing iterator.** The
    /// table is a map whose entries are reached through a raw pointer
    /// derived from a `&mut` walk (`listvars`), so handing out references
    /// into it would reintroduce exactly the provenance question
    /// [`crate::output::Dest`] was created to remove. A listing already
    /// pays for a walk of the whole table; it now also pays for a copy of
    /// it, and gets a value that outlives the next `run` in exchange.
    pub fn vars(&mut self) -> Vec<(bstr::BString, bstr::BString)> {
        unsafe {
            listvars(self, 0, VUNSET)
                .into_iter()
                .filter_map(|p| {
                    let text = CStr::from_ptr(p).to_bytes();
                    let eq = text.iter().position(|&b| b == b'=')?;
                    Some((
                        bstr::BString::from(&text[..eq]),
                        bstr::BString::from(&text[eq + 1..]),
                    ))
                })
                .collect()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{CStr0, lock, s};

    /// The whole buffer `vp->text` points at, `len` bytes of it.
    unsafe fn text_bytes(sh: &mut Shell, name: &str, len: usize) -> Vec<u8> {
        let n = CStr0::new(name);
        let vp = findvar(sh, n.p());
        assert!(!vp.is_null(), "{name} is not in the table");
        core::slice::from_raw_parts((*vp).text.as_ptr() as *const u8, len).to_vec()
    }

    // [spec:dash:sem:var.lookupvar-fn/test]
    /// `$LINENO` is read out of a buffer the table owns, and
    /// `varinit[VLINENO].text` points *into* that buffer -- so the buffer
    /// has to stay where it is while the shell around it moves.
    ///
    /// `Shell::new(crate::streams::Streams::INHERIT)` returns by value, which is already one move; this
    /// does a second one deliberately. A plain `[c_char; 19]` field would
    /// pass the first test by luck of the return slot and fail this one,
    /// because the pointer would still name the old location. The `Box`
    /// is what makes both true, and this is the test that says so.
    #[test]
    fn lineno_buffer_survives_a_shell_move() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("LINENO");
            let mut owned = Shell::new(crate::streams::Streams::INHERIT);
            initvar(&mut owned);

            owned.vars.lineno = 41;
            assert_eq!(s(lookupvar(&mut owned, name.p())), "41");

            /* The move the boxing exists for. */
            let mut moved = owned;
            moved.vars.lineno = 42;
            assert_eq!(s(lookupvar(&mut moved, name.p())), "42");
        }
    }

    // [spec:dash:sem:var.findvar-fn/test]
    /// A builtin entry is filed by index, not by address, so it must
    /// resolve against the `varinit` of the shell being asked -- not the
    /// one the table was built from. Two shells make that observable: the
    /// same name in each must answer with that shell's own entry.
    #[test]
    fn a_builtin_resolves_per_shell() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("PATH");
            let mut one = Shell::new(crate::streams::Streams::INHERIT);
            let mut two = Shell::new(crate::streams::Streams::INHERIT);
            initvar(&mut one);
            initvar(&mut two);

            let a = findvar(&mut one, name.p());
            let b = findvar(&mut two, name.p());
            assert!(!a.is_null() && !b.is_null(), "PATH is one of the sixteen");
            assert_ne!(a, b, "each shell answers with its own varinit entry");
            assert_eq!(a, one.vars.builtin_mut(VPATH));
            assert_eq!(b, two.vars.builtin_mut(VPATH));
        }
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
            let mut owned = Shell::new(crate::streams::Streams::INHERIT);
            let sh = &mut owned;
            let name = CStr0::new("Tsetvar");
            let val = CStr0::new("hello");

            setvar(sh, name.p(), val.p(), 0);
            assert_eq!(text_bytes(sh, "Tsetvar", 14), b"Tsetvar=hello\0".to_vec());
            assert_eq!(s(lookupvar(sh, name.p())), "hello");

            /* VSTRFIXED so the entry survives being unset and can be read. */
            setvar(sh, name.p(), null_mut(), VSTRFIXED);
            assert_eq!(text_bytes(sh, "Tsetvar", 9), b"Tsetvar\0\0".to_vec());
            let vp = findvar(sh, name.p());
            assert_eq!(s(varnull((*vp).text.as_ptr())), "");
            assert!(lookupvar(sh, name.p()).is_null());
        }
    }

    // [spec:dash:sem:var.poplocalvars-fn/test]
    /// A frame restores in reverse order of declaration, so two `local`s
    /// on one name leave the outermost value behind, not the middle one.
    #[test]
    fn a_frame_restores_in_reverse_order() {
        let _g = lock();
        unsafe {
            let mut owned = Shell::new(crate::streams::Streams::INHERIT);
            let sh = &mut owned;
            let name = CStr0::new("Tframe");
            let two = CStr0::new("Tframe=two");
            let three = CStr0::new("Tframe=three");

            setvar(sh, name.p(), CStr0::new("one").p(), 0);
            let stop = pushlocalvars(sh, 1);
            mklocal(sh, two.p() as *mut c_char, 0);
            mklocal(sh, three.p() as *mut c_char, 0);
            assert_eq!(s(lookupvar(sh, name.p())), "three");

            unwindlocalvars(sh, stop);
            assert_eq!(s(lookupvar(sh, name.p())), "one");
            unsetvar(sh, name.p());
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
            let mut owned = Shell::new(crate::streams::Streams::INHERIT);
            let sh = &mut owned;
            let name = CStr0::new("Tchurn");
            let local = CStr0::new("Tchurn=inner");

            setvar(sh, name.p(), CStr0::new("outer").p(), 0);
            let stop = pushlocalvars(sh, 1);
            mklocal(sh, local.p() as *mut c_char, 0);
            let entry = findvar(sh, name.p());

            let filler: Vec<CStr0> = (0..200).map(|i| CStr0::new(&format!("Ta{i:04}"))).collect();
            for f in &filler {
                setvar(sh, f.p(), CStr0::new("x").p(), 0);
            }
            assert_eq!(findvar(sh, name.p()), entry, "the entry moved under the save");

            unwindlocalvars(sh, stop);
            assert_eq!(s(lookupvar(sh, name.p())), "outer");
            for f in &filler {
                unsetvar(sh, f.p());
            }
            unsetvar(sh, name.p());
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
            let mut owned = Shell::new(crate::streams::Streams::INHERIT);
            let sh = &mut owned;
            let name = CStr0::new("LC_COLLATE");
            let saved = libc::getenv(name.p());
            let saved = if saved.is_null() {
                None
            } else {
                Some(core::ffi::CStr::from_ptr(saved).to_bytes().to_vec())
            };

            let text: &[u8] = b"LC_COLLATE=C\0";
            changelocale(sh, text.as_ptr() as *const c_char);

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
            changelocale(sh, b"\0".as_ptr() as *const c_char);
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
