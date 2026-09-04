//! `IFS` field splitting over expanded fields.
//!
//! Split eligibility lives beside each byte, so truncating a parallel linked
//! list of regions is no longer an operation the implementation can forget.

use super::Field;
use crate::characters::Characters;
use crate::context::Shell;
use crate::expand::IfsCharacter;

// [spec:dash:sem:expand.removerecordregions-fn]
pub(super) fn fields(shell: &Shell, fields: Vec<Field>) -> Vec<Field> {
    /* Read rather than derived: what `IFS` names changes when `IFS` or
     * the locale is assigned, and `update_ifs_cache` is where both of
     * those arrive. */
    let separators = &shell.ifs.separators;
    fields
        .into_iter()
        .flat_map(|field| field_into_fields(&shell.locale, field, separators))
        .collect()
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
