//! Literal port of `src/expand.c` / `src/expand.h`.
//! Rules: `docs/spec/port/src/expand.md`.
//!
//! Routines to expand arguments to commands.  We have to deal with
//! backquotes, shell variables, and file metacharacters.
//!
//! This is a deliberately un-idiomatic, line-for-line translation.  The
//! word being expanded is the parser's internal *encoded byte string* — a
//! NUL-terminated `char *` in which `CTLESC`, `CTLVAR`, `CTLENDVAR`,
//! `CTLBACKQ`, `CTLMBCHAR`, `CTLARI`, `CTLENDARI` and `CTLQUOTEMARK` (all
//! negative `signed char` values) are markers.  It is therefore
//! represented here exactly as in C, by `*mut c_char` plus index
//! arithmetic; no `String`/`Vec<u8>` appears anywhere, and `mbnext` keeps
//! its packed `start | end << 8` return rather than becoming a struct.
//!
//! C `goto`s are reproduced with labelled blocks (`'label: { … break
//! 'label; }` for a forward jump) and labelled loops (for the backward
//! jumps `tilde:`, `start:` and `again:`), so the control flow still
//! diffs one-to-one against the C.

#![allow(unknown_lints)]
#![allow(static_mut_refs)]

use core::mem;
use core::ptr;
use std::ffi::CStr;

use bstr::{BStr, BString, ByteSlice};
use libc::{c_char, c_int, c_uint, c_ulong, c_void, intmax_t, size_t, ssize_t, wchar_t};

use crate::error::Error;

// ---------------------------------------------------------------------
// Declarations from <wchar.h> / <wctype.h> that the `libc` crate does not
// expose.  These are plain libc entry points, not ported symbols.
// ---------------------------------------------------------------------

#[allow(non_camel_case_types)]
type wint_t = c_uint;
#[allow(non_camel_case_types)]
type wctype_t = c_ulong;

unsafe extern "C" {
    fn mbrlen(s: *const c_char, n: size_t, ps: *mut libc::mbstate_t) -> size_t;
    fn mbrtowc(pwc: *mut wchar_t, s: *const c_char, n: size_t, ps: *mut libc::mbstate_t) -> size_t;
    fn mbsrtowcs(
        dst: *mut wchar_t,
        src: *mut *const c_char,
        len: size_t,
        ps: *mut libc::mbstate_t,
    ) -> size_t;
    fn iswspace(wc: wint_t) -> c_int;
    fn wctype(name: *const c_char) -> wctype_t;
    fn iswctype(wc: wint_t, desc: wctype_t) -> c_int;
}

// ---------------------------------------------------------------------
// Constants mirrored from the headers this file includes.
//
// The parser's marker bytes and variable-substitution codes come from
// `parser.h`.  They are aliased here as `c_char`/`c_int` so they can be
// used as `match` patterns and so that the numeric type the parser
// module happens to choose does not matter.
// ---------------------------------------------------------------------

const CTLESC: c_char = crate::parser::CTLESC as c_char;
const CTLVAR: c_char = crate::parser::CTLVAR as c_char;
const CTLENDVAR: c_char = crate::parser::CTLENDVAR as c_char;
const CTLBACKQ: c_char = crate::parser::CTLBACKQ as c_char;
const CTLMBCHAR: c_char = crate::parser::CTLMBCHAR as c_char;
const CTLARI: c_char = crate::parser::CTLARI as c_char;
const CTLENDARI: c_char = crate::parser::CTLENDARI as c_char;
const CTLQUOTEMARK: c_char = crate::parser::CTLQUOTEMARK as c_char;

const VSTYPE: c_int = crate::parser::VSTYPE as c_int;
const VSNUL: c_int = crate::parser::VSNUL as c_int;
const VSBIT: c_int = crate::parser::VSBIT as c_int;

const VSNORMAL: c_int = crate::parser::VSNORMAL as c_int;
const VSMINUS: c_int = crate::parser::VSMINUS as c_int;
const VSPLUS: c_int = crate::parser::VSPLUS as c_int;
const VSQUESTION: c_int = crate::parser::VSQUESTION as c_int;
const VSASSIGN: c_int = crate::parser::VSASSIGN as c_int;
const VSTRIMRIGHT: c_int = crate::parser::VSTRIMRIGHT as c_int;
const VSTRIMRIGHTMAX: c_int = crate::parser::VSTRIMRIGHTMAX as c_int;
const VSTRIMLEFT: c_int = crate::parser::VSTRIMLEFT as c_int;
const VSTRIMLEFTMAX: c_int = crate::parser::VSTRIMLEFTMAX as c_int;
const VSLENGTH: c_int = crate::parser::VSLENGTH as c_int;

/// `FNMATCH_IS_ENABLED` / `GLOB_IS_ENABLED` from `mystring.h`: the
/// build-time switch between libc `fnmatch(3)`/`glob(3)` and the shell's
/// own matcher.  `--enable-fnmatch` / `--enable-glob` are opt-in, so both
/// are false in the shipped build.
const FNMATCH_IS_ENABLED: bool = crate::mystring::FNMATCH_IS_ENABLED != 0;
const GLOB_IS_ENABLED: bool = crate::mystring::GLOB_IS_ENABLED != 0;

/// `<limits.h>`
const CHAR_BIT: c_int = 8;

// C character literals used as `switch` labels; Rust `match` patterns
// require named constants, so the ones this file switches on get names.
const C_NUL: c_char = 0;
const C_NL: c_char = b'\n' as c_char;
const C_BANG: c_char = b'!' as c_char;
const C_HASH: c_char = b'#' as c_char;
const C_DOLLAR: c_char = b'$' as c_char;
const C_STAR: c_char = b'*' as c_char;
const C_MINUS: c_char = b'-' as c_char;
const C_DOT: c_char = b'.' as c_char;
const C_SLASH: c_char = b'/' as c_char;
const C_COLON: c_char = b':' as c_char;
const C_QUESTION: c_char = b'?' as c_char;
const C_AT: c_char = b'@' as c_char;
const C_LBRACKET: c_char = b'[' as c_char;
const C_RBRACKET: c_char = b']' as c_char;
const C_BACKSLASH: c_char = b'\\' as c_char;
const C_CARET: c_char = b'^' as c_char;
const C_EQUALS: c_char = b'=' as c_char;
const C_TILDE: c_char = b'~' as c_char;
const C_0: c_char = b'0' as c_char;
const C_9: c_char = b'9' as c_char;

// ---------------------------------------------------------------------
// src/expand.h
// ---------------------------------------------------------------------

// [spec:dash:def:expand.strlist]
///
/// The C's `next` field is gone: the chain is the `Vec` inside
/// [`arglist`], the same shape as [`ifsregion`]'s.  What is left is the
/// text, and the text is the entry's own.
///
/// In the C it is a `char *` into the region, kept alive by whichever
/// `popstackmark` encloses the command — which is why `expandarg` had to
/// copy the word out of the expansion buffer and `addfnamealt` had to copy
/// the candidate out of the glob buffer before either could hand it over.
/// Owning the bytes says that lifetime directly.
///
/// **Invariant: the bytes end with a NUL, and the terminator is counted.**
/// Every reader is a C-string reader — `setvar`, `setvareq`, `execve`,
/// `find_command`, `strcoll`, `patmatch`, `outfmt` — so a field that
/// stopped at `strlen` would have to have a terminator appended at each of
/// them. [`strlist::textp`] asserts it.
pub struct strlist {
    pub text: BString,
}

impl strlist {
    /// A field taken out of a buffer the shell is about to stop owning:
    /// the run `ifsbreakup` has just terminated, or `glob`'s `d_name`.
    #[inline]
    pub unsafe fn from_cstr(p: *const c_char) -> strlist {
        strlist {
            /* The terminator travels: every reader of a word's text
             * reads it as a C string. */
            text: BString::from(CStr::from_ptr(p).to_bytes_with_nul()),
        }
    }

    /// `sp->text`, as the `char *` every reader wants.
    #[inline]
    pub fn textp(&self) -> *mut c_char {
        debug_assert_eq!(self.text.last(), Some(&0), "a field is a C string");
        self.text.as_ptr() as *mut c_char
    }

    /// `rmescapes(sp->text)`, in place as the C does it.
    ///
    /// `_rmescapes` shortens the C string and says nothing about by how
    /// much, so the length is re-derived. No reader of a field uses its
    /// length — they all stop at the terminator, as the C's did — so the
    /// truncation is hygiene rather than correctness: what it buys is that
    /// the entry's length keeps meaning the string's length, which is what
    /// makes the assertion in [`strlist::textp`] worth anything.
    #[inline]
    pub unsafe fn rmescapes(&mut self) {
        let p = self.text.as_mut_ptr() as *mut c_char;
        rmescapes(p);
        self.text.truncate(CStr::from_ptr(p).count_bytes() + 1);
    }
}

// [spec:dash:def:expand.arglist]
///
/// `lastp` goes with `next`.  The C carries it because appending to a
/// singly-linked list needs its tail, and it is always
/// `&(last node)->next` — which is `list.len()`.  The three places that
/// save it across an `expandarg` and read back what that call appended
/// (`eval.c:fill_arglist`, `evalcommand`'s assignment loop, and
/// `expandmeta`'s `savelastp`) save the length instead.
///
/// `arglist->list` is *also* reassigned in one place —
/// `eval.c:parse_command_args`, which advances the head past the
/// `command [-p]` words it consumed while `eval.c:evalcommand` keeps the
/// original head in `osp` for `set -x`.  A `Vec`'s start does not move, so
/// that head travels as an index of its own.
pub struct arglist {
    pub list: Vec<strlist>,
}

impl arglist {
    /// The C writes `struct arglist arglist;` and then
    /// `arglist.lastp = &arglist.list`, which is an empty list.
    pub const fn new() -> arglist {
        arglist { list: Vec::new() }
    }
}

/*
 * expandarg() flags
 */
pub const EXP_FULL: c_int = 0x1; /* perform word splitting & file globbing */
pub const EXP_TILDE: c_int = 0x2; /* do normal tilde expansion */
pub const EXP_VARTILDE: c_int = 0x4; /* expand tildes in an assignment */
pub const EXP_REDIR: c_int = 0x8; /* file glob for a redirection (1 match only) */
pub const EXP_CASE: c_int = 0x10; /* keeps quotes around for CASE pattern */
pub const EXP_MBCHAR: c_int = 0x20; /* mark multi-byte characters */
pub const EXP_VARTILDE2: c_int = 0x40; /* expand tildes after colons only */
pub const EXP_WORD: c_int = 0x80; /* expand word in parameter expansion */
pub const EXP_QUOTED: c_int = 0x100; /* expand word in double quotes */
pub const EXP_KEEPNUL: c_int = 0x200; /* do not skip NUL characters */
pub const EXP_DISCARD: c_int = 0x400; /* discard result of expansion */

/// `expand.h`: `#define rmescapes(p) _rmescapes((p), 0)`
#[inline]
pub unsafe fn rmescapes(p: *mut c_char) -> *mut c_char {
    _rmescapes(p, 0, None)
}

// ---------------------------------------------------------------------
// src/expand.c
// ---------------------------------------------------------------------

/*
 * _rmescape() flags
 */
pub const RMESCAPE_ALLOC: c_int = 0x1; /* Allocate a new string */
pub const RMESCAPE_GLOB: c_int = 0x2; /* Add backslashes for glob */
pub const RMESCAPE_GROW: c_int = 0x8; /* Grow strings instead of stalloc */
pub const RMESCAPE_HEAP: c_int = 0x10; /* Malloc strings instead of stalloc */

/* Add CTLESC when necessary. */
pub const QUOTES_ESC: c_int = EXP_FULL | EXP_CASE;

/*
 * Structure specifying which parts of the string should be searched
 * for IFS characters.
 */

// [spec:dash:def:expand.ifsregion]
///
/// The C's `next` field is gone: the chain is [`ifsregions`], a `Vec`.
/// See it for why "the `Vec` is empty" is exactly "`ifslastp` is NULL".
pub struct ifsregion {
    pub begoff: c_int,  /* offset of start of region */
    pub endoff: c_int,  /* offset of end of region */
    pub nulonly: c_int, /* search for nul bytes only */
}

// [spec:dash:def:expand.ifs-state]
#[repr(C)]
pub struct ifs_state {
    pub ifs: *const c_char,
    pub start: *mut c_char,
    pub r: *mut c_char,
    pub maxargs: c_int,
    pub ifsspc: c_int,
}

/* output of current string */
///
/// The C is `static char *expdest;`, a cursor into the stack block whose
/// base is `stackblock()`.  Here the word owns its bytes and **the cursor
/// is the length**: `expdest` is `expbuf.len()`, `stackblock()` is
/// [`expbase`], `STADJUST` backwards is `truncate`, and `STPUTC` is `push`.
///
/// Two properties of the region that the C leans on are *not* properties of
/// a `Vec`, and both are audited where they matter:
///
///   * **The base does not move.** Every `p = stackblock()` re-read in the
///     C is there because `makestrspace` may have reallocated. `Vec` has
///     exactly the same hazard — `reserve` reallocates — so those re-reads
///     stay, as `expbase()`, and they still mean "a growth can happen
///     here".
///   * **Bytes past the cursor survive a growth.** The region copies the
///     whole block; `Vec::reserve` copies only the first `len` bytes. Two
///     places write past the cursor and read the byte back: `subevalvar`'s
///     closing `*loc = '\0'` (re-supplied by `argstr`'s own terminator
///     before anything reads it) and `expari`'s arithmetic text (read by
///     `arith`, which cannot grow this buffer). Both are argued at the
///     site.
static mut expbuf: BString = BString::new(Vec::new());
/* list of back quote expressions.
 * The C walks a `struct nodelist *` and advances it with `argbackq =
 * argbackq->next`; the list is now the `Vec` inside the NARG node, so what
 * travels is a raw slice cursor over it. */
static mut argbackq: *const [Option<crate::nodes::Node>] =
    ptr::slice_from_raw_parts(ptr::null::<Option<crate::nodes::Node>>(), 0);
/// The list of IFS regions.  The C is two statics — `ifsfirst`, a head
/// node held in `.bss`, and `ifslastp`, a pointer to the last node —
/// with every node after the head `ckmalloc`'d and chained through
/// `next`.
///
/// The model this replaces them with is `ifsregions.is_empty()` **is**
/// `ifslastp == NULL`, and that equality is exact rather than
/// approximate.  `ifslastp` is NULL in three places and all three leave
/// the chain behind the head empty as well: at startup, after `ifsfree`
/// (which frees `ifsfirst.next` and nulls it before nulling `ifslastp`),
/// and in `removerecordregions`' first branch (which frees the whole
/// chain before testing `ifsfirst.begoff`).  So a NULL `ifslastp` never
/// hides a live region, and `ifsfirst`'s stale contents are never read —
/// `recordregion` overwrites them, and every other reader tests
/// `ifslastp` first.
static mut ifsregions: Vec<ifsregion> = Vec::new();
/* holds expanded arg list */
static mut exparg: arglist = arglist::new();

static mut ifsmap: [c_char; 128] = [0; 128];
static mut ncifs: *const c_char = ptr::null();
static mut ifsmb0len: size_t = 0;
/// The wide-character form of `IFS`, built by `changeifs`.
///
/// The C is a `ckmalloc`'d, zero-filled, NUL-terminated `wchar_t *` that
/// is NULL whenever `IFS` holds no byte with the high bit set.  Empty
/// **is** NULL here: the C only allocates under `mb != 0`, which needs a
/// high-bit byte, so the buffer is never zero-length when it exists.
static mut wcifs: Vec<wchar_t> = Vec::new();

/// `&mut ifsregions`, without ever naming a reference to the `static mut`
/// twice at once.
#[inline]
unsafe fn ifsr() -> &'static mut Vec<ifsregion> {
    &mut *ptr::addr_of_mut!(ifsregions)
}

/// `&mut wcifs`, same.
#[inline]
unsafe fn wcifsv() -> &'static mut Vec<wchar_t> {
    &mut *ptr::addr_of_mut!(wcifs)
}

