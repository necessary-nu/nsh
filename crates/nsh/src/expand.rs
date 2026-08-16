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

use crate::context::Shell;
use core::mem;
use core::ptr;
use std::ffi::CStr;

use bstr::{BStr, BString, ByteSlice};
use libc::{c_char, c_int, c_uint, c_ulong, c_void, intmax_t, size_t, ssize_t, wchar_t};

use crate::error::Error;
use crate::mystring::{byte_at, byte_at_i, slice_from};
use crate::pmatch::{pmatch_slices};

// ---------------------------------------------------------------------
// Declarations from <wchar.h> / <wctype.h> that the `libc` crate does not
// expose.  These are plain libc entry points, not ported symbols.
// ---------------------------------------------------------------------

#[allow(non_camel_case_types)]
pub(crate) type wint_t = c_uint;
#[allow(non_camel_case_types)]
pub(crate) type wctype_t = c_ulong;

unsafe extern "C" {
    fn mbrlen(s: *const c_char, n: size_t, ps: *mut libc::mbstate_t) -> size_t;
    pub(crate) fn mbrtowc(pwc: *mut wchar_t, s: *const c_char, n: size_t, ps: *mut libc::mbstate_t) -> size_t;
    fn mbsrtowcs(
        dst: *mut wchar_t,
        src: *mut *const c_char,
        len: size_t,
        ps: *mut libc::mbstate_t,
    ) -> size_t;
    fn iswspace(wc: wint_t) -> c_int;
    pub(crate) fn wctype(name: *const c_char) -> wctype_t;
    pub(crate) fn iswctype(wc: wint_t, desc: wctype_t) -> c_int;
}

// ---------------------------------------------------------------------
// Constants mirrored from the headers this file includes.
//
// The parser's marker bytes and variable-substitution codes come from
// `parser.h`.  They are aliased here as `c_char`/`c_int` so they can be
// used as `match` patterns and so that the numeric type the parser
// module happens to choose does not matter.
// ---------------------------------------------------------------------

pub(crate) const CTLESC: c_char = crate::parser::CTLESC as c_char;
const CTLVAR: c_char = crate::parser::CTLVAR as c_char;
const CTLENDVAR: c_char = crate::parser::CTLENDVAR as c_char;
const CTLBACKQ: c_char = crate::parser::CTLBACKQ as c_char;
pub(crate) const CTLMBCHAR: c_char = crate::parser::CTLMBCHAR as c_char;
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
pub(crate) const FNMATCH_IS_ENABLED: bool = crate::mystring::FNMATCH_IS_ENABLED != 0;
const GLOB_IS_ENABLED: bool = crate::mystring::GLOB_IS_ENABLED != 0;

/// `<limits.h>`
const CHAR_BIT: c_int = 8;

