//! Shell word expansion.
//!
//! Argument and case-pattern expansion is implemented by [`typed`] as
//! structural transformations over parsed word parts. The `read` builtin
//! shares the IFS classifier through a byte string plus a protection mask.

use crate::context::Shell;

use bstr::{BStr, BString};

use crate::error::Error;
use crate::nodes::Node;

mod bytes;
mod mode;
mod typed;

use bytes::at as byte_at;
pub(crate) use mode::ExpansionMode;

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
}

/// Mutable state for one field-splitting pass.
pub struct FieldSplitState {
    pub field_start: usize,
    /// Start of a trailing IFS run that should be removed.
    pub trailing_whitespace_start: Option<usize>,
    remaining_fields: usize,
    pub separator_is_whitespace: bool,
}

/// Owned intermediate buffers for one expansion.
pub(crate) struct ExpandState {
    buffer: BString,
}

impl ExpandState {
    pub(crate) const fn new() -> Self {
        Self {
            buffer: BString::new(Vec::new()),
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

/// The most recent unsplit expansion result.
///
/// Here-document and prompt expansion read these length-delimited bytes
/// immediately after expansion and before another expansion can replace them.
pub fn expansion_result(shell: &crate::context::Shell) -> &BStr {
    BStr::new(shell.expand.buffer.as_slice())
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

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
struct IfsMembership {
    separator: bool,
    default_whitespace: bool,
}

// [spec:dash:sem:expand.ifsisifs-fn]
fn classify_ifs(shell: &Shell, bytes: &[u8], multibyte_length: usize) -> IfsMembership {
    let mut is_default_whitespace = false;
    let mut is_separator = false;
    let mut wide_character = byte_at(bytes, 0) as i32;
    let mut first_separator = None;

    if !shell.ifs.bytes.is_empty() && !shell.ifs.wide_characters.is_empty() {
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
    } else if multibyte_length <= 1 {
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
// [spec:dash:sem:expand.mbnext-fn]
fn character_length(locale: &nsh_platform::Locale, bytes: &[u8]) -> usize {
    let Some(first) = bytes.first() else {
        return 0;
    };
    if first.is_ascii() {
        return 1;
    }

    let mut decoder = locale.decoder();
    for (index, byte) in bytes.iter().copied().take(16).enumerate() {
        match decoder.push(byte) {
            nsh_platform::LocaleDecode::Incomplete => {}
            nsh_platform::LocaleDecode::Complete(_) => return index + 1,
            nsh_platform::LocaleDecode::Invalid => return 1,
        }
    }
    1
}

fn split_fields_slow(
    shell: &Shell,
    split_state: &mut FieldSplitState,
    fields: &mut Vec<ExpandedField>,
    string: &[u8],
    mut cursor: usize,
) -> usize {
    let mut character_start = cursor;
    let multibyte_length =
        character_length(&shell.locale, string.get(cursor..).unwrap_or_default());

    let membership = classify_ifs(
        shell,
        string.get(cursor..).unwrap_or_default(),
        multibyte_length,
    );
    cursor += multibyte_length.max(1);

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
        let separator_is_whitespace = is_default_whitespace;
        split_state.separator_is_whitespace = separator_is_whitespace;

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
    /* `struct ifs_state ifst;` and the three assignments the C makes
     * before the loop, as one initialiser. `mem::zeroed` was standing in
     * for the C leaving `ifs` and `ifsspc` unset here, and both are
     * assigned on every path that reads them; a struct without a pointer
     * in it can say so directly. */
    let mut split_state = FieldSplitState {
        field_start: 0,
        trailing_whitespace_start: None,
        remaining_fields: max_fields,
        separator_is_whitespace: false,
    };
    let mut final_end = string.len();

    for region in regions {
        let end = region.end;
        let mut cursor = region.start;
        debug_assert!(
            end <= string.len(),
            "a recorded region ends past the word it was recorded in"
        );
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

            cursor = split_fields_slow(shell, &mut split_state, fields, string, cursor);
        }
    }
    if let Some(trailing_whitespace_start) = split_state.trailing_whitespace_start {
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

    if split_state.field_start >= final_end {
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
// [spec:dash:sem:expand.ifsfree-fn]
// [spec:dash:sem:expand.recordregion-fn]
pub fn split_fields(
    shell: &Shell,
    string: &[u8],
    protected: &[bool],
    max_fields: usize,
    expanded_fields: &mut ExpandedFields,
) {
    debug_assert_eq!(string.len(), protected.len());
    // Dash records split-eligible offsets in a process-global linked list and
    // clears it after each expansion. This interface derives the same regions
    // from the owned protection mask, and the local vector is dropped when the
    // split finishes.
    let mut regions = Vec::new();
    let mut start = None;
    for (index, protected) in protected.iter().copied().enumerate() {
        match (start, protected) {
            (None, false) => start = Some(index),
            (Some(region_start), true) => {
                regions.push(FieldSplitRegion {
                    start: region_start,
                    end: index,
                });
                start = None;
            }
            _ => {}
        }
    }
    if let Some(region_start) = start {
        regions.push(FieldSplitRegion {
            start: region_start,
            end: string.len(),
        });
    }
    split_regions_into_fields(
        shell,
        &regions,
        string,
        max_fields,
        &mut expanded_fields.fields,
    );
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