/// `&mut exparg.list`, same.  Every `*exparg.lastp = sp` in the C is a
/// `push` on this, and `exparg.lastp = &exparg.list` — the C's way of
/// throwing away whatever the previous expansion left in the head — is a
/// `clear`.
#[inline]
unsafe fn expargl() -> &'static mut Vec<strlist> {
    &mut (*ptr::addr_of_mut!(exparg)).list
}

/// `wcschr(wcifs, wc) != NULL`.
///
/// Transcribed rather than replaced with `contains`, because `wcschr`
/// searches a NUL-*terminated* string and therefore **matches the
/// terminator** when `wc` is 0.  `ifsisifs` reaches here with `wc` taken
/// straight from the byte under the cursor, so a NUL inside an IFS region
/// takes the `isifs` branch in the C and has to here too.
#[inline]
fn wcifs_chr(v: &[wchar_t], wc: wchar_t) -> bool {
    for &c in v {
        if c == wc {
            return true;
        }
        if c == 0 {
            return false;
        }
    }
    false
}

// ---------------------------------------------------------------------
// The expansion buffer.  See [`expbuf`].
// ---------------------------------------------------------------------

/// `&mut expbuf`, without ever naming a reference to the `static mut`
/// twice at once.
#[inline]
unsafe fn expb() -> &'static mut BString {
    &mut *ptr::addr_of_mut!(expbuf)
}

/// `stackblock()`, for the expansion buffer.  Re-read after anything that
/// can grow it, exactly where the C re-reads `stackblock()`.
#[inline]
unsafe fn expbase() -> *mut c_char {
    expb().as_mut_ptr() as *mut c_char
}

/// The C's `expdest`, as a pointer.
#[inline]
unsafe fn expdest() -> *mut c_char {
    let b = expb();
    let n = b.len();
    b.as_mut_ptr().add(n) as *mut c_char
}

/// `expdest - stackblock()`.
#[inline]
unsafe fn expdest_off() -> c_int {
    expb().len() as c_int
}

/// `expdest = p` / `STADJUST(p - expdest, expdest)`.  `p` must point into
/// the buffer; the bytes below it have been written by a raw cursor.
#[inline]
unsafe fn set_expdest(p: *mut c_char) {
    let b = expb();
    let off = p.offset_from(b.as_mut_ptr() as *mut c_char) as usize;
    b.set_len(off);
}

/// `makestrspace(n, expdest)`: make `n` bytes writable past the cursor and
/// return a raw cursor at it.  The caller commits with [`set_expdest`], as
/// the C commits by assigning `expdest`.
#[inline]
unsafe fn expmakestrspace(n: size_t) -> *mut c_char {
    expb().reserve(n);
    expdest()
}

/// `stnputs(s, n, expdest)`, returning the new cursor.
#[inline]
unsafe fn expstnputs(s: *const c_char, n: size_t) -> *mut c_char {
    let b = expb();
    b.reserve(n);
    let len = b.len();
    ptr::copy_nonoverlapping(s as *const u8, b.as_mut_ptr().add(len), n);
    b.set_len(len + n);
    expdest()
}

/// `p = grabstackstr(expdest)`.
///
/// In the C this allocates nothing and copies nothing — it moves the bump
/// pointer past bytes that are already in place, which is how C says "these
/// outlive the next builder".  Owned, that is `mem::take`: the word's
/// buffer *becomes* the caller's, and what the next `expandarg`'s
/// `STARTSTACKSTR` clears is the empty one left behind.
///
/// While `strlist` was still a C structure this was a copy into the region,
/// because the consumers held `char *`.  They hold their own bytes now, so
/// the copy is gone rather than smaller.
unsafe fn grabexpdest() -> BString {
    let b = expb();
    /* `argstr` closes the word by masking its terminating marker to 0
     * (`*(q - 1) &= end - 1`), so the buffer is a C string and the
     * `strlen` `ifsbreakup` and `openhere` perform on it stops inside it.
     * The bytes belong to the word being handed over, terminator included;
     * `clear` on the next entry is what `mem::take` leaves behind. */
    debug_assert_eq!(b.last(), Some(&0), "argstr terminates the word");
    mem::take(b)
}

/// The result of an `expandarg(n, NULL, flag)` — the call that does *not*
/// grab its output.
///
/// Two callers: `redir::openhere` for a here-document and
/// `parser::expandstr` for `PS1`/`PS4`.  Both read the C's `stackblock()`
/// back after the call.  The bytes are NUL-terminated by `argstr`, which
/// forces the word's closing marker to 0 (`*(q - 1) &= end - 1`), and they
/// stay valid until the next expansion begins — where the C's were valid
/// only until the next `stalloc`.
pub unsafe fn expansion_result() -> *mut c_char {
    expbase()
}

// ---------------------------------------------------------------------
// The glob buffer.  See [`globbuf`].
// ---------------------------------------------------------------------

/// The candidate path `expmeta` is building.
///
/// The C has no name for this: it is the stack block, addressed through
/// `expmeta`'s locals `cp` (the base, `growstackto`'s return) and `enddir`
/// (the cursor, `cp + expdir_len` plus whatever has been appended). Every
/// frame of the recursion owns `[0, expdir_len)` — the directory prefix its
/// parent wrote, ending in `/` — and writes the next component above it.
///
/// Owned, the base stops moving for the region's reasons and starts moving
/// for `Vec`'s, so the C's `cp = ...; enddir = cp + expdir_len` re-derivation
/// stays exactly where it is. What changes is the one property the region
/// had and a `Vec` does not: **the region copies the whole block when it
/// grows, `Vec::reserve` copies only the first `len` bytes.** So the
/// invariant this buffer is held to is
///
/// > at every point where the glob buffer can grow, its length is the
/// > current frame's `expdir_len`.
///
/// which is what makes the prefix survive. It is asserted on entry to
/// `expmeta`, re-established by hand at the one place `expdir_len` changes
/// mid-frame, and re-derived from the cursor by [`globstnputs`], exactly as
/// the C's `makestrspace` derives it (`len = p - stacknxt`).
///
/// A `static` rather than a parameter for the same reason [`expbuf`] is one:
/// the cursors are raw pointers that outlive the borrow that produced them,
/// and `expmeta` recurses. `expandmeta` is the only entry and holds `INTOFF`
/// across it, so there is never a second glob in flight.
static mut globbuf: BString = BString::new(Vec::new());

/// `&mut globbuf`, without ever naming a reference to the `static mut`
/// twice at once.
#[inline]
unsafe fn globb() -> &'static mut BString {
    &mut *ptr::addr_of_mut!(globbuf)
}

/// `stackblock()`, for the glob buffer. Re-read after anything that can
/// grow it, exactly where the C re-reads `stackblock()`.
#[inline]
unsafe fn globbase() -> *mut c_char {
    globb().as_mut_ptr() as *mut c_char
}

/// `memalloc.c`: `growstackto(len)` — make `len` bytes writable from the
/// base and return it.
///
/// The reservation is the C's number, `expdir_len + name_len + 1`, and it is
/// exact rather than generous: `expmeta_rmescapes` writes `name` at the
/// cursor through a raw pointer carrying no bound of its own, and what kept
/// the C inside the block is that a region block is never smaller than 504
/// bytes and doubles. So both `expmeta_rmescapes` call sites assert the
/// bound the C left to that — against `len` rather than against the
/// capacity, because a `Vec` over-allocates too and a capacity that fits
/// would prove nothing about the arithmetic.
///
/// The arithmetic is right because `name_len == strlen(name)` at every
/// entry: the top-level call passes `strlen`, and the recursion passes
/// `name_len - (endname - name)` for a `name` whose temporary NUL at
/// `zeroedp` sits strictly below `endname` and so cannot shorten it.
#[inline]
unsafe fn globgrowto(len: size_t) -> *mut c_char {
    let b = globb();
    let have = b.len();
    b.reserve(len.saturating_sub(have));
    b.as_mut_ptr() as *mut c_char
}

/// `memalloc.c`: `stnputs(s, n, p)` — append `n` bytes at the cursor `p`
/// and return the new cursor.
///
/// `p` carries the length, as it does in the C: `makestrspace(n, p)` opens
/// with `len = p - stacknxt`, so an append at a cursor below the end of the
/// buffer discards what was above it. That is not incidental here — it is
/// how the frame that a recursive `expmeta` returned into gets its own
/// `expdir_len` back.
#[inline]
unsafe fn globstnputs(s: *const c_char, n: size_t, p: *mut c_char) -> *mut c_char {
    let b = globb();
    let off = p.offset_from(b.as_mut_ptr() as *mut c_char) as size_t;
    debug_assert!(off <= b.len());
    b.set_len(off);
    b.reserve(n);
    ptr::copy_nonoverlapping(s as *const u8, b.as_mut_ptr().add(off), n);
    b.set_len(off + n);
    b.as_mut_ptr().add(off + n) as *mut c_char
}

/// `syntax.h`: `#define BASESYNTAX (basesyntax + SYNBASE)`
#[inline]
unsafe fn BASESYNTAX() -> *const c_char {
    crate::syntax::basesyntax
        .as_ptr()
        .offset(crate::syntax::SYNBASE as isize)
}

/// `syntax.h`: `#define SQSYNTAX (sqsyntax + SYNBASE)`
#[inline]
unsafe fn SQSYNTAX() -> *const c_char {
    crate::syntax::sqsyntax
        .as_ptr()
        .offset(crate::syntax::SYNBASE as isize)
}

/// Backing store for [`is_type_unbiased`]. See that function for why the
/// 129 leading zero bytes exist; they are never read as data, only as the
/// answer to an out-of-bounds classification query.
static IS_TYPE_UNBIASED_PAD: [c_char; 129 + 257] = {
    let mut t = [0 as c_char; 129 + 257];
    let mut i = 0;
    while i < 257 {
        t[129 + i] = crate::syntax::is_type[i];
        i += 1;
    }
    t
};

/// `syntax.h`: the classification table as `memtodest` uses it.
///
/// `memtodest` passes this table **unbiased** — plain `is_type`, where
/// every other user writes `is_type + SYNBASE`. Its consumers then index
/// it with a `(signed char)`, and `mbtodest` additionally reads
/// `syntax[CTLMBCHAR]` with `CTLMBCHAR == -123`. So in the C, every input
/// byte >= 0x80 (and the CTLMBCHAR probe) reads up to 129 bytes *before*
/// the array — undefined behaviour whose result is decided by whatever
/// the linker happened to place there. In the reference build that is
/// `nodesize` and `defpathvar`.
///
/// This port does NOT reproduce that read, and the deviation is
/// deliberate. Reproducing an out-of-bounds read does not reproduce the
/// C's *behaviour*: the byte it yields is a property of one binary's
/// layout, so the C and the port would each be reading independently
/// arbitrary memory, and could silently disagree the moment either side
/// is relaid out. It is also genuine UB on the Rust side.
///
/// Instead the window is made real and zero-filled. Zero is `CWORD`, and
/// the only question ever asked of these slots is `== CCTL`, so the port
/// answers "no framing" — deterministically, in bounds. That is what both
/// the reference C and this port were measured to do: the byte at
/// `is_type - 123` is `0xE2` in the C binary and the classification is
/// `!= CCTL` in both. `[spec:dash:sem:expand.memtodest-fn]` requires this
/// treatment ("a port must not reproduce the out-of-bounds index").
#[inline]
unsafe fn is_type_unbiased() -> *const c_char {
    IS_TYPE_UNBIASED_PAD.as_ptr().add(129)
}

/// `syntax.h`: syntax class "like CWORD, except it must be escaped".
#[inline]
fn CCTL() -> c_char {
    crate::syntax::CCTL as c_char
}

/// `options.h`: `#define fflag optlist[1]`
#[inline]
unsafe fn fflag() -> c_char {
    crate::options::optlist[1]
}

/// `options.h`: `#define uflag optlist[14]`
#[inline]
unsafe fn uflag() -> c_char {
    crate::options::optlist[14]
}

/// `error.h`: `#define int_pending() intpending`
#[inline]
unsafe fn int_pending() -> c_int {
    crate::error::int_pending()
}

// ---------------------------------------------------------------------
// Flag-value guards.  The C has `#error` directives for both of these;
// the branchless expressions in `memtodest` and `varvalue` are only
// correct for these exact numeric values.
// ---------------------------------------------------------------------

/* #if QUOTES_ESC != 0x11 || EXP_MBCHAR != 0x20 || EXP_QUOTED != 0x100
 * #error QUOTES_ESC != 0x11 || EXP_MBCHAR != 0x20 || EXP_QUOTED != 0x100
 * #endif */
const _: () = assert!(QUOTES_ESC == 0x11 && EXP_MBCHAR == 0x20 && EXP_QUOTED == 0x100);

/* #if EXP_QUOTED >> CHAR_BIT != EXP_FULL
 * #error The following two lines expect EXP_QUOTED == EXP_FULL << CHAR_BIT
 * #endif */
const _: () = assert!(EXP_QUOTED >> CHAR_BIT == EXP_FULL);

/*
 * Prepare a pattern for a glob(3) call.
 *
 * Returns an stalloced string.
 */

// [spec:dash:def:expand.preglob-fn]
// [spec:dash:sem:expand.preglob-fn]
unsafe fn preglob(
    pattern: *const c_char,
    mut flag: c_int,
    heap: Option<&mut Vec<u8>>,
) -> *mut c_char {
    if FNMATCH_IS_ENABLED {
        if flag == 0 {
            flag = RMESCAPE_GROW;
        }
        flag |= RMESCAPE_ALLOC;
    }
    flag |= RMESCAPE_GLOB;
    _rmescapes(pattern as *mut c_char, flag, heap)
}

// [spec:dash:def:expand.mesclen-fn]
// [spec:dash:sem:expand.mesclen-fn]
unsafe fn mesclen(start: *const c_char, mut p: *const c_char, mesc: c_char) -> size_t {
    let mut esc: size_t = 0;

    while p > start && {
        p = p.offset(-1);
        *p == mesc
    } {
        esc += 1;
    }
    esc
}

// [spec:dash:def:expand.esclen-fn]
// [spec:dash:sem:expand.esclen-fn]
unsafe fn esclen(start: *const c_char, p: *const c_char) -> size_t {
    mesclen(start, p, CTLESC)
}

// [spec:dash:def:expand.mbnext-fn]
// [spec:dash:sem:expand.mbnext-fn]
//
// Returns `start | end << 8`: the low byte is the offset from `p` to the
// character's data (past any markers), the next byte the span *from that
// data position* to the end of the encoded character.  The total span
// from `p` is therefore `(mb & 0xff) + (mb >> 8)`, which is why that
// expression appears at every call site.
#[inline(never)]
unsafe fn mbnext(p: *const c_char) -> c_uint {
    let mut start: c_uint = 0;
    let mut end: c_uint = 0;
    let ml: c_uint;
    let c: c_int;

    c = *p.offset(end as isize) as c_int;
    end += 1;

    match c as c_char {
        CTLMBCHAR => {
            if *p.offset(end as isize) == CTLESC {
                end += 1;
            }
            ml = *(p.offset(end as isize) as *const u8) as c_uint;
            end += 1;
            start = end;
            end = ml + 2;
        }
        CTLESC => {
            start += 1;
        }
        _ => {}
    }

    start | end << 8
}

// [spec:dash:def:expand.getpwhome-fn]
// [spec:dash:sem:expand.getpwhome-fn]
#[inline]
unsafe fn getpwhome(name: *const c_char) -> *const c_char {
    /* #ifdef HAVE_GETPWNAM */
    let pw: *mut libc::passwd = libc::getpwnam(name);
    if !pw.is_null() {
        (*pw).pw_dir
    } else {
        ptr::null()
    }
    /* #else
     *	return 0;
     * #endif */
}

/*
 * Perform variable substitution and command substitution on an argument,
 * placing the resulting list of arguments in arglist.  If EXP_FULL is true,
 * perform splitting and file name expansion.  When arglist is NULL, perform
 * here document expansion.
 */