// C character literals used as `switch` labels; Rust `match` patterns
// require named constants, so the ones this file switches on get names.
pub(crate) const C_NUL: c_char = 0;
const C_NL: c_char = b'\n' as c_char;
pub(crate) const C_BANG: c_char = b'!' as c_char;
const C_HASH: c_char = b'#' as c_char;
const C_DOLLAR: c_char = b'$' as c_char;
pub(crate) const C_STAR: c_char = b'*' as c_char;
pub(crate) const C_MINUS: c_char = b'-' as c_char;
const C_DOT: c_char = b'.' as c_char;
const C_SLASH: c_char = b'/' as c_char;
pub(crate) const C_COLON: c_char = b':' as c_char;
pub(crate) const C_QUESTION: c_char = b'?' as c_char;
const C_AT: c_char = b'@' as c_char;
pub(crate) const C_LBRACKET: c_char = b'[' as c_char;
pub(crate) const C_RBRACKET: c_char = b']' as c_char;
const C_BACKSLASH: c_char = b'\\' as c_char;
pub(crate) const C_CARET: c_char = b'^' as c_char;
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

    /// The same field, taken from bytes that already know where they end.
    ///
    /// `ifsbreakup` terminates each field in the word and then copies it
    /// out; with the word a slice there is no pointer to hand to
    /// [`strlist::from_cstr`], and the terminator is re-supplied here
    /// rather than assumed to be in range.
    pub fn from_cbytes(s: &[u8]) -> strlist {
        let mut text = BString::from(crate::mystring::cstr_prefix(s).as_bytes());
        text.push(0);
        strlist { text }
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
        /* A field keeps its terminator: [`strlist::textp`] asserts it. */
        let n = rmescapes_owned(&mut self.text);
        self.text.truncate(n + 1);
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

/// [`rmescapes`] over a buffer that owns its bytes.
///
/// `_rmescapes` shortens the C string in place and says nothing about by
/// how much, so every caller re-derives the length; two did it by hand and
/// spelled the same three operations differently. Returns the length of
/// the unescaped string **without** its terminator, and leaves the
/// terminator to the caller — a `strlist` field keeps it because
/// [`strlist::textp`] asserts it is there, and a here-document delimiter
/// drops it because it is compared as bytes.
pub unsafe fn rmescapes_owned(s: &mut BString) -> usize {
    let p = s.as_mut_ptr() as *mut c_char;
    rmescapes(p);
    CStr::from_ptr(p).count_bytes()
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
pub struct ifs_state {
    /// The C's `ifst->ifs`, which is a `char *` carrying one bit.
    ///
    /// `ifsbreakup` assigns it `nulonly ? nullstr : realifs` and nothing
    /// else ever assigns it, so the pointer's only content is which of the
    /// two it is — and its sole reader, `ifsisifs`, can read `IFS` off the
    /// shell for itself. The bit is stored and the pointer is gone.
    ///
    /// Not to be confused with `ifsbreakup_slow`'s `nulonly` *parameter*,
    /// which the caller passes as `afternul | nulonly` — the previous
    /// region's bit or'd with this one. They are different values and the
    /// C gives them the same name.
    pub nulonly: c_int,
    /// Where the field being built starts, as an offset into the word.
    pub start: usize,
    /// Where the trailing IFS run to be cut starts, if there is one. The
    /// C's `NULL` for "nothing to remove" is the `None`.
    pub r: Option<usize>,
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
///     exactly the same hazard — `reserve` reallocates. The answer is not
///     to keep the re-reads but to stop needing them: a position carried as
///     an offset survives a growth, and [`_rmescapes`] is the last function
///     in this file that still carries one as a pointer and so still calls
///     [`expbase`].
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

/// `IFS`, in the three forms field splitting wants it.
///
/// Rebuilt by [`changeifs`], which is the `varfunc_t` hook hanging off
/// the `IFS` *variable* -- so this is state derived from a shell
/// variable, and two shells with different `IFS` cannot share it. §5 does
/// not list it; that it moves cleanly is owed to `f8267bd`, which gave
/// every variable hook a `&mut Shell` and therefore gave this one
/// somewhere to write.
pub struct IfsCache {
    /// The single-byte members, as a lookup table.
    ifsmap: [c_char; 128],
    /// `IFS` itself, with its terminating NUL counted.
    ///
    /// The C — and this port until now — kept a `const char *` **into the
    /// IFS variable's text**, refreshed only when the `varfunc_t` hook
    /// fires. That is a borrow the type system was not being told about
    /// and the variable table was under no obligation to honour: setting
    /// `IFS` reallocates the text, and every path that changes a
    /// variable's storage without going through the hook leaves this
    /// dangling. It is the shape of the `putenv` use-after-free
    /// [[owned-vars]] fixed, and the fix is the same one — own the bytes.
    ///
    /// The terminator is counted because `ifsisifs` searches *including*
    /// it: `strchr` matches a NUL, which is how a NUL byte counts as an
    /// IFS separator.
    ncifs: BString,
    /// Length of the first multibyte character, or 0.
    ifsmb0len: size_t,
    /// The wide-character form of `IFS`, built by `changeifs`.
    ///
    /// The C is a `ckmalloc`'d, zero-filled, NUL-terminated `wchar_t *`
    /// that is NULL whenever `IFS` holds no byte with the high bit set.
    /// Empty **is** NULL here: the C only allocates under `mb != 0`,
    /// which needs a high-bit byte, so the buffer is never zero-length
    /// when it exists.
    wcifs: Vec<wchar_t>,
}

impl IfsCache {
    pub(crate) const fn new() -> Self {
        IfsCache {
            ifsmap: [0; 128],
            /* The C starts this NULL and `changeifs` runs before any
             * reader; empty is that, and a reader that arrived early
             * would now see an empty `IFS` rather than fault. */
            ncifs: BString::new(Vec::new()),
            ifsmb0len: 0,
            wcifs: Vec::new(),
        }
    }
}

/// `&mut ifsregions`, without ever naming a reference to the `static mut`
/// twice at once.
#[inline]
unsafe fn ifsr() -> &'static mut Vec<ifsregion> {
    &mut *ptr::addr_of_mut!(ifsregions)
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

/// The C's `stackblock()` and `expdest` as pointers, and
/// `makestrspace`/`STADJUST` over them, are gone.
///
/// They survived exactly as long as one function still carried a position
/// in this buffer as a raw pointer. `_rmescapes` was the last, and its
/// `RMESCAPE_GROW` path now takes and returns an offset
/// ([`rmescapes_grow`]), so there is nothing left to re-derive after a
/// growth: an index does not move. What remains is [`expdest_off`], which
/// is the cursor as the length it always was.

/// `expdest - stackblock()`.
#[inline]
unsafe fn expdest_off() -> c_int {
    expb().len() as c_int
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
///
/// This hands back the bytes rather than the base pointer, and it is the
/// only route by which the expansion buffer left this file as a bare
/// `char *`.  Both callers did `CStr::from_ptr` on what they got, so the
/// scan has not moved — it has become [`mystring::cstr_prefix`], which is
/// safe, and the two `CStr::from_ptr` calls and the pointer that fed them
/// are gone.
///
/// The borrow is `'static` because the buffer is, and the liveness the
/// callers rely on is unchanged and still theirs to respect: the bytes
/// last until the next expansion begins.  Nothing between either call and
/// its read expands — `openhere` only pipes and forks, `expandstr` reads
/// on the next line.
pub unsafe fn expansion_result() -> &'static BStr {
    crate::mystring::cstr_prefix(expb().as_slice())
}

// ---------------------------------------------------------------------
// The glob buffer.
//
// The candidate path `expmeta` is building.  The C has no name for it: it
// is the stack block, addressed through `expmeta`'s locals `cp` (the base,
// `growstackto`'s return) and `enddir` (the cursor, `cp + expdir_len` plus
// whatever has been appended).  Every frame of the recursion owns
// `[0, expdir_len)` — the directory prefix its parent wrote, ending in `/`
// — and writes the next component above it.
//
// It is now an ordinary `BString` that `expandmeta` owns and passes down by
// `&mut`, and the whole cursor layer — a `static mut`, a `globb()`
// accessor, `globbase()`, `globgrowto()`, `globstnputs()` — is gone with
// the cursors.  Three things paid for it:
//
//   * `enddir` is `expdir_len`.  The C's `enddir = cp + expdir_len` after
//     anything that could grow the block existed to survive a
//     reallocation; an index does not move, so every re-derivation goes.
//   * `stnputs(s, n, p)` opens with `len = p - stacknxt`, so an append at
//     a cursor *below* the end of the buffer discards what was above it.
//     Said as an index that is **truncate to `p`, then append** — an
//     ordinary operation on an owned buffer, and the way a frame that a
//     recursive `expmeta` returned into gets its own `expdir_len` back.
//   * The bytes are counted as they are written.  The C wrote the
//     unescaped prefix through a raw cursor and left the block's length
//     alone; `expmeta_rmescapes` appends, so `addfnamealt` no longer has
//     to be told how many bytes are really there.  See its comment.
//
// A `static` was needed only while the cursors were raw pointers that
// outlived the borrow producing them.  With `&mut BString` threaded through
// the recursion, "there is never a second glob in flight" stops being an
// argument about `INTOFF` and becomes the borrow checker's.
// ---------------------------------------------------------------------

/// One of the three tables the encoder classifies bytes with, carried the
/// way `syntax.h` carries it: the whole table, indexed from `SYNBASE`.
///
/// The C spells these `basesyntax + SYNBASE` and then indexes with a
/// *signed* char, so the index runs from -129 up. A raw pointer into the
/// middle of the array is how the C gets a negative index, and it is also
/// how the C loses every bound: `mbtodest` probes `syntax[CTLMBCHAR]` at
/// -123, and under `is_type` unbiased that is a read before the array.
///
/// Carried as a slice with the origin folded into the accessor, the
/// negative index is ordinary arithmetic and the read is bounds-checked.
/// Nothing is given up: the deliberate deviation `IS_TYPE_UNBIASED`
/// documents is a *padded* table, so the probe lands on a real zero byte
/// here as it always has, and now the compiler can see that it does. A
/// panic out of `at` would mean a classification query this port has
/// never made.
#[derive(Clone, Copy)]
pub struct SyntaxRef(&'static [c_char]);

impl SyntaxRef {
    /// `syntax[c]`, where `syntax` is the C's `name + SYNBASE`.
    #[inline]
    fn at(self, c: c_int) -> c_char {
        self.0[(c + crate::syntax::SYNBASE) as usize]
    }
}

/// `syntax.h`: `#define BASESYNTAX (basesyntax + SYNBASE)`
///
/// A `const` rather than a function, which is what the C `#define`
/// already was. It had to be spelled as a pointer into the middle of an
/// array to carry the offset; [`SyntaxRef`] puts the offset in the
/// accessor, so the table is once again just a value.
const BASESYNTAX: SyntaxRef = SyntaxRef(&crate::syntax::basesyntax);

/// `syntax.h`: `#define SQSYNTAX (sqsyntax + SYNBASE)`
const SQSYNTAX: SyntaxRef = SyntaxRef(&crate::syntax::sqsyntax);

/// Backing store for [`IS_TYPE_UNBIASED`]. See that constant for why the
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
///
/// The 129 leading bytes are exactly `SYNBASE`, so this table indexes by
/// the same expression as the other two and [`SyntaxRef`] needs no
/// per-table origin.
const IS_TYPE_UNBIASED: SyntaxRef = SyntaxRef(&IS_TYPE_UNBIASED_PAD);

/// `syntax.h`: syntax class "like CWORD, except it must be escaped".
#[inline]
fn CCTL() -> c_char {
    crate::syntax::CCTL as c_char
}

/// `options.h`: `#define fflag optlist[1]`
#[inline]
unsafe fn fflag(sh: &crate::context::Shell) -> c_char {
    sh.options.flag(1)
}

/// `options.h`: `#define uflag optlist[14]`
#[inline]
unsafe fn uflag(sh: &crate::context::Shell) -> c_char {
    sh.options.flag(14)
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
pub(crate) unsafe fn preglob(
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
// [spec:dash:def:expand.esclen-fn]
// [spec:dash:sem:expand.esclen-fn]
//
/// `mesclen`: how many `mesc` bytes immediately precede `at`.
///
/// The C's `p > start` is `at > 0` — the walk cannot leave the string, and
/// with the string as a slice that is the bound rather than a promise about
/// two pointers being into the same allocation.
///
/// The pointer form and `esclen`, the one-argument wrapper over it, are
/// gone: `esclen` had a single caller, `scanright`, which walks the
/// expansion buffer by offset now and passes the subslice from `startp`,
/// so the floor `esclen` existed to carry is the slice's own start.
fn mesclen_bytes(s: &[u8], mut at: usize, mesc: c_char) -> size_t {
    let mut esc: size_t = 0;

    while at > 0 && s[at - 1] as c_char == mesc {
        at -= 1;
        esc += 1;
    }
    esc
}

// [spec:dash:def:expand.mbnext-fn]
// [spec:dash:sem:expand.mbnext-fn]
//
// Returns `start | end << 8`: the low byte is the offset from `p` to the
// character's data (past any markers), the next byte the span *from that
// data position* to the end of the encoded character.  The total span
// from `p` is therefore `(mb & 0xff) + (mb >> 8)`, which is why that
// expression appears at every call site.
// The pointer form is gone with its last caller.  It existed to answer
// "how much of this may I read?" for a walker holding a bare `*const
// c_char` -- three bytes when the first is CTLMBCHAR, one otherwise --
// and every walker that asked now holds a slice that answers it.
//
// The decoding itself, over a slice, so the framing is bounds-checked
// rather than trusted.
pub(crate) fn mbnext_bytes(p: &[u8]) -> c_uint {
    let mut start: c_uint = 0;
    let mut end: c_uint = 0;
    let ml: c_uint;

    let c = byte_at(p, end as usize);
    end += 1;

    match c {
        CTLMBCHAR => {
            if byte_at(p, end as usize) == CTLESC {
                end += 1;
            }
            ml = byte_at(p, end as usize) as u8 as c_uint;
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
    sh: &mut crate::context::Shell,
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
    argstr(sh, arg.narg().text.as_cbytes(), 0, flag)?;
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
            ifsbreakup(sh, &mut p, -1, &mut *ptr::addr_of_mut!(exparg));
            /* `*exparg.lastp = NULL; exparg.lastp = &exparg.list;` —
             * terminate the fields `ifsbreakup` built, then re-point the
             * tail at the head so `expandmeta` rebuilds the list while
             * walking the one it was handed.  The first append there
             * overwrites the head, which is why the C can read `str->next`
             * before the write reaches it; taking the `Vec` is both
             * halves. */
            let words = mem::take(expargl());
            expandmeta(sh, words)?;
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
unsafe fn argstr(
    sh: &mut crate::context::Shell,
    text: &[u8],
    mut p: usize,
    mut flag: c_int,
) -> Result<usize, Error> {
    static spclchars: [u8; 11] = [
        C_EQUALS as u8,
        C_COLON as u8,
        CTLQUOTEMARK as u8,
        CTLENDVAR as u8,
        CTLESC as u8,
        CTLVAR as u8,
        CTLBACKQ as u8,
        CTLMBCHAR as u8,
        CTLARI as u8,
        CTLENDARI as u8,
        0,
    ];
    /* The C advances a `const char *` into `spclchars`; the offset is the
     * whole of what it carries.  `strcspn`'s set is the array from there to
     * its terminator, which is index 10. */
    let mut reject: usize = 0;
    let mut c: c_int;
    let breakall: c_int = ((flag & (EXP_WORD | EXP_QUOTED)) == EXP_WORD) as c_int;
    let mut inquotes: c_int;
    let mut length: size_t;
    let mut startloc: c_int;

    reject += if (flag & EXP_VARTILDE2) != 0 { 1 } else { 0 };
    reject += if (flag & EXP_VARTILDE) != 0 { 0 } else { 2 };
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
            if byte_at(text, p) == C_TILDE {
                p = exptilde(sh, text, p, flag);
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
            let rejectset = &spclchars[reject..10];
            let from = p + length;
            length += (0usize..)
                .take_while(|&i| {
                    let c = byte_at(text, from + i);
                    c != 0 && !rejectset.contains(&(c as u8))
                })
                .count();
            c = byte_at(text, p + length) as c_int;
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
                let q: usize;

                /* `q = stnputs(p, length, expdest)`.  `p` walks the word
                 * text and never the expansion buffer, which is what the
                 * `copy_nonoverlapping` inside the old accessor already
                 * assumed and what makes this an append. */
                let b = expb();
                b.extend_from_slice(&text[p..p + length]);
                q = b.len();
                /* `*(q - 1) &= end - 1` */
                b[q - 1] &= (end - 1) as u8;
                /* `end` is 1 exactly when the byte just written closed the
                 * word (NUL, CTLENDVAR or CTLENDARI), and the line above
                 * has already turned it into a NUL.  Under EXP_WORD the
                 * cursor steps back over it, so it lands past the length —
                 * the outer `argstr` overwrites it on its next append. */
                b.truncate(q - (if (flag & EXP_WORD) != 0 { end } else { 0 }) as usize);
                newloc = q as c_int - end;
                if breakall != 0 && inquotes == 0 && newloc > startloc {
                    recordregion(startloc, newloc, 0);
                }
                startloc = newloc;
            }
            p += length + 1;
            length = 0;

            if end != 0 {
                break 'start;
            }

            match c as c_char {
                C_EQUALS | C_COLON => {
                    if (c as c_char) == C_EQUALS {
                        flag |= EXP_VARTILDE2;
                        reject += 1;
                        /* fall through */
                    }
                    /*
                     * sort of a hack - expand tildes in variable
                     * assignments (after the first '=' and after ':'s).
                     */
                    p -= 1;
                    if byte_at(text, p) == C_TILDE {
                        do_tilde = true;
                        continue 'start; /* goto tilde */
                    }
                    continue;
                }
                CTLQUOTEMARK => {
                    /* "$@" syntax adherence hack */
                    /* `dolatstr + 1` is the five bytes the parser emits for
                     * a bare `"$@"`, terminator excluded. */
                    let dolat = crate::mystring::dolatstr.map(|c| c as u8);
                    if inquotes == 0
                        && crate::mystring::cstr_prefix(slice_from(text, p)) == &dolat[1..6]
                    {
                        p = evalvar(sh, text, p + 1, flag | EXP_QUOTED)? + 1;
                        continue 'start; /* goto start */
                    }
                    inquotes ^= EXP_QUOTED;
                    /* addquote: */
                    if (flag & QUOTES_ESC) != 0 {
                        p -= 1;
                        length += 1;
                        startloc += 1;
                    }
                }
                CTLMBCHAR => {
                    c = byte_at(text, p) as c_int;
                    p -= 1;
                    mb = mbnext_bytes(slice_from(text, p));
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
                        p += (mb & 0xff) as usize;
                        if (flag & EXP_DISCARD) == 0 {
                            expb().extend_from_slice(&text[p..p + ml as usize]);
                        }
                        p += (mb >> 8) as usize;
                    }
                }
                CTLESC => {
                    startloc += 1;
                    length += 1;
                    /* goto addquote */
                    if (flag & QUOTES_ESC) != 0 {
                        p -= 1;
                        length += 1;
                        startloc += 1;
                    }
                }
                CTLVAR => {
                    p = evalvar(sh, text, p, flag | inquotes)?;
                    continue 'start; /* goto start */
                }
                CTLBACKQ => {
                    expbackq(sh, (&*argbackq)[0].as_ref(), flag | inquotes)?;
                    continue 'start; /* goto start */
                }
                CTLARI => {
                    p = expari(sh, text, p, flag | inquotes)?;
                    continue 'start; /* goto start */
                }
                _ => {}
            }
        }
    }
    Ok(p - 1)
}

// [spec:dash:def:expand.exptilde-fn]
// [spec:dash:sem:expand.exptilde-fn]
unsafe fn exptilde(
    sh: &mut crate::context::Shell,
    text: &[u8],
    startp: usize,
    flag: c_int,
) -> usize {
    let mut c: c_char;
    let name: usize;
    let home: *const c_char;
    let mut p: usize;

    p = startp;
    name = p + 1;

    loop {
        p += 1;
        c = byte_at(text, p);
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
        /* `c = *p; *p = '\0'; ...; *p = c;` — the C terminates the user
         * name in place because `getpwnam` and `lookupvar` want a C string
         * and the only one to hand is the word itself.  The word is shared,
         * borrowed and `&[u8]` now, so the name is copied out instead: it
         * is at most a login name long, it happens once per tilde, and it
         * is the last write this cluster made to the text it is reading. */
        let mut namebuf: Vec<u8> = text[name..p.min(text.len())].to_vec();
        namebuf.push(0);
        let namep = namebuf.as_ptr() as *const c_char;

        if namebuf.len() == 1 {
            home = crate::var::lookupvar(sh, crate::mystring::homestr.as_ptr());
        } else {
            home = getpwhome(namep);
        }
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
unsafe fn expari(
    sh: &mut crate::context::Shell,
    text: &[u8],
    start: usize,
    flag: c_int,
) -> Result<usize, Error> {
    let begoff: c_int;
    let len: c_int;
    let result: intmax_t;
    /* The C's `p` doubles as a scratch `stackblock()` before it becomes the
     * return value; only the second use survives. */
    let p: usize;

    begoff = expdest_off();
    p = argstr(sh, text, start, flag & EXP_DISCARD)?;

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
        /* The C reuses `start` for this; it is a position in the
         * *expansion buffer*, not in the word, and the two stopped being
         * the same kind of thing when the word became a slice. */
        let arith_at = expb()[begoff as usize..].as_ptr() as *mut c_char;

        removerecordregions(begoff);

        /* `arith` returns its diagnostic now instead of raising it, and as
         * of this commit so does `expari`, so the bridge that stood here is
         * gone and the value travels. */
        result = crate::arith_yacc::arith(sh, arith_at)?;

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
unsafe fn expbackq(
    sh: &mut crate::context::Shell,
    cmd: Option<&crate::nodes::Node>,
    flag: c_int,
) -> Result<(), Error> {
    let mut in_: crate::eval::backcmd = mem::zeroed();
    let mut i: c_int;
    /* `char buf[128]`, as bytes: it is only ever handed to `read` and to
     * `memtodest`, and both want the bytes rather than the sign. */
    let mut buf: [u8; 128] = [0; 128];
    let mut dest: usize;
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
        crate::eval::evalbackcmd(sh, cmd, &mut in_ as *mut crate::eval::backcmd)?;

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

        i = in_.nleft;
        /* `if (i == 0) goto read;` — skips the first memtodest only.  The
         * C's `p = in.buf` is gone with it: the assertion above says that
         * pointer is always NULL, so the only source this loop ever
         * encodes is `buf`, and after the first pass `p` was `buf`
         * anyway. */
        let mut jump_read = i == 0;
        loop {
            if !jump_read {
                memtodest(&buf[..i as usize], flag, expb());
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
                if let Some(e) = crate::error::poll_interrupt(sh) {
                    return Err(e);
                }
            }
            /* TRACE(("expbackq: read returns %d\n", i)); */
            if i <= 0 {
                break;
            }
        }

        if in_.fd >= 0 {
            libc::close(in_.fd);
            sh.eval.back_exitstatus = crate::jobs::waitforjob(sh, in_.jp)?;
        }
        crate::error::INTON();

        /* Eat all trailing newlines.  The cursor is the length, so the
         * walk is over the buffer's own bytes and `STADJUST` is a
         * `truncate`. */
        dest = expb().len();
        while dest > startloc as usize && expb()[dest - 1] == C_NL as u8 {
            /* STUNPUTC(dest) */
            dest -= 1;
        }
        expb().truncate(dest);

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
/// The C's seven arguments to [`scanleft`] and [`scanright`], five of which
/// are `char *` into the expansion buffer and are offsets here.
///
/// A struct rather than seven parameters because the function-pointer type
/// `subevalvar` selects between them with was the reason this cluster was
/// called indivisible: a `fn(*mut c_char, *mut c_char, *mut c_char, *mut
/// c_char, *mut c_char, c_int, c_int) -> *mut c_char` cannot be changed one
/// argument at a time. Named, it can.
///
/// Both scanners take the buffer by `&[u8]`. The C mutates it — it writes a
/// NUL at the position it is testing, matches, and writes the byte back —
/// and that write is the only reason it needed `*mut`. `pmatch_bytes` reads
/// past the end of a slice as NUL, so the subslice ending where the NUL
/// went is the same string, and the buffer is never written at all.
/// `&b[from..to]`, clamped to the buffer at both ends.
///
/// The scanners' cursors can leave the value — `scanright`'s walks off the
/// bottom on purpose — and every read outside it answers NUL rather than
/// panicking, which is the rule [`byte_at`] already follows and the one
/// `pmatch_bytes` was written to.
fn between(b: &[u8], from: usize, to: usize) -> &[u8] {
    let from = from.min(b.len());
    &b[from..to.clamp(from, b.len())]
}

struct Scan {
    /// The value being trimmed.
    startp: usize,
    /// Its last byte. `scanright` walks down from here.
    endp: usize,
    /// The unescaped copy `_rmescapes` left above the cursor, and its end.
    /// Read only when `FNMATCH_IS_ENABLED`; `loc2` tracks them either way,
    /// because it is what an unquoted match returns.
    rmesc: usize,
    rmescend: usize,
    /// The pattern, `preglob`'d in place.
    pat: usize,
    quotes: c_int,
    zero: c_int,
}

type ScanFn = fn(&[u8], &Scan) -> Option<usize>;

// [spec:dash:def:expand.scanleft-fn]
// [spec:dash:sem:expand.scanleft-fn]
fn scanleft(b: &[u8], a: &Scan) -> Option<usize> {
    let mut loc: usize = a.startp;
    let mut loc2: usize = a.rmesc;
    loop {
        let s: usize = if FNMATCH_IS_ENABLED { loc2 } else { loc };
        let c: c_char = byte_at(b, s);

        /* `c = *s; if (zero) { *s = '\0'; s = startp; } pmatch(str, s);
         * *loc = c;` — the temporary terminator, as a subslice that ends
         * where it went. */
        let subject: &[u8] = if a.zero != 0 {
            let from = if FNMATCH_IS_ENABLED { a.rmesc } else { a.startp };
            between(b, from, s)
        } else {
            slice_from(b, s)
        };
        if pmatch_slices(slice_from(b, a.pat), subject) != 0 {
            return Some(if a.quotes != 0 { loc } else { loc2 });
        }

        if c == C_NUL {
            break;
        }

        let mb: c_uint = mbnext_bytes(slice_from(b, loc));
        loc += ((mb & 0xff) + (mb >> 8)) as usize;
        let ml: c_uint = if (mb >> 8) > 3 { (mb >> 8) - 2 } else { 1 };
        loc2 += ml as usize;
    }
    None
}

// [spec:dash:def:expand.scanright-fn]
// [spec:dash:sem:expand.scanright-fn]
fn scanright(b: &[u8], a: &Scan) -> Option<usize> {
    let mut esc: size_t = 0;
    /* Signed, because the C's `loc--` walks off the bottom of the value on
     * purpose and `if (loc < startp) break` is how it notices.  `byte_at_i`
     * answers 0 for a negative index, so the two `*loc` reads inside the
     * multibyte rewind — which the C performs without a bounds test, on the
     * strength of the frame being well formed — cannot read before the
     * buffer here. */
    let mut loc: isize = a.endp as isize;
    let mut loc2: isize = a.rmescend as isize;
    /* `for (;; loc2--)` — the `continue`s below must still run `loc2--`,
     * hence the inner labelled block. */
    'forloop: loop {
        'cont: {
            let s: isize = if FNMATCH_IS_ENABLED { loc2 } else { loc };
            let ml: c_uint;

            /* `c = *s; if (zero) { *s = '\0'; s = startp; } pmatch(str, s);
             * *loc = c;` — see [`Scan`]: the subslice ends where the C's
             * temporary NUL went, so nothing is written. */
            let subject: &[u8] = if a.zero != 0 {
                let from = if FNMATCH_IS_ENABLED { a.rmesc } else { a.startp };
                between(b, from, s.max(0) as usize)
            } else {
                slice_from(b, s.max(0) as usize)
            };
            if pmatch_slices(slice_from(b, a.pat), subject) != 0 {
                return Some(if a.quotes != 0 { loc } else { loc2 } as usize);
            }
            loc -= 1;
            if loc < a.startp as isize {
                break 'forloop;
            }
            /* if (!esc--) esc = esclen(startp, loc); */
            let was: size_t = esc;
            esc = esc.wrapping_sub(1);
            if was == 0 {
                esc = mesclen_bytes(&b[a.startp..], loc as usize - a.startp, CTLESC);
            }
            if esc % 2 != 0 {
                esc -= 1;
                loc -= 1;
                break 'cont; /* continue */
            }
            if byte_at_i(b, loc) != CTLMBCHAR {
                break 'cont; /* continue */
            }

            loc -= 1;
            ml = byte_at_i(b, loc) as u8 as c_uint;
            loc -= (ml + 2) as isize;
            if byte_at_i(b, loc) == CTLESC {
                loc -= 1;
            }
            /* `loc2 -= ml - 1` with `ml` unsigned: when `ml` is 0 the C
             * subtracts UINT_MAX, not 1, and the widening is zero-extending
             * on both sides. */
            loc2 -= ml.wrapping_sub(1) as isize;
        }
        loc2 -= 1;
    }
    None
}

// [spec:dash:def:expand.subevalvar-fn]
// [spec:dash:sem:expand.subevalvar-fn]
unsafe fn subevalvar(
    sh: &mut crate::context::Shell,
    text: &[u8],
    start: usize,
    /* The C's `char *str`, which is the variable's *name* in the word on
     * entry and NULL for the trimming subtypes.  `Option` is that NULL as
     * a type; the C then reuses the same local for the pattern, which is
     * why the pattern has a name of its own below. */
    str: Option<usize>,
    strloc: c_int,
    startloc: c_int,
    varflags: c_int,
    flag: c_int,
) -> Result<usize, Error> {
    let mut subtype: c_int = varflags & VSTYPE;
    let quotes: c_int = flag & QUOTES_ESC;
    /* Every one of the C's `char *` locals here is a position in the
     * expansion buffer and only ever used as one.  As offsets they stop
     * having to be re-derived: the three `stackblock()` re-reads below the
     * `_rmescapes` call are gone, because an index does not move when the
     * buffer grows.  `str` keeps its pointer type because it is not one of
     * them — it is the variable's *name*, in the word text — and the C
     * reuses the same local for the pattern, which is why that one gets a
     * name of its own. */
    let startp: usize;
    let loc: usize;
    let mut rmesc: usize;
    let mut rmescend: usize;
    let zero: c_int;
    let scan: ScanFn;
    let endp: usize;
    let pat: usize;
    let p: usize;

    p = argstr(
        sh,
        text,
        start,
        (flag & EXP_DISCARD) | EXP_TILDE | (if str.is_some() { 0 } else { EXP_CASE }),
    )?;
    if (flag & EXP_DISCARD) != 0 {
        return Ok(p);
    }

    startp = startloc as usize;

    'out: {
        match subtype {
            VSASSIGN => {
                /* The bridge that stood here retires with this commit. */
                let name = text[str.expect("VSASSIGN carries the variable's name")..].as_ptr()
                    as *const c_char;
                crate::var::setvar(sh, name, expb()[startp..].as_ptr() as *const c_char, 0)?;

                loc = startp;
                break 'out;
            }

            VSQUESTION => {
                /* `varunset` stopped diverging with this commit, so this
                 * has to be a `return` and not a bare call. It was a stop
                 * before — docs/errors-are-values.md 0.2 is the bug that
                 * happens when one of these is missed, and `Error` is
                 * `#[must_use]` so the compiler now names it. */
                let umsg = crate::mystring::cstr_prefix(&expb()[startp..]);
                let var = str.expect("VSQUESTION carries the variable's name");
                return Err(varunset(sh, text, start, var, Some(umsg), varflags));
            }
            _ => {}
        }

        subtype -= VSTRIMRIGHT;
        /* #ifdef DEBUG
         *	if (subtype < 0 || subtype > 3)
         *		abort();
         * #endif */

        rmescend = strloc as usize;
        /* `str = preglob(rmescend, 0, NULL)` — in place while
         * `FNMATCH_IS_ENABLED` is 0, and into the buffer above the cursor
         * when it is not, so its result is a position in this buffer
         * either way. */
        pat = {
            let base = expb().as_mut_ptr() as *mut c_char;
            preglob(base.add(rmescend), 0, None).offset_from(base) as usize
        };

        rmesc = startp;
        if FNMATCH_IS_ENABLED || quotes == 0 {
            /* `_rmescapes` with RMESCAPE_GROW appends an unescaped copy of
             * `startp` past the cursor and moves the cursor over it, so the
             * buffer can have reallocated underneath.  That is what the C's
             * three `stackblock()` re-reads on the lines after this call
             * were for, and they are gone: an offset survives a growth,
             * which is why this hands over one and gets one back. */
            rmesc = rmescapes_grow(expb(), startp, RMESCAPE_ALLOC | RMESCAPE_GROW);
            if rmesc != startp {
                rmescend = expb().len();
            }
        }
        rmescend -= 1;

        /* zero = subtype == VSTRIMLEFT || subtype == VSTRIMLEFTMAX */
        zero = subtype >> 1;
        /* VSTRIMLEFT/VSTRIMRIGHTMAX -> scanleft */
        scan = if ((subtype & 1) ^ zero) != 0 {
            scanleft
        } else {
            scanright
        };

        endp = strloc as usize - 1;
        let found = scan(
            expb(),
            &Scan {
                startp,
                endp,
                rmesc,
                rmescend,
                pat,
                quotes,
                zero,
            },
        );
        match found {
            None => {
                if quotes != 0 {
                    rmesc = startp;
                    rmescend = endp;
                }
            }
            Some(at) if quotes == 0 => {
                if zero != 0 {
                    rmesc = at;
                } else {
                    rmescend = at;
                }
            }
            Some(at) if zero != 0 => {
                rmesc = at;
                rmescend = endp;
            }
            Some(at) => {
                rmesc = startp;
                rmescend = at;
            }
        }

        /* `memmove(startp, rmesc, rmescend - rmesc)` — the two ranges are
         * in one buffer and may overlap, which `copy_within` already
         * knows. */
        expb().copy_within(rmesc..rmescend, startp);
        loc = startp + (rmescend - rmesc);
    }

    /* out: */
    /* `*loc = '\0'; STADJUST(loc - expdest, expdest)` — the terminator is
     * written *at* the new cursor, so it lands one past the length rather
     * than inside it.  `push` then `pop` is how an owned buffer says
     * "write it, do not count it", and it keeps the byte the C wrote: a
     * later reallocation would drop it, because `reserve` copies only the
     * first `len` bytes, but nothing reallocates before `argstr` writes
     * the word's own terminator over it (`*(q - 1) &= end - 1` forces the
     * closing NUL, CTLENDVAR or CTLENDARI to 0).  `amount` was only ever
     * `loc - expdest`. */
    let b = expb();
    debug_assert!(loc <= b.len());
    b.truncate(loc);
    b.push(0);
    b.pop();

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
unsafe fn evalvar(
    sh: &mut crate::context::Shell,
    text: &[u8],
    mut p: usize,
    mut flag: c_int,
) -> Result<usize, Error> {
    let mut subtype: c_int;
    let mut varflags: c_int;
    let var: usize;
    let patloc: c_int;
    let startloc: c_int;
    let mut varlen: ssize_t;
    let mut discard: c_int;
    let mut quoted: c_int;
    let mbchar: c_int;

    varflags = (byte_at(text, p) as c_int) & !VSBIT;
    p += 1;
    subtype = varflags & VSTYPE;

    quoted = flag & EXP_QUOTED;
    var = p;
    startloc = expdest_off();
    /* The parser always writes the `=` that ends the variable name, and
     * the C dereferences `strchr`'s result without checking. */
    p += crate::mystring::cstr_prefix(slice_from(text, p))
        .find_byte(C_EQUALS as u8)
        .expect("the parser ends a variable name with `=`")
        + 1;

    mbchar = match subtype {
        VSTRIMLEFT | VSTRIMLEFTMAX | VSTRIMRIGHT | VSTRIMRIGHTMAX => EXP_MBCHAR,
        _ => 0,
    };

    /* `record:` and `really_record:` are the two joins at the bottom. */
    let mut really_record = false;

    'again: loop {
        /* `varvalue` still takes the name as a `char *`: it reaches
         * `lookupvar` and `atoi`, which measure with `strlen` and `strtol`.
         * The name is in the word and the word outlives the call. */
        let namep = text[var..].as_ptr() as *mut c_char;
        varlen = varvalue(sh, namep, varflags, (flag | mbchar) as c_uint)?;
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

                p = argstr(sh, text, p, flag | EXP_TILDE | EXP_WORD | (discard ^ EXP_DISCARD))?;
                break 'again; /* goto record */
            }

            VSASSIGN | VSQUESTION => {
                p = subevalvar(
                    sh,
                    text,
                    p,
                    Some(var),
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

        if (discard & !flag) != 0 && uflag(sh) != 0 {
            /* A stop before `varunset` stopped diverging, and still one. */
            return Err(varunset(sh, text, p, var, None, 0));
        }

        if subtype == VSLENGTH {
            p += 1;
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
        p = subevalvar(sh, text, p, None, patloc, startloc, varflags, flag)?;
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
        quoted = (byte_at(text, var) == C_AT && sh.options.shellparam.nparam != 0) as c_int;
        if quoted == 0 {
            return Ok(p);
        }
    }
    recordregion(startloc, expdest_off(), quoted);
    Ok(p)
}

// [spec:dash:def:expand.chtodest-fn]
// [spec:dash:sem:expand.chtodest-fn]
/// The cursor the C returns is the destination's own length now, so this
/// appends and returns nothing. It performs no unsafe operation at all.
fn chtodest(c: c_int, syntax: SyntaxRef, out: &mut BString) {
    if syntax.at(c) == CCTL() {
        /* USTPUTC(CTLESC, out) */
        out.push(CTLESC as u8);
    }
    /* USTPUTC(c, out) */
    out.push(c as u8);
}

// [spec:dash:def:expand.mbpair]
#[repr(C)]
pub struct mbpair {
    pub ml: c_uint,
    pub ql: c_uint,
}

// [spec:dash:def:expand.mbtodest-fn]
// [spec:dash:sem:expand.mbtodest-fn]
// `p` and the C's `len` became `src` and the index of the byte *after* the
// one to decode — the position `memtodest`'s cursor is at when it calls,
// which is why the first thing both do is step back over it. `len` is not
// a parameter any more: it was always "bytes from `p - 1` to the end of
// the input", which a slice answers.
//
// Safe, and the slice is the reason: `mbrlen`'s obligation is that `n`
// bytes are readable from `s`, which used to be a number the caller had to
// get right and is now the slice's own length. The initial conversion
// state is all-zero by definition — the C writes `mbstate_t mbs = {}` — so
// `zeroed` produces a valid `mbstate_t` rather than an uninitialised one.
// Two operations move inside the block rather than disappearing.
fn mbtodest(src: &[u8], at: usize, dst: &mut BString, syntax: SyntaxRef) -> mbpair {
    let mut mbs: libc::mbstate_t = unsafe { mem::zeroed() };
    let mbp: mbpair;
    /* The C's `q0`: where this call started writing. A length, because
     * the cursor is one. */
    let q0: usize = dst.len();
    let mut ml: size_t;

    /* `p = p - 1` */
    let p: &[u8] = &src[at - 1..];
    ml = unsafe { mbrlen(p.as_ptr() as *const c_char, p.len(), &mut mbs) };
    'out: {
        if ml == (0 as size_t).wrapping_sub(2) || ml == (0 as size_t).wrapping_sub(1) || ml < 2 {
            chtodest(p[0] as c_char as c_int, syntax, dst);
            ml = 1;
            break 'out;
        }

        /* `syntax[CTLMBCHAR]` — CTLMBCHAR is negative; see the note in
         * `memtodest` about the unbiased `is_type` table. Negative is an
         * ordinary index now, and a checked one. */
        if syntax.at(CTLMBCHAR as c_int) == CCTL() {
            /* USTPUTC(CTLMBCHAR, q); USTPUTC(ml, q); */
            dst.push(CTLMBCHAR as u8);
            dst.push(ml as u8);
        }

        /* `q = mempcpy(q, p, ml)`. The source is the caller's input and
         * never `dst`'s own buffer -- `memtodest` records why -- so the
         * append cannot alias what it reads.  `ml` came from `mbrlen`
         * over this same slice, so it cannot exceed it. */
        dst.extend_from_slice(&p[..ml]);

        if syntax.at(CTLMBCHAR as c_int) == CCTL() {
            /* USTPUTC(ml, q); USTPUTC(CTLMBCHAR, q); */
            dst.push(ml as u8);
            dst.push(CTLMBCHAR as u8);
        }
    }

    /* out: */
    /* `ql` is the C's "how far did q move", which the destination's own
     * length now answers for the only caller. It is still returned
     * because `mbpair` is the C's return type and carries a spec rule;
     * what changed is that nobody has to trust it. */
    mbp = mbpair {
        ml: (ml.wrapping_sub(1)) as c_uint,
        ql: (dst.len() - q0) as c_uint,
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
// `makestrspace` is `reserve` and every `USTPUTC` is a `push`.  There is
// no commit: the length is correct after each write rather than at the
// end.  `p` never points into `dst` — every caller's source is a variable
// value, a `read` buffer, a `getpwnam` field or a stack array — which is
// what makes appending safe while reading `p`.
// `(p, len)` became `src`.  The C's pair is a slice everywhere it is
// constructed — a variable's value, a `read` buffer, a directory entry, a
// stack array — and carrying it as one removes the walk's every bound
// question at once: `p` cannot run past `len`, the eight-byte fast path
// reads eight bytes that exist, and `mbtodest`'s `p - 1` is an index into
// something with a start.
fn memtodest(src: &[u8], flags: c_int, dst: &mut BString) -> size_t {
    let syntax: SyntaxRef;
    let mut count: size_t = 0;
    let expq: c_int;
    /* The C's `p` and `len` are one cursor over `src` and the number of
     * bytes left; `i` is the first and `src.len() - i` the second. */
    let mut i: usize = 0;

    if src.is_empty() {
        return 0;
    }

    /* CTLMBCHAR, 2, c, c, 2, CTLMBCHAR.  A hint now rather than a
     * contract: the writes below are appends, so a short reservation
     * costs a growth instead of running off the end. */
    dst.reserve(src.len() * 3);

    /* Guarded by the `assert!(QUOTES_ESC == 0x11 && …)` above, which is
     * this file's port of the matching `#error`. */
    expq = flags & EXP_QUOTED;
    if (flags & (expq >> 3 | expq >> 4 | expq >> 8) & (QUOTES_ESC | EXP_MBCHAR)) == 0 {
        while src.len() - i >= 8 {
            let x: u64;

            /* `__builtin_memcpy` of eight bytes into a `uint64_t`, which
             * is an unaligned load the C spells with a cast.  Over a
             * slice it is a checked eight-byte read, and the check is
             * the loop condition. */
            x = u64::from_ne_bytes(src[i..i + 8].try_into().unwrap());

            if (x | x.wrapping_sub(0x0101010101010101)) & 0x8080808080808080 != 0 {
                break;
            }

            /* The C's `write_unaligned(q + count, x)` is a copy of the
             * eight bytes just read, and `to_ne_bytes` is that copy: the
             * value round-trips through the same native representation
             * it was loaded from. The C's `q = q + count` after the loop
             * is gone because appending has already moved the cursor. */
            dst.extend_from_slice(&x.to_ne_bytes());

            count += 8;
            i += 8;
        }

        /* NOTE (bug-for-bug): `is_type` is used here *unbiased*, i.e.
         * without the `+ SYNBASE` every other syntax-table user applies.
         * `chtodest` only ever indexes it with 0..127, which is in range
         * and always reads 0 (never CCTL) — that is the point of the
         * choice.  `mbtodest` however indexes it with CTLMBCHAR (-123),
         * a read *before* the array; the C relies on that happening to
         * yield a non-CCTL byte.  Reproduced verbatim, not fixed. */
        syntax = if (flags & (QUOTES_ESC | EXP_MBCHAR)) != 0 {
            BASESYNTAX
        } else {
            IS_TYPE_UNBIASED
        };
    } else {
        syntax = SQSYNTAX;
    }

    /* for (; len; len--) */
    while i < src.len() {
        'cont: {
            let c: c_int = src[i] as c_char as c_int;
            i += 1;

            if c == 0 && (flags & EXP_KEEPNUL) == 0 {
                break 'cont; /* continue */
            }

            count += 1;

            if c < 0 {
                /* `mbtodest(p, ...)` is called with `p` already past the
                 * byte it is about to decode, and starts by stepping
                 * back over it; `i` is that same position. */
                let mbp: mbpair = mbtodest(src, i, dst, syntax);
                let mlm: c_uint;

                /* `q += mbp.ql` — the append did it. */
                mlm = mbp.ml;
                i += mlm as usize;
                break 'cont; /* continue */
            }

            chtodest(c, syntax, dst);
        }
    }

    /* The C's `expdest = q` was this port's `set_len` over bytes a raw
     * cursor had filled. Appending keeps the length correct at every
     * step, so there is nothing to commit and no window in which `dst`
     * has a length that disagrees with its contents. */
    count
}

// [spec:dash:def:expand.strtodest-fn]
// [spec:dash:sem:expand.strtodest-fn]
//
// The C string entry, for the callers that hold one: a variable's value,
// a positional parameter, `getpwnam`'s home directory.  The `strlen` the C
// performs is `to_bytes`, which is the same scan and also the length.
unsafe fn strtodest(p: *const c_char, flags: c_int, dst: &mut BString) -> size_t {
    memtodest(CStr::from_ptr(p).to_bytes(), flags, dst)
}

/*
 * Add the value of a specialized variable to the stack string.
 */

// [spec:dash:def:expand.varvalue-fn]
// [spec:dash:sem:expand.varvalue-fn]
unsafe fn varvalue(
    sh: &mut crate::context::Shell,
    name: *mut c_char,
    varflags: c_int,
    mut flags: c_uint,
) -> Result<ssize_t, Error> {
    let subtype: c_int = varflags & VSTYPE;
    let mut seplen: size_t;
    /* The C's `const char *seps` plus its length.  The comment that stood
     * at the assignment below owed a conversion — it said the pointer was
     * safe *because* of where the storage comes from, which is an argument
     * a slice does not have to make.  Both sources are bytes the shell
     * owns for the whole call, so both are slices. */
    let mut seps: &[u8];
    let mut len: ssize_t = 0;
    let start: size_t;
    let discard: c_int;
    let mut ap: *mut *mut c_char;
    let mut num: c_int = 0;
    let mut p: *mut c_char = ptr::null_mut();

    discard =
        ((subtype == VSPLUS || subtype == VSLENGTH) as c_int) | ((flags as c_int) & EXP_DISCARD);

    if subtype == 0 {
        if discard != 0 {
            return Ok(-1);
        }

        return Err(sh.sh_error_value(b"Bad substitution"));
    }

    flags &= if discard != 0 {
        (!QUOTES_ESC) as c_uint
    } else {
        !(0 as c_uint)
    };
    /* `seps = nullstr` — the empty C string, whose one byte is the
     * terminator, and the terminator is what gets written when the
     * separator is a NUL. */
    seps = &[0u8];
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
                            num = sh.status;
                            break 'numvar;
                        }
                        C_HASH => {
                            num = sh.options.shellparam.nparam;
                            break 'numvar;
                        }
                        C_BANG => {
                            num = sh.backgndpid as c_int;
                            if num == 0 {
                                return Ok(-1);
                            }
                            break 'numvar;
                        }
                        C_MINUS => {
                            /* `makestrspace(NOPTS, expdest)` and a run of
                             * `USTPUTC` through the cursor, committed by
                             * assigning `expdest`.  Appending writes the
                             * same bytes in the same order and makes the
                             * reservation the allocator's business rather
                             * than a bound this loop has to keep -- the
                             * same trade the encoder took in `02bf791`. */
                            let mut i = crate::options::NOPTS;
                            while i > 0 {
                                i -= 1;
                                let letter = crate::options::optletters[i];
                                if sh.options.flag(i) != 0 && letter != 0 {
                                    expb().push(letter as u8);
                                    len += 1;
                                }
                            }
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
                                seps = sh.ifs.ncifs.as_slice();
                            }
                            seplen = (seplen.wrapping_sub(1) & sh.ifs.ifsmb0len.wrapping_sub(1))
                                .wrapping_add(1);
                            break 'param;
                        }
                        c if c >= C_0 && c <= C_9 => {
                            num = libc::atoi(name);
                            if num < 0 || num > sh.options.shellparam.nparam {
                                return Ok(-1);
                            }
                            p = if num != 0 {
                                *sh.options.shellparam.p().offset(num as isize - 1)
                            } else {
                                crate::options::arg0
                            };
                            break 'value;
                        }
                        _ => {
                            /* default: */
                            p = crate::var::lookupvar(sh, name);
                            break 'value;
                        }
                    }
                }
                /* numvar: */
                len = cvtnum(num as intmax_t, flags as c_int, expb()) as ssize_t;
                break 'sw;
            }
            /* param: */
            ap = sh.options.shellparam.p();
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

                /* `memtodest(seps, seplen, ...)` — the C reads `seplen`
                 * bytes from `seps`, and the two are set together above:
                 * one byte of `nullstr`, or `ifsmb0len` bytes of `IFS`,
                 * which is the length of its first character and so at
                 * most `IFS`'s own.  Asserted rather than clamped: a
                 * clamp would turn the C reading past its buffer into a
                 * shorter separator and say nothing, which is the one
                 * outcome worse than either. */
                debug_assert!(
                    seplen <= seps.len(),
                    "varvalue: separator length {seplen} exceeds the {} bytes it names",
                    seps.len()
                );
                len += memtodest(&seps[..seplen], (flags as c_int) | EXP_KEEPNUL, expb()) as ssize_t;
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
unsafe fn ifsisifs(sh: &Shell, s: &[u8], ml: c_uint, nulonly: c_int) -> c_uint {
    let mut isdefifs: bool = false;
    let mut isifs: bool = false;
    let mut wc: wchar_t = byte_at(s, 0) as wchar_t;
    /* C leaves `ifs0` uninitialised; it is only read when `isifs`, which
     * implies one of the branches below assigned it. */
    let mut ifs0: wchar_t = 0;

    /* The C's `ifst->ifs`: `nullstr` when the region is NUL-only, the
     * shell's `IFS` otherwise. Both are NUL-terminated and the terminator
     * is *in* the searched set below, so the empty case is `[0]` rather
     * than `[]` — a NUL byte in a NUL-only region is a separator, and
     * that is the whole of what a NUL-only region means. */
    const NULONLY: &[u8] = &[0];
    let ifs: &[u8] = if nulonly != 0 {
        NULONLY
    } else {
        sh.ifs.ncifs.as_slice()
    };

    'out: {
        if ifs[0] != 0 && !sh.ifs.wcifs.is_empty() {
            if (wc & 0x80) != 0 {
                let mut mbst: libc::mbstate_t = mem::zeroed();
                let mut wc2: wchar_t = 0;

                /* `ml` came from `mbnext` over this same slice, so the
                 * clamp can only bite where the C read past the word's
                 * end -- and a short read fails the `!= ml` test exactly
                 * as a malformed character does.  The same trade
                 * `ccmatch_bytes` records. */
                let n = (ml as size_t).min(s.len());
                if mbrtowc(&mut wc2, s.as_ptr() as *const c_char, n, &mut mbst) != ml as size_t {
                    break 'out;
                }
                wc = wc2;
            }

            isifs = wcifs_chr(&sh.ifs.wcifs, wc);
            ifs0 = sh.ifs.wcifs[0];
        } else if ml == 0 {
            /* `strchr` matches the terminator, so a NUL character --
             * which is what `ml == 0` means -- counts as an IFS byte.
             * The counted terminator on `ncifs` keeps that, and it is why
             * the slice is searched whole rather than trimmed. */
            isifs = ifs.contains(&(wc as u8));
            ifs0 = ifs[0] as wchar_t;
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
    sh: &mut Shell,
    ifst: &mut ifs_state,
    arglist: &mut arglist,
    nulonly: c_int,
    string: &mut [u8],
    mut p: usize,
) -> usize {
    let ifschar: c_uint;
    let sisifs: c_uint;
    let isdefifs: bool;
    let ml: c_uint;
    let isifs: bool;
    let mut q: usize;

    q = p;

    ifschar = mbnext_bytes(slice_from(string, p));
    p += (ifschar & 0xff) as usize;
    ml = if (ifschar >> 8) > 3 {
        (ifschar >> 8) - 2
    } else {
        0
    };

    sisifs = ifsisifs(sh, slice_from(string, p), ml, ifst.nulonly);
    p += (ifschar >> 8) as usize;

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
        if ifst.maxargs == 0 {
            if isdefifs {
                if ifst.r.is_none() {
                    ifst.r = Some(q);
                }
                return p;
            }

            if !(isifs && ifst.ifsspc != 0) {
                ifst.r = None;
            }
        } else if ifst.ifsspc != 0 {
            if isifs {
                q = p;
            }

            ifst.start = q;

            if isdefifs {
                return p;
            }
        } else if isifs {
            let mut ifsspc: c_int = ifst.ifsspc;

            if nulonly == 0 {
                ifsspc = isdefifs as c_int;
                ifst.ifsspc = ifsspc;
            }

            /* Ignore IFS whitespace at start */
            if q == ifst.start && ifsspc != 0 {
                ifst.start = p;
                break 'out_zero_ifsspc; /* goto out_zero_ifsspc */
            }
            /* if (ifst->maxargs > 0 && !--ifst->maxargs) */
            if ifst.maxargs > 0 && {
                ifst.maxargs -= 1;
                ifst.maxargs == 0
            } {
                ifst.r = Some(q);
                return p;
            }
            string[q] = C_NUL as u8;
            arglist.list.push(strlist::from_cbytes(&string[ifst.start..]));
            ifst.start = p;
            return p;
        }
    }

    /* out_zero_ifsspc: */
    ifst.ifsspc = 0;
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
pub unsafe fn ifsbreakup(
    sh: &mut Shell,
    string: &mut [u8],
    maxargs: c_int,
    arglist: &mut arglist,
) {
    let mut ifsp: usize;
    /* `struct ifs_state ifst;` and the three assignments the C makes
     * before the loop, as one initialiser. `mem::zeroed` was standing in
     * for the C leaving `ifs` and `ifsspc` unset here, and both are
     * assigned on every path that reads them; a struct without a pointer
     * in it can say so directly. */
    let mut ifst: ifs_state = ifs_state {
        nulonly: 0,
        start: 0,
        r: None,
        maxargs,
        ifsspc: 0,
    };
    let mut nulonly: c_int;
    let mut p: usize;

    'add: {
        if !ifsr().is_empty() {
            ifst.ifsspc = 0;
            nulonly = 0;
            /* `realifs = ifsset() ? ncifs : nullstr` is gone with the
             * pointer it cached: `ifsisifs` reads `IFS` off the shell,
             * and what it needs from here is the one bit below. */
            ifsp = 0;
            loop {
                let afternul: c_int;
                let endoff: c_int = ifsr()[ifsp].endoff;

                p = ifsr()[ifsp].begoff as usize;
                debug_assert!(
                    endoff as usize <= string.len(),
                    "a recorded region ends past the word it was recorded in"
                );
                afternul = nulonly;
                nulonly = ifsr()[ifsp].nulonly;
                ifst.nulonly = nulonly;
                ifst.ifsspc = 0;
                loop {
                    let p0: usize = p;

                    /* `stackblock() + endoff - p >= 8` — eight bytes of
                     * this region left to look at.  As offsets it is also
                     * the bound that makes the load below a checked one. */
                    while endoff as usize >= p + 8 {
                        /* union { uint64_t qw; unsigned char b[8]; } x; */
                        let b: [u8; 8] = string[p..p + 8].try_into().unwrap();
                        let qw: u64 = u64::from_ne_bytes(b);

                        if (qw & 0x8080808080808080) != 0 {
                            break;
                        }
                        if (sh.ifs.ifsmap[b[0] as usize]
                            | sh.ifs.ifsmap[b[1] as usize]
                            | sh.ifs.ifsmap[b[2] as usize]
                            | sh.ifs.ifsmap[b[3] as usize]
                            | sh.ifs.ifsmap[b[4] as usize]
                            | sh.ifs.ifsmap[b[5] as usize]
                            | sh.ifs.ifsmap[b[6] as usize]
                            | sh.ifs.ifsmap[b[7] as usize])
                            != 0
                        {
                            break;
                        }
                        p += 8;
                    }

                    if p != p0 {
                        if ifst.maxargs == 0 {
                            ifst.r = None;
                        } else if ifst.ifsspc != 0 {
                            ifst.start = p0;
                        }
                        ifst.ifsspc = 0;
                    }

                    if p >= endoff as usize {
                        break;
                    }

                    p = ifsbreakup_slow(sh, &mut ifst, arglist, afternul | nulonly, string, p);
                }

                ifsp += 1;
                if ifsp >= ifsr().len() {
                    break;
                }
            }
            if nulonly != 0 {
                break 'add; /* goto add */
            }
            if let Some(r) = ifst.r {
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
                    r >= ifst.start,
                    "the trailing-IFS truncation lands in an already-taken field"
                );
                string[r] = C_NUL as u8;
            }
        }

        if byte_at(string, ifst.start) == C_NUL {
            return;
        }
    }

    /* add: */
    arglist.list.push(strlist::from_cbytes(&string[ifst.start..]));
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
pub unsafe fn changeifs(sh: &mut crate::context::Shell, mut ifs: *const c_char) {
    let mut mbs: libc::mbstate_t = mem::zeroed();
    let mut nwcifs: Vec<wchar_t>;
    let mut mb: c_uint = 0;
    let len: size_t;
    let mut p: *const c_char;
    let mut ml: size_t;

    if crate::var::ifsset(sh) == 0 {
        ifs = crate::var::defifs();
    }
    /* The hook is still handed a `char *` by the variable table, so the
     * scan to the terminator happens once, here, and the bytes are the
     * shell's from then on. The terminator is kept: `ifsisifs` searches
     * through it. */
    sh.ifs.ncifs = BString::from(CStr::from_ptr(ifs).to_bytes_with_nul());

    /* memset(ifsmap, 0, sizeof(ifsmap)) */
    sh.ifs.ifsmap = [0; 128];

    /* The C walks to the terminator and processes it *before* breaking,
     * so `ifsmap[0]` is set on every call — the loop below keeps that by
     * iterating over the counted terminator rather than stopping short of
     * it. `len` is the length without it, which is what the C's counter
     * held. */
    len = sh.ifs.ncifs.len() - 1;
    for i in 0..sh.ifs.ncifs.len() {
        let c: c_uint = sh.ifs.ncifs[i] as c_uint;

        mb |= c >> 7;
        if (c >> 7) == 0 {
            sh.ifs.ifsmap[c as usize] = 1;
        }
    }

    nwcifs = Vec::new();

    sh.ifs.ifsmb0len = (len != 0) as size_t;

    'out: {
        if mb == 0 {
            break 'out;
        }

        ml = mbrlen(sh.ifs.ncifs.as_ptr() as *const c_char, len, &mut mbs);
        if ml == (0 as size_t).wrapping_sub(2) || ml == (0 as size_t).wrapping_sub(1) {
            ml = 1;
        }
        sh.ifs.ifsmb0len = ml;

        /* The C `ckmalloc`s `len + 1` wide characters and zero-fills them
         * before `mbsrtowcs` writes a prefix; the zero fill is what makes
         * the result NUL-terminated when the conversion fails part-way,
         * and `wcifs_chr` still depends on it. */
        nwcifs = vec![0 as wchar_t; len + 1];

        /* `mbsrtowcs` advances the cursor it is given, so it needs a
         * `char *` and gets one into the shell's own copy rather than
         * into the variable's text. */
        p = sh.ifs.ncifs.as_ptr() as *const c_char;
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
    sh.ifs.wcifs = nwcifs;
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
unsafe fn expandmeta_glob(sh: &mut crate::context::Shell, words: Vec<strlist>) -> Result<(), Error> {
    for mut str in words {
        let p: *const c_char;
        let mut pglob: crate::system::glob64_t = mem::zeroed();
        let i: c_int;

        'sw: {
            'nometa: {
                'nometa2: {
                    if fflag(sh) != 0 {
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
                        return Err(sh.sh_error_value(b"Out of space"));
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
unsafe fn expandmeta(sh: &mut crate::context::Shell, words: Vec<strlist>) -> Result<(), Error> {
    /* TODO - EXP_REDIR */

    if GLOB_IS_ENABLED {
        return expandmeta_glob(sh, words);
    }

    /* The C's `preglob(..., RMESCAPE_HEAP)` result: one `ckmalloc` per
     * word, `ckfree`d as soon as `expmeta` has read it.  That is a local
     * buffer's lifetime exactly, and reusing it across the loop is the
     * only difference — `expmeta` never re-enters `preglob`, because the
     * only `preglob` under it is `patmatch`'s, which does not allocate
     * while `FNMATCH_IS_ENABLED` is 0. */
    let mut pattern: Vec<u8> = Vec::new();

    /* The glob buffer, owned here and lent to `expmeta`.  One allocation
     * per `expandmeta` that globs anything, reused across the word loop
     * exactly as the region's block was; see the comment above
     * [`expmeta`]'s neighbours for why it stopped being a `static`. */
    let mut globbuf: BString = BString::new(Vec::new());

    for mut str in words {
        let savelastp: usize;
        let p: *mut c_char;

        'sw: {
            'nometa: {
                if fflag(sh) != 0 {
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

                /* The C's top-level `expmeta` starts on whatever block the
                 * region is on and gets away with it because `expdir_len`
                 * is 0: it writes from the base and never reads what was
                 * there.  An owned buffer's length is not 0 — the previous
                 * glob's `addfnamealt` left it at that glob's `expdir_len`
                 * — and every consequence of carrying it in is benign,
                 * which is the reason to clear rather than to argue.  The
                 * frame invariant is then an equality, and an equality is
                 * what `expmeta` can assert on entry.
                 *
                 * `p` is `pattern`'s buffer (or `str.text`, when
                 * `_rmescapes` found nothing to remove and returned its
                 * argument).  Neither is the glob buffer, which is what
                 * makes lending both to `expmeta` at once sound; the
                 * pattern is read-only from here down. */
                globbuf.clear();
                expmeta(&mut globbuf, CStr::from_ptr(p).to_bytes(), 0);
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
unsafe fn addfnamealt(b: &mut BString, expdir_len: size_t) {
    /* `name = grabstackstr(enddir)` — in the C this allocates nothing and
     * copies nothing: it moves the region's bump pointer past bytes that
     * are already in place, which is how C says "these outlive the next
     * candidate".
     *
     * The candidate cannot simply be moved out, and that is the one place
     * in this pass where a copy stays.  The field wants the whole buffer
     * and the *next* candidate wants `[0, expdir_len)` — the same bytes —
     * so one of the two has to take a copy.  The C copies the prefix back
     * (`STARTSTACKSTR(enddir); stnputs(name, expdir_len, enddir)`) because
     * `grabstackstr` had already given the block away; this copies the
     * field out and keeps the buffer, which costs the same order and leaves
     * the glob buffer's capacity alone.  What has gone is the region: the
     * copy is into the field's own allocation, not into a block a
     * `popstackmark` has to free.
     *
     * The C's `enddir` parameter is gone, and it is worth saying why,
     * because it was the one real coupling in this conversion.  `enddir`
     * answered "how many bytes of the buffer are the candidate?", and the
     * answer differed from the buffer's own length in exactly one caller:
     * the no-metacharacter branch, whose `expmeta_rmescapes` wrote through
     * a raw cursor and never committed, so the bytes were written but
     * uncounted and `addfnamealt` had to count them itself.  Now that
     * `expmeta_rmescapes` appends, both callers arrive with the candidate
     * counted, `enddir` and `b.len()` say the same number, and the one
     * that has to go is the parameter. */
    debug_assert_eq!(b.last(), Some(&0), "the candidate is a C string");
    addfname_common(BString::from(b.to_vec()));

    /* `STARTSTACKSTR(enddir); return stnputs(name, expdir_len, enddir) -
     * expdir_len;` — the C has to start a new block and copy the directory
     * prefix back into it, because `grabstackstr` gave the old one away.
     * Nothing was given away here, so the prefix is still the first
     * `expdir_len` bytes and re-seeding is `truncate`. */
    b.truncate(expdir_len);
}

// [spec:dash:def:expand.expmeta-rmescapes-fn]
// [spec:dash:sem:expand.expmeta-rmescapes-fn]
/// Unescape `name` and **append** it to the glob buffer.
///
/// The C takes a cursor and returns where it stopped, which is the position
/// of the NUL it wrote; both callers then do arithmetic against that
/// position.  Appending answers both of them with `b.len()` and removes the
/// cursor, so what is left to decide is who owns the terminator.  It is not
/// part of the path — one caller wants it (`lstat` needs a C string) and
/// the other must not have it counted (the terminator is where the next
/// component gets appended) — so this appends the bytes and nothing else,
/// and each caller adds the NUL it needs.
///
/// The C's other parameter is gone the same way: `name` was NUL-terminated
/// by the caller writing a temporary NUL into the pattern and putting it
/// back afterwards (`c = *start; *start = 0; ...; *start = c`).  A subslice
/// says "just this much of the pattern" without writing to it, which is
/// what lets `expmeta`'s `name` be a `&[u8]`.
unsafe fn expmeta_rmescapes(b: &mut BString, name: &[u8]) {
    let at = b.len();

    if !FNMATCH_IS_ENABLED {
        /* The C copies `name` to the cursor and unescapes it in place.
         * `_rmescapes` still speaks C strings — it is the next conversion,
         * not this one — so the copy lands in the buffer with a terminator,
         * is unescaped there, and the terminator is dropped again.
         * `_rmescapes` only ever shortens, so this cannot reach past what
         * was appended. */
        b.extend_from_slice(name);
        b.push(0);
        let p = b[at..].as_mut_ptr() as *mut c_char;
        rmescapes(p);
        let n = CStr::from_ptr(p).count_bytes();
        debug_assert!(n <= name.len());
        b.truncate(at + n);
        return;
    }

    let mut p: usize = 0;
    loop {
        /* `q = strchrnul(p, '\\')`, then `mempcpy(enddir, p, q - p + 1)` —
         * the copy includes the byte *at* `q`, which is either the
         * backslash or the string's terminator. */
        let q: usize = name[p..]
            .find_byte(C_BACKSLASH as u8)
            .map_or(name.len(), |at| p + at);

        b.extend_from_slice(&name[p..q]);
        b.push(byte_at(name, q) as u8);
        p = q;
        if p == name.len() {
            break;
        }
        p += 1;
        if p != name.len() {
            /* `*enddir.offset(-1) = *p` — the escaped byte overwrites the
             * backslash that was just copied. */
            let last = b.len() - 1;
            b[last] = name[p];
            p += 1;
        }
    }

    /* `return enddir - 1` — the C hands back the position of the NUL its
     * last `mempcpy` copied.  Here that NUL is the last byte appended, and
     * the caller's terminator is its own business. */
    b.pop();
    debug_assert!(b.len() >= at);
}

/* #ifndef HAVE_MEMRCHR */
// [spec:dash:def:expand.memrchr-fn]
// [spec:dash:sem:expand.memrchr-fn]
//
// The C's fallback for a libc that lacks `memrchr`, and its one caller in
// `expmeta` is now `rfind_byte` over a slice.  It stays because the C's
// `#ifndef` does, and because the rules above are about this shim rather
// than about whoever happened to call it.
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
unsafe fn expmeta(b: &mut BString, name: &[u8], mut expdir_len: size_t) {
    let mesc: c_char = if FNMATCH_IS_ENABLED {
        C_BACKSLASH
    } else {
        CTLESC
    };
    let mut statb: libc::stat64 = mem::zeroed();
    let mut dp: *mut libc::dirent64;
    let mut endname: usize;
    let mut zeroedp: usize;
    let mut matchdot: bool;
    let mut esc: size_t;
    let start: usize;
    /* `DIR *dirp;` — Rust needs the binding initialised before the
     * volatile store below, which is the C's actual initialisation. */
    let mut dirp: *mut libc::DIR = ptr::null_mut();
    let pat: &[u8];
    let mut p: usize;
    let c: c_char;
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

            /* The glob buffer's frame invariant, stated where it is relied
             * on: this frame's prefix is `[0, expdir_len)` and it is
             * exactly what the buffer counts as written.  `expandmeta`
             * clears for the top-level call; a recursive one arrives
             * straight out of the append that wrote the component.
             *
             * The C's `growstackto(expdir_len + name_len + 1)` was a
             * *bound*, because everything below wrote through a raw
             * cursor.  Appending needs no bound, so the same number is
             * only a hint that says how big this frame's candidate will
             * be before its component. */
            debug_assert_eq!(b.len(), expdir_len);
            b.reserve(name.len() + 1);

            /* `for (;;) { p = strpbrk(p + esc, "*?]"); ... }` — find the
             * first metacharacter that is not itself escaped. */
            p = 0;
            esc = 0;
            let meta: Option<usize> = loop {
                let from = p + esc;
                let Some(at) = name[from..].find_byteset(b"*?]") else {
                    break None;
                };
                p = from + at;
                esc = mesclen_bytes(name, p, mesc) & 1;
                if esc == 0 {
                    break Some(p);
                }
            };
            /* No meta characters */
            let Some(meta) = meta else {
                if expdir_len == 0 {
                    break 'out_opendir; /* goto out_opendir */
                }
                expmeta_rmescapes(b, name);
                /* The C's `enddir` is on the NUL `expmeta_rmescapes` wrote
                 * and `addfnamealt` is handed `enddir + 1`, so the
                 * terminator is part of the candidate.  Appending it here
                 * says that, and `lstat` needs it anyway. */
                b.push(0);
                if libc::lstat64(b.as_ptr() as *const c_char, &mut statb) >= 0 {
                    addfnamealt(b, expdir_len);
                } else {
                    /* The C leaves its uncounted bytes where they are and
                     * returns the base; counted bytes have to be rewound,
                     * so that this frame returns with the buffer holding
                     * its prefix and nothing else. */
                    b.truncate(expdir_len);
                }
                break 'out_opendir; /* goto out_opendir */
            };
            match name[..meta].rfind_byte(C_SLASH as u8) {
                Some(at) => {
                    /* `c = *start; *start = 0; expmeta_rmescapes(enddir,
                     * name); *start = c;` — the C borrows the pattern as
                     * the directory prefix by terminating it in place.  A
                     * subslice is that without the write, and without the
                     * restore. */
                    start = at + 1;
                    expmeta_rmescapes(b, &name[..start]);
                    /* `expdir_len = enddir - cp` — this frame's prefix
                     * grew by the unescaped directory part, and the bytes
                     * it grew over are counted because they were
                     * appended. */
                    expdir_len = b.len();
                }
                None => start = 0,
            }

            /* `*enddir = 0` — the prefix has to be a C string for
             * `opendir`, and only for `opendir`: the terminator is not
             * part of the prefix, which is where the next component goes. */
            b.push(0);
            /* *(DIR *volatile *)&dirp = opendir(expdir_len ? cp : dotdir); */
            ptr::write_volatile(
                &mut dirp,
                libc::opendir(if expdir_len != 0 {
                    b.as_ptr() as *const c_char
                } else {
                    crate::mystring::dotdir.as_ptr()
                }),
            );
            b.truncate(expdir_len);
            if dirp.is_null() {
                break 'out_opendir; /* goto out_opendir */
            }
            /* `p = strchrnul(p + 1, '/')` — the end of the component the
             * metacharacter is in.  The C's `esc = 0` before this is a
             * dead store in both languages: `esc` is read only inside the
             * branch that sets it. */
            p = name[meta + 1..]
                .find_byte(C_SLASH as u8)
                .map_or(name.len(), |at| meta + 1 + at);
            zeroedp = p;
            endname = p;
            if p != name.len() {
                let esc = mesclen_bytes(name, p, mesc) & 1;
                zeroedp -= esc;
                endname += 1;
            }
            /* `c = *zeroedp; *zeroedp = 0;` — the C reads the byte it is
             * about to overwrite so it can put it back, and everything
             * below tests `c` for "is there another component?".  The
             * component is a subslice, so nothing is overwritten and
             * nothing is put back; `c` is just the byte that follows it,
             * or NUL at the end of the pattern.
             *
             * `name_len -= endname - name` is the recursion's argument and
             * is `name[endname..].len()`, which is why it stopped being a
             * parameter. */
            c = byte_at(name, zeroedp);
            matchdot = false;
            pat = &name[start..zeroedp];
            p = 0;
            if byte_at(pat, p) == mesc {
                p += 1;
            }
            if byte_at(pat, p) == C_DOT {
                matchdot = true;
            }
            loop {
                dp = libc::readdir64(dirp);
                if dp.is_null() {
                    break;
                }
                let dnamep: *const c_char = (*dp).d_name.as_ptr();

                'check_int: {
                    if *dnamep == C_DOT && !matchdot {
                        break 'check_int; /* goto check_int */
                    }
                    if c != 0
                        && (*dp).d_type != libc::DT_DIR
                        && (*dp).d_type != libc::DT_LNK
                        && (*dp).d_type != libc::DT_UNKNOWN
                    {
                        break 'check_int; /* goto check_int */
                    }
                    /* `len = strlen(dname) + 1` — the terminator is part
                     * of what gets appended, because the candidate is a C
                     * string and the next component overwrites it. */
                    let dname: &[u8] = CStr::from_ptr(dnamep).to_bytes_with_nul();
                    let len: size_t = dname.len();
                    let subject: &[u8] = if !FNMATCH_IS_ENABLED {
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
                         * `cp = stackblock(); enddir = cp + expdir_len` is
                         * gone with the pointers: it was the C's re-read
                         * after a possible growth, and an index does not
                         * move. */
                        globenc.clear();
                        memtodest(dname, EXP_MBCHAR | EXP_KEEPNUL, &mut globenc);
                        debug_assert_eq!(
                            globenc.last(),
                            Some(&0),
                            "EXP_KEEPNUL carries the entry's terminator through"
                        );
                        &globenc
                    } else {
                        dname
                    };
                    if crate::pmatch::pmatch_slices(pat, subject) != 0 {
                        /* `enddir = stnputs(dname, len, enddir)` — an
                         * append at a cursor below the end, which is
                         * truncate-then-append. */
                        b.truncate(expdir_len);
                        b.extend_from_slice(dname);
                        if c == 0 {
                            addfnamealt(b, expdir_len);
                        } else {
                            /* `*enddir.offset(-1) = C_SLASH` — the entry's
                             * terminator becomes the separator. */
                            let last = b.len() - 1;
                            b[last] = C_SLASH as u8;
                            expmeta(b, &name[endname..], expdir_len + len);
                            /* `enddir = cp + expdir_len` — the frame's
                             * rewind, said out loud.  The child returns
                             * with the buffer holding *its* prefix, which
                             * is this one plus the component just
                             * appended. */
                            b.truncate(expdir_len);
                        }
                    }
                }
                /* check_int: */
                if int_pending() != 0 {
                    break;
                }
            }
        }

        /* out: */
        /* NOTE: `closedir(NULL)` is reachable here in the C when the
         * (never-installed) handler fires before `opendir`; glibc
         * tolerates a NULL argument. */
        libc::closedir(ptr::read_volatile(&dirp));
    }

    /* out_opendir: */
    /* The C returns `cp`, the block's base, and every caller immediately
     * recomputes `cp + expdir_len`.  What that is really saying is a
     * postcondition, and it is this: on return the buffer holds this
     * frame's prefix and nothing above it.  `expdir_len` is the frame's
     * own, which may have grown past the caller's — hence the caller's
     * rewind after the recursive call. */
    debug_assert_eq!(b.len(), expdir_len);
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
 * Remove any CTLESC characters from a string.
 */

// [spec:dash:def:expand.rmescapes-fn]
// [spec:dash:sem:expand.rmescapes-fn]
/// The transform, over one buffer, in place.
///
/// `buf` holds the C string with its terminator; `at` is the index of the
/// first byte in [`cqchars`], which the caller has already scanned for as
/// the C does with `strpbrk`. Returns the length of the result, terminator
/// not counted, and writes the terminator at that index.
///
/// In place is the only shape any caller needs, because **the output never
/// exceeds the input**: `CTLQUOTEMARK` consumes a byte and writes none,
/// `CTLESC` consumes two and writes at most two, both `CTLMBCHAR` arms
/// write no more than they consume, and everything else is one for one.
/// So `q <= p` throughout and the write is always at or behind the read,
/// which is what lets the two allocating callers reach this same body by
/// materialising their source into their destination first.
///
/// Recorded in plan/decisions/owned-data.md, "What this cost in the port:
/// `_rmescapes`", together with the two reach-backs' safety argument and
/// why the one configuration that *could* grow is asserted unreachable
/// rather than given a second engine.
fn rmescapes_compact(buf: &mut [u8], at: usize, flag: c_int) -> usize {
    /* The growing configuration is `FNMATCH_IS_ENABLED` together with
     * globbing, where the `CTLESC` arm can write three bytes for two.
     * Compaction cannot express that -- `q` would overtake `p` and clobber
     * source the walk has not read -- and it is unreachable by
     * construction, because the only producer of `RMESCAPE_GLOB` is
     * `preglob`, which under FNMATCH also sets `RMESCAPE_ALLOC` and so
     * always has the separate, doubled destination the C sized for it.
     * Checked here rather than believed. */
    const _: () = assert!(
        !FNMATCH_IS_ENABLED,
        "rmescapes_compact: FNMATCH_IS_ENABLED with globbing can grow the string, \
         which in-place compaction cannot express; see plan/decisions/owned-data.md"
    );

    let globbing: c_int = flag & RMESCAPE_GLOB;
    let mut inquotes: c_int = 0;
    let mut notescaped: c_int = globbing;
    /* The C's `p` and `q`, which are indices into one buffer here. */
    let mut p: usize = at;
    let mut q: usize = at;

    'whileloop: while byte_at(buf, p) != C_NUL {
        let mut c: c_int = byte_at(buf, p) as c_int;
        let mut newnesc: c_int = globbing;
        let mb: c_uint;
        let mut ml: c_uint;

        'setnesc: {
            if c == CTLQUOTEMARK as c_int {
                p += 1;
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
                        buf[q] = C_BACKSLASH as u8;
                        q += 1;
                    } else {
                        /* Reaches back one byte.  `notescaped` is cleared
                         * only by the naked-backslash arm, which writes a
                         * byte first, so `q` has advanced at least once
                         * before this is reachable -- and the index is
                         * checked, where the C's was not. */
                        buf[q - 1] = C_BACKSLASH as u8;
                    }
                }
                if globbing != 0 {
                    buf[q] = if FNMATCH_IS_ENABLED {
                        C_BACKSLASH
                    } else {
                        CTLESC
                    } as u8;
                    q += 1;
                }

                p += 1;
                c = byte_at(buf, p) as c_int;
            } else if c == CTLMBCHAR as c_int {
                let mut tail: c_uint = 2;

                if !FNMATCH_IS_ENABLED && (globbing ^ notescaped) != 0 {
                    q -= 1;
                }

                mb = mbnext_bytes(slice_from(buf, p));
                ml = mb >> 8;

                if globbing == 0 || FNMATCH_IS_ENABLED {
                    p += (mb & 0xff) as usize;
                    ml -= 2;
                } else {
                    ml += mb & 0xff;
                    tail = 0;
                }

                /* `q` trails `p` through the same buffer, which
                 * `copy_within` already knows -- it is the C's
                 * `memmove`, bounds-checked. */
                buf.copy_within(p..p + ml as usize, q);
                q += ml as usize;
                p += (ml + tail) as usize;
                break 'setnesc; /* goto setnesc */
            }

            buf[q] = c as u8;
            q += 1;
            p += 1;
        }
        /* setnesc: */
        notescaped = newnesc;
    }
    if !FNMATCH_IS_ENABLED && (globbing ^ notescaped) != 0 {
        /* The same reach-back, and the same argument. */
        buf[q - 1] = C_BACKSLASH as u8;
    }
    /* `*q = '\0'` — the loop exited with `p` on the terminator and
     * `q <= p`, so this lands inside the buffer at worst on that
     * terminator. */
    buf[q] = C_NUL as u8;
    q
}

/// The index of the first byte `_rmescapes` has anything to do with, if
/// there is one.
///
/// `strpbrk`'s set is the string without its terminator: it never matches
/// a NUL, which is what stops the scan instead.
fn rmescapes_scan(s: &[u8]) -> Option<usize> {
    let cqset = crate::mystring::cqchars.map(|c| c as u8);
    s.find_byteset(&cqset[..4])
}

// [spec:dash:def:expand.rmescapes-fn]
// [spec:dash:sem:expand.rmescapes-fn]
//
// The in-place and `RMESCAPE_HEAP` entries.  `RMESCAPE_GROW` moved to
// [`rmescapes_grow`], which takes the offset its one caller already has
// instead of a pointer into a buffer that can move under it -- that is
// what retired `expbase`, `expdest`, `set_expdest` and `expmakestrspace`.
//
// The C's `fulllen` arithmetic is gone with the raw cursor it bounded:
// both destinations are appended to, so a short reservation costs a growth
// instead of a heap overflow, and there is no number left to assert
// against.
pub unsafe fn _rmescapes(
    str: *mut c_char,
    flag: c_int,
    heap: Option<&mut Vec<u8>>,
) -> *mut c_char {
    debug_assert!(
        (flag & RMESCAPE_GROW) == 0,
        "_rmescapes: RMESCAPE_GROW goes to rmescapes_grow"
    );

    /* The source, terminator included, as the buffer the transform works
     * over.  In place this *is* the destination. */
    let n: usize = CStr::from_ptr(str).count_bytes();
    let src: &mut [u8] = core::slice::from_raw_parts_mut(str as *mut u8, n + 1);

    let Some(at) = rmescapes_scan(&src[..n]) else {
        return str;
    };

    if (flag & RMESCAPE_ALLOC) == 0 {
        rmescapes_compact(src, at, flag);
        return str;
    }

    /* The C splits the allocating case in two: `ckmalloc(fulllen)` under
     * RMESCAPE_HEAP and `stalloc(fulllen)` otherwise.  The `stalloc` half
     * is unreachable, and the reason is one constant away:
     * `RMESCAPE_ALLOC` is only ever set by `preglob`, which sets it under
     * `if (FNMATCH_IS_ENABLED)`, and by `subevalvar`'s `ALLOC | GROW`,
     * which is now [`rmescapes_grow`].  `FNMATCH_IS_ENABLED` is 0, so the
     * only caller that arrives here is `expandmeta`'s
     * `preglob(text, RMESCAPE_ALLOC | RMESCAPE_HEAP)` -- and that is the
     * caller supplying `heap`.  Asserted rather than claimed, because
     * [dec:nsh:owned-data] records this exact flag being reasoned about
     * wrongly once already. */
    debug_assert!(
        (flag & RMESCAPE_HEAP) != 0 && heap.is_some(),
        "_rmescapes: RMESCAPE_ALLOC without GROW reaches only the HEAP arm"
    );
    let out = heap.expect("_rmescapes: RMESCAPE_ALLOC without GROW needs a heap buffer");

    /* `mempcpy(q, str, len)` copies the verbatim prefix and the walk
     * writes the rest.  Copying the whole source and compacting it is the
     * same result -- the transform below `at` is the identity -- and it is
     * what lets one body serve every destination. */
    out.clear();
    out.extend_from_slice(src);
    let m = rmescapes_compact(out, at, flag);
    out.truncate(m + 1);
    out.as_mut_ptr() as *mut c_char
}

// [spec:dash:def:expand.rmescapes-fn]
// [spec:dash:sem:expand.rmescapes-fn]
//
/// `_rmescapes(b + at, RMESCAPE_ALLOC | RMESCAPE_GROW)`: unescape the C
/// string at `at` into fresh space at the end of the same buffer, and
/// return where it landed.
///
/// The C takes a pointer, calls `makestrspace`, and then re-reads
/// `stackblock()` three times because that call can move the block. An
/// offset does not move, so the caller passes one and gets one back, and
/// the `expdest`/`stackblock` accessors retire with the last pointer.
///
/// `expdest = r; STADJUST(q - r + 1)` is the `truncate` below. The C runs
/// that assignment on the `RMESCAPE_HEAP` path too, where `r` is a block
/// the caller frees moments later -- so the C leaves `expdest` pointing
/// into freed memory. It is harmless only because of where `expandmeta`
/// sits, after `grabstackstr` has taken the word and before the next
/// `STARTSTACKSTR`. An owned buffer cannot hold that pointer and has no
/// reason to, so that store is not transcribed on the heap path: a
/// deliberate divergence from a write with no observable value.
pub unsafe fn rmescapes_grow(b: &mut BString, at: usize, flag: c_int) -> usize {
    debug_assert!(
        (flag & (RMESCAPE_ALLOC | RMESCAPE_GROW)) == (RMESCAPE_ALLOC | RMESCAPE_GROW),
        "rmescapes_grow is the RMESCAPE_ALLOC | RMESCAPE_GROW path"
    );

    let n: usize = crate::mystring::cstr_prefix(&b[at..]).len();
    if rmescapes_scan(&b[at..at + n]).is_none() {
        /* `return str` — before the block is grown, so the cursor is
         * untouched and the caller's `rmesc == startp` test sees it. */
        return at;
    }
    let at_rel = rmescapes_scan(&b[at..at + n]).expect("scanned once already");

    /* `r = makestrspace(fulllen); mempcpy(q, str, len)` — the destination
     * is the space past the cursor, and the source is below it in the same
     * buffer, which is exactly what `extend_from_within` is for. */
    let r: usize = b.len();
    b.extend_from_within(at..at + n + 1);
    let m = rmescapes_compact(&mut b[r..], at_rel, flag);
    b.truncate(r + m + 1);
    r
}

/*
 * See if a pattern matches in a case statement.
 */

// [spec:dash:def:expand.casematch-fn]
// [spec:dash:sem:expand.casematch-fn]
pub unsafe fn casematch(
    sh: &mut crate::context::Shell,
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
    argstr(sh, pattern.narg().text.as_cbytes(), 0, EXP_TILDE | EXP_CASE)?;
    ifsfree();
    /* The C reads the word back as `stackblock()`. */
    result = crate::pmatch::patmatch(expb().as_mut_ptr() as *mut c_char, val);
    Ok(result)
}

/*
 * Our own itoa().
 */

// [spec:dash:def:expand.cvtnum-fn]
// [spec:dash:sem:expand.cvtnum-fn]
unsafe fn cvtnum(num: intmax_t, flags: c_int, dst: &mut BString) -> size_t {
    let value = format!("{num}");
    memtodest(value.as_bytes(), flags, dst)
}

// [spec:dash:def:expand.varunset-fn]
// [spec:dash:sem:expand.varunset-fn]
unsafe fn varunset(sh: &mut crate::context::Shell, 
    text: &[u8],
    end: usize,
    var: usize,
    umsg: Option<&[u8]>,
    varflags: c_int,
) -> Error {
    /* The C's three `char *` here are a NULL test and two `%s` arguments,
     * and every one of them is spent on the next five lines.  `nullstr` was
     * the empty tail and `msg` a string literal; as byte slices the
     * terminator is not part of either, so the two `CStr::from_ptr` scans
     * that used to re-measure them are gone.  `umsg`'s `Option` is the
     * NULL test said as a type — its one non-null caller hands over the
     * expansion buffer's message, which is a slice at the call site rather
     * than a pointer here. */
    let mut tail: &[u8] = b"";
    let mut msg: &[u8] = b"parameter not set";
    if let Some(umsg) = umsg {
        if byte_at(text, end) == CTLENDVAR {
            if (varflags & VSNUL) != 0 {
                tail = b" or null";
            }
        } else {
            msg = umsg;
        }
    }
    /* `end - var - 1` — the variable's name, without the `=` the parser
     * writes after it.  Saturating because the C's subtraction is signed
     * and it clamped at zero. */
    let name_len = end.saturating_sub(var + 1);
    let mut message = Vec::new();
    message.extend_from_slice(&text[var..(var + name_len).min(text.len())]);
    message.extend_from_slice(b": ");
    message.extend_from_slice(msg);
    message.extend_from_slice(tail);
    sh.sh_error_value(&message)
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
/* Unused as an import and kept as a symbol: it is this rule's target
 * site, and nothing in the crate calls `arith` through `expand`. It was
 * reachable as `nsh::expand::arith` until the surface closed, which is
 * what had been standing in for a use. */
#[allow(unused_imports)]
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
/* Kept for the same reason as `arith` above. */
#[allow(unused_imports)]
pub use crate::arith_yylex::yylex;

