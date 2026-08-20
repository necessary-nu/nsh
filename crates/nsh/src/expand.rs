//! Shell word expansion.
//!
//! Active argument and case-pattern expansion is implemented by [`typed`]
//! as structural transformations over parsed word parts. The remaining
//! translated helpers in this file serve compatibility call sites such as
//! `read` field splitting; later cleanup leaves remove that inactive port
//! machinery once its callers have typed interfaces of their own.

use crate::context::Shell;

use bstr::{BStr, BString, ByteSlice};

use crate::error::Error;
use crate::nodes::Node;

mod bytes;
mod mode;
mod typed;

use bytes::at as byte_at;
pub(crate) use mode::ExpansionMode;

const LEGACY_ESCAPE: u8 = crate::parser::LEGACY_ESCAPE;
const LEGACY_MULTIBYTE: u8 = crate::parser::LEGACY_MULTIBYTE;
const LEGACY_QUOTE: u8 = crate::parser::LEGACY_QUOTE;
const BACKSLASH: u8 = b'\\';

// ---------------------------------------------------------------------
// src/expand.h
// ---------------------------------------------------------------------

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
    // [spec:posix:req:expand.quote-removal]
    // [spec:posix:sem:expand.quote-removal-quoting-remembered]
    // [spec:dash:sem:expand.rmescapes-fn]
    pub fn remove_escapes(&mut self) {
        let unescaped_length = remove_escapes_in_buffer(&mut self.text);
        self.text.truncate(unescaped_length);
    }
}

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

/// A byte range eligible for field splitting.
pub struct FieldSplitRegion {
    pub start: usize,
    pub end: usize,
    pub nul_only: bool,
}

/// Mutable state for one field-splitting pass.
pub struct FieldSplitState {
    pub nul_only: bool,
    pub field_start: usize,
    /// Start of a trailing IFS run that should be removed.
    pub trailing_whitespace_start: Option<usize>,
    remaining_fields: usize,
    pub separator_is_whitespace: bool,
}

/// Owned intermediate buffers for one expansion.
pub(crate) struct ExpandState {
    buffer: BString,
    ifs_regions: Vec<FieldSplitRegion>,
}

impl ExpandState {
    pub(crate) const fn new() -> Self {
        Self {
            buffer: BString::new(Vec::new()),
            ifs_regions: Vec::new(),
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

/// The most recent unsplit expansion result.
///
/// Here-document and prompt expansion read these length-delimited bytes
/// immediately after expansion and before another expansion can replace them.
pub fn expansion_result(shell: &crate::context::Shell) -> &BStr {
    BStr::new(shell.expand.buffer.as_slice())
}

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

// [spec:dash:sem:expand.getpwhome-fn]
/*
 * Perform variable substitution and command substitution on an argument,
 * placing the resulting list of arguments in arglist.  If EXP_FULL is true,
 * perform splitting and file name expansion.  When arglist is NULL, perform
 * here document expansion.
 */

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

/*
 * Record the fact that we have to scan this region of the
 * string for IFS characters.
 */

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
    // [spec:nsh:sem:idiom.specified-defects+1]
    // NUL-only splitting has no first IFS character. Represent that absence
    // instead of reading the reference's uninitialized `ifs0` slot.
    let mut first_separator = None;

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
        first_separator = shell.ifs.wide_characters.first().copied();
    } else if multibyte_length == 0 {
        is_separator = shell.ifs.bytes.contains(&(wide_character as u8));
        first_separator = shell.ifs.bytes.first().copied().map(i32::from);
    }

    if is_separator {
        is_default_whitespace = shell.locale.wide_is_space(if wide_character != 0 {
            wide_character
        } else {
            first_separator.unwrap_or(wide_character)
        });
    }
    IfsMembership {
        separator: is_separator,
        default_whitespace: is_default_whitespace,
    }
}

// [spec:dash:sem:expand.ifsbreakup-slow-fn]
fn split_fields_slow(
    shell: &Shell,
    split_state: &mut FieldSplitState,
    fields: &mut Vec<ExpandedField>,
    after_nul_region: bool,
    string: &[u8],
    mut cursor: usize,
) -> usize {
    let mut character_start = cursor;
    let character = next_encoded_character(string.get(cursor..).unwrap_or_default());
    cursor += character.prefix;
    let multibyte_length = if character.remainder > 3 {
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

    let is_separator = membership.separator;
    let is_default_whitespace = membership.default_whitespace;

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
    if split_state.remaining_fields == 0 {
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
            split_state.remaining_fields = split_state.remaining_fields.saturating_sub(1);
            let last_field = split_state.remaining_fields == 0;
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

// [spec:dash:sem:expand.ifsbreakup-fn]
fn split_regions_into_fields(
    shell: &Shell,
    regions: &[FieldSplitRegion],
    string: &[u8],
    max_fields: usize,
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
        remaining_fields: max_fields,
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
            let end = regions[region_index].end;

            cursor = regions[region_index].start;
            debug_assert!(
                end <= string.len(),
                "a recorded region ends past the word it was recorded in"
            );
            let after_nul_region = nul_only;
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
                    if split_state.remaining_fields == 0 {
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
        max_fields,
        &mut expanded_fields.fields,
    );
}

// [spec:dash:sem:expand.ifsfree-fn]
pub(crate) fn clear_split_regions(state: &mut ExpandState) {
    /* Emptying the owned region list replaces freeing the C chain and
     * nulling its tail pointer. */
    if split_regions(state).len() > 1 {
        split_regions(state).truncate(1);
    }
    split_regions(state).clear();
}

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
 * Remove any CTLESC characters from a string.
 */

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
fn compact_escapes(buffer: &mut [u8], start: usize) -> usize {
    let mut read_index = start;
    let mut write_index = start;

    while read_index < buffer.len() {
        let mut character = byte_at(buffer, read_index);

        let copy_byte = if character == LEGACY_QUOTE {
            read_index += 1;
            continue;
        } else if character == LEGACY_ESCAPE {
            read_index += 1;
            character = byte_at(buffer, read_index);
            true
        } else if character == LEGACY_MULTIBYTE {
            let span = next_encoded_character(buffer.get(read_index..).unwrap_or_default());
            read_index += span.prefix;
            let span_to_copy = span.remainder - 2;

            buffer.copy_within(read_index..read_index + span_to_copy, write_index);
            write_index += span_to_copy;
            read_index += span_to_copy + 2;
            false
        } else {
            true
        };

        if copy_byte {
            buffer[write_index] = character;
            write_index += 1;
            read_index += 1;
        }
    }
    write_index
}

/// The index of the first byte `_rmescapes` has anything to do with, if
/// there is one.
///
/// The marker set is independent of byte value zero; the slice length stops
/// the scan.
fn first_escape_offset(bytes: &[u8]) -> Option<usize> {
    let markers = [BACKSLASH, LEGACY_ESCAPE, LEGACY_MULTIBYTE, LEGACY_QUOTE];
    bytes.find_byteset(markers)
}

/// Apply quote removal to one owned byte buffer and return its new length.
fn remove_escapes_in_buffer(bytes: &mut [u8]) -> usize {
    let Some(first_escape) = first_escape_offset(bytes) else {
        return bytes.len();
    };
    compact_escapes(bytes, first_escape)
}

/*
 * See if a pattern matches in a case statement.
 */

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