// [spec:dash:def:expand.expandarg-fn]
// [spec:dash:sem:expand.expandarg-fn]
pub unsafe fn expandarg(
    arg: &crate::nodes::Node,
    arglist: Option<&mut arglist>,
    flag: c_int,
) -> Result<(), Error> {
    let mut p: BString;

    argbackq = arg.narg().backquote.as_slice();
    /* STARTSTACKSTR(expdest) */
    expb().clear();
    /* The `?`s in this function return past the `ifsfree()` below, exactly
     * as the longjmp they replace jumped past it. The IFS regions are
     * reclaimed by the catch frame instead — `restore_handler_expandarg`'s
     * swallowing arm and `init::exitreset` both call `ifsfree`, which is
     * docs/errors-are-values.md 2.2's mark-keyed cleanup working as
     * designed. Adding one here would free them twice. */
    argstr(arg.narg().text.as_ptr(), flag)?;
    'out: {
        let Some(arglist) = arglist else {
            /* here document expanded — the caller reads the buffer back
             * through `expansion_result()`. */
            break 'out;
        };
        p = grabexpdest();
        /* `exparg.lastp = &exparg.list`.  It re-points the tail at the
         * head, which discards whatever the previous call left there —
         * reachable only when that call unwound between building the list
         * and splicing it into its caller's. */
        expargl().clear();
        /*
         * TODO - EXP_REDIR
         */
        if (flag & EXP_FULL) != 0 {
            /* The fields copy out of the word rather than pointing into
             * it, so the word itself is a local that dies at the end of
             * this block.  The C could not do that: its fields *are*
             * offsets into the grabbed block, which is why the block had to
             * outlive them and why the enclosing mark had to be the thing
             * that freed it. */
            ifsbreakup(
                p.as_mut_ptr() as *mut c_char,
                -1,
                &mut *ptr::addr_of_mut!(exparg),
            );
            /* `*exparg.lastp = NULL; exparg.lastp = &exparg.list;` —
             * terminate the fields `ifsbreakup` built, then re-point the
             * tail at the head so `expandmeta` rebuilds the list while
             * walking the one it was handed.  The first append there
             * overwrites the head, which is why the C can read `str->next`
             * before the write reaches it; taking the `Vec` is both
             * halves. */
            let words = mem::take(expargl());
            expandmeta(words)?;
        } else {
            expargl().push(strlist { text: p });
        }
        /* `if (exparg.list) { *arglist->lastp = exparg.list; arglist->lastp
         * = exparg.lastp; }`.  The C guards on emptiness because splicing a
         * NULL head would leave the caller's tail pointing at `exparg`'s
         * own head; appending an empty `Vec` is already a no-op. */
        arglist.list.append(expargl());
    }

    /* out: */
    ifsfree();
    Ok(())
}

/*
 * Perform variable and command substitution.  If EXP_FULL is set, output CTLESC
 * characters to allow for further processing.  Otherwise treat
 * $@ like $* since no splitting will be performed.
 */

// [spec:dash:def:expand.argstr-fn]
// [spec:dash:sem:expand.argstr-fn]
unsafe fn argstr(mut p: *mut c_char, mut flag: c_int) -> Result<*mut c_char, Error> {
    static spclchars: [c_char; 11] = [
        C_EQUALS,
        C_COLON,
        CTLQUOTEMARK,
        CTLENDVAR,
        CTLESC,
        CTLVAR,
        CTLBACKQ,
        CTLMBCHAR,
        CTLARI,
        CTLENDARI,
        0,
    ];
    let mut reject: *const c_char = spclchars.as_ptr();
    let mut c: c_int;
    let breakall: c_int = ((flag & (EXP_WORD | EXP_QUOTED)) == EXP_WORD) as c_int;
    let mut inquotes: c_int;
    let mut length: size_t;
    let mut startloc: c_int;

    reject = reject.offset(if (flag & EXP_VARTILDE2) != 0 { 1 } else { 0 });
    reject = reject.offset(if (flag & EXP_VARTILDE) != 0 { 0 } else { 2 });
    inquotes = 0;
    length = 0;

    /* `tilde:` is a label inside this `if`; `goto tilde` re-runs only the
     * `*p == '~'` test, which is what `do_tilde` models. */
    let mut do_tilde = false;
    if (flag & EXP_TILDE) != 0 {
        flag &= !EXP_TILDE;
        do_tilde = true;
    }

    'start: loop {
        if do_tilde {
            /* tilde: */
            do_tilde = false;
            if *p == C_TILDE {
                p = exptilde(p, flag);
            }
        }
        /* start: */
        startloc = expdest_off();
        loop {
            let ml: c_uint;
            let mb: c_uint;
            let end: c_int;

            /* `strcspn(p + length, reject)`: the run of bytes that are
             * neither the terminator nor in the reject set. Counted
             * rather than found with `find_byteset`, because this loop
             * re-enters after every control byte and taking the whole
             * remaining string each time would turn one pass over a word
             * into one pass per escape. */
            let rejectset = CStr::from_ptr(reject).to_bytes();
            let from = p.offset(length as isize);
            length += (0usize..)
                .take_while(|&i| {
                    let c = *from.add(i);
                    c != 0 && !rejectset.contains(&(c as u8))
                })
                .count();
            c = *p.offset(length as isize) as c_int;
            if (c & 0x80) == 0 || c == CTLENDARI as c_int || c == CTLENDVAR as c_int {
                /*
                 * c == '=' || c == ':' || c == '\0' ||
                 * c == CTLENDARI || c == CTLENDVAR
                 */
                length += 1;
                /* c == '\0' || c == CTLENDARI || c == CTLENDVAR */
                end = (((c - 1) & 0x80) != 0) as c_int;
            } else {
                end = 0;
            }
            if length > 0 && (flag & EXP_DISCARD) == 0 {
                let newloc: c_int;
                let q: *mut c_char;

                q = expstnputs(p, length);
                *q.offset(-1) &= (end - 1) as c_char;
                /* `end` is 1 exactly when the byte just written closed the
                 * word (NUL, CTLENDVAR or CTLENDARI), and the line above
                 * has already turned it into a NUL.  Under EXP_WORD the
                 * cursor steps back over it, so it lands past the length —
                 * the outer `argstr` overwrites it on its next append. */
                let q_off = q.offset_from(expbase()) as c_int;
                set_expdest(q.offset(-((if (flag & EXP_WORD) != 0 { end } else { 0 }) as isize)));
                newloc = q_off - end;
                if breakall != 0 && inquotes == 0 && newloc > startloc {
                    recordregion(startloc, newloc, 0);
                }
                startloc = newloc;
            }
            p = p.offset(length as isize + 1);
            length = 0;

            if end != 0 {
                break 'start;
            }

            match c as c_char {
                C_EQUALS | C_COLON => {
                    if (c as c_char) == C_EQUALS {
                        flag |= EXP_VARTILDE2;
                        reject = reject.offset(1);
                        /* fall through */
                    }
                    /*
                     * sort of a hack - expand tildes in variable
                     * assignments (after the first '=' and after ':'s).
                     */
                    p = p.offset(-1);
                    if *p == C_TILDE {
                        do_tilde = true;
                        continue 'start; /* goto tilde */
                    }
                    continue;
                }
                CTLQUOTEMARK => {
                    /* "$@" syntax adherence hack */
                    if inquotes == 0
                        && CStr::from_ptr(p).to_bytes()
                            == CStr::from_ptr(crate::mystring::dolatstr.as_ptr().offset(1))
                                .to_bytes()
                    {
                        p = evalvar(p.offset(1), flag | EXP_QUOTED)?.offset(1);
                        continue 'start; /* goto start */
                    }
                    inquotes ^= EXP_QUOTED;
                    /* addquote: */
                    if (flag & QUOTES_ESC) != 0 {
                        p = p.offset(-1);
                        length += 1;
                        startloc += 1;
                    }
                }
                CTLMBCHAR => {
                    c = *p as c_int;
                    p = p.offset(-1);
                    mb = mbnext(p);
                    ml = (mb >> 8) - 2;
                    if (flag & (QUOTES_ESC | EXP_MBCHAR)) != 0 {
                        length = ((mb >> 8) + (mb & 0xff)) as size_t;
                        if (c as c_char) == CTLESC {
                            startloc += length as c_int;
                        }
                    } else {
                        if c == CTLESC as c_int {
                            startloc += ml as c_int;
                        }
                        p = p.offset((mb & 0xff) as isize);
                        if (flag & EXP_DISCARD) == 0 {
                            expstnputs(p, ml as size_t);
                        }
                        p = p.offset((mb >> 8) as isize);
                    }
                }
                CTLESC => {
                    startloc += 1;
                    length += 1;
                    /* goto addquote */
                    if (flag & QUOTES_ESC) != 0 {
                        p = p.offset(-1);
                        length += 1;
                        startloc += 1;
                    }
                }
                CTLVAR => {
                    p = evalvar(p, flag | inquotes)?;
                    continue 'start; /* goto start */
                }
                CTLBACKQ => {
                    expbackq((&*argbackq)[0].as_ref(), flag | inquotes)?;
                    continue 'start; /* goto start */
                }
                CTLARI => {
                    p = expari(p, flag | inquotes)?;
                    continue 'start; /* goto start */
                }
                _ => {}
            }
        }
    }
    Ok(p.offset(-1))
}

// [spec:dash:def:expand.exptilde-fn]
// [spec:dash:sem:expand.exptilde-fn]
unsafe fn exptilde(startp: *mut c_char, flag: c_int) -> *mut c_char {
    let mut c: c_char;
    let name: *mut c_char;
    let home: *const c_char;
    let mut p: *mut c_char;

    p = startp;
    name = p.offset(1);

    loop {
        p = p.offset(1);
        c = *p;
        if c == C_NUL {
            break;
        }
        match c {
            CTLESC => return startp,
            CTLQUOTEMARK => return startp,
            C_COLON => {
                if (flag & EXP_VARTILDE) != 0 {
                    break; /* goto done */
                }
            }
            C_SLASH | CTLENDVAR => break, /* goto done */
            _ => {}
        }
    }
    /* done: */
    'out: {
        if (flag & EXP_DISCARD) != 0 {
            break 'out;
        }
        *p = C_NUL;
        if *name == C_NUL {
            home = crate::var::lookupvar(crate::mystring::homestr.as_ptr());
        } else {
            home = getpwhome(name);
        }
        *p = c;
        if home.is_null() {
            /* lose: */
            return startp;
        }
        strtodest(home, flag | EXP_QUOTED, expb());
    }
    /* out: */
    p
}

// [spec:dash:def:expand.removerecordregions-fn]
// [spec:dash:sem:expand.removerecordregions-fn]
pub unsafe fn removerecordregions(endoff: c_int) {
    /* `ifslastp == NULL` */
    if ifsr().is_empty() {
        return;
    }

    /* `ifsfirst` is index 0; `ifslastp` is the index the walk below
     * settles on, and dropping the tail is `truncate`.  The C frees one
     * node per INTOFF/INTON pair, so the loops do too. */
    if ifsr()[0].endoff > endoff {
        while ifsr().len() > 1 {
            crate::error::INTOFF();
            ifsr().pop();
            crate::error::INTON();
        }
        if ifsr()[0].begoff > endoff {
            ifsr().clear();
        } else {
            ifsr()[0].endoff = endoff;
        }
        return;
    }

    let mut last: usize = 0;
    while last + 1 < ifsr().len() && ifsr()[last + 1].begoff < endoff {
        last += 1;
    }
    while ifsr().len() > last + 1 {
        crate::error::INTOFF();
        ifsr().pop();
        crate::error::INTON();
    }
    if ifsr()[last].endoff > endoff {
        ifsr()[last].endoff = endoff;
    }
}

/*
 * Expand arithmetic expression.  Backup to start of expression,
 * evaluate, place result in (backed up) result, adjust string position.
 */

// [spec:dash:def:expand.expari-fn]
// [spec:dash:sem:expand.expari-fn]
unsafe fn expari(mut start: *mut c_char, flag: c_int) -> Result<*mut c_char, Error> {
    let begoff: c_int;
    let len: c_int;
    let result: intmax_t;
    /* The C's `p` doubles as a scratch `stackblock()` before it becomes the
     * return value; only the second use survives. */
    let p: *mut c_char;

    begoff = expdest_off();
    p = argstr(start, flag & EXP_DISCARD)?;

    'out: {
        if (flag & EXP_DISCARD) != 0 {
            break 'out;
        }

        /* `start = stackblock() + begoff; STADJUST(start - expdest, expdest)`
         * winds the cursor back over the arithmetic text, which then stays
         * where it is and is read by `arith` from *past* the cursor.
         *
         * The C protects it with `pushstackmark(&sm, endoff)`, which does
         * two jobs: `endoff` is how much of the region the half-built word
         * occupies, and grabbing that much keeps `arith`'s own `stalloc`s
         * off it, while the save/restore releases them afterwards.  Neither
         * job has a customer left.
         *
         * The reservation is replaced by an argument rather than a
         * mechanism: `arith` (and `yylex` under it) do not touch this
         * buffer, so the bytes above the truncated length cannot be moved
         * or overwritten while `arith` reads them.  `Vec::reserve` would
         * move them — it copies only the first `len` bytes — but nothing on
         * this path reserves.  And what the restore released,
         * `arith_yylex`'s variable names, is a list `arith` clears on
         * entry. */
        expb().truncate(begoff as usize);
        start = expbase().offset(begoff as isize);

        removerecordregions(begoff);

        /* `arith` returns its diagnostic now instead of raising it, and as
         * of this commit so does `expari`, so the bridge that stood here is
         * gone and the value travels. */
        result = crate::arith_yacc::arith(start)?;

        len = cvtnum(result, flag, expb()) as c_int;

        if (flag & EXP_QUOTED) == 0 {
            recordregion(begoff, begoff + len, 0);
        }
    }

    /* out: */
    Ok(p)
}

/*
 * Expand stuff in backwards quotes.
 */

// [spec:dash:def:expand.expbackq-fn]
// [spec:dash:sem:expand.expbackq-fn]
unsafe fn expbackq(cmd: Option<&crate::nodes::Node>, flag: c_int) -> Result<(), Error> {
    let mut in_: crate::eval::backcmd = mem::zeroed();
    let mut i: c_int;
    let mut buf: [c_char; 128] = [0; 128];
    let mut p: *mut c_char;
    let mut dest: *mut c_char;
    let startloc: c_int;

    'out: {
        if (flag & EXP_DISCARD) != 0 {
            break 'out;
        }

        crate::error::INTOFF();
        startloc = expdest_off();
        /* `pushstackmark(&smark, startloc)`: the length kept `makejob`'s
         * region allocations off the half-built word, and the save/restore
         * released them afterwards.  The word is not in the region and
         * neither is anything `evalbackcmd` reaches, so both halves are
         * gone. */
        /* This `?` and the one below return between this frame's `INTOFF`
         * and its `INTON`, which is where the longjmp went too — it skipped
         * the same `INTON`. docs/errors-are-values.md 2.4: do not pair
         * them. */
        /* TRANSITIONAL: `expbackq` has no context to pass on yet.
         * Threading `expand.rs` removes this. */
        let mut sh = crate::context::Shell::detached();
        crate::eval::evalbackcmd(&mut sh, cmd, &mut in_ as *mut crate::eval::backcmd)?;

        /* `backcmd.buf` is ash's read-ahead buffer.  `evalbackcmd` writes
         * NULL to it and to `nleft` and never writes either again, so the
         * first `memtodest` below is always the one `goto read` skips and
         * the C's closing `if (in.buf) ckfree(in.buf)` is unreachable.
         * There is no allocation on this path to own, and the free is
         * asserted away rather than transcribed. */
        debug_assert!(
            in_.buf.is_null() && in_.nleft == 0,
            "expbackq: evalbackcmd left a read-ahead buffer nothing frees"
        );

        p = in_.buf;
        i = in_.nleft;
        /* `if (i == 0) goto read;` — skips the first memtodest only. */
        let mut jump_read = i == 0;
        loop {
            if !jump_read {
                memtodest(p, i as size_t, flag, expb());
            }
            jump_read = false;
            /* read: */
            if in_.fd < 0 {
                break;
            }
            loop {
                i = libc::read(
                    in_.fd,
                    buf.as_mut_ptr() as *mut c_void,
                    mem::size_of_val(&buf),
                ) as c_int;
                if !(i < 0 && crate::system::errno() == libc::EINTR) {
                    break;
                }
                /* One of the three EINTR sites the C retries blindly.
                 * Reading a command substitution's output can block for
                 * as long as the substituted command runs, so this is
                 * where a ^C during `x=$(sleep 5)` is noticed. */
                if let Some(e) = crate::error::poll_interrupt() {
                    return Err(e);
                }
            }
            /* TRACE(("expbackq: read returns %d\n", i)); */
            if i <= 0 {
                break;
            }
            p = buf.as_mut_ptr();
        }

        if in_.fd >= 0 {
            libc::close(in_.fd);
            crate::eval::back_exitstatus = crate::jobs::waitforjob(in_.jp)?;
        }
        crate::error::INTON();

        /* Eat all trailing newlines */
        dest = expdest();
        while dest > expbase().offset(startloc as isize) && *dest.offset(-1) == C_NL {
            /* STUNPUTC(dest) */
            dest = dest.offset(-1);
        }
        set_expdest(dest);

        if (flag & EXP_QUOTED) == 0 {
            recordregion(startloc, expdest_off(), 0);
        }
        /* TRACE(("evalbackq: size=%d: \"%.*s\"\n", ...)); */
    }

    /* out: */
    argbackq = (&*argbackq)[1..].as_ref() as *const [Option<crate::nodes::Node>];
    Ok(())
}

