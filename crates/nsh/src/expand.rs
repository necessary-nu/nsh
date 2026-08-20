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

use crate::error::Error;
use crate::nodes::Node;
use crate::options::{OPTION_SPECS, ShellOption};
// [spec:nsh:def:idiom.shell-options]
use crate::pattern::pattern_matches;

mod bytes;
mod mode;
mod typed;

use bytes::{at as byte_at, at_signed as byte_at_i};
use mode::EscapeMode;
pub(crate) use mode::ExpansionMode;

// ---------------------------------------------------------------------
// Internal marker bytes shared with the parser.
// ---------------------------------------------------------------------

pub(crate) const LEGACY_ESCAPE: u8 = crate::parser::LEGACY_ESCAPE;
const LEGACY_PARAMETER: u8 = crate::parser::LEGACY_PARAMETER;
const LEGACY_END_PARAMETER: u8 = crate::parser::LEGACY_END_PARAMETER;
const LEGACY_COMMAND_SUBSTITUTION: u8 = crate::parser::LEGACY_COMMAND_SUBSTITUTION;
pub(crate) const LEGACY_MULTIBYTE: u8 = crate::parser::LEGACY_MULTIBYTE;
const LEGACY_ARITHMETIC: u8 = crate::parser::LEGACY_ARITHMETIC;
const LEGACY_END_ARITHMETIC: u8 = crate::parser::LEGACY_END_ARITHMETIC;
const LEGACY_QUOTE: u8 = crate::parser::LEGACY_QUOTE;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum VariableExpansion {
    Invalid,
    Normal,
    Default,
    Alternative,
    Error,
    Assign,
    TrimRight,
    TrimRightLongest,
    TrimLeft,
    TrimLeftLongest,
    Length,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct VariableFlags {
    expansion: VariableExpansion,
    null_is_unset: bool,
}

impl VariableFlags {
    fn decode(encoded: u8) -> Self {
        let expansion = match encoded & crate::parser::PARAMETER_KIND_MASK {
            crate::parser::PARAMETER_NORMAL => VariableExpansion::Normal,
            crate::parser::PARAMETER_DEFAULT => VariableExpansion::Default,
            crate::parser::PARAMETER_ALTERNATIVE => VariableExpansion::Alternative,
            crate::parser::PARAMETER_ERROR => VariableExpansion::Error,
            crate::parser::PARAMETER_ASSIGN => VariableExpansion::Assign,
            crate::parser::PARAMETER_TRIM_SUFFIX => VariableExpansion::TrimRight,
            crate::parser::PARAMETER_TRIM_LONGEST_SUFFIX => VariableExpansion::TrimRightLongest,
            crate::parser::PARAMETER_TRIM_PREFIX => VariableExpansion::TrimLeft,
            crate::parser::PARAMETER_TRIM_LONGEST_PREFIX => VariableExpansion::TrimLeftLongest,
            crate::parser::PARAMETER_LENGTH => VariableExpansion::Length,
            _ => VariableExpansion::Invalid,
        };
        Self {
            expansion,
            null_is_unset: encoded & crate::parser::PARAMETER_COLON != 0,
        }
    }

    const fn normal() -> Self {
        Self {
            expansion: VariableExpansion::Normal,
            null_is_unset: false,
        }
    }
}

// C character literals used as `switch` labels; Rust `match` patterns
// require named constants, so the ones this file switches on get names.
pub(crate) const BANG: u8 = b'!';
const HASH: u8 = b'#';
const DOLLAR: u8 = b'$';
pub(crate) const STAR: u8 = b'*';
pub(crate) const MINUS: u8 = b'-';
const DOT: u8 = b'.';
const SLASH: u8 = b'/';
pub(crate) const COLON: u8 = b':';
pub(crate) const QUESTION: u8 = b'?';
const AT: u8 = b'@';
pub(crate) const LEFT_BRACKET: u8 = b'[';
pub(crate) const RIGHT_BRACKET: u8 = b']';
const BACKSLASH: u8 = b'\\';
pub(crate) const CARET: u8 = b'^';
const EQUALS: u8 = b'=';
const TILDE: u8 = b'~';
const ZERO: u8 = b'0';
const NINE: u8 = b'9';

// ---------------------------------------------------------------------
// src/expand.h
// ---------------------------------------------------------------------

// [spec:dash:def:expand.strlist]
// [spec:nsh:req:idiom.no-c-strings-core]
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
/// The bytes are length-delimited shell data; no framing byte is stored.
pub struct ExpandedField {
    pub text: BString,
}

impl ExpandedField {
    /// Copy one complete length-delimited field.
    pub fn from_bytes(bytes: &[u8]) -> ExpandedField {
        ExpandedField {
            text: BString::from(bytes),
        }
    }

    /// `rmescapes(sp->text)`, in place as the C does it.
    ///
    /// Quote removal compacts the field in place and returns its new length.
    #[inline]
    pub fn remove_escapes(&mut self) {
        let unescaped_length = remove_escapes_owned(&mut self.text);
        self.text.truncate(unescaped_length);
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
pub struct ExpandedFields {
    pub fields: Vec<ExpandedField>,
}

impl ExpandedFields {
    /// The C writes `struct arglist arglist;` and then
    /// `arglist.lastp = &arglist.list`, which is an empty list.
    pub const fn new() -> ExpandedFields {
        ExpandedFields { fields: Vec::new() }
    }
}

/// [`rmescapes`] over a buffer that owns its bytes.
///
/// Quote removal shortens the owned bytes in place. Returning the compacted
/// length lets callers truncate the allocation without manufacturing a
/// framing byte.
// [spec:posix:req:expand.quote-removal]
// [spec:posix:sem:expand.quote-removal-quoting-remembered]
pub fn remove_escapes_owned(s: &mut BString) -> usize {
    remove_escapes_in_buffer(s, EscapeMode::Plain)
}

// [spec:dash:def:expand.ifsregion]
/// A byte range eligible for field splitting.
pub struct FieldSplitRegion {
    pub start: usize,
    pub end: usize,
    pub nul_only: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FieldLimit {
    Unlimited,
    Remaining(usize),
}

// [spec:dash:def:expand.ifs-state]
/// Mutable state for one field-splitting pass.
pub struct FieldSplitState {
    pub nul_only: bool,
    pub field_start: usize,
    /// Start of a trailing IFS run that should be removed.
    pub trailing_whitespace_start: Option<usize>,
    max_fields: FieldLimit,
    pub separator_is_whitespace: bool,
}

/// Owned intermediate buffers for one expansion.
pub(crate) struct ExpandState {
    buffer: BString,
    command_substitutions: Vec<Option<crate::nodes::Node>>,
    next_command_substitution: usize,
    ifs_regions: Vec<FieldSplitRegion>,
    fields: Vec<ExpandedField>,
}

impl ExpandState {
    pub(crate) const fn new() -> Self {
        Self {
            buffer: BString::new(Vec::new()),
            command_substitutions: Vec::new(),
            next_command_substitution: 0,
            ifs_regions: Vec::new(),
            fields: Vec::new(),
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
    ascii_membership: [bool; 128],
    /// `IFS` itself as length-delimited shell bytes.
    bytes: BString,
    /// Length of the first multibyte character, or 0.
    first_character_length: usize,
    /// The wide-character form of `IFS`, built by `changeifs`.
    wide_characters: Vec<i32>,
}

impl IfsCache {
    pub(crate) const fn new() -> Self {
        IfsCache {
            ascii_membership: [false; 128],
            bytes: BString::new(Vec::new()),
            first_character_length: 0,
            wide_characters: Vec::new(),
        }
    }
}

#[inline]
fn split_regions(state: &mut ExpandState) -> &mut Vec<FieldSplitRegion> {
    &mut state.ifs_regions
}

/// `&mut exparg.list`, same.  Every `*exparg.lastp = sp` in the C is a
/// `push` on this, and `exparg.lastp = &exparg.list` — the C's way of
/// throwing away whatever the previous expansion left in the head — is a
/// `clear`.
#[inline]
fn expansion_fields(state: &mut ExpandState) -> &mut Vec<ExpandedField> {
    &mut state.fields
}

// ---------------------------------------------------------------------
// The expansion buffer.  See [`expbuf`].
// ---------------------------------------------------------------------

#[inline]
fn expansion_buffer(state: &mut ExpandState) -> &mut BString {
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
/// The result of an `expandarg(n, NULL, flag)` — the call that does *not*
/// grab its output.
///
/// Two callers: `redir::openhere` for a here-document and
/// `parser::expandstr` for `PS1`/`PS4`.  Both read the C's `stackblock()`
/// back after the call. The length-delimited result stays valid until the
/// next expansion begins — where the C's was valid only until the next
/// `stalloc`.
///
/// This hands back the length-delimited bytes rather than a pointer into
/// mutable expansion storage.
///
/// The borrow is `'static` because the buffer is, and the liveness the
/// callers rely on is unchanged and still theirs to respect: the bytes
/// last until the next expansion begins.  Nothing between either call and
/// its read expands — `openhere` only pipes and forks, `expandstr` reads
/// on the next line.
pub fn expansion_result(shell: &crate::context::Shell) -> &BStr {
    BStr::new(shell.expand.buffer.as_slice())
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
fn interrupt_pending() -> bool {
    crate::error::interrupt_pending()
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
fn encoded_character_len(s: &[u8], mut at: usize, escape_marker: u8) -> usize {
    let mut esc: usize = 0;

    while at > 0 && s[at - 1] == escape_marker {
        at -= 1;
        esc += 1;
    }
    esc
}

// [spec:dash:def:expand.mbnext-fn]
// [spec:dash:sem:expand.mbnext-fn]
//
#[derive(Clone, Copy)]
struct EncodedCharacterSpan {
    prefix: usize,
    remainder: usize,
}

// The pointer form is gone with its last caller.  It existed to answer
// "how much of this may I read?" for a walker holding a bare `*const
// i8` -- three bytes when the first is CTLMBCHAR, one otherwise --
// and every walker that asked now holds a slice that answers it.
//
// The decoding itself, over a slice, so the framing is bounds-checked
// rather than trusted.
fn next_encoded_character(encoded: &[u8]) -> EncodedCharacterSpan {
    let mut prefix = 0usize;
    let mut remainder = 0usize;

    let character = byte_at(encoded, remainder);
    remainder += 1;

    match character {
        LEGACY_MULTIBYTE => {
            if byte_at(encoded, remainder) == LEGACY_ESCAPE {
                remainder += 1;
            }
            let payload = usize::from(byte_at(encoded, remainder));
            remainder += 1;
            prefix = remainder;
            remainder = payload + 2;
        }
        LEGACY_ESCAPE => {
            prefix += 1;
        }
        _ => {}
    }

    EncodedCharacterSpan { prefix, remainder }
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
pub fn expand_argument(
    shell: &mut crate::context::Shell,
    arg: &crate::nodes::Node,
    expanded_fields: Option<&mut ExpandedFields>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let Node::Word(word) = arg else {
        return Err(shell
            .diagnostics()
            .shell_error(b"word expansion requires a word node"));
    };
    // [spec:nsh:def:idiom.word-ir]
    // [spec:nsh:sem:idiom.typed-expansion]
    typed::expand_argument(shell, &word.word, expanded_fields, mode)
}

// [spec:nsh:req:idiom.parser-control-flow]
fn expand_argument_inner(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    expanded_fields: Option<&mut ExpandedFields>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let mut expanded_word: BString;

    /* STARTSTACKSTR(expdest) */
    expansion_buffer(state).clear();
    /* The `?`s in this function return past the `ifsfree()` below, exactly
     * as the longjmp they replace jumped past it. The IFS regions are
     * reclaimed by the catch frame instead — `restore_handler_expandarg`'s
     * swallowing arm and `Shell::clear_evaluation_resources` both clear it, which is
     * docs/errors-are-values.md 2.2's mark-keyed cleanup working as
     * designed. Adding one here would free them twice. */
    expand_encoded_word(shell, state, text, 0, mode)?;
    if let Some(expanded_fields) = expanded_fields {
        expanded_word = mem::take(expansion_buffer(state));
        /* `exparg.lastp = &exparg.list`.  It re-points the tail at the
         * head, which discards whatever the previous call left there —
         * reachable only when that call unwound between building the list
         * and splicing it into its caller's. */
        expansion_fields(state).clear();
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
            split_regions_into_fields(
                shell,
                &state.ifs_regions,
                &mut expanded_word,
                FieldLimit::Unlimited,
                &mut state.fields,
            );
            /* `*exparg.lastp = NULL; exparg.lastp = &exparg.list;` —
             * terminate the fields `ifsbreakup` built, then re-point the
             * tail at the head so `expandmeta` rebuilds the list while
             * walking the one it was handed.  The first append there
             * overwrites the head, which is why the C can read `str->next`
             * before the write reaches it; taking the `Vec` is both
             * halves. */
            let words = mem::take(expansion_fields(state));
            expand_pathnames(shell, state, words)?;
        } else {
            expansion_fields(state).push(ExpandedField {
                text: expanded_word,
            });
        }
        /* `if (exparg.list) { *arglist->lastp = exparg.list; arglist->lastp
         * = exparg.lastp; }`.  The C guards on emptiness because splicing a
         * NULL head would leave the caller's tail pointing at `exparg`'s
         * own head; appending an empty `Vec` is already a no-op. */
        expanded_fields.fields.append(expansion_fields(state));
    }

    clear_split_regions(state);
    Ok(())
}

/*
 * Perform variable and command substitution.  If EXP_FULL is set, output CTLESC
 * characters to allow for further processing.  Otherwise treat
 * $@ like $* since no splitting will be performed.
 */

// [spec:dash:def:expand.argstr-fn]
// [spec:dash:sem:expand.argstr-fn]
fn expand_encoded_word(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    mut cursor: usize,
    mut mode: ExpansionMode,
) -> Result<usize, Error> {
    static SPECIAL_CHARACTERS: [u8; 10] = [
        EQUALS,
        COLON,
        LEGACY_QUOTE,
        LEGACY_END_PARAMETER,
        LEGACY_ESCAPE,
        LEGACY_PARAMETER,
        LEGACY_COMMAND_SUBSTITUTION,
        LEGACY_MULTIBYTE,
        LEGACY_ARITHMETIC,
        LEGACY_END_ARITHMETIC,
    ];
    /* The C advances a pointer into `spclchars`; the offset is the whole of
     * what it carries. The slice spells that set directly. */
    let mut special_start_index = 0;
    let mut control: u8;
    let record_parameter_word_regions =
        mode.contains(ExpansionMode::PARAMETER_WORD) && !mode.contains(ExpansionMode::QUOTED);
    let mut in_quotes: bool;
    let mut run_length: usize;
    let mut region_start: usize;

    special_start_index += usize::from(mode.contains(ExpansionMode::COLON_TILDE));
    special_start_index += if mode.contains(ExpansionMode::ASSIGNMENT_TILDE) {
        0
    } else {
        2
    };
    in_quotes = false;
    run_length = 0;

    if mode.contains(ExpansionMode::TILDE) {
        mode = mode.without(ExpansionMode::TILDE);
        if byte_at(text, cursor) == TILDE {
            cursor = expand_tilde(shell, state, text, cursor, mode);
        }
    }

    'expansion: loop {
        region_start = state.buffer.len();
        loop {
            let payload_length: usize;
            let span: EncodedCharacterSpan;
            let closes_word: bool;

            /* The run of bytes outside the active control set. Counted
             * rather than found with `find_byteset`, because this loop
             * re-enters after every control byte and taking the whole
             * remaining word each time would turn one pass into one pass
             * per escape. */
            let active_controls = &SPECIAL_CHARACTERS[special_start_index..];
            let from = cursor + run_length;
            run_length += text
                .get(from..)
                .unwrap_or_default()
                .iter()
                .take_while(|byte| !active_controls.contains(byte))
                .count();
            let Some(&next_control) = text.get(cursor + run_length) else {
                if run_length > 0 && !mode.contains(ExpansionMode::DISCARD) {
                    expansion_buffer(state).extend_from_slice(&text[cursor..]);
                    let expansion_end = state.buffer.len();
                    if record_parameter_word_regions && !in_quotes && expansion_end > region_start {
                        record_split_region(state, region_start, expansion_end, false);
                    }
                }
                return Ok(text.len());
            };
            control = next_control;
            if (control & 0x80) == 0
                || control == LEGACY_END_ARITHMETIC
                || control == LEGACY_END_PARAMETER
            {
                run_length += 1;
                closes_word = control == LEGACY_END_ARITHMETIC || control == LEGACY_END_PARAMETER;
            } else {
                closes_word = false;
            }
            if run_length > 0 && !mode.contains(ExpansionMode::DISCARD) {
                /* `cursor` walks the word
                 * text and never the expansion buffer, which is what the
                 * `copy_nonoverlapping` inside the old accessor already
                 * assumed and what makes this an append. */
                let buffer = expansion_buffer(state);
                let emitted = run_length - usize::from(closes_word);
                buffer.extend_from_slice(&text[cursor..cursor + emitted]);
                let expansion_end = buffer.len();
                if record_parameter_word_regions && !in_quotes && expansion_end > region_start {
                    record_split_region(state, region_start, expansion_end, false);
                }
                region_start = expansion_end;
            }
            cursor += run_length + 1;
            run_length = 0;

            if closes_word {
                return Ok(cursor - 1);
            }

            match control {
                EQUALS | COLON => {
                    if control == EQUALS {
                        mode = mode | ExpansionMode::COLON_TILDE;
                        special_start_index += 1;
                        /* fall through */
                    }
                    /*
                     * sort of a hack - expand tildes in variable
                     * assignments (after the first '=' and after ':'s).
                     */
                    cursor -= 1;
                    if byte_at(text, cursor) == TILDE {
                        cursor = expand_tilde(shell, state, text, cursor, mode);
                        continue 'expansion;
                    }
                    continue;
                }
                LEGACY_QUOTE => {
                    /* "$@" syntax adherence hack */
                    /* These are the five bytes the parser emits for a bare
                     * `"$@"`. */
                    let quoted_at_tail = [
                        LEGACY_PARAMETER,
                        crate::parser::PARAMETER_NORMAL | crate::parser::PARAMETER_PRESENT,
                        b'@',
                        b'=',
                        LEGACY_QUOTE,
                    ];
                    if !in_quotes && text.get(cursor..).unwrap_or_default() == quoted_at_tail {
                        cursor = expand_parameter(
                            shell,
                            state,
                            text,
                            cursor + 1,
                            mode | ExpansionMode::QUOTED,
                        )? + 1;
                        continue 'expansion;
                    }
                    in_quotes = !in_quotes;
                    /* addquote: */
                    if mode.escapes_quotes() {
                        cursor -= 1;
                        run_length += 1;
                        region_start += 1;
                    }
                }
                LEGACY_MULTIBYTE => {
                    control = byte_at(text, cursor);
                    cursor -= 1;
                    span = next_encoded_character(text.get(cursor..).unwrap_or_default());
                    payload_length = span.remainder - 2;
                    if mode.escapes_quotes() || mode.contains(ExpansionMode::PRESERVE_MULTIBYTE) {
                        run_length = span.prefix + span.remainder;
                        if control == LEGACY_ESCAPE {
                            region_start += run_length;
                        }
                    } else {
                        if control == LEGACY_ESCAPE {
                            region_start += payload_length;
                        }
                        cursor += span.prefix;
                        if !mode.contains(ExpansionMode::DISCARD) {
                            expansion_buffer(state)
                                .extend_from_slice(&text[cursor..cursor + payload_length]);
                        }
                        cursor += span.remainder;
                    }
                }
                LEGACY_ESCAPE => {
                    region_start += 1;
                    run_length += 1;
                    if mode.escapes_quotes() {
                        cursor -= 1;
                        run_length += 1;
                        region_start += 1;
                    }
                }
                LEGACY_PARAMETER => {
                    cursor = expand_parameter(
                        shell,
                        state,
                        text,
                        cursor,
                        mode.with_if(ExpansionMode::QUOTED, in_quotes),
                    )?;
                    continue 'expansion;
                }
                LEGACY_COMMAND_SUBSTITUTION => {
                    let substitution_index = state.next_command_substitution;
                    state.next_command_substitution += 1;
                    let command = state
                        .command_substitutions
                        .get_mut(substitution_index)
                        .and_then(Option::take);
                    expand_command_substitution(
                        shell,
                        state,
                        command.as_ref(),
                        mode.with_if(ExpansionMode::QUOTED, in_quotes),
                    )?;
                    continue 'expansion;
                }
                LEGACY_ARITHMETIC => {
                    cursor = expand_arithmetic(
                        shell,
                        state,
                        text,
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
fn expand_tilde(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    start: usize,
    mode: ExpansionMode,
) -> usize {
    let mut cursor = start;
    let name_start = cursor + 1;

    loop {
        cursor += 1;
        let Some(&byte) = text.get(cursor) else {
            break;
        };
        match byte {
            LEGACY_ESCAPE => return start,
            LEGACY_QUOTE => return start,
            COLON => {
                if mode.contains(ExpansionMode::ASSIGNMENT_TILDE) {
                    break;
                }
            }
            SLASH | LEGACY_END_PARAMETER => break,
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
        let name_bytes = &text[name_start..cursor.min(text.len())];

        if name_bytes.is_empty() {
            let Some(home) = crate::variables::lookup_bytes(shell, BStr::new(b"HOME")) else {
                return start;
            };
            push_bytes(
                &shell.locale,
                &home,
                mode | ExpansionMode::QUOTED,
                expansion_buffer(state),
            );
        } else {
            let Ok(name) = name_bytes.try_to_os_string() else {
                return start;
            };
            let Some(home) = nsh_platform::named_user_home(&name) else {
                /* lose: */
                return start;
            };
            let home = home.to_shell_bytes();
            push_bytes(
                &shell.locale,
                &home,
                mode | ExpansionMode::QUOTED,
                expansion_buffer(state),
            );
        }
    }
    cursor
}

// [spec:dash:def:expand.removerecordregions-fn]
// [spec:dash:sem:expand.removerecordregions-fn]
fn truncate_split_regions(state: &mut ExpandState, end: usize) {
    /* `ifslastp == NULL` */
    if split_regions(state).is_empty() {
        return;
    }

    /* `ifsfirst` is index 0; `ifslastp` is the index the walk below
     * settles on, and dropping the tail is `truncate`. */
    if split_regions(state)[0].end > end {
        while split_regions(state).len() > 1 {
            split_regions(state).pop();
        }
        if split_regions(state)[0].start > end {
            split_regions(state).clear();
        } else {
            split_regions(state)[0].end = end;
        }
        return;
    }

    let mut last: usize = 0;
    while last + 1 < split_regions(state).len() && split_regions(state)[last + 1].start < end {
        last += 1;
    }
    while split_regions(state).len() > last + 1 {
        split_regions(state).pop();
    }
    if split_regions(state)[last].end > end {
        split_regions(state)[last].end = end;
    }
}

/*
 * Expand arithmetic expression.  Backup to start of expression,
 * evaluate, place result in (backed up) result, adjust string position.
 */

// [spec:dash:def:expand.expari-fn]
// [spec:dash:sem:expand.expari-fn]
// [spec:posix:req:expand.arith-token-expansion]
fn expand_arithmetic(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    mode: ExpansionMode,
) -> Result<usize, Error> {
    let expression_start: usize;
    let rendered_length: usize;
    let result: i64;
    /* The C's `p` doubles as a scratch `stackblock()` before it becomes the
     * return value; only the second use survives. */
    let expansion_end: usize;

    expression_start = state.buffer.len();
    expansion_end = expand_encoded_word(
        shell,
        state,
        text,
        expression_start,
        mode.intersection(ExpansionMode::DISCARD),
    )?;

    if !mode.contains(ExpansionMode::DISCARD) {
        /* `start = stackblock() + begoff; STADJUST(start - expdest, expdest)`
         * made the C parser read the expression through a pointer beyond
         * the stack allocator's restored cursor.  The expression has value
         * semantics now: copy the counted bytes before rewinding the output
         * buffer, then lend that slice to the arithmetic parser. */
        let arithmetic = BStr::new(&expansion_buffer(state)[expression_start..]).to_owned();
        expansion_buffer(state).truncate(expression_start);

        truncate_split_regions(state, expression_start);

        /* `arith` returns its diagnostic now instead of raising it, and as
         * of this commit so does `expari`, so the bridge that stood here is
         * gone and the value travels. */
        result = crate::arithmetic::evaluate(shell, arithmetic.as_bstr())?;

        rendered_length = push_integer(&shell.locale, result, mode, expansion_buffer(state));

        if !mode.contains(ExpansionMode::QUOTED) {
            record_split_region(
                state,
                expression_start,
                expression_start + rendered_length,
                false,
            );
        }
    }

    Ok(expansion_end)
}

/*
 * Expand stuff in backwards quotes.
 */

// [spec:dash:def:expand.expbackq-fn]
// [spec:dash:sem:expand.expbackq-fn]
// [spec:posix:req:expand.cmdsub-semantics]
// [spec:posix:req:expand.cmdsub-no-reexpansion]
fn expand_command_substitution(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    command: Option<&crate::nodes::Node>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let mut substitution = crate::evaluation::CommandSubstitution {
        descriptor: None,
        job_id: None,
    };
    /* `char buf[128]`, as bytes: it is only ever handed to `read` and to
     * `memtodest`, and both want the bytes rather than the sign. */
    let mut buffer = [0; 128];

    if !mode.contains(ExpansionMode::DISCARD) {
        let expansion_start = crate::error::with_interrupts_deferred(shell, |shell| {
            let expansion_start = state.buffer.len();
            /* `pushstackmark(&smark, startloc)`: the length kept `makejob`'s
             * region allocations off the half-built word, and the save/restore
             * released them afterwards. The word is not in the region and
             * neither is anything `evalbackcmd` reaches, so both halves are
             * gone. */
            crate::evaluation::evaluate_command_substitution(shell, command, &mut substitution)?;

            /* `evalbackcmd` always returns a pipe with an empty read-ahead
             * area, so reading starts directly from that pipe. */
            loop {
                let Some(descriptor) = substitution.descriptor.as_ref() else {
                    break;
                };
                let count = loop {
                    match nsh_platform::read_once(descriptor, &mut buffer) {
                        Ok(count) => break count,
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                        Err(_) => break 0,
                    }
                };
                if count == 0 {
                    break;
                }
                push_bytes(
                    &shell.locale,
                    &buffer[..count],
                    mode,
                    expansion_buffer(state),
                );
            }

            if substitution.descriptor.take().is_some() {
                shell.evaluation.command_substitution_status =
                    crate::jobs::wait_for_job(shell, substitution.job_id)?;
            }
            Ok::<_, Error>(expansion_start)
        })?;

        if let Some(error) = crate::error::poll_interrupt(shell.interrupt_context()) {
            return Err(error);
        }

        /* Eat all trailing newlines. The cursor is the length, so the
         * walk is over the buffer's own bytes and `STADJUST` is a
         * `truncate`. */
        nsh_platform::trim_command_substitution_output(expansion_buffer(state), expansion_start);

        if !mode.contains(ExpansionMode::QUOTED) {
            let expansion_end = state.buffer.len();
            record_split_region(state, expansion_start, expansion_end, false);
        }
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
/// called indivisible: a `fn(*mut i8, *mut i8, *mut i8, *mut
/// i8, *mut i8, i32, i32) -> *mut i8` cannot be changed one
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
fn between(bytes: &[u8], from: usize, to: usize) -> &[u8] {
    let from = from.min(bytes.len());
    &bytes[from..to.clamp(from, bytes.len())]
}

struct PatternScan {
    /// The value being trimmed.
    value_start: usize,
    /// Its last byte. `scanright` walks down from here.
    value_end: usize,
    /// The unescaped copy `_rmescapes` left above the cursor, and its end.
    /// `loc2` tracks it because it is what an unquoted match returns.
    unescaped_start: usize,
    unescaped_end: usize,
    /// The pattern, `preglob`'d in place.
    pattern_start: usize,
    preserve_quotes: bool,
    match_prefix: bool,
}

type PatternScanFn = fn(&nsh_platform::Locale, &[u8], &PatternScan) -> Option<usize>;

// [spec:dash:def:expand.scanleft-fn]
// [spec:dash:sem:expand.scanleft-fn]
fn scan_left(locale: &nsh_platform::Locale, bytes: &[u8], scan: &PatternScan) -> Option<usize> {
    let mut encoded_cursor = scan.value_start;
    let mut unescaped_cursor = scan.unescaped_start;
    loop {
        let candidate_start = encoded_cursor;

        /* `c = *s; if (zero) { *s = '\0'; s = startp; } pmatch(str, s);
         * *loc = c;` — the temporary terminator, as a subslice that ends
         * where it went. */
        let subject: &[u8] = if scan.match_prefix {
            between(bytes, scan.value_start, candidate_start)
        } else {
            bytes.get(candidate_start..).unwrap_or_default()
        };
        if pattern_matches(
            locale,
            bytes.get(scan.pattern_start..).unwrap_or_default(),
            subject,
        ) {
            return Some(if scan.preserve_quotes {
                encoded_cursor
            } else {
                unescaped_cursor
            });
        }

        if encoded_cursor >= scan.value_end {
            break;
        }

        let span = next_encoded_character(bytes.get(encoded_cursor..).unwrap_or_default());
        encoded_cursor += span.prefix + span.remainder;
        unescaped_cursor += if span.remainder > 3 {
            span.remainder - 2
        } else {
            1
        };
    }
    None
}

// [spec:dash:def:expand.scanright-fn]
// [spec:dash:sem:expand.scanright-fn]
fn scan_right(locale: &nsh_platform::Locale, bytes: &[u8], scan: &PatternScan) -> Option<usize> {
    let mut escape_count: usize = 0;
    /* Signed, because the C's `loc--` walks off the bottom of the value on
     * purpose and `if (loc < startp) break` is how it notices.  `byte_at_i`
     * answers 0 for a negative index, so the two `*loc` reads inside the
     * multibyte rewind — which the C performs without a bounds test, on the
     * strength of the frame being well formed — cannot read before the
     * buffer here. */
    let mut encoded_cursor = scan.value_end as isize;
    let mut unescaped_cursor = scan.unescaped_end as isize;
    loop {
        let candidate_start = encoded_cursor;

        /* `c = *s; if (zero) { *s = '\0'; s = startp; } pmatch(str, s);
         * *loc = c;` — see [`Scan`]: the subslice ends where the C's
         * temporary NUL went, so nothing is written. */
        let subject: &[u8] = if scan.match_prefix {
            between(bytes, scan.value_start, candidate_start.max(0) as usize)
        } else {
            bytes
                .get(candidate_start.max(0) as usize..)
                .unwrap_or_default()
        };
        if pattern_matches(
            locale,
            bytes.get(scan.pattern_start..).unwrap_or_default(),
            subject,
        ) {
            return Some(if scan.preserve_quotes {
                encoded_cursor
            } else {
                unescaped_cursor
            } as usize);
        }
        encoded_cursor -= 1;
        if encoded_cursor < scan.value_start as isize {
            break;
        }
        /* if (!esc--) esc = esclen(startp, loc); */
        let previous_escape_count = escape_count;
        escape_count = escape_count.wrapping_sub(1);
        if previous_escape_count == 0 {
            escape_count = encoded_character_len(
                &bytes[scan.value_start..],
                encoded_cursor as usize - scan.value_start,
                LEGACY_ESCAPE,
            );
        }
        if escape_count % 2 != 0 {
            escape_count -= 1;
            encoded_cursor -= 1;
        } else if byte_at_i(bytes, encoded_cursor) == LEGACY_MULTIBYTE {
            encoded_cursor -= 1;
            let payload_length = usize::from(byte_at_i(bytes, encoded_cursor));
            encoded_cursor -=
                isize::try_from(payload_length + 2).expect("encoded character fits isize");
            if byte_at_i(bytes, encoded_cursor) == LEGACY_ESCAPE {
                encoded_cursor -= 1;
            }
            unescaped_cursor -= isize::try_from(payload_length.saturating_sub(1))
                .expect("encoded character fits isize");
        }
        unescaped_cursor -= 1;
    }
    None
}

// [spec:dash:def:expand.subevalvar-fn]
// [spec:dash:sem:expand.subevalvar-fn]
fn apply_parameter_operator(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    input_start: usize,
    /* The C's `char *str`, which is the variable's *name* in the word on
     * entry and NULL for the trimming subtypes.  `Option` is that NULL as
     * a type; the C then reuses the same local for the pattern, which is
     * why the pattern has a name of its own below. */
    variable_name_start: Option<usize>,
    pattern_boundary: usize,
    expansion_start: usize,
    variable: VariableFlags,
    mode: ExpansionMode,
) -> Result<usize, Error> {
    let preserve_quotes = mode.escapes_quotes();
    /* Every one of the C's `char *` locals here is a position in the
     * expansion buffer and only ever used as one.  As offsets they stop
     * having to be re-derived: the three `stackblock()` re-reads below the
     * `_rmescapes` call are gone, because an index does not move when the
     * buffer grows.  `str` keeps its pointer type because it is not one of
     * them — it is the variable's *name*, in the word text — and the C
     * reuses the same local for the pattern, which is why that one gets a
     * name of its own. */
    let next_input = expand_encoded_word(
        shell,
        state,
        text,
        input_start,
        mode.intersection(ExpansionMode::DISCARD)
            | ExpansionMode::TILDE
            | if variable_name_start.is_some() {
                ExpansionMode::PLAIN
            } else {
                ExpansionMode::CASE_PATTERN
            },
    )?;
    if mode.contains(ExpansionMode::DISCARD) {
        return Ok(next_input);
    }

    let result_end = if variable.expansion == VariableExpansion::Assign {
        let name =
            BStr::new(&text[variable_name_start.expect("VSASSIGN carries the variable's name")..]);
        let name = crate::variables::assignment_name(name);
        let value = BStr::new(&expansion_buffer(state)[expansion_start..]);
        crate::variables::set_bytes(
            shell,
            name,
            Some(value),
            crate::variables::VariableAttributes::NONE,
        )?;

        expansion_start
    } else {
        if variable.expansion == VariableExpansion::Error {
            /* `varunset` stopped diverging with this commit, so this
             * has to be a `return` and not a bare call. It was a stop
             * before — docs/errors-are-values.md 0.2 is the bug that
             * happens when one of these is missed, and `Error` is
             * `#[must_use]` so the compiler now names it. */
            let unset_message = BStr::new(&expansion_buffer(state)[expansion_start..]);
            let variable_name_start =
                variable_name_start.expect("VSQUESTION carries the variable's name");
            return Err(unset_parameter_error(
                shell,
                text,
                input_start,
                variable_name_start,
                Some(unset_message),
                variable.null_is_unset,
            ));
        }

        let mut unescaped_end = pattern_boundary;
        /* `str = preglob(rmescend, 0, NULL)` — the pattern is unescaped in
         * place, so its result remains a position in this buffer. */
        remove_escapes_in_buffer(
            &mut expansion_buffer(state)[unescaped_end..],
            EscapeMode::Glob,
        );
        let pattern_start = unescaped_end;

        let mut unescaped_start = expansion_start;
        if !preserve_quotes {
            /* `_rmescapes` with RMESCAPE_GROW appends an unescaped copy of
             * `startp` past the cursor and moves the cursor over it, so the
             * buffer can have reallocated underneath.  That is what the C's
             * three `stackblock()` re-reads on the lines after this call
             * were for, and they are gone: an offset survives a growth,
             * which is why this hands over one and gets one back. */
            unescaped_start = remove_escapes_from_offset(expansion_buffer(state), expansion_start);
            if unescaped_start != expansion_start {
                unescaped_end = expansion_buffer(state).len();
            }
        }
        unescaped_end -= 1;

        let (match_prefix, scan_pattern) = match variable.expansion {
            VariableExpansion::TrimRight => (false, scan_right as PatternScanFn),
            VariableExpansion::TrimRightLongest => (false, scan_left as PatternScanFn),
            VariableExpansion::TrimLeft => (true, scan_left as PatternScanFn),
            VariableExpansion::TrimLeftLongest => (true, scan_right as PatternScanFn),
            _ => unreachable!("subevalvar only trims, assigns, or reports an unset variable"),
        };

        let value_end = pattern_boundary - 1;
        let found = scan_pattern(
            &shell.locale,
            expansion_buffer(state),
            &PatternScan {
                value_start: expansion_start,
                value_end,
                unescaped_start,
                unescaped_end,
                pattern_start,
                preserve_quotes,
                match_prefix,
            },
        );
        match found {
            None => {
                if preserve_quotes {
                    unescaped_start = expansion_start;
                    unescaped_end = value_end;
                }
            }
            Some(at) if !preserve_quotes => {
                if match_prefix {
                    unescaped_start = at;
                } else {
                    unescaped_end = at;
                }
            }
            Some(at) if match_prefix => {
                unescaped_start = at;
                unescaped_end = value_end;
            }
            Some(at) => {
                unescaped_start = expansion_start;
                unescaped_end = at;
            }
        }

        /* `memmove(startp, rmesc, rmescend - rmesc)` — the two ranges are
         * in one buffer and may overlap, which `copy_within` already
         * knows. */
        expansion_buffer(state).copy_within(unescaped_start..unescaped_end, expansion_start);
        expansion_start + (unescaped_end - unescaped_start)
    };

    /* The compacted value ends at `loc`; its length is the boundary. */
    let buffer = expansion_buffer(state);
    debug_assert!(result_end <= buffer.len());
    buffer.truncate(result_end);

    /* Remove any recorded regions beyond start of variable */
    truncate_split_regions(state, expansion_start);

    Ok(next_input)
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
fn expand_parameter(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    text: &[u8],
    mut cursor: usize,
    mut mode: ExpansionMode,
) -> Result<usize, Error> {
    let mut variable: VariableFlags;
    let variable_name_start: usize;
    let pattern_start: usize;
    let expansion_start: usize;
    let mut value_length: Option<usize>;
    let mut discard: bool;
    let quoted = mode.contains(ExpansionMode::QUOTED);
    let multibyte_mode: ExpansionMode;

    variable = VariableFlags::decode(byte_at(text, cursor));
    cursor += 1;

    variable_name_start = cursor;
    expansion_start = state.buffer.len();
    /* The parser always writes the `=` that ends the variable name, and
     * the C dereferences `strchr`'s result without checking. */
    cursor += BStr::new(text.get(cursor..).unwrap_or_default())
        .find_byte(EQUALS)
        .expect("the parser ends a variable name with `=`")
        + 1;

    multibyte_mode = match variable.expansion {
        VariableExpansion::TrimLeft
        | VariableExpansion::TrimLeftLongest
        | VariableExpansion::TrimRight
        | VariableExpansion::TrimRightLongest => ExpansionMode::PRESERVE_MULTIBYTE,
        _ => ExpansionMode::PLAIN,
    };

    enum RecordPolicy {
        IfPresent,
        Always,
    }

    let record_policy = loop {
        value_length = parameter_value(
            shell,
            state,
            BStr::new(&text[variable_name_start..cursor]),
            variable.expansion,
            mode | multibyte_mode,
        )?;
        if variable.null_is_unset && value_length == Some(0) {
            value_length = None;
        }

        discard = value_length.is_none();

        match variable.expansion {
            VariableExpansion::Alternative
            | VariableExpansion::Invalid
            | VariableExpansion::Default => {
                if variable.expansion == VariableExpansion::Alternative {
                    discard = !discard;
                    /* fall through */
                }

                cursor = expand_encoded_word(
                    shell,
                    state,
                    text,
                    cursor,
                    (mode | ExpansionMode::TILDE | ExpansionMode::PARAMETER_WORD)
                        .with_if(ExpansionMode::DISCARD, !discard),
                )?;
                break RecordPolicy::IfPresent;
            }

            VariableExpansion::Assign | VariableExpansion::Error => {
                cursor = apply_parameter_operator(
                    shell,
                    state,
                    text,
                    cursor,
                    Some(variable_name_start),
                    0,
                    expansion_start,
                    variable,
                    mode.without(ExpansionMode::SPLIT | ExpansionMode::CASE_PATTERN)
                        .with_if(ExpansionMode::DISCARD, !discard),
                )?;

                if mode.contains(ExpansionMode::DISCARD) || !discard {
                    break RecordPolicy::IfPresent;
                }

                variable = VariableFlags::normal();
                continue;
            }
            _ => {}
        }

        if discard
            && !mode.contains(ExpansionMode::DISCARD)
            && shell.options.enabled(ShellOption::Nounset)
        {
            /* A stop before `varunset` stopped diverging, and still one. */
            return Err(unset_parameter_error(
                shell,
                text,
                cursor,
                variable_name_start,
                None,
                false,
            ));
        }

        if variable.expansion == VariableExpansion::Length {
            cursor += 1;
            if mode.contains(ExpansionMode::DISCARD) {
                return Ok(cursor);
            }
            push_integer(
                &shell.locale,
                i64::try_from(value_length.unwrap_or(0)).unwrap_or(i64::MAX),
                mode,
                expansion_buffer(state),
            );
            break RecordPolicy::Always;
        }

        if variable.expansion == VariableExpansion::Normal {
            break RecordPolicy::IfPresent;
        }

        mode = mode.with_if(ExpansionMode::DISCARD, discard);
        /* `patloc` is the length-delimited boundary between value and pattern. */
        pattern_start = state.buffer.len();
        cursor = apply_parameter_operator(
            shell,
            state,
            text,
            cursor,
            None,
            pattern_start,
            expansion_start,
            variable,
            mode,
        )?;
        break RecordPolicy::IfPresent;
    };

    if matches!(record_policy, RecordPolicy::IfPresent)
        && (mode.contains(ExpansionMode::DISCARD) || discard)
    {
        return Ok(cursor);
    }

    let quoted_at = if quoted {
        byte_at(text, variable_name_start) == AT
            && shell.options.positional_parameters.parameter_count != 0
    } else {
        false
    };
    if quoted && !quoted_at {
        return Ok(cursor);
    }
    let expansion_end = state.buffer.len();
    record_split_region(state, expansion_start, expansion_end, quoted_at);
    Ok(cursor)
}

// [spec:dash:def:expand.chtodest-fn]
// [spec:dash:sem:expand.chtodest-fn]
/// The cursor the C returns is the destination's own length now, so this
/// appends and returns nothing. It performs no unsafe operation at all.
fn push_character(byte: u8, syntax: DestinationSyntax, output: &mut BString) {
    if syntax.escapes(byte) {
        /* USTPUTC(CTLESC, out) */
        output.push(LEGACY_ESCAPE);
    }
    /* USTPUTC(c, out) */
    output.push(byte);
}

// [spec:dash:def:expand.mbpair]
// The translated `mbpair { ml, ql }` record is gone. The destination owns
// its cursor, so only the number of additional source bytes remains a return
// value.

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
fn push_multibyte_character(
    locale: &nsh_platform::Locale,
    source: &[u8],
    at: usize,
    output: &mut BString,
    syntax: DestinationSyntax,
) -> usize {
    let mut multibyte_length: usize;

    /* `p = p - 1` */
    let source_character = &source[at - 1..];
    multibyte_length = locale.multibyte_len(source_character).unwrap_or(usize::MAX);
    if multibyte_length == (0 as usize).wrapping_sub(2)
        || multibyte_length == (0 as usize).wrapping_sub(1)
        || multibyte_length < 2
    {
        push_character(source_character[0], syntax, output);
        multibyte_length = 1;
    } else {
        /* `syntax[CTLMBCHAR]` — CTLMBCHAR is negative; see the note in
         * `memtodest` about the unbiased `is_type` table. Negative is an
         * ordinary index now, and a checked one. */
        if syntax.escapes(LEGACY_MULTIBYTE) {
            /* USTPUTC(CTLMBCHAR, q); USTPUTC(ml, q); */
            output.push(LEGACY_MULTIBYTE);
            output.push(multibyte_length as u8);
        }

        /* `q = mempcpy(q, p, ml)`. The source is the caller's input and
         * never `dst`'s own buffer -- `memtodest` records why -- so the
         * append cannot alias what it reads.  `ml` came from `mbrlen`
         * over this same slice, so it cannot exceed it. */
        output.extend_from_slice(&source_character[..multibyte_length]);

        if syntax.escapes(LEGACY_MULTIBYTE) {
            /* USTPUTC(ml, q); USTPUTC(CTLMBCHAR, q); */
            output.push(multibyte_length as u8);
            output.push(LEGACY_MULTIBYTE);
        }
    }

    multibyte_length.wrapping_sub(1)
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
fn push_bytes(
    locale: &nsh_platform::Locale,
    source: &[u8],
    mode: ExpansionMode,
    output: &mut BString,
) -> usize {
    let syntax: DestinationSyntax;
    let mut count: usize = 0;
    /* The C's `p` and `len` are one cursor over `src` and the number of
     * bytes left; `i` is the first and `src.len() - i` the second. */
    let mut source_index = 0;

    if source.is_empty() {
        return 0;
    }

    /* CTLMBCHAR, 2, c, c, 2, CTLMBCHAR.  A hint now rather than a
     * contract: the writes below are appends, so a short reservation
     * costs a growth instead of running off the end. */
    output.reserve(source.len() * 3);

    let framed = mode.escapes_quotes() || mode.contains(ExpansionMode::PRESERVE_MULTIBYTE);
    if !mode.contains(ExpansionMode::QUOTED) || !framed {
        while source.len() - source_index >= 8 {
            /* `__builtin_memcpy` of eight bytes into a `uint64_t`, which
             * is an unaligned load the C spells with a cast.  Over a
             * slice it is a checked eight-byte read, and the check is
             * the loop condition. */
            let chunk =
                u64::from_ne_bytes(source[source_index..source_index + 8].try_into().unwrap());

            if (chunk | chunk.wrapping_sub(0x0101010101010101)) & 0x8080808080808080 != 0 {
                break;
            }

            /* The C's `write_unaligned(q + count, x)` is a copy of the
             * eight bytes just read, and `to_ne_bytes` is that copy: the
             * value round-trips through the same native representation
             * it was loaded from. The C's `q = q + count` after the loop
             * is gone because appending has already moved the cursor. */
            output.extend_from_slice(&chunk.to_ne_bytes());

            count += 8;
            source_index += 8;
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
    while source_index < source.len() {
        let byte = source[source_index];
        source_index += 1;

        if byte == 0 && !mode.contains(ExpansionMode::KEEP_NUL) {
            continue;
        }

        count += 1;

        if byte & 0x80 != 0 {
            /* `mbtodest(p, ...)` is called with `p` already past the
             * byte it is about to decode, and starts by stepping
             * back over it; `i` is that same position. */
            let additional = push_multibyte_character(locale, source, source_index, output, syntax);
            source_index += additional;
            continue;
        }

        push_character(byte, syntax, output);
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
fn push_text(
    locale: &nsh_platform::Locale,
    value: &[u8],
    mode: ExpansionMode,
    output: &mut BString,
) -> usize {
    push_bytes(locale, value, mode, output)
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
fn parameter_value(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    name: &BStr,
    expansion: VariableExpansion,
    mut mode: ExpansionMode,
) -> Result<Option<usize>, Error> {
    let mut separator_length: usize;
    /* The C's `const char *seps` plus its length.  The comment that stood
     * at the assignment below owed a conversion — it said the pointer was
     * safe *because* of where the storage comes from, which is an argument
     * a slice does not have to make.  Both sources are bytes the shell
     * owns for the whole call, so both are slices. */
    let mut separators: &[u8];
    let mut value_length = 0;
    let expansion_start: usize;
    let discard: bool;
    let name = crate::variables::assignment_name(name);
    let name_byte = name.first().copied().unwrap_or_default();

    discard = matches!(
        expansion,
        VariableExpansion::Alternative | VariableExpansion::Length
    ) || mode.contains(ExpansionMode::DISCARD);

    if expansion == VariableExpansion::Invalid {
        if discard {
            return Ok(None);
        }

        return Err(shell.diagnostics().shell_error(b"Bad substitution"));
    }

    if discard {
        mode = mode.without(ExpansionMode::SPLIT | ExpansionMode::CASE_PATTERN);
    }
    /* `seps = nullstr` — the empty C string, whose one byte is the
     * terminator, and the terminator is what gets written when the
     * separator is a NUL. */
    separators = &[0u8];
    separator_length = usize::from(mode.contains(ExpansionMode::SPLIT));
    expansion_start = state.buffer.len();

    match name_byte {
        DOLLAR | QUESTION | HASH | BANG => {
            let number = match name_byte {
                DOLLAR => i64::from(shell.root_pid.get()),
                QUESTION => i64::from(shell.status.code()),
                HASH => i64::try_from(shell.options.positional_parameters.parameter_count)
                    .unwrap_or(i64::MAX),
                BANG => {
                    let Some(process_id) = shell.background_process else {
                        return Ok(None);
                    };
                    i64::from(process_id.get())
                }
                _ => unreachable!(),
            };
            value_length = push_integer(&shell.locale, number, mode, expansion_buffer(state));
        }
        MINUS => {
            for spec in OPTION_SPECS.iter().rev() {
                if shell.options.enabled(spec.option)
                    && let Some(letter) = spec.letter
                {
                    expansion_buffer(state).push(letter);
                    value_length += 1;
                }
            }
        }
        AT | STAR => {
            if name_byte != AT
                || !(mode.contains(ExpansionMode::QUOTED) && mode.contains(ExpansionMode::SPLIT))
            {
                if mode.contains(ExpansionMode::QUOTED) {
                    separator_length = 0;
                }
                if separator_length == 0 {
                    separators = shell.ifs.bytes.as_slice();
                }
                separator_length = (separator_length.wrapping_sub(1)
                    & shell.ifs.first_character_length.wrapping_sub(1))
                .wrapping_add(1);
            }

            for (index, parameter) in shell
                .options
                .positional_parameters
                .words()
                .iter()
                .enumerate()
            {
                if index != 0 {
                    debug_assert!(
                        separator_length <= separators.len(),
                        "parameter separator length {separator_length} exceeds the {} bytes it names",
                        separators.len()
                    );
                    value_length += push_bytes(
                        &shell.locale,
                        &separators[..separator_length],
                        mode | ExpansionMode::KEEP_NUL,
                        expansion_buffer(state),
                    );
                }

                value_length += push_text(&shell.locale, parameter, mode, expansion_buffer(state));
            }
        }
        digit if (ZERO..=NINE).contains(&digit) => {
            let position = crate::number::parse_decimal(name)
                .and_then(|number| usize::try_from(number).ok())
                .unwrap_or(0);
            if position > shell.options.positional_parameters.parameter_count {
                return Ok(None);
            }
            let value = if position != 0 {
                shell
                    .options
                    .positional_parameters
                    .words()
                    .get(position - 1)
                    .cloned()
            } else {
                shell.options.argument_zero().map(BStr::to_owned)
            };
            let Some(value) = value else {
                return Ok(None);
            };
            value_length = push_text(&shell.locale, &value, mode, expansion_buffer(state));
        }
        _ => {
            let Some(value) = crate::variables::lookup_bytes(shell, name) else {
                return Ok(None);
            };
            value_length = push_text(&shell.locale, &value, mode, expansion_buffer(state));
        }
    }

    if discard {
        expansion_buffer(state).truncate(expansion_start);
    }

    Ok(Some(value_length))
}

/*
 * Record the fact that we have to scan this region of the
 * string for IFS characters.
 */

// [spec:dash:def:expand.recordregion-fn]
// [spec:dash:sem:expand.recordregion-fn]
pub(crate) fn record_split_region(
    state: &mut ExpandState,
    start: usize,
    end: usize,
    nul_only: bool,
) {
    split_regions(state).push(FieldSplitRegion {
        start,
        end,
        nul_only,
    });
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct IfsMembership {
    separator: bool,
    default_whitespace: bool,
}

// [spec:dash:def:expand.ifsisifs-fn]
// [spec:dash:sem:expand.ifsisifs-fn]
fn classify_ifs(
    shell: &Shell,
    bytes: &[u8],
    multibyte_length: usize,
    nul_only: bool,
) -> IfsMembership {
    let mut is_default_whitespace = false;
    let mut is_separator = false;
    let mut wide_character = byte_at(bytes, 0) as i32;
    /* C leaves `ifs0` uninitialised; it is only read when `isifs`, which
     * implies one of the branches below assigned it. */
    let mut first_separator = 0;

    if nul_only {
        is_separator = wide_character == 0;
    } else if !shell.ifs.bytes.is_empty() && !shell.ifs.wide_characters.is_empty() {
        if (wide_character & 0x80) != 0 {
            /* `ml` came from `mbnext` over this same slice, so the
             * clamp can only bite where the C read past the word's
             * end -- and a short read fails the `!= ml` test exactly
             * as a malformed character does.  The same trade
             * `ccmatch_bytes` records. */
            let available_length = multibyte_length.min(bytes.len());
            let Some(decoded_character) = shell
                .locale
                .decode_exact(&bytes[..available_length], multibyte_length)
            else {
                return IfsMembership::default();
            };
            wide_character = decoded_character;
        }

        is_separator = shell.ifs.wide_characters.contains(&wide_character);
        first_separator = shell.ifs.wide_characters[0];
    } else if multibyte_length == 0 {
        is_separator = shell.ifs.bytes.contains(&(wide_character as u8));
        first_separator = shell.ifs.bytes.first().copied().unwrap_or(0) as i32;
    }

    if is_separator {
        is_default_whitespace = shell.locale.wide_is_space(if wide_character != 0 {
            wide_character
        } else {
            first_separator
        });
    }
    IfsMembership {
        separator: is_separator,
        default_whitespace: is_default_whitespace,
    }
}

// [spec:dash:def:expand.ifsbreakup-slow-fn]
// [spec:dash:sem:expand.ifsbreakup-slow-fn]
fn split_fields_slow(
    shell: &Shell,
    split_state: &mut FieldSplitState,
    fields: &mut Vec<ExpandedField>,
    after_nul_region: bool,
    string: &[u8],
    mut cursor: usize,
) -> usize {
    let character: EncodedCharacterSpan;
    let is_default_whitespace: bool;
    let multibyte_length: usize;
    let is_separator: bool;
    let mut character_start: usize;

    character_start = cursor;

    character = next_encoded_character(string.get(cursor..).unwrap_or_default());
    cursor += character.prefix;
    multibyte_length = if character.remainder > 3 {
        character.remainder - 2
    } else {
        0
    };

    let membership = classify_ifs(
        shell,
        string.get(cursor..).unwrap_or_default(),
        multibyte_length,
        split_state.nul_only,
    );
    cursor += character.remainder;

    is_separator = membership.separator;
    is_default_whitespace = membership.default_whitespace;

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
    if matches!(split_state.max_fields, FieldLimit::Remaining(0)) {
        if is_default_whitespace {
            if split_state.trailing_whitespace_start.is_none() {
                split_state.trailing_whitespace_start = Some(character_start);
            }
            return cursor;
        }

        if !(is_separator && split_state.separator_is_whitespace) {
            split_state.trailing_whitespace_start = None;
        }
    } else if split_state.separator_is_whitespace {
        if is_separator {
            character_start = cursor;
        }

        split_state.field_start = character_start;

        if is_default_whitespace {
            return cursor;
        }
    } else if is_separator {
        let mut separator_is_whitespace = split_state.separator_is_whitespace;

        if !after_nul_region {
            separator_is_whitespace = is_default_whitespace;
            split_state.separator_is_whitespace = separator_is_whitespace;
        }

        /* Ignore IFS whitespace at start. */
        if character_start == split_state.field_start && separator_is_whitespace {
            split_state.field_start = cursor;
        } else {
            let last_field = match &mut split_state.max_fields {
                FieldLimit::Unlimited => false,
                FieldLimit::Remaining(remaining) => {
                    *remaining = remaining.saturating_sub(1);
                    *remaining == 0
                }
            };
            if last_field {
                split_state.trailing_whitespace_start = Some(character_start);
                return cursor;
            }
            fields.push(ExpandedField::from_bytes(
                &string[split_state.field_start..character_start],
            ));
            split_state.field_start = cursor;
            return cursor;
        }
    }

    split_state.separator_is_whitespace = false;
    cursor
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
fn split_regions_into_fields(
    shell: &Shell,
    regions: &[FieldSplitRegion],
    string: &[u8],
    max_fields: FieldLimit,
    fields: &mut Vec<ExpandedField>,
) {
    let mut region_index: usize;
    /* `struct ifs_state ifst;` and the three assignments the C makes
     * before the loop, as one initialiser. `mem::zeroed` was standing in
     * for the C leaving `ifs` and `ifsspc` unset here, and both are
     * assigned on every path that reads them; a struct without a pointer
     * in it can say so directly. */
    let mut split_state = FieldSplitState {
        nul_only: false,
        field_start: 0,
        trailing_whitespace_start: None,
        max_fields,
        separator_is_whitespace: false,
    };
    let mut nul_only: bool;
    let mut cursor: usize;
    let mut preserve_nul_field = false;
    let mut final_end = string.len();

    if !regions.is_empty() {
        split_state.separator_is_whitespace = false;
        nul_only = false;
        /* `realifs = ifsset() ? ncifs : nullstr` is gone with the
         * pointer it cached: `ifsisifs` reads `IFS` off the shell,
         * and what it needs from here is the one bit below. */
        region_index = 0;
        loop {
            let after_nul_region: bool;
            let end = regions[region_index].end;

            cursor = regions[region_index].start;
            debug_assert!(
                end <= string.len(),
                "a recorded region ends past the word it was recorded in"
            );
            after_nul_region = nul_only;
            nul_only = regions[region_index].nul_only;
            split_state.nul_only = nul_only;
            split_state.separator_is_whitespace = false;
            loop {
                let scan_start = cursor;

                /* `stackblock() + endoff - p >= 8` — eight bytes of
                 * this region left to look at.  As offsets it is also
                 * the bound that makes the load below a checked one. */
                while end >= cursor + 8 {
                    /* union { uint64_t qw; unsigned char b[8]; } x; */
                    let chunk_bytes: [u8; 8] = string[cursor..cursor + 8].try_into().unwrap();
                    let chunk_bits = u64::from_ne_bytes(chunk_bytes);

                    if (chunk_bits & 0x8080808080808080) != 0 {
                        break;
                    }
                    if chunk_bytes
                        .iter()
                        .any(|byte| shell.ifs.ascii_membership[*byte as usize])
                    {
                        break;
                    }
                    cursor += 8;
                }

                if cursor != scan_start {
                    if matches!(split_state.max_fields, FieldLimit::Remaining(0)) {
                        split_state.trailing_whitespace_start = None;
                    } else if split_state.separator_is_whitespace {
                        split_state.field_start = scan_start;
                    }
                    split_state.separator_is_whitespace = false;
                }

                if cursor >= end {
                    break;
                }

                cursor = split_fields_slow(
                    shell,
                    &mut split_state,
                    fields,
                    after_nul_region || nul_only,
                    string,
                    cursor,
                );
            }

            region_index += 1;
            if region_index >= regions.len() {
                break;
            }
        }
        if nul_only {
            preserve_nul_field = true;
        } else if let Some(trailing_whitespace_start) = split_state.trailing_whitespace_start {
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
                trailing_whitespace_start >= split_state.field_start,
                "the trailing-IFS truncation lands in an already-taken field"
            );
            final_end = trailing_whitespace_start;
        }
    }

    if !preserve_nul_field && split_state.field_start >= final_end {
        return;
    }

    fields.push(ExpandedField::from_bytes(
        &string[split_state.field_start..final_end],
    ));
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
pub fn split_fields(
    shell: &Shell,
    string: &[u8],
    max_fields: usize,
    expanded_fields: &mut ExpandedFields,
) {
    split_regions_into_fields(
        shell,
        &shell.expand.ifs_regions,
        string,
        FieldLimit::Remaining(max_fields),
        &mut expanded_fields.fields,
    );
}

// [spec:dash:def:expand.ifsfree-fn]
// [spec:dash:sem:expand.ifsfree-fn]
pub(crate) fn clear_split_regions(state: &mut ExpandState) {
    /* Emptying the owned region list replaces freeing the C chain and
     * nulling its tail pointer. */
    if split_regions(state).len() > 1 {
        split_regions(state).truncate(1);
    }
    split_regions(state).clear();
}

// [spec:dash:def:expand.changeifs-fn]
// [spec:dash:sem:expand.changeifs-fn]
pub fn update_ifs_cache(shell: &mut crate::context::Shell, ifs: &BStr) {
    let mut has_multibyte = false;
    shell.ifs.bytes = ifs.to_owned();

    shell.ifs.ascii_membership = [false; 128];

    let byte_length = shell.ifs.bytes.len();
    for &byte in shell.ifs.bytes.iter() {
        has_multibyte |= !byte.is_ascii();
        if byte.is_ascii() {
            shell.ifs.ascii_membership[usize::from(byte)] = true;
        }
    }

    shell.ifs.first_character_length = usize::from(!shell.ifs.bytes.is_empty());
    shell.ifs.wide_characters = if !has_multibyte {
        Vec::new()
    } else {
        let (first_character_length, wide_characters) =
            shell.locale.wide_chars(&shell.ifs.bytes[..byte_length]);
        shell.ifs.first_character_length = first_character_length;
        wide_characters
    };
}

/*
 * Expand shell metacharacters.  At this point, the only control characters
 * should be escapes.  The results are stored in the list exparg.
 */

/* The shell's byte-preserving glob implementation is the only supported
 * pathname-expansion engine.
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
fn expand_pathnames(
    shell: &mut crate::context::Shell,
    state: &mut ExpandState,
    words: Vec<ExpandedField>,
) -> Result<(), Error> {
    /* TODO - EXP_REDIR */

    /* The C's `preglob(..., RMESCAPE_HEAP)` result: one `ckmalloc` per
     * word, `ckfree`d as soon as `expmeta` has read it.  That is a local
     * buffer's lifetime exactly, and reusing it across the loop is the
     * only difference — `expmeta` never re-enters `preglob`, because the
     * only `preglob` under it is `patmatch`'s and does not allocate. */
    let mut pattern: BString = BString::new(Vec::new());

    /* The glob buffer, owned here and lent to `expmeta`.  One allocation
     * per `expandmeta` that globs anything, reused across the word loop
     * exactly as the region's block was; see the comment above
     * [`expmeta`]'s neighbours for why it stopped being a `static`. */
    let mut pathname_buffer = BString::new(Vec::new());

    for mut field in words {
        let text = field.as_bstr();
        let has_meta = !shell.options.enabled(ShellOption::NoGlob)
            && text.find_byteset(b"*?]").is_some()
            && text != b"]";
        if has_meta {
            /* `savelastp = exparg.lastp` — where this word's matches
             * will start, so that the sort below covers them and not
             * the words already in the list. */
            let first_match_index = expansion_fields(state).len();

            crate::error::with_interrupts_deferred(shell, |shell| {
                pattern.clear();
                pattern.extend_from_slice(text);
                let pattern_len = remove_escapes_in_buffer(&mut pattern, EscapeMode::Glob);
                pattern.truncate(pattern_len);

                /* The C's top-level `expmeta` starts on whatever block the
                 * region is on and gets away with it because `expdir_len`
                 * is 0: it writes from the base and never reads what was
                 * there. An owned buffer's length is not 0 — the previous
                 * glob's `addfnamealt` left it at that glob's `expdir_len`
                 * — and every consequence of carrying it in is benign,
                 * which is the reason to clear rather than to argue. */
                pathname_buffer.clear();
                expand_pathname_component(&shell.locale, state, &mut pathname_buffer, &pattern, 0);
            });
            if expansion_fields(state).len() != first_match_index {
                /* `*exparg.lastp = NULL; sp = expsort(*savelastp);
                 * *savelastp = sp; while (sp->next) sp = sp->next;
                 * exparg.lastp = &sp->next;` — terminate the run this
                 * word added, sort it, splice it back and walk to its
                 * new end.  Three of those four exist to re-find the
                 * tail of a list the sort reordered; a slice's tail
                 * does not move. */
                sort_fields(
                    &shell.locale,
                    &mut expansion_fields(state)[first_match_index..],
                );
                continue;
            }
        }
        field.remove_escapes();
        expansion_fields(state).push(field);
    }
    Ok(())
}

// [spec:dash:def:expand.addfname-common-fn]
// [spec:dash:sem:expand.addfname-common-fn]
fn add_pathname(state: &mut ExpandState, name: BString) {
    expansion_fields(state).push(ExpandedField { text: name });
}

// [spec:dash:def:expand.addfnamealt-fn]
// [spec:dash:sem:expand.addfnamealt-fn]
fn add_literal_pathname(
    state: &mut ExpandState,
    path_buffer: &mut BString,
    directory_prefix_length: usize,
) {
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
    add_pathname(state, BString::from(path_buffer.to_vec()));

    /* `STARTSTACKSTR(enddir); return stnputs(name, expdir_len, enddir) -
     * expdir_len;` — the C has to start a new block and copy the directory
     * prefix back into it, because `grabstackstr` gave the old one away.
     * Nothing was given away here, so the prefix is still the first
     * `expdir_len` bytes and re-seeding is `truncate`. */
    path_buffer.truncate(directory_prefix_length);
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
fn remove_pathname_escapes(path: &mut BString, name: &[u8]) {
    let appended_start = path.len();
    /* The bytes use nsh's internal escaping, so copy and compact them in
     * place. The transform only shortens the appended input. */
    path.extend_from_slice(name);
    let unescaped_length = remove_escapes_in_buffer(&mut path[appended_start..], EscapeMode::Plain);
    debug_assert!(unescaped_length <= name.len());
    path.truncate(appended_start + unescaped_length);
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
fn expand_pathname_component(
    locale: &nsh_platform::Locale,
    state: &mut ExpandState,
    path_buffer: &mut BString,
    name: &[u8],
    mut directory_prefix_length: usize,
) {
    let escape_marker = LEGACY_ESCAPE;
    let mut remainder_start: usize;
    let mut component_end: usize;
    let mut match_leading_dot: bool;
    let mut escape_count: usize;
    let component_start: usize;
    let component_pattern: &[u8];
    let mut cursor: usize;
    let following_byte: u8;
    /* Scratch for the encoded form of each directory entry; see the
     * `memtodest` call below.  A local rather than a static because
     * `expmeta` recurses, one frame per path component. */
    let mut encoded_entry: BString = BString::new(Vec::new());

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
    debug_assert_eq!(path_buffer.len(), directory_prefix_length);
    path_buffer.reserve(name.len() + 1);

    /* `for (;;) { p = strpbrk(p + esc, "*?]"); ... }` — find the
     * first metacharacter that is not itself escaped. */
    cursor = 0;
    escape_count = 0;
    let meta: Option<usize> = loop {
        let from = cursor + escape_count;
        let Some(at) = name[from..].find_byteset(b"*?]") else {
            break None;
        };
        cursor = from + at;
        escape_count = encoded_character_len(name, cursor, escape_marker) & 1;
        if escape_count == 0 {
            break Some(cursor);
        }
    };
    /* No meta characters */
    let Some(meta) = meta else {
        if directory_prefix_length == 0 {
            debug_assert_eq!(path_buffer.len(), directory_prefix_length);
            return;
        }
        remove_pathname_escapes(path_buffer, name);
        let exists = path_buffer
            .try_to_path_buf()
            .is_ok_and(|path| nsh_platform::path_metadata(&path, false).is_ok());
        if exists {
            add_literal_pathname(state, path_buffer, directory_prefix_length);
        } else {
            /* The C leaves its uncounted bytes where they are and
             * returns the base; counted bytes have to be rewound,
             * so that this frame returns with the buffer holding
             * its prefix and nothing else. */
            path_buffer.truncate(directory_prefix_length);
        }
        debug_assert_eq!(path_buffer.len(), directory_prefix_length);
        return;
    };
    match name[..meta].rfind_byte(SLASH as u8) {
        Some(at) => {
            /* `c = *start; *start = 0; expmeta_rmescapes(enddir,
             * name); *start = c;` — the C borrows the pattern as
             * the directory prefix by terminating it in place.  A
             * subslice is that without the write, and without the
             * restore. */
            component_start = at + 1;
            remove_pathname_escapes(path_buffer, &name[..component_start]);
            /* `expdir_len = enddir - cp` — this frame's prefix
             * grew by the unescaped directory part, and the bytes
             * it grew over are counted because they were
             * appended. */
            directory_prefix_length = path_buffer.len();
        }
        None => component_start = 0,
    }

    let directory = if directory_prefix_length != 0 {
        &path_buffer[..directory_prefix_length]
    } else {
        b"."
    };
    let Ok(directory) = directory.try_to_path_buf() else {
        debug_assert_eq!(path_buffer.len(), directory_prefix_length);
        return;
    };
    let Ok(entries) = nsh_platform::read_directory(&directory) else {
        debug_assert_eq!(path_buffer.len(), directory_prefix_length);
        return;
    };
    /* `p = strchrnul(p + 1, '/')` — the end of the component the
     * metacharacter is in.  The C's `esc = 0` before this is a
     * dead store in both languages: `esc` is read only inside the
     * branch that sets it. */
    cursor = name[meta + 1..]
        .find_byte(SLASH as u8)
        .map_or(name.len(), |at| meta + 1 + at);
    component_end = cursor;
    remainder_start = cursor;
    if cursor != name.len() {
        let delimiter_escape_count = encoded_character_len(name, cursor, escape_marker) & 1;
        component_end -= delimiter_escape_count;
        remainder_start += 1;
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
    following_byte = byte_at(name, component_end);
    match_leading_dot = false;
    component_pattern = &name[component_start..component_end];
    cursor = 0;
    if byte_at(component_pattern, cursor) == escape_marker {
        cursor += 1;
    }
    if byte_at(component_pattern, cursor) == DOT {
        match_leading_dot = true;
    }
    /* `read_dir` intentionally omits `.` and `..`; `readdir`
     * included both, so put them back before the native entries. */
    let synthetic = [(b".".to_vec(), true), (b"..".to_vec(), true)];
    let entries = synthetic.into_iter().chain(
        entries
            .into_iter()
            .map(|entry| (entry.name.to_shell_bytes(), entry.may_descend)),
    );
    for (entry_name, may_descend) in entries {
        let eligible = (entry_name[0] != DOT as u8 || match_leading_dot)
            && (following_byte == 0 || may_descend);
        if eligible {
            let entry_name: &[u8] = &entry_name;
            let entry_length = entry_name.len();
            /* Encode the directory entry as matcher input in separate
             * scratch storage. The candidate path itself stays raw and is
             * only appended after a successful match. */
            encoded_entry.clear();
            push_bytes(
                locale,
                entry_name,
                ExpansionMode::PRESERVE_MULTIBYTE,
                &mut encoded_entry,
            );
            let subject = encoded_entry.as_slice();
            if crate::pattern::pattern_matches(locale, component_pattern, subject) {
                /* `enddir = stnputs(dname, len, enddir)` — an
                 * append at a cursor below the end, which is
                 * truncate-then-append. */
                path_buffer.truncate(directory_prefix_length);
                path_buffer.extend_from_slice(entry_name);
                if following_byte == 0 {
                    add_literal_pathname(state, path_buffer, directory_prefix_length);
                } else {
                    path_buffer.push(SLASH as u8);
                    expand_pathname_component(
                        locale,
                        state,
                        path_buffer,
                        &name[remainder_start..],
                        directory_prefix_length + entry_length + 1,
                    );
                    /* `enddir = cp + expdir_len` — the frame's
                     * rewind, said out loud.  The child returns
                     * with the buffer holding *its* prefix, which
                     * is this one plus the component just
                     * appended. */
                    path_buffer.truncate(directory_prefix_length);
                }
            }
        }
        if interrupt_pending() {
            break;
        }
    }

    /* The C returns `cp`, the block's base, and every caller immediately
     * recomputes `cp + expdir_len`.  What that is really saying is a
     * postcondition, and it is this: on return the buffer holds this
     * frame's prefix and nothing above it.  `expdir_len` is the frame's
     * own, which may have grown past the caller's — hence the caller's
     * rewind after the recursive call. */
    debug_assert_eq!(path_buffer.len(), directory_prefix_length);
}

/*
 * Sort the results of file name expansion.  It calculates the number of
 * strings to sort and then calls msort (short for merge sort) to do the
 * work.
 */

// [spec:dash:def:expand.expsort-fn]
// [spec:dash:sem:expand.expsort-fn]
// [spec:posix:req:pattern.replacement-sorted]
fn sort_fields(locale: &nsh_platform::Locale, fields: &mut [ExpandedField]) {
    /* The C walks the chain to count it and hands the count to `msort`,
     * because a singly-linked list does not know its own length. */
    merge_sort_fields(locale, fields)
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
fn merge_sort_fields(locale: &nsh_platform::Locale, list: &mut [ExpandedField]) {
    if list.len() <= 1 {
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
/// `at` is the index of the first byte that needs quote removal. Returns
/// the length of the compacted result.
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
/// `_rmescapes`", together with the two reach-backs' safety argument.
// [spec:posix:syn:pattern.backslash-escape-with-shell-quoting]
// [spec:posix:syn:pattern.backslash-escape-without-shell-quoting]
// [spec:posix:req:pattern.escaping-follows-quoting-rules]
// [spec:posix:syn:pattern.trailing-backslash-unspecified]
// [spec:posix:req:pattern.quote-to-match-literally]
fn compact_escapes(buffer: &mut [u8], start: usize, mode: EscapeMode) -> usize {
    let globbing = mode == EscapeMode::Glob;
    let mut in_quotes = false;
    let mut not_escaped = globbing;
    /* The C's `p` and `q`, which are indices into one buffer here. */
    let mut read_index = start;
    let mut write_index = start;

    while read_index < buffer.len() {
        let mut character = byte_at(buffer, read_index);
        let mut newly_not_escaped = globbing;
        let span: EncodedCharacterSpan;
        let mut span_to_copy: usize;

        let copy_byte = if character == LEGACY_QUOTE {
            read_index += 1;
            in_quotes ^= globbing;
            continue;
        } else if character == BACKSLASH {
            /* naked back slash */
            newly_not_escaped ^= not_escaped;
            /* naked backslashes can only occur outside quotes */
            in_quotes = false;
            if not_escaped {
                character = LEGACY_ESCAPE;
            }
            true
        } else if character == LEGACY_ESCAPE {
            if !not_escaped && in_quotes {
                /* Reaches back one byte.  `notescaped` is cleared only by
                 * the naked-backslash arm, which writes a byte first, so
                 * `q` has advanced before this is reachable. */
                buffer[write_index - 1] = BACKSLASH;
            }
            if globbing {
                buffer[write_index] = LEGACY_ESCAPE;
                write_index += 1;
            }

            read_index += 1;
            character = byte_at(buffer, read_index);
            true
        } else if character == LEGACY_MULTIBYTE {
            let mut suffix = 2usize;

            if globbing ^ not_escaped {
                write_index -= 1;
            }

            span = next_encoded_character(buffer.get(read_index..).unwrap_or_default());
            span_to_copy = span.remainder;

            if !globbing {
                read_index += span.prefix;
                span_to_copy -= 2;
            } else {
                span_to_copy += span.prefix;
                suffix = 0;
            }

            /* `q` trails `p` through the same buffer, which
             * `copy_within` already knows -- it is the C's
             * `memmove`, bounds-checked. */
            buffer.copy_within(read_index..read_index + span_to_copy, write_index);
            write_index += span_to_copy;
            read_index += span_to_copy + suffix;
            false
        } else {
            true
        };

        if copy_byte {
            buffer[write_index] = character;
            write_index += 1;
            read_index += 1;
        }
        not_escaped = newly_not_escaped;
    }
    if globbing ^ not_escaped {
        /* The same reach-back, and the same argument. */
        buffer[write_index - 1] = BACKSLASH;
    }
    write_index
}

/// The index of the first byte `_rmescapes` has anything to do with, if
/// there is one.
///
/// The marker set is independent of byte value zero; the slice length stops
/// the scan.
fn first_escape_offset(bytes: &[u8]) -> Option<usize> {
    let markers = [
        BACKSLASH as u8,
        LEGACY_ESCAPE as u8,
        LEGACY_MULTIBYTE as u8,
        LEGACY_QUOTE as u8,
    ];
    bytes.find_byteset(&markers)
}

/// Apply quote removal to one owned byte buffer and return its new length.
fn remove_escapes_in_buffer(bytes: &mut [u8], mode: EscapeMode) -> usize {
    let Some(first_escape) = first_escape_offset(bytes) else {
        return bytes.len();
    };
    compact_escapes(bytes, first_escape, mode)
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
pub fn remove_escapes_from_offset(buffer: &mut BString, start: usize) -> usize {
    let remaining_length = buffer.len().saturating_sub(start);
    if first_escape_offset(&buffer[start..start + remaining_length]).is_none() {
        /* `return str` — before the block is grown, so the cursor is
         * untouched and the caller's `rmesc == startp` test sees it. */
        return start;
    }
    let relative_escape = first_escape_offset(&buffer[start..start + remaining_length])
        .expect("scanned once already");

    /* `r = makestrspace(fulllen); mempcpy(q, str, len)` — the destination
     * is the space past the cursor, and the source is below it in the same
     * buffer, which is exactly what `extend_from_within` is for. */
    let destination_start = buffer.len();
    buffer.extend_from_within(start..start + remaining_length);
    let compacted_length = compact_escapes(
        &mut buffer[destination_start..],
        relative_escape,
        EscapeMode::Plain,
    );
    buffer.truncate(destination_start + compacted_length);
    destination_start
}

/*
 * See if a pattern matches in a case statement.
 */

// [spec:dash:def:expand.casematch-fn]
// [spec:dash:sem:expand.casematch-fn]
pub fn case_pattern_matches(
    shell: &mut crate::context::Shell,
    pattern: &crate::nodes::Node,
    value: &BStr,
) -> Result<bool, Error> {
    let Node::Word(word) = pattern else {
        return Err(shell
            .diagnostics()
            .shell_error(b"case matching requires a word node"));
    };
    typed::case_matches(shell, &word.word, value)
}

/*
 * Our own itoa().
 */

// [spec:dash:def:expand.cvtnum-fn]
// [spec:dash:sem:expand.cvtnum-fn]
fn push_integer(
    locale: &nsh_platform::Locale,
    number: i64,
    mode: ExpansionMode,
    output: &mut BString,
) -> usize {
    let value = format!("{number}");
    push_bytes(locale, value.as_bytes(), mode, output)
}

// [spec:dash:def:expand.varunset-fn]
// [spec:dash:sem:expand.varunset-fn]
fn unset_parameter_error(
    shell: &mut crate::context::Shell,
    text: &[u8],
    end: usize,
    variable_name_start: usize,
    custom_message: Option<&[u8]>,
    null_is_unset: bool,
) -> Error {
    /* The C's three `char *` here are a NULL test and two `%s` arguments,
     * and every one of them is spent on the next five lines.  `nullstr` was
     * the empty tail and `msg` a string literal; as byte slices the
     * bounds are carried by each slice, so the two raw scans that used to
     * re-measure them are gone.  `umsg`'s `Option` is the
     * NULL test said as a type — its one non-null caller hands over the
     * expansion buffer's message, which is a slice at the call site rather
     * than a pointer here. */
    let mut tail: &[u8] = b"";
    let mut reason: &[u8] = b"parameter not set";
    if let Some(custom_message) = custom_message {
        if byte_at(text, end) == LEGACY_END_PARAMETER {
            if null_is_unset {
                tail = b" or null";
            }
        } else {
            reason = custom_message;
        }
    }
    /* `end - var - 1` — the variable's name, without the `=` the parser
     * writes after it.  Saturating because the C's subtraction is signed
     * and it clamped at zero. */
    let name_length = end.saturating_sub(variable_name_start + 1);
    let mut message = Vec::new();
    message.extend_from_slice(
        &text[variable_name_start..(variable_name_start + name_length).min(text.len())],
    );
    message.extend_from_slice(b": ");
    message.extend_from_slice(reason);
    message.extend_from_slice(tail);
    if shell.evaluation.expanding_trace_prompt {
        shell.diagnostics().shell_error(&message)
    } else {
        // [spec:nsh:req:compat.smoosh.error-contracts]
        shell.diagnostics().expansion_error_value(&message)
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
pub fn recover_expansion(
    shell: &mut crate::context::Shell,
    caught: Option<crate::error::Error>,
) -> Option<crate::error::Error> {
    match &caught {
        /* Not this frame's to keep, and never was: the C re-raised it
         * from here. */
        Some(error) if error.is_interrupt() => {}
        Some(_) => clear_split_regions(&mut shell.expand),
        None => {}
    }
    caught
}

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
pub use crate::arithmetic::evaluate;

// The `expcmd(int, char **)` declaration in `expand.h` has no definition
// or caller. It is intentionally represented by no Rust item.
// [spec:dash:def:expand.expcmd-fn]
// [spec:dash:sem:expand.expcmd-fn]
