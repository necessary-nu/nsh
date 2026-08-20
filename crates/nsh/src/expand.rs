//! Shell word expansion.
//!
//! Active argument and case-pattern expansion is implemented by [`typed`]
//! as structural transformations over parsed word parts. The remaining
//! translated helpers in this file serve compatibility call sites such as
//! `read` field splitting; later cleanup leaves remove that inactive port
//! machinery once its callers have typed interfaces of their own.

#![allow(unknown_lints)]
use crate::context::Shell;
use core::mem;
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};

use bstr::{BStr, BString, ByteSlice};
use core::ffi::{c_char, c_int, c_uint};

use crate::error::Error;
use crate::mystring::{byte_at, byte_at_i, slice_from};
use crate::nodes::Node;
use crate::options::{OPTION_SPECS, ShellOption};
// [spec:nsh:def:idiom.shell-options]
use crate::pmatch::pmatch_slices;

mod mode;
mod typed;

use mode::EscapeMode;
pub(crate) use mode::ExpansionMode;

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

// C character literals used as `switch` labels; Rust `match` patterns
// require named constants, so the ones this file switches on get names.
pub(crate) const C_NUL: c_char = 0;
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

    /// `rmescapes(sp->text)`, in place as the C does it.
    ///
    /// `_rmescapes` shortens the C string and says nothing about by how
    /// much, so the length is re-derived. No reader of a field uses its
    /// length — they all stop at the terminator, as the C's did — so the
    /// truncation is hygiene rather than correctness: what it buys is that
    /// the entry's length keeps meaning the string's length, which is what
    /// makes the assertion in [`strlist::textp`] worth anything.
    #[inline]
    pub fn rmescapes(&mut self) {
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

/// [`rmescapes`] over a buffer that owns its bytes.
///
/// `_rmescapes` shortens the C string in place and says nothing about by
/// how much, so every caller re-derives the length; two did it by hand and
/// spelled the same three operations differently. Returns the length of
/// the unescaped string **without** its terminator, and leaves the
/// terminator to the caller — a `strlist` field keeps it because
/// [`strlist::textp`] asserts it is there, and a here-document delimiter
/// drops it because it is compared as bytes.
// [spec:posix:req:expand.quote-removal]
// [spec:posix:sem:expand.quote-removal-quoting-remembered]
pub fn rmescapes_owned(s: &mut BString) -> usize {
    rmescapes_buffer(s, EscapeMode::Plain)
}

// [spec:dash:def:expand.ifsregion]
/// A byte range eligible for field splitting.
pub struct ifsregion {
    pub begoff: usize,
    pub endoff: usize,
    pub nulonly: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FieldLimit {
    Unlimited,
    Remaining(usize),
}

// [spec:dash:def:expand.ifs-state]
/// Mutable state for one field-splitting pass.
pub struct ifs_state {
    pub nulonly: bool,
    pub start: usize,
    /// Start of a trailing IFS run that should be removed.
    pub r: Option<usize>,
    max_fields: FieldLimit,
    pub ifsspc: bool,
}

/// Owned intermediate buffers for one expansion.
pub(crate) struct ExpandState {
    buffer: BString,
    backquotes: Vec<Option<crate::nodes::Node>>,
    next_backquote: usize,
    ifs_regions: Vec<ifsregion>,
    args: Vec<strlist>,
}

impl ExpandState {
    pub(crate) const fn new() -> Self {
        Self {
            buffer: BString::new(Vec::new()),
            backquotes: Vec::new(),
            next_backquote: 0,
            ifs_regions: Vec::new(),
            args: Vec::new(),
        }
    }
}

impl Default for ExpandState {
    fn default() -> Self {
        Self::new()
    }
}

/// Per-shell `IFS` data in byte, first-character, and wide-character forms.
pub struct IfsCache {
    /// The single-byte members, as a lookup table.
    ifsmap: [bool; 128],
    /// `IFS` itself, including a terminating NUL used by field splitting.
    ncifs: BString,
    /// Length of the first multibyte character, or 0.
    ifsmb0len: usize,
    /// The wide-character form of `IFS`, built by `changeifs`.
    wcifs: Vec<i32>,
}

impl IfsCache {
    pub(crate) const fn new() -> Self {
        IfsCache {
            ifsmap: [false; 128],
            ncifs: BString::new(Vec::new()),
            ifsmb0len: 0,
            wcifs: Vec::new(),
        }
    }
}

#[inline]
fn ifsr(state: &mut ExpandState) -> &mut Vec<ifsregion> {
    &mut state.ifs_regions
}

/// `&mut exparg.list`, same.  Every `*exparg.lastp = sp` in the C is a
/// `push` on this, and `exparg.lastp = &exparg.list` — the C's way of
/// throwing away whatever the previous expansion left in the head — is a
/// `clear`.
#[inline]
fn expargl(state: &mut ExpandState) -> &mut Vec<strlist> {
    &mut state.args
}

/// `wcschr(wcifs, wc) != NULL`.
///
/// Transcribed rather than replaced with `contains`, because `wcschr`
/// searches a NUL-*terminated* string and therefore **matches the
/// terminator** when `wc` is 0.  `ifsisifs` reaches here with `wc` taken
/// straight from the byte under the cursor, so a NUL inside an IFS region
/// takes the `isifs` branch in the C and has to here too.
#[inline]
fn wcifs_chr(v: &[i32], wc: i32) -> bool {
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

#[inline]
fn expb(state: &mut ExpandState) -> &mut BString {
    &mut state.buffer
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
fn expdest_off(state: &mut ExpandState) -> c_int {
    expb(state).len() as c_int
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
fn grabexpdest(state: &mut ExpandState) -> BString {
    let b = expb(state);
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
pub fn expansion_result(sh: &crate::context::Shell) -> &BStr {
    crate::mystring::cstr_prefix(sh.expand.buffer.as_slice())
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
// argument about interrupt counters and becomes the borrow checker's.
// ---------------------------------------------------------------------

/// Escaping policy used while copying expansion bytes to their destination.
// [spec:nsh:req:idiom.lexer-tokens]
#[derive(Clone, Copy)]
enum DestinationSyntax {
    Base,
    SingleQuoted,
    /// The old unbiased `is_type` use deterministically escaped nothing.
    Unframed,
}

impl DestinationSyntax {
    #[inline]
    fn escapes(self, byte: u8) -> bool {
        let context = match self {
            Self::Base => Some(crate::syntax::SyntaxContext::Base),
            Self::SingleQuoted => Some(crate::syntax::SyntaxContext::SingleQuoted),
            Self::Unframed => None,
        };
        context.is_some_and(|context| {
            context.classify(crate::syntax::InputUnit::Byte(byte))
                == crate::syntax::SyntaxClass::Control
        })
    }
}

/// `error.h`: `#define int_pending() intpending`
#[inline]
fn int_pending() -> c_int {
    crate::error::int_pending()
}

/*
 * Prepare a pattern for a glob(3) call.
 *
 * Returns an stalloced string.
 */

// [spec:dash:def:expand.preglob-fn]
// [spec:dash:sem:expand.preglob-fn]
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
fn mesclen_bytes(s: &[u8], mut at: usize, mesc: c_char) -> usize {
    let mut esc: usize = 0;

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
/*
 * Perform variable substitution and command substitution on an argument,
 * placing the resulting list of arguments in arglist.  If EXP_FULL is true,
 * perform splitting and file name expansion.  When arglist is NULL, perform
 * here document expansion.
 */

// [spec:dash:def:expand.expandarg-fn]
// [spec:dash:sem:expand.expandarg-fn]
// [spec:posix:req:expand.order]
// [spec:posix:req:expand.single-field]
// [spec:posix:req:expand.brace-implementation-defined]
// [spec:posix:req:expand.execution-environment]
// [spec:posix:req:expand.assignment-redirection-environment]
// [spec:posix:def:expand.dollar-introducer]
// [spec:posix:req:expand.dollar-invalid-follower]
// [spec:posix:req:expand.dollar-literal]
// [spec:posix:sem:shell.word-processing]
// [spec:nsh:req:idiom.structural-ast]
pub fn expandarg(
    sh: &mut crate::context::Shell,
    arg: &crate::nodes::Node,
    arglist: Option<&mut arglist>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let Node::Word(word) = arg else {
        return Err(sh.sh_error_value(b"word expansion requires a word node"));
    };
    // [spec:nsh:def:idiom.word-ir]
    // [spec:nsh:sem:idiom.typed-expansion]
    typed::expand_argument(sh, &word.word, arglist, mode)
}

// [spec:nsh:req:idiom.parser-control-flow]
fn expandarg_inner(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    arglist: Option<&mut arglist>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let mut p: BString;

    /* STARTSTACKSTR(expdest) */
    expb(state).clear();
    /* The `?`s in this function return past the `ifsfree()` below, exactly
     * as the longjmp they replace jumped past it. The IFS regions are
     * reclaimed by the catch frame instead — `restore_handler_expandarg`'s
     * swallowing arm and `init::exitreset` both call `ifsfree`, which is
     * docs/errors-are-values.md 2.2's mark-keyed cleanup working as
     * designed. Adding one here would free them twice. */
    argstr(sh, state, text, 0, mode)?;
    if let Some(arglist) = arglist {
        p = grabexpdest(state);
        /* `exparg.lastp = &exparg.list`.  It re-points the tail at the
         * head, which discards whatever the previous call left there —
         * reachable only when that call unwound between building the list
         * and splicing it into its caller's. */
        expargl(state).clear();
        /*
         * TODO - EXP_REDIR
         */
        if mode.contains(ExpansionMode::SPLIT) {
            /* The fields copy out of the word rather than pointing into
             * it, so the word itself is a local that dies at the end of
             * this block.  The C could not do that: its fields *are*
             * offsets into the grabbed block, which is why the block had to
             * outlive them and why the enclosing mark had to be the thing
             * that freed it. */
            ifsbreakup_regions(
                sh,
                &state.ifs_regions,
                &mut p,
                FieldLimit::Unlimited,
                &mut state.args,
            );
            /* `*exparg.lastp = NULL; exparg.lastp = &exparg.list;` —
             * terminate the fields `ifsbreakup` built, then re-point the
             * tail at the head so `expandmeta` rebuilds the list while
             * walking the one it was handed.  The first append there
             * overwrites the head, which is why the C can read `str->next`
             * before the write reaches it; taking the `Vec` is both
             * halves. */
            let words = mem::take(expargl(state));
            expandmeta(sh, state, words)?;
        } else {
            expargl(state).push(strlist { text: p });
        }
        /* `if (exparg.list) { *arglist->lastp = exparg.list; arglist->lastp
         * = exparg.lastp; }`.  The C guards on emptiness because splicing a
         * NULL head would leave the caller's tail pointing at `exparg`'s
         * own head; appending an empty `Vec` is already a no-op. */
        arglist.list.append(expargl(state));
    }

    ifsfree(state);
    Ok(())
}

/*
 * Perform variable and command substitution.  If EXP_FULL is set, output CTLESC
 * characters to allow for further processing.  Otherwise treat
 * $@ like $* since no splitting will be performed.
 */

// [spec:dash:def:expand.argstr-fn]
// [spec:dash:sem:expand.argstr-fn]
fn argstr(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    mut p: usize,
    mut mode: ExpansionMode,
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
    let break_all =
        mode.contains(ExpansionMode::PARAMETER_WORD) && !mode.contains(ExpansionMode::QUOTED);
    let mut in_quotes: bool;
    let mut length: usize;
    let mut startloc: c_int;

    reject += usize::from(mode.contains(ExpansionMode::COLON_TILDE));
    reject += if mode.contains(ExpansionMode::ASSIGNMENT_TILDE) {
        0
    } else {
        2
    };
    in_quotes = false;
    length = 0;

    if mode.contains(ExpansionMode::TILDE) {
        mode = mode.without(ExpansionMode::TILDE);
        if byte_at(text, p) == C_TILDE {
            p = exptilde(sh, state, text, p, mode);
        }
    }

    'expansion: loop {
        startloc = expdest_off(state);
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
            if length > 0 && !mode.contains(ExpansionMode::DISCARD) {
                let newloc: c_int;
                let q: usize;

                /* `q = stnputs(p, length, expdest)`.  `p` walks the word
                 * text and never the expansion buffer, which is what the
                 * `copy_nonoverlapping` inside the old accessor already
                 * assumed and what makes this an append. */
                let b = expb(state);
                b.extend_from_slice(&text[p..p + length]);
                q = b.len();
                /* `*(q - 1) &= end - 1` */
                b[q - 1] &= (end - 1) as u8;
                /* `end` is 1 exactly when the byte just written closed the
                 * word (NUL, CTLENDVAR or CTLENDARI), and the line above
                 * has already turned it into a NUL.  Under EXP_WORD the
                 * cursor steps back over it, so it lands past the length —
                 * the outer `argstr` overwrites it on its next append. */
                b.truncate(
                    q - (if mode.contains(ExpansionMode::PARAMETER_WORD) {
                        end
                    } else {
                        0
                    }) as usize,
                );
                newloc = q as c_int - end;
                if break_all && !in_quotes && newloc > startloc {
                    recordregion(
                        state,
                        usize::try_from(startloc).expect("expansion offsets are nonnegative"),
                        usize::try_from(newloc).expect("expansion offsets are nonnegative"),
                        false,
                    );
                }
                startloc = newloc;
            }
            p += length + 1;
            length = 0;

            if end != 0 {
                return Ok(p - 1);
            }

            match c as c_char {
                C_EQUALS | C_COLON => {
                    if (c as c_char) == C_EQUALS {
                        mode = mode | ExpansionMode::COLON_TILDE;
                        reject += 1;
                        /* fall through */
                    }
                    /*
                     * sort of a hack - expand tildes in variable
                     * assignments (after the first '=' and after ':'s).
                     */
                    p -= 1;
                    if byte_at(text, p) == C_TILDE {
                        p = exptilde(sh, state, text, p, mode);
                        continue 'expansion;
                    }
                    continue;
                }
                CTLQUOTEMARK => {
                    /* "$@" syntax adherence hack */
                    /* `dolatstr + 1` is the five bytes the parser emits for
                     * a bare `"$@"`, terminator excluded. */
                    let dolat = crate::mystring::dolatstr.map(|c| c as u8);
                    if !in_quotes
                        && crate::mystring::cstr_prefix(slice_from(text, p)) == &dolat[1..6]
                    {
                        p = evalvar(sh, state, text, p + 1, mode | ExpansionMode::QUOTED)? + 1;
                        continue 'expansion;
                    }
                    in_quotes = !in_quotes;
                    /* addquote: */
                    if mode.escapes_quotes() {
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
                    if mode.escapes_quotes() || mode.contains(ExpansionMode::PRESERVE_MULTIBYTE) {
                        length = ((mb >> 8) + (mb & 0xff)) as usize;
                        if (c as c_char) == CTLESC {
                            startloc += length as c_int;
                        }
                    } else {
                        if c == CTLESC as c_int {
                            startloc += ml as c_int;
                        }
                        p += (mb & 0xff) as usize;
                        if !mode.contains(ExpansionMode::DISCARD) {
                            expb(state).extend_from_slice(&text[p..p + ml as usize]);
                        }
                        p += (mb >> 8) as usize;
                    }
                }
                CTLESC => {
                    startloc += 1;
                    length += 1;
                    if mode.escapes_quotes() {
                        p -= 1;
                        length += 1;
                        startloc += 1;
                    }
                }
                CTLVAR => {
                    p = evalvar(
                        sh,
                        state,
                        text,
                        p,
                        mode.with_if(ExpansionMode::QUOTED, in_quotes),
                    )?;
                    continue 'expansion;
                }
                CTLBACKQ => {
                    let at = state.next_backquote;
                    state.next_backquote += 1;
                    let cmd = state.backquotes.get_mut(at).and_then(Option::take);
                    expbackq(
                        sh,
                        state,
                        cmd.as_ref(),
                        mode.with_if(ExpansionMode::QUOTED, in_quotes),
                    )?;
                    continue 'expansion;
                }
                CTLARI => {
                    p = expari(
                        sh,
                        state,
                        text,
                        p,
                        mode.with_if(ExpansionMode::QUOTED, in_quotes),
                    )?;
                    continue 'expansion;
                }
                _ => {}
            }
        }
    }
}

// [spec:dash:def:expand.exptilde-fn]
// [spec:dash:sem:expand.exptilde-fn]
// [spec:posix:def:expand.tilde-prefix]
// [spec:posix:def:expand.tilde-prefix-in-assignment]
// [spec:posix:req:expand.tilde-home]
// [spec:posix:req:expand.tilde-login-name]
// [spec:posix:sem:expand.tilde-no-further-expansion]
// [spec:posix:req:expand.tilde-replacement-pathname]
// [spec:posix:req:expand.tilde-result-quoted]
fn exptilde(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    startp: usize,
    mode: ExpansionMode,
) -> usize {
    let mut c: c_char;
    let name: usize;
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
                if mode.contains(ExpansionMode::ASSIGNMENT_TILDE) {
                    break;
                }
            }
            C_SLASH | CTLENDVAR => break,
            _ => {}
        }
    }
    if !mode.contains(ExpansionMode::DISCARD) {
        /* `c = *p; *p = '\0'; ...; *p = c;` — the C terminates the user
         * name in place because `getpwnam` and `lookupvar` want a C string
         * and the only one to hand is the word itself.  The word is shared,
         * borrowed and `&[u8]` now, so the name is copied out instead: it
         * is at most a login name long, it happens once per tilde, and it
         * is the last write this cluster made to the text it is reading. */
        let namebuf: &[u8] = &text[name..p.min(text.len())];

        if namebuf.is_empty() {
            let Some(home) = crate::var::lookup_bytes(sh, BStr::new(b"HOME")) else {
                return startp;
            };
            memtodest(&sh.locale, &home, mode | ExpansionMode::QUOTED, expb(state));
        } else {
            let Ok(name) = namebuf.try_to_os_string() else {
                return startp;
            };
            let Some(home) = nsh_platform::named_user_home(&name) else {
                /* lose: */
                return startp;
            };
            let home = home.to_shell_bytes();
            memtodest(&sh.locale, &home, mode | ExpansionMode::QUOTED, expb(state));
        }
    }
    p
}

// [spec:dash:def:expand.removerecordregions-fn]
// [spec:dash:sem:expand.removerecordregions-fn]
fn removerecordregions(state: &mut ExpandState, endoff: usize) {
    /* `ifslastp == NULL` */
    if ifsr(state).is_empty() {
        return;
    }

    /* `ifsfirst` is index 0; `ifslastp` is the index the walk below
     * settles on, and dropping the tail is `truncate`. */
    if ifsr(state)[0].endoff > endoff {
        while ifsr(state).len() > 1 {
            ifsr(state).pop();
        }
        if ifsr(state)[0].begoff > endoff {
            ifsr(state).clear();
        } else {
            ifsr(state)[0].endoff = endoff;
        }
        return;
    }

    let mut last: usize = 0;
    while last + 1 < ifsr(state).len() && ifsr(state)[last + 1].begoff < endoff {
        last += 1;
    }
    while ifsr(state).len() > last + 1 {
        ifsr(state).pop();
    }
    if ifsr(state)[last].endoff > endoff {
        ifsr(state)[last].endoff = endoff;
    }
}

/*
 * Expand arithmetic expression.  Backup to start of expression,
 * evaluate, place result in (backed up) result, adjust string position.
 */

// [spec:dash:def:expand.expari-fn]
// [spec:dash:sem:expand.expari-fn]
// [spec:posix:req:expand.arith-token-expansion]
fn expari(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    start: usize,
    mode: ExpansionMode,
) -> Result<usize, Error> {
    let begoff: c_int;
    let len: c_int;
    let result: i64;
    /* The C's `p` doubles as a scratch `stackblock()` before it becomes the
     * return value; only the second use survives. */
    let p: usize;

    begoff = expdest_off(state);
    p = argstr(
        sh,
        state,
        text,
        start,
        mode.intersection(ExpansionMode::DISCARD),
    )?;

    if !mode.contains(ExpansionMode::DISCARD) {
        /* `start = stackblock() + begoff; STADJUST(start - expdest, expdest)`
         * made the C parser read the expression through a pointer beyond
         * the stack allocator's restored cursor.  The expression has value
         * semantics now: copy the counted bytes before rewinding the output
         * buffer, then lend that slice to the arithmetic parser. */
        let arithmetic = crate::mystring::cstr_prefix(&expb(state)[begoff as usize..]).to_owned();
        expb(state).truncate(begoff as usize);

        removerecordregions(
            state,
            usize::try_from(begoff).expect("expansion offsets are nonnegative"),
        );

        /* `arith` returns its diagnostic now instead of raising it, and as
         * of this commit so does `expari`, so the bridge that stood here is
         * gone and the value travels. */
        result = crate::arith_yacc::arith(sh, arithmetic.as_bstr())?;

        len = cvtnum(&sh.locale, result, mode, expb(state)) as c_int;

        if !mode.contains(ExpansionMode::QUOTED) {
            recordregion(
                state,
                usize::try_from(begoff).expect("expansion offsets are nonnegative"),
                usize::try_from(begoff + len).expect("expansion offsets are nonnegative"),
                false,
            );
        }
    }

    Ok(p)
}

/*
 * Expand stuff in backwards quotes.
 */

// [spec:dash:def:expand.expbackq-fn]
// [spec:dash:sem:expand.expbackq-fn]
// [spec:posix:req:expand.cmdsub-semantics]
// [spec:posix:req:expand.cmdsub-no-reexpansion]
fn expbackq(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    cmd: Option<&crate::nodes::Node>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let mut in_ = crate::eval::backcmd { fd: None, jp: None };
    /* `char buf[128]`, as bytes: it is only ever handed to `read` and to
     * `memtodest`, and both want the bytes rather than the sign. */
    let mut buf: [u8; 128] = [0; 128];

    if !mode.contains(ExpansionMode::DISCARD) {
        let startloc = crate::error::with_interrupts_deferred(sh, |sh| {
            let startloc = expdest_off(state);
            /* `pushstackmark(&smark, startloc)`: the length kept `makejob`'s
             * region allocations off the half-built word, and the save/restore
             * released them afterwards. The word is not in the region and
             * neither is anything `evalbackcmd` reaches, so both halves are
             * gone. */
            crate::eval::evalbackcmd(sh, cmd, &mut in_)?;

            /* `evalbackcmd` always returns a pipe with an empty read-ahead
             * area, so reading starts directly from that pipe. */
            loop {
                let Some(fd) = in_.fd.as_ref() else {
                    break;
                };
                let count = loop {
                    match nsh_platform::read_once(fd, &mut buf) {
                        Ok(count) => break count,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break 0,
                    }
                };
                /* TRACE(("expbackq: read returns %d\n", count)); */
                if count == 0 {
                    break;
                }
                memtodest(&sh.locale, &buf[..count], mode, expb(state));
            }

            if in_.fd.take().is_some() {
                sh.eval.back_exitstatus = crate::jobs::waitforjob(sh, in_.jp)?;
            }
            Ok::<_, Error>(startloc)
        })?;

        if let Some(error) = crate::error::poll_interrupt(sh) {
            return Err(error);
        }

        /* Eat all trailing newlines. The cursor is the length, so the
         * walk is over the buffer's own bytes and `STADJUST` is a
         * `truncate`. */
        nsh_platform::trim_command_substitution_output(expb(state), startloc as usize);

        if !mode.contains(ExpansionMode::QUOTED) {
            let endloc = expdest_off(state);
            recordregion(
                state,
                usize::try_from(startloc).expect("expansion offsets are nonnegative"),
                usize::try_from(endloc).expect("expansion offsets are nonnegative"),
                false,
            );
        }
        /* TRACE(("evalbackq: size=%d: \"%.*s\"\n", ...)); */
    }

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
    quotes: bool,
    zero: bool,
}

type ScanFn = fn(&nsh_platform::Locale, &[u8], &Scan) -> Option<usize>;

// [spec:dash:def:expand.scanleft-fn]
// [spec:dash:sem:expand.scanleft-fn]
fn scanleft(locale: &nsh_platform::Locale, b: &[u8], a: &Scan) -> Option<usize> {
    let mut loc: usize = a.startp;
    let mut loc2: usize = a.rmesc;
    loop {
        let s: usize = if FNMATCH_IS_ENABLED { loc2 } else { loc };
        let c: c_char = byte_at(b, s);

        /* `c = *s; if (zero) { *s = '\0'; s = startp; } pmatch(str, s);
         * *loc = c;` — the temporary terminator, as a subslice that ends
         * where it went. */
        let subject: &[u8] = if a.zero {
            let from = if FNMATCH_IS_ENABLED {
                a.rmesc
            } else {
                a.startp
            };
            between(b, from, s)
        } else {
            slice_from(b, s)
        };
        if pmatch_slices(locale, slice_from(b, a.pat), subject) != 0 {
            return Some(if a.quotes { loc } else { loc2 });
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
fn scanright(locale: &nsh_platform::Locale, b: &[u8], a: &Scan) -> Option<usize> {
    let mut esc: usize = 0;
    /* Signed, because the C's `loc--` walks off the bottom of the value on
     * purpose and `if (loc < startp) break` is how it notices.  `byte_at_i`
     * answers 0 for a negative index, so the two `*loc` reads inside the
     * multibyte rewind — which the C performs without a bounds test, on the
     * strength of the frame being well formed — cannot read before the
     * buffer here. */
    let mut loc: isize = a.endp as isize;
    let mut loc2: isize = a.rmescend as isize;
    loop {
        let s: isize = if FNMATCH_IS_ENABLED { loc2 } else { loc };

        /* `c = *s; if (zero) { *s = '\0'; s = startp; } pmatch(str, s);
         * *loc = c;` — see [`Scan`]: the subslice ends where the C's
         * temporary NUL went, so nothing is written. */
        let subject: &[u8] = if a.zero {
            let from = if FNMATCH_IS_ENABLED {
                a.rmesc
            } else {
                a.startp
            };
            between(b, from, s.max(0) as usize)
        } else {
            slice_from(b, s.max(0) as usize)
        };
        if pmatch_slices(locale, slice_from(b, a.pat), subject) != 0 {
            return Some(if a.quotes { loc } else { loc2 } as usize);
        }
        loc -= 1;
        if loc < a.startp as isize {
            break;
        }
        /* if (!esc--) esc = esclen(startp, loc); */
        let was: usize = esc;
        esc = esc.wrapping_sub(1);
        if was == 0 {
            esc = mesclen_bytes(&b[a.startp..], loc as usize - a.startp, CTLESC);
        }
        if esc % 2 != 0 {
            esc -= 1;
            loc -= 1;
        } else if byte_at_i(b, loc) == CTLMBCHAR {
            let ml: c_uint;

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
fn subevalvar(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
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
    mode: ExpansionMode,
) -> Result<usize, Error> {
    let mut subtype: c_int = varflags & VSTYPE;
    let quotes = mode.escapes_quotes();
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
    let zero: bool;
    let scan: ScanFn;
    let endp: usize;
    let pat: usize;
    let p: usize;

    p = argstr(
        sh,
        state,
        text,
        start,
        mode.intersection(ExpansionMode::DISCARD)
            | ExpansionMode::TILDE
            | if str.is_some() {
                ExpansionMode::PLAIN
            } else {
                ExpansionMode::CASE_PATTERN
            },
    )?;
    if mode.contains(ExpansionMode::DISCARD) {
        return Ok(p);
    }

    startp = startloc as usize;

    if subtype == VSASSIGN {
        let name = crate::mystring::cstr_prefix(
            &text[str.expect("VSASSIGN carries the variable's name")..],
        );
        let name = crate::var::varname(name);
        let value = crate::mystring::cstr_prefix(&expb(state)[startp..]);
        crate::var::set_bytes(sh, name, Some(value), crate::var::VariableAttributes::NONE)?;

        loc = startp;
    } else {
        if subtype == VSQUESTION {
            /* `varunset` stopped diverging with this commit, so this
             * has to be a `return` and not a bare call. It was a stop
             * before — docs/errors-are-values.md 0.2 is the bug that
             * happens when one of these is missed, and `Error` is
             * `#[must_use]` so the compiler now names it. */
            let umsg = crate::mystring::cstr_prefix(&expb(state)[startp..]);
            let var = str.expect("VSQUESTION carries the variable's name");
            return Err(varunset(sh, text, start, var, Some(umsg), varflags));
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
        rmescapes_buffer(&mut expb(state)[rmescend..], EscapeMode::Glob);
        pat = rmescend;

        rmesc = startp;
        if FNMATCH_IS_ENABLED || !quotes {
            /* `_rmescapes` with RMESCAPE_GROW appends an unescaped copy of
             * `startp` past the cursor and moves the cursor over it, so the
             * buffer can have reallocated underneath.  That is what the C's
             * three `stackblock()` re-reads on the lines after this call
             * were for, and they are gone: an offset survives a growth,
             * which is why this hands over one and gets one back. */
            rmesc = rmescapes_grow(expb(state), startp);
            if rmesc != startp {
                rmescend = expb(state).len();
            }
        }
        rmescend -= 1;

        /* zero = subtype == VSTRIMLEFT || subtype == VSTRIMLEFTMAX */
        zero = subtype >= 2;
        /* VSTRIMLEFT/VSTRIMRIGHTMAX -> scanleft */
        scan = if ((subtype & 1) != 0) ^ zero {
            scanleft
        } else {
            scanright
        };

        endp = strloc as usize - 1;
        let found = scan(
            &sh.locale,
            expb(state),
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
                if quotes {
                    rmesc = startp;
                    rmescend = endp;
                }
            }
            Some(at) if !quotes => {
                if zero {
                    rmesc = at;
                } else {
                    rmescend = at;
                }
            }
            Some(at) if zero => {
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
        expb(state).copy_within(rmesc..rmescend, startp);
        loc = startp + (rmescend - rmesc);
    }

    /* `*loc = '\0'; STADJUST(loc - expdest, expdest)` — the terminator is
     * written *at* the new cursor, so it lands one past the length rather
     * than inside it.  `push` then `pop` is how an owned buffer says
     * "write it, do not count it", and it keeps the byte the C wrote: a
     * later reallocation would drop it, because `reserve` copies only the
     * first `len` bytes, but nothing reallocates before `argstr` writes
     * the word's own terminator over it (`*(q - 1) &= end - 1` forces the
     * closing NUL, CTLENDVAR or CTLENDARI to 0).  `amount` was only ever
     * `loc - expdest`. */
    let b = expb(state);
    debug_assert!(loc <= b.len());
    b.truncate(loc);
    b.push(0);
    b.pop();

    /* Remove any recorded regions beyond start of variable */
    removerecordregions(
        state,
        usize::try_from(startloc).expect("expansion offsets are nonnegative"),
    );

    Ok(p)
}

/*
 * Expand a variable, and return a pointer to the next character in the
 * input string.
 */

// [spec:dash:def:expand.evalvar-fn]
// [spec:dash:sem:expand.evalvar-fn]
// [spec:posix:syn:expand.param-format]
// [spec:posix:req:expand.param-simple]
// [spec:posix:syn:expand.param-braces-optional]
// [spec:posix:syn:expand.param-unbraced-resolution]
// [spec:posix:req:expand.param-word-expansion]
// [spec:posix:req:expand.param-use-default]
// [spec:posix:req:expand.param-assign-default]
// [spec:posix:req:expand.param-error-if-unset]
// [spec:posix:req:expand.param-use-alternative]
// [spec:posix:req:expand.param-colon-effect]
// [spec:posix:req:expand.param-hash-requires-word]
// [spec:posix:req:expand.param-string-length]
// [spec:posix:req:expand.param-substring-common]
// [spec:posix:req:expand.param-remove-smallest-suffix]
// [spec:posix:req:expand.param-remove-largest-suffix]
// [spec:posix:req:expand.param-remove-smallest-prefix]
// [spec:posix:req:expand.param-remove-largest-prefix]
fn evalvar(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    mut p: usize,
    mut mode: ExpansionMode,
) -> Result<usize, Error> {
    let mut subtype: c_int;
    let mut varflags: c_int;
    let var: usize;
    let patloc: c_int;
    let startloc: c_int;
    let mut varlen: isize;
    let mut discard: bool;
    let quoted = mode.contains(ExpansionMode::QUOTED);
    let multibyte_mode: ExpansionMode;

    varflags = (byte_at(text, p) as c_int) & !VSBIT;
    p += 1;
    subtype = varflags & VSTYPE;

    var = p;
    startloc = expdest_off(state);
    /* The parser always writes the `=` that ends the variable name, and
     * the C dereferences `strchr`'s result without checking. */
    p += crate::mystring::cstr_prefix(slice_from(text, p))
        .find_byte(C_EQUALS as u8)
        .expect("the parser ends a variable name with `=`")
        + 1;

    multibyte_mode = match subtype {
        VSTRIMLEFT | VSTRIMLEFTMAX | VSTRIMRIGHT | VSTRIMRIGHTMAX => {
            ExpansionMode::PRESERVE_MULTIBYTE
        }
        _ => ExpansionMode::PLAIN,
    };

    enum RecordPolicy {
        IfPresent,
        Always,
    }

    let record_policy = loop {
        varlen = varvalue(
            sh,
            state,
            BStr::new(&text[var..p]),
            varflags,
            mode | multibyte_mode,
        )?;
        if (varflags & VSNUL) != 0 {
            varlen -= 1;
        }

        discard = varlen < 0;

        match subtype {
            VSPLUS | 0 | VSMINUS => {
                if subtype == VSPLUS {
                    discard = !discard;
                    /* fall through */
                }

                p = argstr(
                    sh,
                    state,
                    text,
                    p,
                    (mode | ExpansionMode::TILDE | ExpansionMode::PARAMETER_WORD)
                        .with_if(ExpansionMode::DISCARD, !discard),
                )?;
                break RecordPolicy::IfPresent;
            }

            VSASSIGN | VSQUESTION => {
                p = subevalvar(
                    sh,
                    state,
                    text,
                    p,
                    Some(var),
                    0,
                    startloc,
                    varflags,
                    mode.without(ExpansionMode::SPLIT | ExpansionMode::CASE_PATTERN)
                        .with_if(ExpansionMode::DISCARD, !discard),
                )?;

                if mode.contains(ExpansionMode::DISCARD) || !discard {
                    break RecordPolicy::IfPresent;
                }

                varflags &= !VSNUL;
                subtype = VSNORMAL;
                continue;
            }
            _ => {}
        }

        if discard
            && !mode.contains(ExpansionMode::DISCARD)
            && sh.options.enabled(ShellOption::Nounset)
        {
            /* A stop before `varunset` stopped diverging, and still one. */
            return Err(varunset(sh, text, p, var, None, 0));
        }

        if subtype == VSLENGTH {
            p += 1;
            if mode.contains(ExpansionMode::DISCARD) {
                return Ok(p);
            }
            cvtnum(
                &sh.locale,
                (if varlen > 0 { varlen } else { 0 }) as i64,
                mode,
                expb(state),
            );
            break RecordPolicy::Always;
        }

        if subtype == VSNORMAL {
            break RecordPolicy::IfPresent;
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

        mode = mode.with_if(ExpansionMode::DISCARD, discard);
        if !mode.contains(ExpansionMode::DISCARD) {
            /*
             * Terminate the string and start recording the pattern
             * right after it
             */
            /* STPUTC('\0', expdest) */
            expb(state).push(0);
        }

        patloc = expdest_off(state);
        p = subevalvar(sh, state, text, p, None, patloc, startloc, varflags, mode)?;
        break RecordPolicy::IfPresent;
    };

    if matches!(record_policy, RecordPolicy::IfPresent)
        && (mode.contains(ExpansionMode::DISCARD) || discard)
    {
        return Ok(p);
    }

    let quoted_at = if quoted {
        byte_at(text, var) == C_AT && sh.options.shellparam.nparam != 0
    } else {
        false
    };
    if quoted && !quoted_at {
        return Ok(p);
    }
    let endloc = expdest_off(state);
    recordregion(
        state,
        usize::try_from(startloc).expect("expansion offsets are nonnegative"),
        usize::try_from(endloc).expect("expansion offsets are nonnegative"),
        quoted_at,
    );
    Ok(p)
}

// [spec:dash:def:expand.chtodest-fn]
// [spec:dash:sem:expand.chtodest-fn]
/// The cursor the C returns is the destination's own length now, so this
/// appends and returns nothing. It performs no unsafe operation at all.
fn chtodest(c: c_int, syntax: DestinationSyntax, out: &mut BString) {
    if syntax.escapes(c as u8) {
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
fn mbtodest(
    locale: &nsh_platform::Locale,
    src: &[u8],
    at: usize,
    dst: &mut BString,
    syntax: DestinationSyntax,
) -> mbpair {
    let mbp: mbpair;
    /* The C's `q0`: where this call started writing. A length, because
     * the cursor is one. */
    let q0: usize = dst.len();
    let mut ml: usize;

    /* `p = p - 1` */
    let p: &[u8] = &src[at - 1..];
    ml = locale.multibyte_len(p).unwrap_or(usize::MAX);
    if ml == (0 as usize).wrapping_sub(2) || ml == (0 as usize).wrapping_sub(1) || ml < 2 {
        chtodest(p[0] as c_char as c_int, syntax, dst);
        ml = 1;
    } else {
        /* `syntax[CTLMBCHAR]` — CTLMBCHAR is negative; see the note in
         * `memtodest` about the unbiased `is_type` table. Negative is an
         * ordinary index now, and a checked one. */
        if syntax.escapes(CTLMBCHAR as u8) {
            /* USTPUTC(CTLMBCHAR, q); USTPUTC(ml, q); */
            dst.push(CTLMBCHAR as u8);
            dst.push(ml as u8);
        }

        /* `q = mempcpy(q, p, ml)`. The source is the caller's input and
         * never `dst`'s own buffer -- `memtodest` records why -- so the
         * append cannot alias what it reads.  `ml` came from `mbrlen`
         * over this same slice, so it cannot exceed it. */
        dst.extend_from_slice(&p[..ml]);

        if syntax.escapes(CTLMBCHAR as u8) {
            /* USTPUTC(ml, q); USTPUTC(CTLMBCHAR, q); */
            dst.push(ml as u8);
            dst.push(CTLMBCHAR as u8);
        }
    }

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
fn memtodest(
    locale: &nsh_platform::Locale,
    src: &[u8],
    mode: ExpansionMode,
    dst: &mut BString,
) -> usize {
    let syntax: DestinationSyntax;
    let mut count: usize = 0;
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

    let framed = mode.escapes_quotes() || mode.contains(ExpansionMode::PRESERVE_MULTIBYTE);
    if !mode.contains(ExpansionMode::QUOTED) || !framed {
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
        syntax = if framed {
            DestinationSyntax::Base
        } else {
            DestinationSyntax::Unframed
        };
    } else {
        syntax = DestinationSyntax::SingleQuoted;
    }

    /* for (; len; len--) */
    while i < src.len() {
        let c: c_int = src[i] as c_char as c_int;
        i += 1;

        if c == 0 && !mode.contains(ExpansionMode::KEEP_NUL) {
            continue;
        }

        count += 1;

        if c < 0 {
            /* `mbtodest(p, ...)` is called with `p` already past the
             * byte it is about to decode, and starts by stepping
             * back over it; `i` is that same position. */
            let mbp: mbpair = mbtodest(locale, src, i, dst, syntax);
            let mlm: c_uint;

            /* `q += mbp.ql` — the append did it. */
            mlm = mbp.ml;
            i += mlm as usize;
            continue;
        }

        chtodest(c, syntax, dst);
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
// The C string entry became a counted byte slice. Every caller now already
// knows the value's bounds, so the old `strlen` scan and its raw pointer are
// both redundant.
fn strtodest(
    locale: &nsh_platform::Locale,
    value: &[u8],
    mode: ExpansionMode,
    dst: &mut BString,
) -> usize {
    memtodest(locale, value, mode, dst)
}

/*
 * Add the value of a specialized variable to the stack string.
 */

// [spec:dash:def:expand.varvalue-fn]
// [spec:dash:sem:expand.varvalue-fn]
// [spec:posix:def:param.positional-definition]
// [spec:posix:req:param.positional-decimal-digits]
// [spec:posix:syn:param.positional-multi-digit-braces]
// [spec:posix:sem:param.positional-zero-not-positional]
// [spec:posix:def:param.special-parameters]
// [spec:posix:req:param.special-at]
// [spec:posix:req:param.special-at-double-quotes]
// [spec:posix:req:param.special-at-no-positional]
// [spec:posix:req:param.special-asterisk]
// [spec:posix:req:param.special-hash]
// [spec:posix:req:param.special-question]
// [spec:posix:sem:param.special-question-assignment]
// [spec:posix:req:param.special-hyphen]
// [spec:posix:req:param.special-dollar]
// [spec:posix:req:param.special-bang]
// [spec:posix:req:param.special-zero]
// [spec:posix:def:exit.expansion-error]
fn varvalue(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    name: &BStr,
    varflags: c_int,
    mut mode: ExpansionMode,
) -> Result<isize, Error> {
    let subtype: c_int = varflags & VSTYPE;
    let mut seplen: usize;
    /* The C's `const char *seps` plus its length.  The comment that stood
     * at the assignment below owed a conversion — it said the pointer was
     * safe *because* of where the storage comes from, which is an argument
     * a slice does not have to make.  Both sources are bytes the shell
     * owns for the whole call, so both are slices. */
    let mut seps: &[u8];
    let mut len: isize = 0;
    let start: usize;
    let discard: bool;
    let name = crate::var::varname(name);
    let name_byte = name.first().copied().unwrap_or_default() as c_char;

    discard = subtype == VSPLUS || subtype == VSLENGTH || mode.contains(ExpansionMode::DISCARD);

    if subtype == 0 {
        if discard {
            return Ok(-1);
        }

        return Err(sh.sh_error_value(b"Bad substitution"));
    }

    if discard {
        mode = mode.without(ExpansionMode::SPLIT | ExpansionMode::CASE_PATTERN);
    }
    /* `seps = nullstr` — the empty C string, whose one byte is the
     * terminator, and the terminator is what gets written when the
     * separator is a NUL. */
    seps = &[0u8];
    seplen = usize::from(mode.contains(ExpansionMode::SPLIT));
    start = expdest_off(state) as usize;

    match name_byte {
        C_DOLLAR | C_QUESTION | C_HASH | C_BANG => {
            let num = match name_byte {
                C_DOLLAR => i64::from(sh.root_pid.get()),
                C_QUESTION => i64::from(sh.status.code()),
                C_HASH => i64::from(sh.options.shellparam.nparam),
                C_BANG => {
                    let Some(pid) = sh.backgndpid else {
                        return Ok(-1);
                    };
                    i64::from(pid.get())
                }
                _ => unreachable!(),
            };
            len = cvtnum(&sh.locale, num, mode, expb(state)) as isize;
        }
        C_MINUS => {
            for spec in OPTION_SPECS.iter().rev() {
                if sh.options.enabled(spec.option)
                    && let Some(letter) = spec.letter
                {
                    expb(state).push(letter);
                    len += 1;
                }
            }
        }
        C_AT | C_STAR => {
            if name_byte != C_AT
                || !(mode.contains(ExpansionMode::QUOTED) && mode.contains(ExpansionMode::SPLIT))
            {
                if mode.contains(ExpansionMode::QUOTED) {
                    seplen = 0;
                }
                if seplen == 0 {
                    seps = sh.ifs.ncifs.as_slice();
                }
                seplen =
                    (seplen.wrapping_sub(1) & sh.ifs.ifsmb0len.wrapping_sub(1)).wrapping_add(1);
            }

            for (index, param) in sh.options.shellparam.words().iter().enumerate() {
                if index != 0 {
                    debug_assert!(
                        seplen <= seps.len(),
                        "varvalue: separator length {seplen} exceeds the {} bytes it names",
                        seps.len()
                    );
                    len += memtodest(
                        &sh.locale,
                        &seps[..seplen],
                        mode | ExpansionMode::KEEP_NUL,
                        expb(state),
                    ) as isize;
                }

                len += strtodest(
                    &sh.locale,
                    crate::mystring::cstr_prefix(param).as_bytes(),
                    mode,
                    expb(state),
                ) as isize;
            }
        }
        c if (C_0..=C_9).contains(&c) => {
            let position = crate::mystring::decimal_digits(name)
                .unwrap_or(0)
                .min(c_int::MAX as u64) as c_int;
            if position > sh.options.shellparam.nparam {
                return Ok(-1);
            }
            let value = if position != 0 {
                sh.options
                    .shellparam
                    .words()
                    .get(position as usize - 1)
                    .cloned()
            } else {
                sh.options.arg0().map(BStr::to_owned)
            };
            let Some(value) = value else {
                return Ok(-1);
            };
            len = strtodest(
                &sh.locale,
                crate::mystring::cstr_prefix(&value).as_bytes(),
                mode,
                expb(state),
            ) as isize;
        }
        _ => {
            let Some(value) = crate::var::lookup_bytes(sh, name) else {
                return Ok(-1);
            };
            len = strtodest(
                &sh.locale,
                crate::mystring::cstr_prefix(&value).as_bytes(),
                mode,
                expb(state),
            ) as isize;
        }
    }

    if discard {
        expb(state).truncate(start);
    }

    Ok(len)
}

/*
 * Record the fact that we have to scan this region of the
 * string for IFS characters.
 */

// [spec:dash:def:expand.recordregion-fn]
// [spec:dash:sem:expand.recordregion-fn]
pub(crate) fn recordregion(state: &mut ExpandState, start: usize, end: usize, nulonly: bool) {
    let r = ifsregion {
        begoff: start,
        endoff: end,
        nulonly,
    };

    ifsr(state).push(r);
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct IfsMembership {
    separator: bool,
    default_whitespace: bool,
}

// [spec:dash:def:expand.ifsisifs-fn]
// [spec:dash:sem:expand.ifsisifs-fn]
fn ifsisifs(sh: &Shell, s: &[u8], multibyte_len: usize, nulonly: bool) -> IfsMembership {
    let mut isdefifs: bool = false;
    let mut isifs: bool = false;
    let mut wc: i32 = byte_at(s, 0) as i32;
    /* C leaves `ifs0` uninitialised; it is only read when `isifs`, which
     * implies one of the branches below assigned it. */
    let mut ifs0: i32 = 0;

    /* The C's `ifst->ifs`: `nullstr` when the region is NUL-only, the
     * shell's `IFS` otherwise. Both are NUL-terminated and the terminator
     * is *in* the searched set below, so the empty case is `[0]` rather
     * than `[]` — a NUL byte in a NUL-only region is a separator, and
     * that is the whole of what a NUL-only region means. */
    const NULONLY: &[u8] = &[0];
    let ifs: &[u8] = if nulonly {
        NULONLY
    } else {
        sh.ifs.ncifs.as_slice()
    };

    if ifs[0] != 0 && !sh.ifs.wcifs.is_empty() {
        if (wc & 0x80) != 0 {
            /* `ml` came from `mbnext` over this same slice, so the
             * clamp can only bite where the C read past the word's
             * end -- and a short read fails the `!= ml` test exactly
             * as a malformed character does.  The same trade
             * `ccmatch_bytes` records. */
            let n = multibyte_len.min(s.len());
            let Some(wc2) = sh.locale.decode_exact(&s[..n], multibyte_len) else {
                return IfsMembership::default();
            };
            wc = wc2;
        }

        isifs = wcifs_chr(&sh.ifs.wcifs, wc);
        ifs0 = sh.ifs.wcifs[0];
    } else if multibyte_len == 0 {
        /* `strchr` matches the terminator, so a NUL character --
         * which is what `ml == 0` means -- counts as an IFS byte.
         * The counted terminator on `ncifs` keeps that, and it is why
         * the slice is searched whole rather than trimmed. */
        isifs = ifs.contains(&(wc as u8));
        ifs0 = ifs[0] as i32;
    }

    if isifs {
        isdefifs = sh.locale.wide_is_space(if wc != 0 { wc } else { ifs0 });
    }
    IfsMembership {
        separator: isifs,
        default_whitespace: isdefifs,
    }
}

// [spec:dash:def:expand.ifsbreakup-slow-fn]
// [spec:dash:sem:expand.ifsbreakup-slow-fn]
fn ifsbreakup_slow(
    sh: &Shell,
    ifst: &mut ifs_state,
    fields: &mut Vec<strlist>,
    nulonly: bool,
    string: &mut [u8],
    mut p: usize,
) -> usize {
    let ifschar: c_uint;
    let isdefifs: bool;
    let multibyte_len: usize;
    let isifs: bool;
    let mut q: usize;

    q = p;

    ifschar = mbnext_bytes(slice_from(string, p));
    p += (ifschar & 0xff) as usize;
    multibyte_len = if (ifschar >> 8) > 3 {
        ((ifschar >> 8) - 2) as usize
    } else {
        0
    };

    let membership = ifsisifs(sh, slice_from(string, p), multibyte_len, ifst.nulonly);
    p += (ifschar >> 8) as usize;

    isifs = membership.separator;
    isdefifs = membership.default_whitespace;

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
    if matches!(ifst.max_fields, FieldLimit::Remaining(0)) {
        if isdefifs {
            if ifst.r.is_none() {
                ifst.r = Some(q);
            }
            return p;
        }

        if !(isifs && ifst.ifsspc) {
            ifst.r = None;
        }
    } else if ifst.ifsspc {
        if isifs {
            q = p;
        }

        ifst.start = q;

        if isdefifs {
            return p;
        }
    } else if isifs {
        let mut ifsspc = ifst.ifsspc;

        if !nulonly {
            ifsspc = isdefifs;
            ifst.ifsspc = ifsspc;
        }

        /* Ignore IFS whitespace at start. */
        if q == ifst.start && ifsspc {
            ifst.start = p;
        } else {
            let last_field = match &mut ifst.max_fields {
                FieldLimit::Unlimited => false,
                FieldLimit::Remaining(remaining) => {
                    *remaining = remaining.saturating_sub(1);
                    *remaining == 0
                }
            };
            if last_field {
                ifst.r = Some(q);
                return p;
            }
            string[q] = C_NUL as u8;
            fields.push(strlist::from_cbytes(&string[ifst.start..]));
            ifst.start = p;
            return p;
        }
    }

    ifst.ifsspc = false;
    p
}

/*
 * Break the argument string into pieces based upon IFS and add the
 * strings to the argument list.  The regions of the string to be
 * searched for IFS characters have been stored by recordregion.
 * A finite field limit joins the remainder into its last field; an unlimited
 * expansion emits every field.
 */

// [spec:dash:def:expand.ifsbreakup-fn]
// [spec:dash:sem:expand.ifsbreakup-fn]
fn ifsbreakup_regions(
    sh: &Shell,
    regions: &[ifsregion],
    string: &mut [u8],
    max_fields: FieldLimit,
    fields: &mut Vec<strlist>,
) {
    let mut ifsp: usize;
    /* `struct ifs_state ifst;` and the three assignments the C makes
     * before the loop, as one initialiser. `mem::zeroed` was standing in
     * for the C leaving `ifs` and `ifsspc` unset here, and both are
     * assigned on every path that reads them; a struct without a pointer
     * in it can say so directly. */
    let mut ifst: ifs_state = ifs_state {
        nulonly: false,
        start: 0,
        r: None,
        max_fields,
        ifsspc: false,
    };
    let mut nulonly: bool;
    let mut p: usize;
    let mut preserve_nul_field = false;

    if !regions.is_empty() {
        ifst.ifsspc = false;
        nulonly = false;
        /* `realifs = ifsset() ? ncifs : nullstr` is gone with the
         * pointer it cached: `ifsisifs` reads `IFS` off the shell,
         * and what it needs from here is the one bit below. */
        ifsp = 0;
        loop {
            let afternul: bool;
            let endoff = regions[ifsp].endoff;

            p = regions[ifsp].begoff;
            debug_assert!(
                endoff <= string.len(),
                "a recorded region ends past the word it was recorded in"
            );
            afternul = nulonly;
            nulonly = regions[ifsp].nulonly;
            ifst.nulonly = nulonly;
            ifst.ifsspc = false;
            loop {
                let p0: usize = p;

                /* `stackblock() + endoff - p >= 8` — eight bytes of
                 * this region left to look at.  As offsets it is also
                 * the bound that makes the load below a checked one. */
                while endoff >= p + 8 {
                    /* union { uint64_t qw; unsigned char b[8]; } x; */
                    let b: [u8; 8] = string[p..p + 8].try_into().unwrap();
                    let qw: u64 = u64::from_ne_bytes(b);

                    if (qw & 0x8080808080808080) != 0 {
                        break;
                    }
                    if b.iter().any(|byte| sh.ifs.ifsmap[*byte as usize]) {
                        break;
                    }
                    p += 8;
                }

                if p != p0 {
                    if matches!(ifst.max_fields, FieldLimit::Remaining(0)) {
                        ifst.r = None;
                    } else if ifst.ifsspc {
                        ifst.start = p0;
                    }
                    ifst.ifsspc = false;
                }

                if p >= endoff {
                    break;
                }

                p = ifsbreakup_slow(sh, &mut ifst, fields, afternul || nulonly, string, p);
            }

            ifsp += 1;
            if ifsp >= regions.len() {
                break;
            }
        }
        if nulonly {
            preserve_nul_field = true;
        } else if let Some(r) = ifst.r {
            /* This is the one write into `string` that happens after
             * `ifsbreakup_slow` has stopped emitting fields, and the
             * fields no longer alias `string` — they copied out at the
             * instant each was terminated.  So it has to land in the
             * field that has *not* been created yet, which is the one
             * `add:` below takes from `ifst.start`.  It does: `r` is
             * only ever set once the field limit is exhausted, and the two
             * branches that set it both return without emitting, so no
             * field is taken between the two points. */
            debug_assert!(
                r >= ifst.start,
                "the trailing-IFS truncation lands in an already-taken field"
            );
            string[r] = C_NUL as u8;
        }
    }

    if !preserve_nul_field && byte_at(string, ifst.start) == C_NUL {
        return;
    }

    fields.push(strlist::from_cbytes(&string[ifst.start..]));
}

// [spec:posix:req:expand.field-splitting-applies]
// [spec:posix:def:expand.field-splitting-results-of-expansion]
// [spec:posix:req:expand.field-splitting-empty-ifs]
// [spec:posix:req:expand.field-splitting-order]
// [spec:posix:req:expand.field-splitting-unexpanded-fields]
// [spec:posix:def:expand.ifs-white-space]
// [spec:posix:req:expand.ifs-unset-default]
// [spec:posix:req:expand.ifs-delimiters]
// [spec:posix:sem:expand.field-splitting-arbitrary-bytes]
// [spec:posix:req:expand.field-splitting-zero-fields]
// [spec:posix:def:expand.field-splitting-delimited]
// [spec:posix:req:expand.field-splitting-algorithm]
// [spec:posix:req:expand.field-splitting-output-replaces-input]
pub fn ifsbreakup(sh: &Shell, string: &mut [u8], max_fields: usize, arglist: &mut arglist) {
    ifsbreakup_regions(
        sh,
        &sh.expand.ifs_regions,
        string,
        FieldLimit::Remaining(max_fields),
        &mut arglist.list,
    );
}

// [spec:dash:def:expand.ifsfree-fn]
// [spec:dash:sem:expand.ifsfree-fn]
pub(crate) fn ifsfree(state: &mut ExpandState) {
    /* Emptying the owned region list replaces freeing the C chain and
     * nulling its tail pointer. */
    if ifsr(state).len() > 1 {
        ifsr(state).truncate(1);
    }
    ifsr(state).clear();
}

// [spec:dash:def:expand.changeifs-fn]
// [spec:dash:sem:expand.changeifs-fn]
pub fn changeifs_bytes(sh: &mut crate::context::Shell, ifs: &BStr) {
    let mut mb: c_uint = 0;
    sh.ifs.ncifs = ifs.to_owned();
    sh.ifs.ncifs.push(0);

    sh.ifs.ifsmap = [false; 128];

    /* The C walks to the terminator and processes it *before* breaking,
     * so `ifsmap[0]` is set on every call — the loop below keeps that by
     * iterating over the counted terminator rather than stopping short of
     * it. `len` is the length without it, which is what the C's counter
     * held. */
    let len = sh.ifs.ncifs.len() - 1;
    for i in 0..sh.ifs.ncifs.len() {
        let c: c_uint = sh.ifs.ncifs[i] as c_uint;

        mb |= c >> 7;
        if (c >> 7) == 0 {
            sh.ifs.ifsmap[c as usize] = true;
        }
    }

    sh.ifs.ifsmb0len = (len != 0) as usize;
    sh.ifs.wcifs = if mb == 0 {
        Vec::new()
    } else {
        let (first_len, wide) = sh.locale.wide_chars(&sh.ifs.ncifs[..len]);
        sh.ifs.ifsmb0len = first_len;
        wide
    };
}

/*
 * Expand shell metacharacters.  At this point, the only control characters
 * should be escapes.  The results are stored in the list exparg.
 */

/* The libc `glob64` arm and its callback table were compile-time dead:
 * `GLOB_IS_ENABLED` is zero in every supported build. The shell's own
 * byte-preserving glob implementation below is the only reachable arm.
 * [spec:dash:def:expand.opendir-interruptible-fn]
 * [spec:dash:sem:expand.opendir-interruptible-fn]
 * [spec:dash:def:expand.expandmeta-glob-fn]
 * [spec:dash:sem:expand.expandmeta-glob-fn]
 * [spec:dash:def:expand.addglob-fn]
 * [spec:dash:sem:expand.addglob-fn] */

// [spec:dash:def:expand.expandmeta-fn]
// [spec:dash:sem:expand.expandmeta-fn]
// [spec:posix:req:expand.pathname]
// [spec:posix:req:pattern.no-match-unchanged]
// [spec:posix:req:pattern.no-special-chars-unchanged]
fn expandmeta(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    words: Vec<strlist>,
) -> Result<(), Error> {
    /* TODO - EXP_REDIR */

    /* The C's `preglob(..., RMESCAPE_HEAP)` result: one `ckmalloc` per
     * word, `ckfree`d as soon as `expmeta` has read it.  That is a local
     * buffer's lifetime exactly, and reusing it across the loop is the
     * only difference — `expmeta` never re-enters `preglob`, because the
     * only `preglob` under it is `patmatch`'s, which does not allocate
     * while `FNMATCH_IS_ENABLED` is 0. */
    let mut pattern: BString = BString::new(Vec::new());

    /* The glob buffer, owned here and lent to `expmeta`.  One allocation
     * per `expandmeta` that globs anything, reused across the word loop
     * exactly as the region's block was; see the comment above
     * [`expmeta`]'s neighbours for why it stopped being a `static`. */
    let mut globbuf: BString = BString::new(Vec::new());

    for mut str in words {
        let text = crate::mystring::cstr_prefix(&str.text);
        let has_meta = !sh.options.enabled(ShellOption::NoGlob)
            && text.find_byteset(b"*?]").is_some()
            && text != b"]";
        if has_meta {
            /* `savelastp = exparg.lastp` — where this word's matches
             * will start, so that the sort below covers them and not
             * the words already in the list. */
            let savelastp = expargl(state).len();

            crate::error::with_interrupts_deferred(sh, |sh| {
                pattern.clear();
                pattern.extend_from_slice(text);
                pattern.push(0);
                let pattern_len = rmescapes_buffer(&mut pattern, EscapeMode::Glob);
                pattern.truncate(pattern_len + 1);

                /* The C's top-level `expmeta` starts on whatever block the
                 * region is on and gets away with it because `expdir_len`
                 * is 0: it writes from the base and never reads what was
                 * there. An owned buffer's length is not 0 — the previous
                 * glob's `addfnamealt` left it at that glob's `expdir_len`
                 * — and every consequence of carrying it in is benign,
                 * which is the reason to clear rather than to argue. */
                globbuf.clear();
                expmeta(
                    &sh.locale,
                    state,
                    &mut globbuf,
                    crate::mystring::cstr_prefix(&pattern),
                    0,
                );
            });
            if expargl(state).len() != savelastp {
                /* `*exparg.lastp = NULL; sp = expsort(*savelastp);
                 * *savelastp = sp; while (sp->next) sp = sp->next;
                 * exparg.lastp = &sp->next;` — terminate the run this
                 * word added, sort it, splice it back and walk to its
                 * new end.  Three of those four exist to re-find the
                 * tail of a list the sort reordered; a slice's tail
                 * does not move. */
                expsort(&sh.locale, &mut expargl(state)[savelastp..]);
                continue;
            }
        }
        str.rmescapes();
        expargl(state).push(str);
    }
    Ok(())
}

// [spec:dash:def:expand.addfname-common-fn]
// [spec:dash:sem:expand.addfname-common-fn]
fn addfname_common(state: &mut ExpandState, name: BString) {
    expargl(state).push(strlist { text: name });
}

// [spec:dash:def:expand.addfnamealt-fn]
// [spec:dash:sem:expand.addfnamealt-fn]
fn addfnamealt(state: &mut ExpandState, b: &mut BString, expdir_len: usize) {
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
    addfname_common(state, BString::from(b.to_vec()));

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
fn expmeta_rmescapes(b: &mut BString, name: &[u8]) {
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
        let n = rmescapes_buffer(&mut b[at..], EscapeMode::Plain);
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

/*
 * Do metacharacter (i.e. *, ?, [...]) expansion.
 */

// [spec:dash:def:expand.expmeta-fn]
// [spec:dash:sem:expand.expmeta-fn]
// [spec:posix:def:pattern.filename-expansion-qualification]
// [spec:posix:req:pattern.slash-explicit-match]
// [spec:posix:syn:pattern.slash-terminates-bracket]
// [spec:posix:req:pattern.leading-period]
// [spec:posix:req:pattern.leading-period-in-bracket-unspecified]
// [spec:posix:req:pattern.filename-expansion-trigger]
// [spec:posix:req:pattern.directory-permissions]
// [spec:posix:req:pattern.permission-errors-not-fatal]
// [spec:posix:req:pattern.unmatched-open-bracket-unspecified]
fn expmeta(
    locale: &nsh_platform::Locale,
    state: &mut ExpandState,
    b: &mut BString,
    name: &[u8],
    mut expdir_len: usize,
) {
    let mesc: c_char = if FNMATCH_IS_ENABLED {
        C_BACKSLASH
    } else {
        CTLESC
    };
    let mut endname: usize;
    let mut zeroedp: usize;
    let mut matchdot: bool;
    let mut esc: usize;
    let start: usize;
    let pat: &[u8];
    let mut p: usize;
    let c: c_char;
    /* Scratch for the encoded form of each directory entry; see the
     * `memtodest` call below.  A local rather than a static because
     * `expmeta` recurses, one frame per path component. */
    let mut globenc: BString = BString::new(Vec::new());

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
            debug_assert_eq!(b.len(), expdir_len);
            return;
        }
        expmeta_rmescapes(b, name);
        /* The C's `enddir` is on the NUL `expmeta_rmescapes` wrote
         * and `addfnamealt` is handed `enddir + 1`, so the
         * terminator is part of the candidate.  Appending it here
         * says that, and `lstat` needs it anyway. */
        b.push(0);
        let exists = b[..b.len() - 1]
            .try_to_path_buf()
            .is_ok_and(|path| nsh_platform::path_metadata(&path, false).is_ok());
        if exists {
            addfnamealt(state, b, expdir_len);
        } else {
            /* The C leaves its uncounted bytes where they are and
             * returns the base; counted bytes have to be rewound,
             * so that this frame returns with the buffer holding
             * its prefix and nothing else. */
            b.truncate(expdir_len);
        }
        debug_assert_eq!(b.len(), expdir_len);
        return;
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

    let directory = if expdir_len != 0 {
        &b[..expdir_len]
    } else {
        b"."
    };
    let Ok(directory) = directory.try_to_path_buf() else {
        debug_assert_eq!(b.len(), expdir_len);
        return;
    };
    let Ok(entries) = nsh_platform::read_directory(&directory) else {
        debug_assert_eq!(b.len(), expdir_len);
        return;
    };
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
    /* `read_dir` intentionally omits `.` and `..`; `readdir`
     * included both, so put them back before the native entries. */
    let synthetic = [(b".".to_vec(), true), (b"..".to_vec(), true)];
    let entries = synthetic.into_iter().chain(
        entries
            .into_iter()
            .map(|entry| (entry.name.to_shell_bytes(), entry.may_descend)),
    );
    for (mut dname, may_descend) in entries {
        dname.push(0);

        let eligible = (dname[0] != C_DOT as u8 || matchdot) && (c == 0 || may_descend);
        if eligible {
            /* `len = strlen(dname) + 1` — the terminator is part
             * of what gets appended, because the candidate is a C
             * string and the next component overwrites it. */
            let dname: &[u8] = &dname;
            let len: usize = dname.len();
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
                memtodest(
                    locale,
                    dname,
                    ExpansionMode::PRESERVE_MULTIBYTE | ExpansionMode::KEEP_NUL,
                    &mut globenc,
                );
                debug_assert_eq!(
                    globenc.last(),
                    Some(&0),
                    "EXP_KEEPNUL carries the entry's terminator through"
                );
                &globenc
            } else {
                dname
            };
            if crate::pmatch::pmatch_slices(locale, pat, subject) != 0 {
                /* `enddir = stnputs(dname, len, enddir)` — an
                 * append at a cursor below the end, which is
                 * truncate-then-append. */
                b.truncate(expdir_len);
                b.extend_from_slice(dname);
                if c == 0 {
                    addfnamealt(state, b, expdir_len);
                } else {
                    /* `*enddir.offset(-1) = C_SLASH` — the entry's
                     * terminator becomes the separator. */
                    let last = b.len() - 1;
                    b[last] = C_SLASH as u8;
                    expmeta(locale, state, b, &name[endname..], expdir_len + len);
                    /* `enddir = cp + expdir_len` — the frame's
                     * rewind, said out loud.  The child returns
                     * with the buffer holding *its* prefix, which
                     * is this one plus the component just
                     * appended. */
                    b.truncate(expdir_len);
                }
            }
        }
        if int_pending() != 0 {
            break;
        }
    }

    /* The C returns `cp`, the block's base, and every caller immediately
     * recomputes `cp + expdir_len`.  What that is really saying is a
     * postcondition, and it is this: on return the buffer holds this
     * frame's prefix and nothing above it.  `expdir_len` is the frame's
     * own, which may have grown past the caller's — hence the caller's
     * rewind after the recursive call. */
    debug_assert_eq!(b.len(), expdir_len);
}

/*
 * Sort the results of file name expansion.  It calculates the number of
 * strings to sort and then calls msort (short for merge sort) to do the
 * work.
 */

// [spec:dash:def:expand.expsort-fn]
// [spec:dash:sem:expand.expsort-fn]
// [spec:posix:req:pattern.replacement-sorted]
fn expsort(locale: &nsh_platform::Locale, str: &mut [strlist]) {
    /* The C walks the chain to count it and hands the count to `msort`,
     * because a singly-linked list does not know its own length. */
    msort(locale, str, str.len() as c_int)
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
fn msort(locale: &nsh_platform::Locale, list: &mut [strlist], len: c_int) {
    if len <= 1 {
        return;
    }
    list.sort_by(|p, q| locale.collate(&p.text, &q.text));
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
// [spec:posix:syn:pattern.backslash-escape-with-shell-quoting]
// [spec:posix:syn:pattern.backslash-escape-without-shell-quoting]
// [spec:posix:req:pattern.escaping-follows-quoting-rules]
// [spec:posix:syn:pattern.trailing-backslash-unspecified]
// [spec:posix:req:pattern.quote-to-match-literally]
fn rmescapes_compact(buf: &mut [u8], at: usize, mode: EscapeMode) -> usize {
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

    let globbing = mode == EscapeMode::Glob;
    let mut in_quotes = false;
    let mut not_escaped = globbing;
    /* The C's `p` and `q`, which are indices into one buffer here. */
    let mut p: usize = at;
    let mut q: usize = at;

    while byte_at(buf, p) != C_NUL {
        let mut c: c_int = byte_at(buf, p) as c_int;
        let mut newly_not_escaped = globbing;
        let mb: c_uint;
        let mut ml: c_uint;

        let copy_byte = if c == CTLQUOTEMARK as c_int {
            p += 1;
            in_quotes ^= globbing;
            continue;
        } else if c == C_BACKSLASH as c_int {
            /* naked back slash */
            newly_not_escaped ^= not_escaped;
            /* naked backslashes can only occur outside quotes */
            in_quotes = false;
            if !FNMATCH_IS_ENABLED && not_escaped {
                c = CTLESC as c_int;
            }
            true
        } else if c == CTLESC as c_int {
            if !not_escaped && in_quotes {
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
            if globbing {
                buf[q] = if FNMATCH_IS_ENABLED {
                    C_BACKSLASH
                } else {
                    CTLESC
                } as u8;
                q += 1;
            }

            p += 1;
            c = byte_at(buf, p) as c_int;
            true
        } else if c == CTLMBCHAR as c_int {
            let mut tail: c_uint = 2;

            if !FNMATCH_IS_ENABLED && (globbing ^ not_escaped) {
                q -= 1;
            }

            mb = mbnext_bytes(slice_from(buf, p));
            ml = mb >> 8;

            if !globbing || FNMATCH_IS_ENABLED {
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
            false
        } else {
            true
        };

        if copy_byte {
            buf[q] = c as u8;
            q += 1;
            p += 1;
        }
        not_escaped = newly_not_escaped;
    }
    if !FNMATCH_IS_ENABLED && (globbing ^ not_escaped) {
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

/// Apply `_rmescapes` to one owned, NUL-terminated byte buffer and return
/// the resulting length without the terminator.
fn rmescapes_buffer(bytes: &mut [u8], mode: EscapeMode) -> usize {
    let len = bytes
        .iter()
        .position(|&byte| byte == 0)
        .unwrap_or(bytes.len());
    let Some(at) = rmescapes_scan(&bytes[..len]) else {
        return len;
    };
    if len == bytes.len() {
        return len;
    }
    rmescapes_compact(&mut bytes[..=len], at, mode)
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
pub fn rmescapes_grow(b: &mut BString, at: usize) -> usize {
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
    let m = rmescapes_compact(&mut b[r..], at_rel, EscapeMode::Plain);
    b.truncate(r + m + 1);
    r
}

/*
 * See if a pattern matches in a case statement.
 */

// [spec:dash:def:expand.casematch-fn]
// [spec:dash:sem:expand.casematch-fn]
pub fn casematch(
    sh: &mut crate::context::Shell,
    pattern: &crate::nodes::Node,
    val: &BStr,
) -> Result<c_int, Error> {
    let Node::Word(word) = pattern else {
        return Err(sh.sh_error_value(b"case matching requires a word node"));
    };
    typed::case_matches(sh, &word.word, val).map(c_int::from)
}

fn casematch_inner(
    sh: &mut crate::context::Shell,
    state: &mut ExpandState,
    pattern: &[u8],
    val: &BStr,
) -> Result<c_int, Error> {
    let result: c_int;

    /* `setstackmark(&smark)` — it released what `argstr` allocated from the
     * region for backquotes and arithmetic.  Neither allocates from it. */
    /* STARTSTACKSTR(expdest) */
    expb(state).clear();
    /* As in `expandarg`: this `?` returns past the `ifsfree()`, which is
     * where the longjmp went too, and the catch frame reclaims the
     * regions. */
    argstr(
        sh,
        state,
        pattern,
        0,
        ExpansionMode::TILDE | ExpansionMode::CASE_PATTERN,
    )?;
    ifsfree(state);
    /* The C reads the word back as `stackblock()`. */
    rmescapes_buffer(expb(state), EscapeMode::Glob);
    result =
        crate::pmatch::pmatch_slices(&sh.locale, crate::mystring::cstr_prefix(expb(state)), val);
    Ok(result)
}

/*
 * Our own itoa().
 */

// [spec:dash:def:expand.cvtnum-fn]
// [spec:dash:sem:expand.cvtnum-fn]
fn cvtnum(
    locale: &nsh_platform::Locale,
    num: i64,
    mode: ExpansionMode,
    dst: &mut BString,
) -> usize {
    let value = format!("{num}");
    memtodest(locale, value.as_bytes(), mode, dst)
}

// [spec:dash:def:expand.varunset-fn]
// [spec:dash:sem:expand.varunset-fn]
fn varunset(
    sh: &mut crate::context::Shell,
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
    if sh.eval.inps4 != 0 {
        sh.sh_error_value(&message)
    } else {
        // [spec:nsh:req:compat.smoosh.error-contracts]
        sh.expansion_error_value(&message)
    }
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
pub fn restore_handler_expandarg(
    sh: &mut crate::context::Shell,
    caught: Option<crate::error::Error>,
) -> Option<crate::error::Error> {
    match &caught {
        /* Not this frame's to keep, and never was: the C re-raised it
         * from here. */
        Some(e) if e.is_interrupt() => {}
        Some(_) => ifsfree(&mut sh.expand),
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

/// `i64 arith(const char *)` — prototype only; the definition lives
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
pub fn expcmd(_argc: c_int, _argv: *mut *mut c_char) -> c_int {
    /* No definition exists in the C tree; calling this is a bug. */
    unreachable!("expcmd: declared in expand.h, never defined")
}