// [spec:dash:def:expand.scanleft-fn]
// [spec:dash:sem:expand.scanleft-fn]
unsafe fn scanleft(
    startp: *mut c_char,
    endp: *mut c_char,
    rmesc: *mut c_char,
    rmescend: *mut c_char,
    str: *mut c_char,
    quotes: c_int,
    zero: c_int,
) -> *mut c_char {
    let mut loc: *mut c_char;
    let mut loc2: *mut c_char;
    let mut c: c_char;

    loc = startp;
    loc2 = rmesc;
    loop {
        let mut s: *mut c_char = if FNMATCH_IS_ENABLED { loc2 } else { loc };
        let mb: c_uint;
        let ml: c_uint;
        let match_: c_int;

        c = *s;
        if zero != 0 {
            *s = C_NUL;
            s = if FNMATCH_IS_ENABLED { rmesc } else { startp };
        }
        match_ = pmatch(str, s);
        *(if FNMATCH_IS_ENABLED { loc2 } else { loc }) = c;
        if match_ != 0 {
            return if quotes != 0 { loc } else { loc2 };
        }

        if c == C_NUL {
            break;
        }

        mb = mbnext(loc);
        loc = loc.offset(((mb & 0xff) + (mb >> 8)) as isize);
        ml = if (mb >> 8) > 3 { (mb >> 8) - 2 } else { 1 };
        loc2 = loc2.offset(ml as isize);
    }
    ptr::null_mut()
}

// [spec:dash:def:expand.scanright-fn]
// [spec:dash:sem:expand.scanright-fn]
unsafe fn scanright(
    startp: *mut c_char,
    endp: *mut c_char,
    rmesc: *mut c_char,
    rmescend: *mut c_char,
    str: *mut c_char,
    quotes: c_int,
    zero: c_int,
) -> *mut c_char {
    let mut esc: size_t = 0;
    let mut loc: *mut c_char;
    let mut loc2: *mut c_char;

    loc = endp;
    loc2 = rmescend;
    /* `for (;; loc2--)` — the `continue`s below must still run `loc2--`,
     * hence the inner labelled block. */
    'forloop: loop {
        'cont: {
            let mut s: *mut c_char = if FNMATCH_IS_ENABLED { loc2 } else { loc };
            let c: c_char = *s;
            let ml: c_uint;
            let match_: c_int;

            if zero != 0 {
                *s = C_NUL;
                s = if FNMATCH_IS_ENABLED { rmesc } else { startp };
            }
            match_ = pmatch(str, s);
            *(if FNMATCH_IS_ENABLED { loc2 } else { loc }) = c;
            if match_ != 0 {
                return if quotes != 0 { loc } else { loc2 };
            }
            loc = loc.offset(-1);
            if loc < startp {
                break 'forloop;
            }
            /* if (!esc--) esc = esclen(startp, loc); */
            let was: size_t = esc;
            esc = esc.wrapping_sub(1);
            if was == 0 {
                esc = esclen(startp, loc);
            }
            if esc % 2 != 0 {
                esc -= 1;
                loc = loc.offset(-1);
                break 'cont; /* continue */
            }
            if *loc != CTLMBCHAR {
                break 'cont; /* continue */
            }

            loc = loc.offset(-1);
            ml = *(loc as *const u8) as c_uint;
            loc = loc.offset(-((ml + 2) as isize));
            if *loc == CTLESC {
                loc = loc.offset(-1);
            }
            loc2 = loc2.offset(-((ml.wrapping_sub(1)) as isize));
        }
        loc2 = loc2.offset(-1);
    }
    ptr::null_mut()
}

// [spec:dash:def:expand.subevalvar-fn]
// [spec:dash:sem:expand.subevalvar-fn]
unsafe fn subevalvar(
    start: *mut c_char,
    mut str: *mut c_char,
    strloc: c_int,
    startloc: c_int,
    varflags: c_int,
    flag: c_int,
) -> Result<*mut c_char, Error> {
    let mut subtype: c_int = varflags & VSTYPE;
    let quotes: c_int = flag & QUOTES_ESC;
    let mut startp: *mut c_char;
    let mut loc: *mut c_char;
    let mut rmesc: *mut c_char;
    let mut rmescend: *mut c_char;
    let zero: c_int;
    let scan: unsafe fn(
        *mut c_char,
        *mut c_char,
        *mut c_char,
        *mut c_char,
        *mut c_char,
        c_int,
        c_int,
    ) -> *mut c_char;
    let mut nstrloc: c_int = strloc;
    let endp: *mut c_char;
    let p: *mut c_char;

    p = argstr(
        start,
        (flag & EXP_DISCARD) | EXP_TILDE | (if !str.is_null() { 0 } else { EXP_CASE }),
    )?;
    if (flag & EXP_DISCARD) != 0 {
        return Ok(p);
    }

    startp = expbase().offset(startloc as isize);

    'out: {
        match subtype {
            VSASSIGN => {
                /* The bridge that stood here retires with this commit. */
                crate::var::setvar(str, startp, 0)?;

                loc = startp;
                break 'out;
            }

            VSQUESTION => {
                /* `varunset` stopped diverging with this commit, so this
                 * has to be a `return` and not a bare call. It was a stop
                 * before — docs/errors-are-values.md 0.2 is the bug that
                 * happens when one of these is missed, and `Error` is
                 * `#[must_use]` so the compiler now names it. */
                return Err(varunset(start, str, startp, varflags));
            }
            _ => {}
        }

        subtype -= VSTRIMRIGHT;
        /* #ifdef DEBUG
         *	if (subtype < 0 || subtype > 3)
         *		abort();
         * #endif */

        rmescend = expbase().offset(strloc as isize);
        str = preglob(rmescend, 0, None);
        if FNMATCH_IS_ENABLED {
            startp = expbase().offset(startloc as isize);
            rmescend = expbase().offset(strloc as isize);
            nstrloc = str.offset_from(expbase()) as c_int;
        }

        rmesc = startp;
        if FNMATCH_IS_ENABLED || quotes == 0 {
            /* `_rmescapes` with RMESCAPE_GROW appends an unescaped copy of
             * `startp` past the cursor and moves the cursor over it, so the
             * buffer can have reallocated underneath.  `rmesc` (its return)
             * and `rmescend` (the cursor it left) are both derived *after*
             * that growth and stay valid; `startp` and `str` are from
             * before and are re-derived, which is exactly why the C
             * re-reads `stackblock()` on these two lines. */
            rmesc = _rmescapes(startp, RMESCAPE_ALLOC | RMESCAPE_GROW, None);
            if rmesc != startp {
                rmescend = expdest();
            }
            startp = expbase().offset(startloc as isize);
            str = expbase().offset(nstrloc as isize);
        }
        rmescend = rmescend.offset(-1);

        /* zero = subtype == VSTRIMLEFT || subtype == VSTRIMLEFTMAX */
        zero = subtype >> 1;
        /* VSTRIMLEFT/VSTRIMRIGHTMAX -> scanleft */
        scan = if ((subtype & 1) ^ zero) != 0 {
            scanleft
        } else {
            scanright
        };

        endp = expbase().offset(strloc as isize - 1);
        loc = scan(startp, endp, rmesc, rmescend, str, quotes, zero);
        if loc.is_null() {
            if quotes != 0 {
                rmesc = startp;
                rmescend = endp;
            }
        } else if quotes == 0 {
            if zero != 0 {
                rmesc = loc;
            } else {
                rmescend = loc;
            }
        } else if zero != 0 {
            rmesc = loc;
            rmescend = endp;
        } else {
            rmesc = startp;
            rmescend = loc;
        }

        /* The two ranges are cursors into one buffer and may overlap,
         * so this is `ptr::copy` and not `copy_nonoverlapping`. */
        core::ptr::copy(rmesc, startp, rmescend.offset_from(rmesc) as usize);
        loc = startp.offset(rmescend.offset_from(rmesc));
    }

    /* out: */
    /* `*loc = '\0'; STADJUST(loc - expdest, expdest)` — the terminator is
     * written *at* the new cursor, so it lands one past the length rather
     * than inside it.  In the region that byte survives; in a `Vec` a later
     * reallocation would drop it, because `reserve` copies only the first
     * `len` bytes.  It does not matter: every path out of `argstr` writes
     * the word's own terminator (`*(q - 1) &= end - 1` forces the closing
     * NUL, CTLENDVAR or CTLENDARI to 0) before anything reads the buffer as
     * a string, and `loc` is always strictly below the cursor here, so the
     * byte is inside the initialised area until then.  `amount` was only
     * ever `loc - expdest`. */
    debug_assert!(loc <= expdest());
    *loc = C_NUL;
    set_expdest(loc);

    /* Remove any recorded regions beyond start of variable */
    removerecordregions(startloc);

    Ok(p)
}

/*
 * Expand a variable, and return a pointer to the next character in the
 * input string.
 */

// [spec:dash:def:expand.evalvar-fn]
// [spec:dash:sem:expand.evalvar-fn]
unsafe fn evalvar(mut p: *mut c_char, mut flag: c_int) -> Result<*mut c_char, Error> {
    let mut subtype: c_int;
    let mut varflags: c_int;
    let var: *mut c_char;
    let patloc: c_int;
    let startloc: c_int;
    let mut varlen: ssize_t;
    let mut discard: c_int;
    let mut quoted: c_int;
    let mbchar: c_int;

    varflags = (*p as c_int) & !VSBIT;
    p = p.offset(1);
    subtype = varflags & VSTYPE;

    quoted = flag & EXP_QUOTED;
    var = p;
    startloc = expdest_off();
    /* The parser always writes the `=` that ends the variable name, and
     * the C dereferences `strchr`'s result without checking. */
    p = p.add(
        CStr::from_ptr(p)
            .to_bytes()
            .find_byte(C_EQUALS as u8)
            .expect("the parser ends a variable name with `=`")
            + 1,
    );

    mbchar = match subtype {
        VSTRIMLEFT | VSTRIMLEFTMAX | VSTRIMRIGHT | VSTRIMRIGHTMAX => EXP_MBCHAR,
        _ => 0,
    };

    /* `record:` and `really_record:` are the two joins at the bottom. */
    let mut really_record = false;

    'again: loop {
        varlen = varvalue(var, varflags, (flag | mbchar) as c_uint)?;
        if (varflags & VSNUL) != 0 {
            varlen -= 1;
        }

        discard = if varlen < 0 { EXP_DISCARD } else { 0 };

        match subtype {
            VSPLUS | 0 | VSMINUS => {
                if subtype == VSPLUS {
                    discard ^= EXP_DISCARD;
                    /* fall through */
                }

                p = argstr(p, flag | EXP_TILDE | EXP_WORD | (discard ^ EXP_DISCARD))?;
                break 'again; /* goto record */
            }

            VSASSIGN | VSQUESTION => {
                p = subevalvar(
                    p,
                    var,
                    0,
                    startloc,
                    varflags,
                    (flag & !QUOTES_ESC) | (discard ^ EXP_DISCARD),
                )?;

                if ((flag | !discard) & EXP_DISCARD) != 0 {
                    break 'again; /* goto record */
                }

                varflags &= !VSNUL;
                subtype = VSNORMAL;
                continue 'again;
            }
            _ => {}
        }

        if (discard & !flag) != 0 && uflag() != 0 {
            /* A stop before `varunset` stopped diverging, and still one. */
            return Err(varunset(p, var, ptr::null(), 0));
        }

        if subtype == VSLENGTH {
            p = p.offset(1);
            if (flag & EXP_DISCARD) != 0 {
                return Ok(p);
            }
            cvtnum(
                (if varlen > 0 { varlen } else { 0 }) as intmax_t,
                flag,
                expb(),
            );
            really_record = true;
            break 'again; /* goto really_record */
        }

        if subtype == VSNORMAL {
            break 'again; /* goto record */
        }

        /* #ifdef DEBUG
         *	switch (subtype) {
         *	case VSTRIMLEFT: case VSTRIMLEFTMAX:
         *	case VSTRIMRIGHT: case VSTRIMRIGHTMAX:
         *		break;
         *	default:
         *		abort();
         *	}
         * #endif */

        flag |= discard;
        if (flag & EXP_DISCARD) == 0 {
            /*
             * Terminate the string and start recording the pattern
             * right after it
             */
            /* STPUTC('\0', expdest) */
            expb().push(0);
        }

        patloc = expdest_off();
        p = subevalvar(p, ptr::null_mut(), patloc, startloc, varflags, flag)?;
        break 'again;
    }

    /* record: */
    if !really_record {
        if ((flag | discard) & EXP_DISCARD) != 0 {
            return Ok(p);
        }
    }

    /* really_record: */
    if quoted != 0 {
        quoted = (*var == C_AT && crate::options::shellparam.nparam != 0) as c_int;
        if quoted == 0 {
            return Ok(p);
        }
    }
    recordregion(startloc, expdest_off(), quoted);
    Ok(p)
}

// [spec:dash:def:expand.chtodest-fn]
// [spec:dash:sem:expand.chtodest-fn]
unsafe fn chtodest(c: c_int, syntax: *const c_char, mut out: *mut c_char) -> *mut c_char {
    if *syntax.offset(c as isize) == CCTL() {
        /* USTPUTC(CTLESC, out) */
        *out = CTLESC;
        out = out.offset(1);
    }
    /* USTPUTC(c, out) */
    *out = c as c_char;
    out = out.offset(1);

    out
}

// [spec:dash:def:expand.mbpair]
#[repr(C)]
pub struct mbpair {
    pub ml: c_uint,
    pub ql: c_uint,
}

// [spec:dash:def:expand.mbtodest-fn]
// [spec:dash:sem:expand.mbtodest-fn]
unsafe fn mbtodest(
    mut p: *const c_char,
    mut q: *mut c_char,
    syntax: *const c_char,
    len: size_t,
) -> mbpair {
    let mut mbs: libc::mbstate_t = mem::zeroed();
    let mbp: mbpair;
    let q0: *mut c_char = q;
    let mut ml: size_t;

    p = p.offset(-1);
    ml = mbrlen(p, len, &mut mbs);
    'out: {
        if ml == (0 as size_t).wrapping_sub(2) || ml == (0 as size_t).wrapping_sub(1) || ml < 2 {
            q = chtodest(*p as c_int, syntax, q);
            ml = 1;
            break 'out;
        }

        /* `syntax[CTLMBCHAR]` — CTLMBCHAR is negative; see the note in
         * `memtodest` about the unbiased `is_type` table. */
        if *syntax.offset(CTLMBCHAR as isize) == CCTL() {
            /* USTPUTC(CTLMBCHAR, q); USTPUTC(ml, q); */
            *q = CTLMBCHAR;
            q = q.offset(1);
            *q = ml as c_char;
            q = q.offset(1);
        }

        q = crate::system::mempcpy(q as *mut c_void, p as *const c_void, ml) as *mut c_char;

        if *syntax.offset(CTLMBCHAR as isize) == CCTL() {
            /* USTPUTC(ml, q); USTPUTC(CTLMBCHAR, q); */
            *q = ml as c_char;
            q = q.offset(1);
            *q = CTLMBCHAR;
            q = q.offset(1);
        }
    }

    /* out: */
    mbp = mbpair {
        ml: (ml.wrapping_sub(1)) as c_uint,
        ql: q.offset_from(q0) as c_uint,
    };
    mbp
}

