//! `IFS` field splitting over expanded fields.
//!
//! Split eligibility lives beside each byte, so truncating a parallel linked
//! list of regions is no longer an operation the implementation can forget.

use bstr::BString;

use super::{Field, character_end, effective_ifs};
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
        let end = character_end(locale, ifs, at);
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
    locale: &nsh_platform::Locale,
    field: &Field,
    ifs: &'a [IfsCharacter],
    at: usize,
) -> Option<(&'a IfsCharacter, usize)> {
    let end = character_end(locale, &field.bytes, at);
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

    let mut result = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < field.bytes.len() {
        let Some((separator, mut next)) = separator_at(locale, &field, ifs, at) else {
            at = character_end(locale, &field.bytes, at);
            continue;
        };

        if separator.whitespace {
            next = skip_whitespace(locale, &field, ifs, next);
            let following_nonwhite = (next < field.bytes.len())
                .then(|| separator_at(locale, &field, ifs, next))
                .flatten()
                .filter(|(following, _)| !following.whitespace);
            if let Some((_, end)) = following_nonwhite {
                result.push(field.slice(start..at));
                start = skip_whitespace(locale, &field, ifs, end);
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
        start = skip_whitespace(locale, &field, ifs, next);
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
    locale: &nsh_platform::Locale,
    field: &Field,
    ifs: &[IfsCharacter],
    mut at: usize,
) -> usize {
    while at < field.bytes.len() {
        let Some((following, end)) = separator_at(locale, field, ifs, at) else {
            break;
        };
        if !following.whitespace {
            break;
        }
        at = end;
    }
    at
}
