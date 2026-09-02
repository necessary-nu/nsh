//! `IFS` field splitting over expanded fields.
//!
//! Split eligibility lives beside each byte, so truncating a parallel linked
//! list of regions is no longer an operation the implementation can forget.

use bstr::BString;

use super::{Field, effective_ifs};
use crate::characters::{Characters, width};
use crate::context::Shell;

// [spec:dash:sem:expand.removerecordregions-fn]
pub(super) fn fields(shell: &Shell, fields: Vec<Field>) -> Vec<Field> {
    let separators = separators(&shell.locale, effective_ifs(shell));
    fields
        .into_iter()
        .flat_map(|field| field_into_fields(&shell.locale, field, &separators))
        .collect()
}

struct IfsCharacter {
    bytes: BString,
    whitespace: bool,
}

fn separators(locale: &nsh_platform::Locale, ifs: &[u8]) -> Vec<IfsCharacter> {
    let mut result = Vec::new();
    let mut at = 0;
    while at < ifs.len() {
        let end = at + width(locale, &ifs[at..]);
        let bytes = BString::from(&ifs[at..end]);
        let whitespace = locale
            .decode_exact(&bytes, bytes.len())
            .is_some_and(|wide| locale.wide_is_space(wide));
        result.push(IfsCharacter { bytes, whitespace });
        at = end;
    }
    result
}

fn separator_at<'a>(
    characters: &mut Characters<'_>,
    field: &Field,
    ifs: &'a [IfsCharacter],
    at: usize,
) -> Option<(&'a IfsCharacter, usize)> {
    let end = characters.end(at);
    field
        .range_is_splittable(at..end)
        .then(|| {
            ifs.iter()
                .find(|separator| separator.bytes.as_slice() == &field.bytes[at..end])
                .map(|separator| (separator, end))
        })
        .flatten()
}

fn field_into_fields(
    locale: &nsh_platform::Locale,
    field: Field,
    ifs: &[IfsCharacter],
) -> Vec<Field> {
    if ifs.is_empty() || !field.any_splittable() {
        return if field.bytes.is_empty() && !field.has_empty_anchor(0..=0) {
            Vec::new()
        } else {
            vec![field]
        };
    }

    /* One width table for the whole field: `separator_at` asks where the
     * character at an offset ends, and the walk asks the same question
     * again at every offset that is not a separator. */
    let mut characters = Characters::of(locale, &field.bytes);
    let mut result = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < field.bytes.len() {
        let Some((separator, mut next)) = separator_at(&mut characters, &field, ifs, at) else {
            at = characters.end(at);
            continue;
        };

        if separator.whitespace {
            next = skip_whitespace(&mut characters, &field, ifs, next);
            let following_nonwhite = (next < field.bytes.len())
                .then(|| separator_at(&mut characters, &field, ifs, next))
                .flatten()
                .filter(|(following, _)| !following.whitespace);
            if let Some((_, end)) = following_nonwhite {
                result.push(field.slice(start..at));
                start = skip_whitespace(&mut characters, &field, ifs, end);
                next = start;
            } else if at > start || field.has_empty_anchor(start..=at) {
                result.push(field.slice(start..at));
                start = next;
            } else {
                start = next;
            }
            at = next;
            continue;
        }

        result.push(field.slice(start..at));
        start = skip_whitespace(&mut characters, &field, ifs, next);
        at = start;
    }

    if start < field.bytes.len() {
        result.push(field.slice(start..field.bytes.len()));
    } else if field.has_empty_anchor(start..=field.bytes.len()) {
        let mut empty = Field::default();
        empty.anchor_empty();
        result.push(empty);
    }
    result
}

fn skip_whitespace(
    characters: &mut Characters<'_>,
    field: &Field,
    ifs: &[IfsCharacter],
    mut at: usize,
) -> usize {
    while at < field.bytes.len() {
        let Some((following, end)) = separator_at(characters, field, ifs, at) else {
            break;
        };
        if !following.whitespace {
            break;
        }
        at = end;
    }
    at
}