/*
 * Put a string on the stack.
 */

// [spec:dash:def:expand.memtodest-fn]
// [spec:dash:sem:expand.memtodest-fn]
//
// PORT: the C reads and writes the global `expdest`; here the destination
// cursor is a parameter.  This is not a tidying — the C's `expdest` is not
// *the* expansion's cursor, it is *a* cursor, and `expmeta` borrows it:
//
//     expdest = enddir;
//     memtodest(p, len, EXP_MBCHAR | EXP_KEEPNUL);
//     cp = stackblock();
//     enddir = cp + expdir_len;
//
// `enddir` points into the glob buffer, not into the word being expanded,
// so `memtodest` is already a generic "encode these bytes at this cursor"
// routine that happens to pass its argument through a global.  Naming the
// argument makes `expmeta` stop touching `expdest` at all, which is what
// lets the expansion buffer and the glob buffer be converted separately.
//
// `chtodest` and `mbtodest` already take theirs (`out`, `q`).
//
// The destination is an owned buffer and the cursor is its length, so
// `makestrspace` is `reserve` and the commit is a `set_len` over bytes this
// function's raw cursor has filled — the same shape `parser::getmbc_at`
// uses.  `p` never points into `dst`: every caller's source is a variable
// value, a `read` buffer, a `getpwnam` field or a stack array.
unsafe fn memtodest(
    mut p: *const c_char,
    mut len: size_t,
    flags: c_int,
    dst: &mut BString,
) -> size_t {
    let syntax: *const c_char;
    let mut count: size_t = 0;
    let expq: c_int;
    let mut q: *mut c_char;
    let base: *mut c_char;

    if len == 0 {
        return 0;
    }

    /* CTLMBCHAR, 2, c, c, 2, CTLMBCHAR */
    dst.reserve(len * 3);
    base = dst.as_mut_ptr() as *mut c_char;
    q = base.add(dst.len());

    /* Guarded by the `assert!(QUOTES_ESC == 0x11 && …)` above, which is
     * this file's port of the matching `#error`. */
    expq = flags & EXP_QUOTED;
    if (flags & (expq >> 3 | expq >> 4 | expq >> 8) & (QUOTES_ESC | EXP_MBCHAR)) == 0 {
        while len >= 8 {
            let x: u64;

            x = ptr::read_unaligned(p.offset(count as isize) as *const u64);

            if (x | x.wrapping_sub(0x0101010101010101)) & 0x8080808080808080 != 0 {
                break;
            }

            ptr::write_unaligned(q.offset(count as isize) as *mut u64, x);

            count += 8;
            len -= 8;
        }

        q = q.offset(count as isize);
        p = p.offset(count as isize);

        /* NOTE (bug-for-bug): `is_type` is used here *unbiased*, i.e.
         * without the `+ SYNBASE` every other syntax-table user applies.
         * `chtodest` only ever indexes it with 0..127, which is in range
         * and always reads 0 (never CCTL) — that is the point of the
         * choice.  `mbtodest` however indexes it with CTLMBCHAR (-123),
         * a read *before* the array; the C relies on that happening to
         * yield a non-CCTL byte.  Reproduced verbatim, not fixed. */
        syntax = if (flags & (QUOTES_ESC | EXP_MBCHAR)) != 0 {
            BASESYNTAX()
        } else {
            is_type_unbiased()
        };
    } else {
        syntax = SQSYNTAX();
    }

    /* for (; len; len--) */
    while len != 0 {
        'cont: {
            let c: c_int = *p as c_int;
            p = p.offset(1);

            if c == 0 && (flags & EXP_KEEPNUL) == 0 {
                break 'cont; /* continue */
            }

            count += 1;

            if c < 0 {
                let mbp: mbpair = mbtodest(p, q, syntax, len);
                let mlm: c_uint;

                q = q.offset(mbp.ql as isize);
                mlm = mbp.ml;
                p = p.offset(mlm as isize);
                len -= mlm as size_t;
                break 'cont; /* continue */
            }

            q = chtodest(c, syntax, q);
        }
        len -= 1;
    }

    /* `expdest = q` */
    dst.set_len(q.offset_from(base) as usize);
    count
}

// [spec:dash:def:expand.strtodest-fn]
// [spec:dash:sem:expand.strtodest-fn]
unsafe fn strtodest(p: *const c_char, flags: c_int, dst: &mut BString) -> size_t {
    let len: size_t = CStr::from_ptr(p).count_bytes();
    memtodest(p, len, flags, dst)
}

/*
 * Add the value of a specialized variable to the stack string.
 */

// [spec:dash:def:expand.varvalue-fn]
// [spec:dash:sem:expand.varvalue-fn]
unsafe fn varvalue(
    name: *mut c_char,
    varflags: c_int,
    mut flags: c_uint,
) -> Result<ssize_t, Error> {
    let subtype: c_int = varflags & VSTYPE;
    let mut seplen: size_t;
    let mut seps: *const c_char;
    let mut len: ssize_t = 0;
    let start: size_t;
    let discard: c_int;
    let mut ap: *mut *mut c_char;
    let mut num: c_int = 0;
    let mut p: *mut c_char = ptr::null_mut();
    let mut i: c_int;

    discard =
        ((subtype == VSPLUS || subtype == VSLENGTH) as c_int) | ((flags as c_int) & EXP_DISCARD);

    if subtype == 0 {
        if discard != 0 {
            return Ok(-1);
        }

        return Err(crate::error::sh_error_value(b"Bad substitution"));
    }

    flags &= if discard != 0 {
        (!QUOTES_ESC) as c_uint
    } else {
        !(0 as c_uint)
    };
    seps = crate::shell::nullstr.as_ptr();
    seplen = ((flags as c_int) & EXP_FULL) as size_t;
    start = expdest_off() as size_t;

    'sw: {
        'value: {
            'param: {
                'numvar: {
                    match *name {
                        C_DOLLAR => {
                            num = crate::shellmain::rootpid;
                            break 'numvar;
                        }
                        C_QUESTION => {
                            num = crate::eval::exitstatus;
                            break 'numvar;
                        }
                        C_HASH => {
                            num = crate::options::shellparam.nparam;
                            break 'numvar;
                        }
                        C_BANG => {
                            num = crate::jobs::backgndpid as c_int;
                            if num == 0 {
                                return Ok(-1);
                            }
                            break 'numvar;
                        }
                        C_MINUS => {
                            p = expmakestrspace(crate::options::NOPTS as size_t);
                            i = crate::options::NOPTS as c_int - 1;
                            while i >= 0 {
                                if crate::options::optlist[i as usize] != 0
                                    && crate::options::optletters[i as usize] != 0
                                {
                                    /* USTPUTC(optletters[i], p) */
                                    *p = crate::options::optletters[i as usize];
                                    p = p.offset(1);
                                    len += 1;
                                }
                                i -= 1;
                            }
                            set_expdest(p);
                            break 'sw;
                        }
                        C_AT | C_STAR => {
                            if *name == C_AT {
                                if ((flags as c_int) & (EXP_QUOTED | EXP_FULL))
                                    == (EXP_QUOTED | EXP_FULL)
                                {
                                    break 'param;
                                }
                                /* fall through to case '*' */
                            }
                            /* We will set seplen to 0 or !0 depending on
                             * whether we're doing field splitting.  We
                             * won't do field splitting if either we're
                             * quoted or seplen is zero.
                             *
                             * Instead of testing (quoted || !sep) the
                             * following trick optimises away any branches
                             * by using the fact that EXP_QUOTED (which is
                             * the only bit that can be set in quoted) is
                             * the same as EXP_FULL << CHAR_BIT (which is
                             * the only bit that can be set in sep).
                             */
                            seplen &= (!(flags >> CHAR_BIT)) as size_t;
                            if seplen == 0 {
                                seps = ncifs;
                            }
                            seplen = (seplen.wrapping_sub(1) & ifsmb0len.wrapping_sub(1))
                                .wrapping_add(1);
                            break 'param;
                        }
                        c if c >= C_0 && c <= C_9 => {
                            num = libc::atoi(name);
                            if num < 0 || num > crate::options::shellparam.nparam {
                                return Ok(-1);
                            }
                            p = if num != 0 {
                                *crate::options::shellparam_p().offset(num as isize - 1)
                            } else {
                                crate::options::arg0
                            };
                            break 'value;
                        }
                        _ => {
                            /* default: */
                            p = crate::var::lookupvar(name);
                            break 'value;
                        }
                    }
                }
                /* numvar: */
                len = cvtnum(num as intmax_t, flags as c_int, expb()) as ssize_t;
                break 'sw;
            }
            /* param: */
            ap = crate::options::shellparam_p();
            if ap.is_null() {
                return Ok(-1);
            }
            p = *ap;
            if p.is_null() {
                break 'sw;
            }
            loop {
                len += strtodest(p, flags as c_int, expb()) as ssize_t;

                ap = ap.offset(1);
                p = *ap;
                if p.is_null() {
                    break;
                }

                len += memtodest(seps, seplen, (flags as c_int) | EXP_KEEPNUL, expb()) as ssize_t;
            }
            break 'sw;
        }
        /* value: */
        if p.is_null() {
            return Ok(-1);
        }

        len = strtodest(p, flags as c_int, expb()) as ssize_t;
    }

    if discard != 0 {
        expb().truncate(start);
    }

    Ok(len)
}

/*
 * Record the fact that we have to scan this region of the
 * string for IFS characters.
 */

// [spec:dash:def:expand.recordregion-fn]
// [spec:dash:sem:expand.recordregion-fn]
pub unsafe fn recordregion(start: c_int, end: c_int, nulonly: c_int) {
    let r = ifsregion {
        begoff: start,
        endoff: end,
        nulonly,
    };

    /* Reusing the static head node and `ckmalloc`ing a fresh one are the
     * same push; what differs is the INTOFF/INTON bracket, which the C
     * only takes on the allocating path and which is kept because it
     * fixes where a pending SIGINT is delivered. */
    if ifsr().is_empty() {
        ifsr().push(r);
    } else {
        crate::error::INTOFF();
        ifsr().push(r);
        crate::error::INTON();
    }
}

// [spec:dash:def:expand.ifsisifs-fn]
// [spec:dash:sem:expand.ifsisifs-fn]
unsafe fn ifsisifs(p: *const c_char, ml: c_uint, ifs: *const c_char) -> c_uint {
    let mut isdefifs: bool = false;
    let mut isifs: bool = false;
    let mut wc: wchar_t = *p as wchar_t;
    /* C leaves `ifs0` uninitialised; it is only read when `isifs`, which
     * implies one of the branches below assigned it. */
    let mut ifs0: wchar_t = 0;

    'out: {
        if *ifs != 0 && !wcifsv().is_empty() {
            if (wc & 0x80) != 0 {
                let mut mbst: libc::mbstate_t = mem::zeroed();
                let mut wc2: wchar_t = 0;

                if mbrtowc(&mut wc2, p, ml as size_t, &mut mbst) != ml as size_t {
                    break 'out;
                }
                wc = wc2;
            }

            isifs = wcifs_chr(wcifsv(), wc);
            ifs0 = wcifsv()[0];
        } else if ml == 0 {
            /* `strchr` matches the terminator, so a NUL character --
             * which is what `ml == 0` means -- counts as an IFS byte.
             * `to_bytes_with_nul` keeps that. */
            isifs = CStr::from_ptr(ifs)
                .to_bytes_with_nul()
                .contains(&(wc as u8));
            ifs0 = *ifs as wchar_t;
        }

        if isifs {
            isdefifs = iswspace((if wc != 0 { wc } else { ifs0 }) as wint_t) != 0;
        }
    }

    /* out: */
    (isifs as c_uint) << 1 | (isdefifs as c_uint)
}

// [spec:dash:def:expand.ifsbreakup-slow-fn]
// [spec:dash:sem:expand.ifsbreakup-slow-fn]
unsafe fn ifsbreakup_slow(
    ifst: *mut ifs_state,
    arglist: &mut arglist,
    nulonly: c_int,
    mut p: *mut c_char,
) -> *mut c_char {
    let ifschar: c_uint;
    let sisifs: c_uint;
    let isdefifs: bool;
    let ml: c_uint;
    let isifs: bool;
    let mut q: *mut c_char;

    q = p;

    ifschar = mbnext(p);
    p = p.offset((ifschar & 0xff) as isize);
    ml = if (ifschar >> 8) > 3 {
        (ifschar >> 8) - 2
    } else {
        0
    };

    sisifs = ifsisifs(p, ml, (*ifst).ifs);
    p = p.offset((ifschar >> 8) as isize);

    isifs = (sisifs >> 1) != 0;
    isdefifs = (sisifs & 1) != 0;

    /* If only reading one more argument:
     * If we have exactly one field,
     * read that field without its terminator.
     * If we have more than one field,
     * read all fields including their terminators,
     * except for trailing IFS whitespace.
     *
     * This means that if we have only IFS
     * characters left, and at most one
     * of them is non-whitespace, we stop
     * reading here.
     * Otherwise, we read all the remaining
     * characters except for trailing
     * IFS whitespace.
     *
     * In any case, r indicates the start
     * of the characters to remove, or NULL
     * if no characters should be removed.
     */
    'out_zero_ifsspc: {
        if (*ifst).maxargs == 0 {
            if isdefifs {
                if (*ifst).r.is_null() {
                    (*ifst).r = q;
                }
                return p;
            }

            if !(isifs && (*ifst).ifsspc != 0) {
                (*ifst).r = ptr::null_mut();
            }
        } else if (*ifst).ifsspc != 0 {
            if isifs {
                q = p;
            }

            (*ifst).start = q;

            if isdefifs {
                return p;
            }
        } else if isifs {
            let mut ifsspc: c_int = (*ifst).ifsspc;

            if nulonly == 0 {
                ifsspc = isdefifs as c_int;
                (*ifst).ifsspc = ifsspc;
            }

            /* Ignore IFS whitespace at start */
            if q == (*ifst).start && ifsspc != 0 {
                (*ifst).start = p;
                break 'out_zero_ifsspc; /* goto out_zero_ifsspc */
            }
            /* if (ifst->maxargs > 0 && !--ifst->maxargs) */
            if (*ifst).maxargs > 0 && {
                (*ifst).maxargs -= 1;
                (*ifst).maxargs == 0
            } {
                (*ifst).r = q;
                return p;
            }
            *q = C_NUL;
            arglist.list.push(strlist::from_cstr((*ifst).start));
            (*ifst).start = p;
            return p;
        }
    }

    /* out_zero_ifsspc: */
    (*ifst).ifsspc = 0;
    p
}

/*
 * Break the argument string into pieces based upon IFS and add the
 * strings to the argument list.  The regions of the string to be
 * searched for IFS characters have been stored by recordregion.
 * If maxargs is non-negative, at most maxargs arguments will be created, by
 * joining together the last arguments.
 */

// [spec:dash:def:expand.ifsbreakup-fn]
// [spec:dash:sem:expand.ifsbreakup-fn]
pub unsafe fn ifsbreakup(string: *mut c_char, maxargs: c_int, arglist: &mut arglist) {
    let mut ifsp: usize;
    let mut ifst: ifs_state = mem::zeroed();
    let realifs: *const c_char;
    let mut nulonly: c_int;
    let mut p: *mut c_char;

    ifst.r = ptr::null_mut();
    ifst.start = string;
    ifst.maxargs = maxargs;
    'add: {
        if !ifsr().is_empty() {
            ifst.ifsspc = 0;
            nulonly = 0;
            realifs = ncifs;
            ifsp = 0;
            loop {
                let afternul: c_int;
                let endoff: c_int = ifsr()[ifsp].endoff;

                p = string.offset(ifsr()[ifsp].begoff as isize);
                afternul = nulonly;
                nulonly = ifsr()[ifsp].nulonly;
                ifst.ifs = if nulonly != 0 {
                    crate::shell::nullstr.as_ptr()
                } else {
                    realifs
                };
                ifst.ifsspc = 0;
                loop {
                    let p0: *mut c_char = p;

                    while string.offset(endoff as isize).offset_from(p) >= 8 {
                        /* union { uint64_t qw; unsigned char b[8]; } x; */
                        let qw: u64 = ptr::read_unaligned(p as *const u64);
                        let b: [u8; 8] = qw.to_ne_bytes();

                        if (qw & 0x8080808080808080) != 0 {
                            break;
                        }
                        if (ifsmap[b[0] as usize]
                            | ifsmap[b[1] as usize]
                            | ifsmap[b[2] as usize]
                            | ifsmap[b[3] as usize]
                            | ifsmap[b[4] as usize]
                            | ifsmap[b[5] as usize]
                            | ifsmap[b[6] as usize]
                            | ifsmap[b[7] as usize])
                            != 0
                        {
                            break;
                        }
                        p = p.offset(8);
                    }

                    if p != p0 {
                        if ifst.maxargs == 0 {
                            ifst.r = ptr::null_mut();
                        } else if ifst.ifsspc != 0 {
                            ifst.start = p0;
                        }
                        ifst.ifsspc = 0;
                    }

                    if p >= string.offset(endoff as isize) {
                        break;
                    }

                    p = ifsbreakup_slow(&mut ifst, arglist, afternul | nulonly, p);
                }

                ifsp += 1;
                if ifsp >= ifsr().len() {
                    break;
                }
            }
            if nulonly != 0 {
                break 'add; /* goto add */
            }
            if !ifst.r.is_null() {
                /* This is the one write into `string` that happens after
                 * `ifsbreakup_slow` has stopped emitting fields, and the
                 * fields no longer alias `string` — they copied out at the
                 * instant each was terminated.  So it has to land in the
                 * field that has *not* been created yet, which is the one
                 * `add:` below takes from `ifst.start`.  It does: `r` is
                 * only ever set once `maxargs` has reached 0, and the two
                 * branches that set it both return without emitting, so no
                 * field is taken between the two points. */
                debug_assert!(
                    ifst.r >= ifst.start,
                    "the trailing-IFS truncation lands in an already-taken field"
                );
                *ifst.r = C_NUL;
            }
        }

        if *ifst.start == C_NUL {
            return;
        }
    }

    /* add: */
    arglist.list.push(strlist::from_cstr(ifst.start));
}

// [spec:dash:def:expand.ifsfree-fn]
// [spec:dash:sem:expand.ifsfree-fn]
pub unsafe fn ifsfree() {
    /* The C frees the chain behind the static head under one
     * INTOFF/INTON and then nulls `ifslastp`; the head keeps its stale
     * contents, which is unobservable because every reader tests
     * `ifslastp` first.  Emptying the `Vec` is both halves. */
    if ifsr().len() > 1 {
        crate::error::INTOFF();
        ifsr().truncate(1);
        crate::error::INTON();
    }
    ifsr().clear();
}

// [spec:dash:def:expand.changeifs-fn]
// [spec:dash:sem:expand.changeifs-fn]
pub unsafe fn changeifs(mut ifs: *const c_char) {
    let mut mbs: libc::mbstate_t = mem::zeroed();
    let mut nwcifs: Vec<wchar_t>;
    let mut mb: c_uint = 0;
    let mut len: size_t = 0;
    let mut p: *const c_char;
    let mut ml: size_t;

    if crate::var::ifsset() == 0 {
        ifs = crate::var::defifs();
    }
    ncifs = ifs;

    /* memset(ifsmap, 0, sizeof(ifsmap)) */
    ifsmap = [0; 128];

    p = ifs;
    loop {
        let c: c_uint = *(p as *const u8) as c_uint;

        mb |= c >> 7;
        if (c >> 7) == 0 {
            ifsmap[c as usize] = 1;
        }

        if c == 0 {
            break;
        }

        len += 1;
        p = p.offset(1);
    }

    nwcifs = Vec::new();

    ifsmb0len = (len != 0) as size_t;

    'out: {
        if mb == 0 {
            break 'out;
        }

        ml = mbrlen(ifs, len, &mut mbs);
        if ml == (0 as size_t).wrapping_sub(2) || ml == (0 as size_t).wrapping_sub(1) {
            ml = 1;
        }
        ifsmb0len = ml;

        /* The C `ckmalloc`s `len + 1` wide characters and zero-fills them
         * before `mbsrtowcs` writes a prefix; the zero fill is what makes
         * the result NUL-terminated when the conversion fails part-way,
         * and `wcifs_chr` still depends on it. */
        nwcifs = vec![0 as wchar_t; len + 1];

        p = ifs;
        mbsrtowcs(nwcifs.as_mut_ptr(), &mut p, len + 1, &mut mbs);

        /* `mb != 0` means `IFS` holds a high-bit byte, so `len >= 1` and
         * the allocation is never zero-length — which is what lets
         * "empty" stand in for the C's NULL, and what lets `ifsisifs`
         * read `wcifs[0]`. */
        debug_assert!(
            len > 0 && !nwcifs.is_empty(),
            "changeifs: mb != 0 implies IFS holds a high-bit byte, so len >= 1"
        );
    }

    /* out: */
    *wcifsv() = nwcifs;
}

/*
 * Expand shell metacharacters.  At this point, the only control characters
 * should be escapes.  The results are stored in the list exparg.
 */

/* #ifdef __GLIBC__ */
// [spec:dash:def:expand.opendir-interruptible-fn]
// [spec:dash:sem:expand.opendir-interruptible-fn]
unsafe extern "C" fn opendir_interruptible(pathname: *const c_char) -> *mut c_void {
    if int_pending() != 0 {
        /* The C calls `onint()` here, which longjmps *out of glibc* --
         * back through `glob`'s frames, which own memory it then leaks.
         * That is the sharpest example of what step F removes, and it is
         * a callback into a C library, so it could never have unwound
         * under `panic = "abort"` at all.
         *
         * Dropping the counter and returning leaves `intpending` set.
         * `expmeta`'s own `check_int` already breaks its loop on it, and
         * delivery happens at the next poll site. */
        crate::error::suppressint = 0;
    }

    libc::opendir(pathname) as *mut c_void
}
/* #else
 * #define GLOB_ALTDIRFUNC 0
 * #endif */

// [spec:dash:def:expand.expandmeta-glob-fn]
// [spec:dash:sem:expand.expandmeta-glob-fn]
unsafe fn expandmeta_glob(words: Vec<strlist>) -> Result<(), Error> {
    for mut str in words {
        let p: *const c_char;
        let mut pglob: crate::system::glob64_t = mem::zeroed();
        let i: c_int;

        'sw: {
            'nometa: {
                'nometa2: {
                    if fflag() != 0 {
                        break 'nometa;
                    }

                    /* #ifdef __GLIBC__ */
                    pglob.gl_closedir = Some(mem::transmute(libc::closedir as *const () as usize));
                    pglob.gl_readdir = Some(mem::transmute(libc::readdir64 as *const () as usize));
                    pglob.gl_opendir = Some(opendir_interruptible);
                    pglob.gl_lstat = Some(mem::transmute(libc::lstat64 as *const () as usize));
                    pglob.gl_stat = Some(mem::transmute(libc::stat64 as *const () as usize));
                    /* #endif */

                    crate::error::INTOFF();
                    /* No RMESCAPE_ALLOC, so `_rmescapes` rewrites the
                     * word in place and returns it; the cursor therefore
                     * has to come from `&mut str` and not from `textp`. */
                    p = preglob(str.text.as_mut_ptr() as *mut c_char, RMESCAPE_HEAP, None);
                    i = crate::system::glob64(
                        p,
                        crate::system::GLOB_ALTDIRFUNC | crate::system::GLOB_NOMAGIC,
                        None,
                        &mut pglob,
                    );
                    /* `if (p != str->text) ckfree(p)` — the C asking "did
                     * `_rmescapes` allocate?".  It allocates only under
                     * RMESCAPE_ALLOC, which `preglob` sets only
                     * `if (FNMATCH_IS_ENABLED)`; without it the word was
                     * rewritten in place and `p` is `str.text`'s own
                     * pointer, so the answer is a build constant and there
                     * is nothing to free. */
                    debug_assert_eq!(
                        p as *const c_char,
                        str.text.as_ptr() as *const c_char,
                        "expandmeta_glob: preglob allocated without RMESCAPE_ALLOC"
                    );
                    if i == 0 {
                        if (pglob.gl_flags
                            & (crate::system::GLOB_NOMAGIC | crate::system::GLOB_NOCHECK))
                            == (crate::system::GLOB_NOMAGIC | crate::system::GLOB_NOCHECK)
                        {
                            break 'nometa2; /* goto nometa2 */
                        }
                        addglob(&pglob);
                        crate::system::globfree64(&mut pglob);
                        crate::error::INTON();
                        break 'sw;
                    } else if i == crate::system::GLOB_NOMATCH {
                        break 'nometa2;
                    } else {
                        /* default:  GLOB_NOSPACE. A stop before and after:
                         * the arm falls into `nometa2` otherwise. */
                        return Err(crate::error::sh_error_value(b"Out of space"));
                    }
                }
                /* nometa2: */
                crate::system::globfree64(&mut pglob);
                crate::error::INTON();
                /* fall through to nometa */
            }
            /* nometa: */
            str.rmescapes();
            expargl().push(str);
        }
    }
    Ok(())
}

/*
 * Add the result of glob(3) to the list.
 */

// [spec:dash:def:expand.addglob-fn]
// [spec:dash:sem:expand.addglob-fn]
unsafe fn addglob(pglob: *const crate::system::glob64_t) {
    let mut p: *mut *mut c_char = (*pglob).gl_pathv;

    loop {
        addfname(*p);
        p = p.offset(1);
        if (*p).is_null() {
            break;
        }
    }
}

// [spec:dash:def:expand.expandmeta-fn]
// [spec:dash:sem:expand.expandmeta-fn]
unsafe fn expandmeta(words: Vec<strlist>) -> Result<(), Error> {
    /* TODO - EXP_REDIR */

    if GLOB_IS_ENABLED {
        return expandmeta_glob(words);
    }

    /* The C's `preglob(..., RMESCAPE_HEAP)` result: one `ckmalloc` per
     * word, `ckfree`d as soon as `expmeta` has read it.  That is a local
     * buffer's lifetime exactly, and reusing it across the loop is the
     * only difference — `expmeta` never re-enters `preglob`, because the
     * only `preglob` under it is `patmatch`'s, which does not allocate
     * while `FNMATCH_IS_ENABLED` is 0. */
    let mut pattern: Vec<u8> = Vec::new();

    for mut str in words {
        let savelastp: usize;
        let p: *mut c_char;
        let len: c_uint;

        'sw: {
            'nometa: {
                if fflag() != 0 {
                    break 'nometa;
                }
                let text = CStr::from_ptr(str.textp()).to_bytes();
                if text.find_byteset(b"*?]").is_none() || text == b"]" {
                    break 'nometa;
                }
                /* `savelastp = exparg.lastp` — where this word's matches
                 * will start, so that the sort below covers them and not
                 * the words already in the list. */
                savelastp = expargl().len();

                crate::error::INTOFF();
                p = preglob(
                    str.textp(),
                    RMESCAPE_ALLOC | RMESCAPE_HEAP,
                    Some(&mut pattern),
                );
                len = CStr::from_ptr(p).count_bytes() as c_uint;

                /* The C's top-level `expmeta` starts on whatever block the
                 * region is on and gets away with it because `expdir_len`
                 * is 0: it writes from the base and never reads what was
                 * there.  An owned buffer's length is not 0 — the previous
                 * glob's `addfnamealt` left it at that glob's `expdir_len`
                 * — and every consequence of carrying it in is benign,
                 * which is the reason to clear rather than to argue.  The
                 * invariant [`globbuf`] states is then an equality, and an
                 * equality is what `expmeta` can assert. */
                globb().clear();
                expmeta(p, len, 0);
                /* `if (p != str->text) ckfree(p)` — the C's way of asking
                 * "did `_rmescapes` allocate?".  `pattern` owns the bytes
                 * either way now, and the next iteration reuses it. */
                crate::error::INTON();
                if expargl().len() == savelastp {
                    /*
                     * no matches
                     */
                    break 'nometa;
                } else {
                    /* `*exparg.lastp = NULL; sp = expsort(*savelastp);
                     * *savelastp = sp; while (sp->next) sp = sp->next;
                     * exparg.lastp = &sp->next;` — terminate the run this
                     * word added, sort it, splice it back and walk to its
                     * new end.  Three of those four exist to re-find the
                     * tail of a list the sort reordered; a slice's tail
                     * does not move. */
                    expsort(&mut expargl()[savelastp..]);
                    break 'sw;
                }
            }
            /* nometa: */
            str.rmescapes();
            expargl().push(str);
        }
    }
    Ok(())
}

// [spec:dash:def:expand.addfname-common-fn]
// [spec:dash:sem:expand.addfname-common-fn]
unsafe fn addfname_common(name: BString) {
    expargl().push(strlist { text: name });
}

// [spec:dash:def:expand.addfnamealt-fn]
// [spec:dash:sem:expand.addfnamealt-fn]
unsafe fn addfnamealt(enddir: *mut c_char, expdir_len: size_t) -> *mut c_char {
    /* `name = grabstackstr(enddir)` — in the C this allocates nothing and
     * copies nothing: it moves the region's bump pointer past bytes that
     * are already in place, which is how C says "these outlive the next
     * candidate".
     *
     * The candidate cannot simply be moved out, and that is the one place
     * in this pass where a copy stays.  The field wants `[0, n)` and the
     * *next* candidate wants `[0, expdir_len)` — the same bytes — so one of
     * the two has to take a copy.  The C copies the prefix back
     * (`STARTSTACKSTR(enddir); stnputs(name, expdir_len, enddir)`) because
     * `grabstackstr` had already given the block away; this copies the
     * field out and keeps the buffer, which costs the same order and leaves
     * the glob buffer's capacity and its `expdir_len` invariant alone.
     * What has gone is the region: the copy is into the field's own
     * allocation, not into a block a `popstackmark` has to free.
     *
     * `n` runs past the length when the caller is the no-metacharacter
     * branch, whose `expmeta_rmescapes` wrote through a raw cursor without
     * committing.  Those bytes are written; they are only uncounted, and
     * counting them here is what makes the read below a read of the
     * buffer rather than of its spare capacity. */
    let name: BString = {
        let b = globb();
        let n: size_t = enddir.offset_from(b.as_mut_ptr() as *mut c_char) as size_t;
        debug_assert!(n <= b.capacity());
        b.set_len(n);
        debug_assert_eq!(b.last(), Some(&0), "the candidate is a C string");
        BString::from(b.to_vec())
    };
    addfname_common(name);

    /* `STARTSTACKSTR(enddir); return stnputs(name, expdir_len, enddir) -
     * expdir_len;` — the C has to start a new block and copy the directory
     * prefix back into it, because `grabstackstr` gave the old one away.
     * Nothing was given away here, so the prefix is still the first
     * `expdir_len` bytes and re-seeding is `set_len`. */
    let b = globb();
    b.set_len(expdir_len);
    b.as_mut_ptr() as *mut c_char
}

// [spec:dash:def:expand.expmeta-rmescapes-fn]
// [spec:dash:sem:expand.expmeta-rmescapes-fn]
unsafe fn expmeta_rmescapes(mut enddir: *mut c_char, name: *const c_char) -> *mut c_char {
    let mut p: *const c_char;

    if !FNMATCH_IS_ENABLED {
        let src = CStr::from_ptr(name).to_bytes_with_nul();
        core::ptr::copy_nonoverlapping(src.as_ptr(), enddir as *mut u8, src.len());
        return crate::system::strchrnul(rmescapes(enddir), 0);
    }

    p = name;
    loop {
        let q: *mut c_char = crate::system::strchrnul(p, C_BACKSLASH as c_int);

        enddir = crate::system::mempcpy(
            enddir as *mut c_void,
            p as *const c_void,
            (q.offset_from(p) + 1) as size_t,
        ) as *mut c_char;
        p = q;
        if *p == C_NUL {
            break;
        }
        p = p.offset(1);
        if *p != C_NUL {
            *enddir.offset(-1) = *p;
            p = p.offset(1);
        }
    }

    enddir.offset(-1)
}

/* #ifndef HAVE_MEMRCHR */
// [spec:dash:def:expand.memrchr-fn]
// [spec:dash:sem:expand.memrchr-fn]
unsafe fn memrchr(s: *const c_void, c: c_int, n: size_t) -> *mut c_void {
    let str: *const u8 = s as *const u8;
    let mut cp: *const u8;

    /* `cp = str + n - 1` is `str - 1` when n == 0, so the arithmetic is
     * wrapping here — the loop then never runs, as in C. */
    cp = str.wrapping_offset(n as isize - 1);
    while cp >= str {
        if *cp as c_int == c {
            return cp as *mut c_void;
        }
        cp = cp.wrapping_offset(-1);
    }
    ptr::null_mut()
}
/* #endif */

/*
 * Do metacharacter (i.e. *, ?, [...]) expansion.
 */

// [spec:dash:def:expand.expmeta-fn]
// [spec:dash:sem:expand.expmeta-fn]
unsafe fn expmeta(name: *mut c_char, mut name_len: c_uint, mut expdir_len: size_t) -> *mut c_char {
    let mesc: c_char = if FNMATCH_IS_ENABLED {
        C_BACKSLASH
    } else {
        CTLESC
    };
    let mut statb: libc::stat64 = mem::zeroed();
    let mut dp: *mut libc::dirent64;
    let mut endname: *mut c_char = ptr::null_mut();
    let mut zeroedp: *mut c_char = ptr::null_mut();
    let mut enddir: *mut c_char;
    let mut matchdot: c_int;
    let mut esc: c_uint;
    let mut start: *mut c_char;
    let mut len: size_t;
    /* `DIR *dirp;` — Rust needs the binding initialised before the
     * volatile store below, which is the C's actual initialisation. */
    let mut dirp: *mut libc::DIR = ptr::null_mut();
    let pat: *mut c_char;
    let mut cp: *mut c_char = ptr::null_mut();
    let mut p: *mut c_char;
    let mut c: c_int = 0;
    /* Scratch for the encoded form of each directory entry; see the
     * `memtodest` call below.  A local rather than a static because
     * `expmeta` recurses, one frame per path component. */
    let mut globenc: BString = BString::new(Vec::new());

    /* *(DIR *volatile *)&dirp = NULL; */
    ptr::write_volatile(&mut dirp, ptr::null_mut());
    /* The C has `if (unlikely(err = setjmp(jmploc.loc))) goto out;` here
     * and a matching `longjmp` at `out_opendir`, over a `jmploc` it never
     * installs — `handler = &jmploc` is missing, so nothing can jump into
     * it, `setjmp` can only return 0, and both arms are unreachable. The
     * port reproduced that verbatim rather than "fixing" it. It went with
     * the machinery: there is no `jmploc` left to be dead code over, and
     * the `sem` rule's claim that the handler is installed was never true
     * of either language. */

    'out_opendir: {
        'out: {
            if false {
                break 'out; /* the C's unreachable `goto out` */
            }

            /* The glob buffer's invariant, stated where it is relied on:
             * this frame's prefix is `[0, expdir_len)` and it is exactly
             * what the buffer counts as written, so `globgrowto`'s
             * `reserve` copies it and nothing else.  `expandmeta` clears
             * for the top-level call; a recursive one arrives straight out
             * of the `globstnputs` that appended the component. */
            debug_assert_eq!(globb().len(), expdir_len);
            len = expdir_len + name_len as size_t + 1;
            cp = globgrowto(len);
            enddir = cp.offset(expdir_len as isize);

            p = name;
            esc = 0;
            loop {
                let from = p.offset(esc as isize);
                p = CStr::from_ptr(from)
                    .to_bytes()
                    .find_byteset(b"*?]")
                    .map_or(ptr::null_mut(), |at| from.add(at));
                if p.is_null() {
                    break;
                }
                esc = (mesclen(name, p, mesc) & 1) as c_uint;
                if esc == 0 {
                    break;
                }
            }
            /* No meta characters */
            if p.is_null() {
                if expdir_len == 0 {
                    break 'out_opendir; /* goto out_opendir */
                }
                enddir = expmeta_rmescapes(enddir, name);
                /* See [`globgrowto`]: `len` is the whole bound on what
                 * `expmeta_rmescapes` just wrote, and `enddir` is on the
                 * NUL it wrote last.  Asserted against `len` and not
                 * against the capacity because `Vec` over-allocates, so a
                 * capacity that fits proves nothing about the arithmetic. */
                debug_assert!((enddir.offset_from(cp) as size_t) < len);
                if libc::lstat64(cp, &mut statb) >= 0 {
                    cp = addfnamealt(enddir.offset(1), expdir_len);
                }
                break 'out_opendir; /* goto out_opendir */
            }
            start = memrchr(
                name as *const c_void,
                C_SLASH as c_int,
                p.offset_from(name) as size_t,
            ) as *mut c_char;
            if !start.is_null() {
                start = start.offset(1);
                c = *start as c_int;
                *start = 0;
                enddir = expmeta_rmescapes(enddir, name);
                *start = c as c_char;
                expdir_len = enddir.offset_from(cp) as size_t;
                /* `expdir_len` grew, and the bytes it grew over were
                 * written by `expmeta_rmescapes` through a raw cursor.
                 * Count them: the invariant above has to hold again before
                 * the readdir loop, whose `globstnputs` can reallocate.
                 * The assertion is the same one as in the branch above,
                 * and here it also covers the `*enddir = 0` below. */
                debug_assert!(expdir_len < len);
                globb().set_len(expdir_len);
            } else {
                start = name;
            }
            *enddir = 0;

            /* *(DIR *volatile *)&dirp = opendir(expdir_len ? cp : dotdir); */
            ptr::write_volatile(
                &mut dirp,
                libc::opendir(if expdir_len != 0 {
                    cp
                } else {
                    crate::mystring::dotdir.as_ptr()
                }),
            );
            if dirp.is_null() {
                break 'out_opendir; /* goto out_opendir */
            }
            esc = 0;
            p = crate::system::strchrnul(p.offset(1), C_SLASH as c_int);
            zeroedp = p;
            endname = p;
            if *p != C_NUL {
                esc = (mesclen(name, p, mesc) & 1) as c_uint;
                zeroedp = zeroedp.offset(-(esc as isize));
                endname = endname.offset(1);
            }
            c = *zeroedp as c_int;
            *zeroedp = C_NUL;
            name_len = name_len.wrapping_sub(endname.offset_from(name) as c_uint);
            matchdot = 0;
            pat = start;
            p = pat;
            if *p == mesc {
                p = p.offset(1);
            }
            if *p == C_DOT {
                matchdot += 1;
            }
            loop {
                dp = libc::readdir64(dirp);
                if dp.is_null() {
                    break;
                }
                let dname: *mut c_char = (*dp).d_name.as_mut_ptr();

                'check_int: {
                    if *dname == C_DOT && matchdot == 0 {
                        break 'check_int; /* goto check_int */
                    }
                    if c != 0
                        && (*dp).d_type != libc::DT_DIR
                        && (*dp).d_type != libc::DT_LNK
                        && (*dp).d_type != libc::DT_UNKNOWN
                    {
                        break 'check_int; /* goto check_int */
                    }
                    len = CStr::from_ptr(dname).to_bytes_with_nul().len();
                    p = dname;
                    if !FNMATCH_IS_ENABLED {
                        /* The C encodes the directory entry's name at
                         * `enddir` — inside the glob buffer, past the
                         * prefix — by parking `enddir` in the global
                         * `expdest` for the length of the call.  Those bytes
                         * are pure scratch: they exist only for `pmatch`
                         * below, and the branch that keeps the entry
                         * immediately overwrites them with the raw name via
                         * `stnputs`.  So the encoding goes to its own buffer
                         * and the candidate path never holds it.  That is
                         * what let the expansion buffer and this one be
                         * converted separately.
                         *
                         * `cp = stackblock()` is kept and is a no-op:
                         * `memtodest` writes to `globenc`, so the glob
                         * buffer cannot have moved across it.  The re-read
                         * is the C's marker for "a growth can happen here",
                         * and `globstnputs` in this same loop still is
                         * one. */
                        globenc.clear();
                        memtodest(p, len, EXP_MBCHAR | EXP_KEEPNUL, &mut globenc);
                        cp = globbase();
                        enddir = cp.offset(expdir_len as isize);
                        p = globenc.as_mut_ptr() as *mut c_char;
                    }
                    if pmatch(pat, p) != 0 {
                        enddir = globstnputs(dname, len, enddir);
                        if c == 0 {
                            cp = addfnamealt(enddir, expdir_len);
                        } else {
                            *enddir.offset(-1) = C_SLASH;
                            len += expdir_len;
                            cp = expmeta(endname, name_len, len);
                        }
                        enddir = cp.offset(expdir_len as isize);
                    }
                }
                /* check_int: */
                if int_pending() != 0 {
                    break;
                }
            }
            *zeroedp = c as c_char;
        }

        /* out: */
        /* NOTE: `closedir(NULL)` is reachable here in the C when the
         * (never-installed) handler fires before `opendir`; glibc
         * tolerates a NULL argument. */
        libc::closedir(ptr::read_volatile(&dirp));
    }

    /* out_opendir: */
    cp
}

/*
 * Add a file name to the list.
 */

// [spec:dash:def:expand.addfname-fn]
// [spec:dash:sem:expand.addfname-fn]
unsafe fn addfname(name: *mut c_char) {
    /* `sstrdup(name)`: the C copies `glob`'s `gl_pathv` entry into the
     * region because `globfree64` is about to free it.  The field owns the
     * copy now, which is the same statement without the allocator. */
    /* Terminator included -- `addfname_common`'s readers are C-shaped. */
    addfname_common(BString::from(CStr::from_ptr(name).to_bytes_with_nul()));
}

/*
 * Sort the results of file name expansion.  It calculates the number of
 * strings to sort and then calls msort (short for merge sort) to do the
 * work.
 */

// [spec:dash:def:expand.expsort-fn]
// [spec:dash:sem:expand.expsort-fn]
unsafe fn expsort(str: &mut [strlist]) {
    /* The C walks the chain to count it and hands the count to `msort`,
     * because a singly-linked list does not know its own length. */
    msort(str, str.len() as c_int)
}

// [spec:dash:def:expand.msort-fn]
// [spec:dash:sem:expand.msort-fn]
///
/// The C's merge sort, as `sort_by`.  Two properties have to match, and
/// both do:
///
///   * **Order.**  `q` is the sorted *first* half and `p` the second, and
///     the merge takes `p` only on `strcoll(p->text, q->text) < 0`, so the
///     comparison is ascending by `strcoll`.
///   * **Stability.**  That same test takes `q` — the earlier half — when
///     the two compare equal, and a top-down merge sort whose merge is
///     stable is stable.  `strcoll` can return 0 for byte-different
///     strings under a collating locale, so this is not vacuous.
///     `slice::sort_by` is stable.
unsafe fn msort(list: &mut [strlist], len: c_int) {
    if len <= 1 {
        return;
    }
    list.sort_by(|p, q| libc::strcoll(p.textp(), q.textp()).cmp(&0));
}

/*
 * Returns true if the pattern matches the string.
 */

// [spec:dash:def:expand.patmatch-fn]
// [spec:dash:sem:expand.patmatch-fn]
#[inline]
unsafe fn patmatch(pattern: *mut c_char, string: *const c_char) -> c_int {
    pmatch(preglob(pattern, 0, None), string)
}

// [spec:dash:def:expand.ccmatch-fn]
// [spec:dash:sem:expand.ccmatch-fn]
#[inline(never)]
unsafe fn ccmatch(mut p: *mut c_char, mbc: *const c_char, ml: c_int, r: *mut *mut c_char) -> c_int {
    let mut mbst: libc::mbstate_t = mem::zeroed();
    let type_: wctype_t;
    let mut wc: wchar_t = 0;
    let q: *mut c_char;

    *r = ptr::null_mut();

    if *p != C_COLON {
        return 0;
    }
    p = p.offset(1);

    q = match CStr::from_ptr(p).to_bytes().find(b":]") {
        Some(at) => p.add(at),
        None => return 0,
    };

    *q = 0;
    type_ = wctype(p);
    *q = C_COLON;

    if type_ == 0 as wctype_t {
        return 0;
    }

    *r = q.offset(2);

    if mbrtowc(&mut wc, mbc, ml as size_t, &mut mbst) != ml as size_t {
        return 0;
    }

    iswctype(wc as wint_t, type_)
}

// [spec:dash:def:expand.pmatch-fn]
// [spec:dash:sem:expand.pmatch-fn]
unsafe fn pmatch(pattern: *mut c_char, string: *const c_char) -> c_int {
    let mut q: *const c_char;
    let mut mb: c_uint;
    let mut p: *mut c_char;
    let mut c: c_char;

    if FNMATCH_IS_ENABLED {
        return (libc::fnmatch(pattern, string, 0) == 0) as c_int;
    }

    p = pattern;
    q = string;
    'forever: loop {
        'dft: {
            c = *p;
            p = p.offset(1);
            match c {
                C_NUL => break 'forever, /* goto breakloop */
                CTLESC => {
                    c = *p;
                    p = p.offset(1);
                    /* break — fall through to dft */
                }
                C_QUESTION => {
                    if *q == C_NUL {
                        return 0;
                    }
                    mb = mbnext(q);
                    q = q.offset(((mb >> 8) + (mb & 0xff)) as isize);
                    continue 'forever;
                }
                C_STAR => {
                    c = *p;
                    while c == C_STAR {
                        p = p.offset(1);
                        c = *p;
                    }
                    if c == C_NUL {
                        return 1;
                    }
                    if c == C_QUESTION || c == C_LBRACKET {
                        c = CTLESC;
                    }
                    loop {
                        if c != CTLESC {
                            /* The C's comment here is `Stop should be
                             * null-terminated as it is passed as a string
                             * to strpbrk(3)`, and the terminator was the
                             * whole reason for the fourth element. The
                             * set is the three bytes; the scan stops at
                             * the string's own NUL, which is a miss.
                             *
                             * Walked rather than taken as a slice: this
                             * runs once per candidate position under a
                             * `*`, and measuring the whole tail each time
                             * would cost a pass per position. */
                            let stop: [u8; 3] = [c as u8, CTLESC as u8, CTLMBCHAR as u8];
                            let at = (0usize..)
                                .find(|&i| {
                                    let b = *q.add(i) as u8;
                                    b == 0 || stop.contains(&b)
                                })
                                .expect("the scan ends at the terminator");
                            if *q.add(at) == C_NUL {
                                return 0;
                            }
                            q = q.add(at);
                        }
                        if pmatch(p, q) != 0 {
                            return 1;
                        }
                        if *q == C_NUL {
                            break;
                        }
                        mb = mbnext(q);
                        q = q.offset(((mb >> 8) + (mb & 0xff)) as isize);
                    }
                    return 0;
                }
                C_LBRACKET => {
                    let startp: *mut c_char;
                    let mut invert: c_int;
                    let mut found: c_int;
                    let chr: c_char;

                    startp = p;
                    invert = 0;
                    if *p == C_BANG || *p == C_CARET {
                        invert += 1;
                        p = p.offset(1);
                    }
                    found = 0;
                    mb = mbnext(q);
                    q = q.offset((mb & 0xff) as isize);
                    mb >>= 8;
                    chr = *q;
                    if chr == C_NUL {
                        return 0;
                    }
                    c = *p;
                    p = p.offset(1);
                    loop {
                        'cont: {
                            let mut mbp: c_uint = 0;
                            /* NOTE (bug-for-bug): `mbs` starts as the
                             * address of the *local* `c`; when the string
                             * character is multibyte the `strncmp` below
                             * reads `mb` bytes from it, past the end of
                             * that single byte.  Reproduced. */
                            let mut mbs: *const c_char = &c as *const c_char;

                            if c == C_NUL {
                                p = startp;
                                c = C_LBRACKET;
                                break 'dft; /* goto dft */
                            }
                            if c == C_LBRACKET {
                                let mut r: *mut c_char = ptr::null_mut();

                                found |= (ccmatch(
                                    p,
                                    q,
                                    (if mb > 1 { mb - 2 } else { mb }) as c_int,
                                    &mut r,
                                ) != 0) as c_int;
                                if !r.is_null() {
                                    p = r;
                                    break 'cont; /* continue */
                                }
                            } else if c == CTLESC {
                                c = *p;
                                p = p.offset(1);
                            } else if c == CTLMBCHAR {
                                p = p.offset(-1);
                                mbp = mbnext(p);
                                p = p.offset((mbp & 0xff) as isize);
                                mbs = p;
                                mbp >>= 8;
                                p = p.offset(mbp as isize);
                            }
                            if *p == C_MINUS && *p.offset(1) != C_NUL && *p.offset(1) != C_RBRACKET
                            {
                                p = p.offset(1);
                                if *p == CTLESC {
                                    p = p.offset(1);
                                } else if *p == CTLMBCHAR {
                                    mbp = mbnext(p);
                                    p = p.offset((mbp & 0xff) as isize);
                                    p = p.offset((mbp >> 8) as isize);
                                    break 'cont; /* continue */
                                }
                                if (mbp | mb.wrapping_sub(1)) == 0 && chr >= c && chr <= *p {
                                    found = 1;
                                }
                                p = p.offset(1);
                            } else if crate::mystring::ncmp_eq(mbs, q, mb as usize) {
                                found = 1;
                            }
                        }
                        /* } while ((c = *p++) != ']'); */
                        c = *p;
                        p = p.offset(1);
                        if c == C_RBRACKET {
                            break;
                        }
                    }
                    if found == invert {
                        return 0;
                    }
                    q = q.offset(mb as isize);
                    continue 'forever;
                }
                CTLMBCHAR => {
                    p = p.offset(-1);
                    mb = mbnext(p);
                    p = p.offset((mb & 0xff) as isize);
                    mb = mbnext(q);
                    q = q.offset((mb & 0xff) as isize);
                    mb >>= 8;

                    if !crate::mystring::ncmp_eq(p.offset(-1), q.offset(-1), (mb + 1) as usize) {
                        return 0;
                    }

                    p = p.offset(mb as isize);
                    q = q.offset(mb as isize);
                    continue 'forever;
                }
                _ => {}
            }
        }
        /* dft: */
        mb = mbnext(q);
        if (mb >> 8) > 1 {
            return 0;
        }
        q = q.offset((mb & 0xff) as isize);
        if *q != c {
            return 0;
        }
        q = q.offset((mb >> 8) as isize);
    }
    /* breakloop: */
    if *q != C_NUL {
        return 0;
    }
    1
}

/*
 * Remove any CTLESC characters from a string.
 */

// [spec:dash:def:expand.rmescapes-fn]
// [spec:dash:sem:expand.rmescapes-fn]
pub unsafe fn _rmescapes(
    mut str: *mut c_char,
    flag: c_int,
    mut heap: Option<&mut Vec<u8>>,
) -> *mut c_char {
    let mut p: *mut c_char;
    let mut q: *mut c_char;
    let mut r: *mut c_char;
    let mut notescaped: c_int;
    let globbing: c_int;
    let mut inquotes: c_int;
    let mut fulllen: size_t = 0;

    /* `strpbrk`'s set is the string without its terminator: it never
     * matches a NUL, which is what stops the scan instead. */
    let cqset = crate::mystring::cqchars.map(|c| c as u8);
    p = match CStr::from_ptr(str).to_bytes().find_byteset(&cqset[..4]) {
        Some(at) => str.add(at),
        None => return str,
    };
    q = p;
    r = str;
    globbing = flag & RMESCAPE_GLOB;

    if (flag & RMESCAPE_ALLOC) != 0 {
        let len: size_t = p.offset_from(str) as size_t;
        fulllen = CStr::from_ptr(p).count_bytes();

        if FNMATCH_IS_ENABLED && globbing != 0 {
            fulllen *= 2;
        }

        fulllen += len + 1;

        if (flag & RMESCAPE_GROW) != 0 {
            /* RMESCAPE_GROW means "the destination is the expansion
             * buffer", and `str` is always inside it on this path — the one
             * caller is `subevalvar`'s `_rmescapes(startp, ALLOC | GROW)`.
             * `reserve` can reallocate, which is why the C re-reads
             * `stackblock()` on the next line. */
            let strloc: c_int = str.offset_from(expbase()) as c_int;

            r = expmakestrspace(fulllen);
            str = expbase().offset(strloc as isize);
            p = str.offset(len as isize);
        } else {
            /* The C splits this arm in two: `ckmalloc(fulllen)` under
             * RMESCAPE_HEAP and `stalloc(fulllen)` otherwise.  The
             * `stalloc` half is unreachable, and the reason is one
             * constant away: `RMESCAPE_ALLOC` is only ever set by
             * `preglob`, which sets it under `if (FNMATCH_IS_ENABLED)`,
             * and by `subevalvar`'s `ALLOC | GROW`, which took the branch
             * above.  `FNMATCH_IS_ENABLED` is 0, so the only caller that
             * arrives here is `expandmeta`'s
             * `preglob(text, RMESCAPE_ALLOC | RMESCAPE_HEAP)` — and that
             * is the caller supplying `heap`.  Asserted rather than
             * claimed, because [dec:nsh:owned-data] records this exact
             * flag being reasoned about wrongly once already. */
            debug_assert!(
                (flag & RMESCAPE_HEAP) != 0 && heap.is_some(),
                "_rmescapes: RMESCAPE_ALLOC without GROW reaches only the HEAP arm"
            );
            let out = heap
                .as_deref_mut()
                .expect("_rmescapes: RMESCAPE_ALLOC without GROW needs a heap buffer");
            out.clear();
            out.reserve(fulllen);
            r = out.as_mut_ptr() as *mut c_char;
        }
        q = r;
        if len > 0 {
            q = crate::system::mempcpy(q as *mut c_void, str as *const c_void, len) as *mut c_char;
        }
    }
    inquotes = 0;
    notescaped = globbing;
    'whileloop: while *p != C_NUL {
        let mut c: c_int = *p as c_int;
        let mut newnesc: c_int = globbing;
        let mb: c_uint;
        let mut ml: c_uint;

        'setnesc: {
            if c == CTLQUOTEMARK as c_int {
                p = p.offset(1);
                inquotes ^= globbing;
                continue 'whileloop;
            } else if c == C_BACKSLASH as c_int {
                /* naked back slash */
                newnesc ^= notescaped;
                /* naked backslashes can only occur outside quotes */
                inquotes = 0;
                if !FNMATCH_IS_ENABLED && notescaped != 0 {
                    c = CTLESC as c_int;
                }
            } else if c == CTLESC as c_int {
                if ((notescaped ^ inquotes) & inquotes) != 0 {
                    if FNMATCH_IS_ENABLED {
                        *q = C_BACKSLASH;
                        q = q.offset(1);
                    } else {
                        *q.offset(-1) = C_BACKSLASH;
                    }
                }
                if globbing != 0 {
                    *q = if FNMATCH_IS_ENABLED {
                        C_BACKSLASH
                    } else {
                        CTLESC
                    };
                    q = q.offset(1);
                }

                p = p.offset(1);
                c = *p as c_int;
            } else if c == CTLMBCHAR as c_int {
                let mut tail: c_uint = 2;

                if !FNMATCH_IS_ENABLED && (globbing ^ notescaped) != 0 {
                    q = q.offset(-1);
                }

                mb = mbnext(p);
                ml = mb >> 8;

                if globbing == 0 || FNMATCH_IS_ENABLED {
                    p = p.offset((mb & 0xff) as isize);
                    ml -= 2;
                } else {
                    ml += mb & 0xff;
                    tail = 0;
                }

                /* `q` trails `p` through the same buffer. */
                core::ptr::copy(p, q, ml as usize);
                q = q.offset(ml as isize);
                p = p.offset((ml + tail) as isize);
                break 'setnesc; /* goto setnesc */
            }

            *q = c as c_char;
            q = q.offset(1);
            p = p.offset(1);
        }
        /* setnesc: */
        notescaped = newnesc;
    }
    if !FNMATCH_IS_ENABLED && (globbing ^ notescaped) != 0 {
        *q.offset(-1) = C_BACKSLASH;
    }
    *q = C_NUL;
    if (flag & RMESCAPE_GROW) != 0 {
        /* `expdest = r; STADJUST(q - r + 1, expdest)` — but only when `r`
         * is in the expansion buffer, which is RMESCAPE_GROW and nothing
         * else.
         *
         * The other live arm is `expandmeta`'s
         * `preglob(text, RMESCAPE_ALLOC | RMESCAPE_HEAP)`, where `r` is a
         * `ckmalloc`'d block that the caller `ckfree`s a few lines later.
         * The C runs this same assignment there and so leaves `expdest`
         * pointing into freed memory.  It is harmless only because of where
         * `expandmeta` sits: at the tail of `expandarg`, after
         * `grabstackstr(expdest)` has taken the word, and every entry to the
         * expansion re-opens with `STARTSTACKSTR`.  So the value is written
         * and never read.  An owned buffer cannot hold that pointer and has
         * no reason to, so the assignment is dropped for the non-GROW arms
         * rather than transcribed — a deliberate divergence from a store
         * that has no observable value. */
        set_expdest(r.offset(q.offset_from(r) + 1));
    } else if (flag & RMESCAPE_ALLOC) != 0 {
        /* The bytes went into the caller's buffer through a raw cursor
         * that carries no bound of its own, so the only thing standing
         * between `fulllen` and a heap overflow is the C's arithmetic
         * being right.  Asserted against `fulllen` — the number the C
         * computed — and *not* against `Vec::capacity()`, which
         * over-allocates and would make the assertion vacuous. */
        let written: usize = q.offset_from(r) as usize + 1;
        debug_assert!(
            written <= fulllen,
            "_rmescapes wrote {written} bytes into a {fulllen}-byte reservation"
        );
        let out = heap
            .as_deref_mut()
            .expect("_rmescapes: RMESCAPE_ALLOC without GROW needs a heap buffer");
        out.set_len(written);
    }
    r
}

/*
 * See if a pattern matches in a case statement.
 */

// [spec:dash:def:expand.casematch-fn]
// [spec:dash:sem:expand.casematch-fn]
pub unsafe fn casematch(
    pattern: &crate::nodes::Node,
    val: *const c_char,
) -> Result<c_int, Error> {
    let result: c_int;

    /* `setstackmark(&smark)` — it released what `argstr` allocated from the
     * region for backquotes and arithmetic.  Neither allocates from it. */
    argbackq = pattern.narg().backquote.as_slice();
    /* STARTSTACKSTR(expdest) */
    expb().clear();
    /* As in `expandarg`: this `?` returns past the `ifsfree()`, which is
     * where the longjmp went too, and the catch frame reclaims the
     * regions. */
    argstr(pattern.narg().text.as_ptr(), EXP_TILDE | EXP_CASE)?;
    ifsfree();
    /* The C reads the word back as `stackblock()`. */
    result = patmatch(expbase(), val);
    Ok(result)
}

/*
 * Our own itoa().
 */

// [spec:dash:def:expand.cvtnum-fn]
// [spec:dash:sem:expand.cvtnum-fn]
unsafe fn cvtnum(num: intmax_t, flags: c_int, dst: &mut BString) -> size_t {
    let value = format!("{num}");
    memtodest(value.as_ptr() as *const c_char, value.len(), flags, dst)
}

// [spec:dash:def:expand.varunset-fn]
// [spec:dash:sem:expand.varunset-fn]
unsafe fn varunset(
    end: *const c_char,
    var: *const c_char,
    umsg: *const c_char,
    varflags: c_int,
) -> Error {
    let mut msg: *const c_char;
    let mut tail: *const c_char;

    tail = crate::shell::nullstr.as_ptr();
    msg = b"parameter not set\0".as_ptr() as *const c_char;
    if !umsg.is_null() {
        if *end == CTLENDVAR {
            if (varflags & VSNUL) != 0 {
                tail = b" or null\0".as_ptr() as *const c_char;
            }
        } else {
            msg = umsg;
        }
    }
    let name_len = (end.offset_from(var) - 1).max(0) as usize;
    let mut message = Vec::new();
    message.extend_from_slice(core::slice::from_raw_parts(var as *const u8, name_len));
    message.extend_from_slice(b": ");
    message.extend_from_slice(CStr::from_ptr(msg).to_bytes());
    message.extend_from_slice(CStr::from_ptr(tail).to_bytes());
    crate::error::sh_error_value(&message)
}

/// The `out:` tail `redirectsafe` and `expandstr` share: decide whether
/// what came back is this frame's to keep.
///
/// It kept its C name and lost its first job. The C's version restores
/// `handler` and then asks a global which exception arrived —
/// `if (err) { if (exception != EXERROR) longjmp(handler->loc, 1); ifsfree(); }`
/// — and both halves of that are gone: there is no handler to restore,
/// and nothing to re-raise. What is left is the half that was always the
/// real decision, and it is a match on the value's own type.
///
/// `ifsfree` belongs to the swallowing arm alone. The regions the failed
/// expansion recorded would otherwise mis-split the *next* word, and the
/// frame that takes an interrupt is not the frame that owns them.
// [spec:dash:def:expand.restore-handler-expandarg-fn]
// [spec:dash:sem:expand.restore-handler-expandarg-fn]
pub unsafe fn restore_handler_expandarg(
    caught: Option<crate::error::Error>,
) -> Option<crate::error::Error> {
    match &caught {
        /* Not this frame's to keep, and never was: the C re-raised it
         * from here. */
        Some(e) if e.is_interrupt() => {}
        Some(_) => ifsfree(),
        None => {}
    }
    caught
}

/* #ifdef mkinit
 *
 * INCLUDE "expand.h"
 *
 * EXITRESET {
 *	ifsfree();
 * }
 *
 * #endif
 *
 * The EXITRESET hook is emitted into init.c by mkinit; it belongs to the
 * generated `init` module, not here.
 */

// ---------------------------------------------------------------------
// Prototypes declared in expand.h that have no definition in expand.c.
// They exist here only so that every manifest symbol has a target site.
// ---------------------------------------------------------------------

/// `intmax_t arith(const char *)` — prototype only; the definition lives
/// in `arith.y` / `arith_yacc.c`.  Re-exported so that `expand`'s view of
/// the symbol resolves to the real one.
// [spec:dash:def:expand.arith-fn]
// [spec:dash:sem:expand.arith-fn]
pub use crate::arith_yacc::arith;

/// `int expcmd(int, char **)` — declared in `expand.h` but defined
/// nowhere in the C tree; a vestige of a removed builtin.  There is
/// nothing to port, so this is an unreachable stub kept purely as the
/// symbol's target site.
// [spec:dash:def:expand.expcmd-fn]
// [spec:dash:sem:expand.expcmd-fn]
pub unsafe fn expcmd(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    /* No definition exists in the C tree; calling this is a bug. */
    unreachable!("expcmd: declared in expand.h, never defined")
}

/// ```c
/// #ifdef USE_LEX
/// void arith_lex_reset(void);
/// #else
/// #define arith_lex_reset()
/// #endif
/// ```
/// In the shipped build (`arith_yylex.c`, no generated lexer) this is a
/// macro expanding to nothing, so the port is an empty function.
// [spec:dash:def:expand.arith-lex-reset-fn]
// [spec:dash:sem:expand.arith-lex-reset-fn]
#[inline]
pub unsafe fn arith_lex_reset() {}

/// `int yylex(void)` — prototype only, declared in `expand.h` for the
/// arithmetic parser's benefit; the definition lives in `arith_yylex.c`.
// [spec:dash:def:expand.yylex-fn]
// [spec:dash:sem:expand.yylex-fn]
pub use crate::arith_yylex::yylex;

