//! Bash-only parameter expansions and the pattern options they read.
//!
//! Every entry point here is reached only from Bash mode: the parser does
//! not produce these operations otherwise, and the option lookups return
//! the all-off value in POSIX mode.

use bstr::{BStr, BString, ByteSlice as _};

use super::{Context, Expansion, Field, Value, character_end, expand_parts, value_expansion};
use crate::context::Shell;
use crate::error::Error;
use crate::options::{BashShopt, Dialect};
use crate::pattern::{Pattern, PatternOptions};
use crate::word::{
    ParameterExpansion, ParameterOperation, ParsedWord, QuoteBoundary, WordPart, WordUnit,
};

/// Pattern options for `case` and `[[ … ]]`, which read `nocasematch`.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn match_options(shell: &Shell) -> PatternOptions {
    if shell.options.dialect() != Dialect::Bash {
        return PatternOptions::NONE;
    }
    PatternOptions {
        extended: shell.options.shopt(BashShopt::ExtGlob),
        ignore_case: shell.options.shopt(BashShopt::NoCaseMatch),
    }
}

/// Pattern options for the parameter operators, which honour `extglob`
/// but never fold case.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn trim_options(shell: &Shell) -> PatternOptions {
    if shell.options.dialect() != Dialect::Bash {
        return PatternOptions::NONE;
    }
    PatternOptions {
        extended: shell.options.shopt(BashShopt::ExtGlob),
        ignore_case: false,
    }
}

/// The `${!…}` forms that name variables rather than read one.
///
/// `${!prefix@}` and `${!prefix*}` list every variable whose name starts
/// with the prefix; `${!name[@]}` lists the subscripts of an array. Both
/// answer with a word list, so neither reaches the value operators.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn indirect_names(
    shell: &mut Shell,
    name: &BStr,
    context: Context,
) -> Result<Option<Expansion>, Error> {
    if let Some((base, subscript)) = super::split_subscript(name)
        && matches!(subscript.as_ref() as &[u8], b"@" | b"*")
    {
        let base = base.to_owned();
        let keys = crate::variables::value::variable_value(shell, base.as_bstr())
            .map(crate::variables::arrays::keys)
            .unwrap_or_default();
        return words(shell, name, keys, subscript == "@", context).map(Some);
    }
    let selector = name.last().copied();
    if !matches!(selector, Some(b'@') | Some(b'*')) || name.len() < 2 {
        return Ok(None);
    }
    let prefix = name[..name.len() - 1].to_vec();
    /* Every name that has an entry, not only every name with a scalar
     * in it: `hello=()` declares `hello` and Bash lists it here even
     * though there is no element to read. */
    let mut names = crate::variables::value::declared_names(shell)
        .into_iter()
        .filter(|name| name.starts_with(&prefix))
        .collect::<Vec<_>>();
    names.sort();
    words(shell, name, names, selector == Some(b'@'), context).map(Some)
}

fn words(
    shell: &mut Shell,
    name: &BStr,
    words: Vec<BString>,
    separate: bool,
    context: Context,
) -> Result<Expansion, Error> {
    let value = if separate {
        Value::At(words)
    } else {
        Value::Star(words)
    };
    value_expansion(shell, name, value, context)
}

/// `${name:offset:length}` over a string, the positional parameters, or
/// an array.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn substring(
    shell: &mut Shell,
    parameter: &ParameterExpansion,
    name: &BStr,
    value: Value,
    context: Context,
) -> Result<Expansion, Error> {
    let units = operand_units(parameter);
    let cut = boundary_at(&units, b':');
    let (offset_units, length_units) = match cut {
        Some(at) => (&units[..at], Some(&units[at + 1..])),
        None => (units.as_slice(), None),
    };
    let offset = arithmetic_operand(shell, offset_units, context)?;
    let length = match length_units {
        Some(units) => Some(arithmetic_operand(shell, units, context)?),
        None => None,
    };

    match value {
        Value::At(elements) | Value::Star(elements) => {
            let positional = matches!(name.as_ref() as &[u8], b"@" | b"*");
            let mut all = Vec::new();
            if positional {
                all.push(
                    shell
                        .options
                        .argument_zero()
                        .map(BStr::to_owned)
                        .unwrap_or_default(),
                );
            }
            all.extend(elements);
            let selected = select(all, offset, length);
            let star = name.last() == Some(&b'*') || matches!(name.as_ref() as &[u8], b"*");
            words(shell, name, selected, !star, context)
        }
        value => {
            let text = super::value_bytes(shell, value, context);
            let sliced = slice_characters(&shell.locale, &text, offset, length);
            Ok(Expansion::one(Field::from_bytes(
                &sliced,
                context.protects(),
                context.splits(),
                context.quoted,
            )))
        }
    }
}

/// Clamp one offset/length pair onto a list of elements. A negative
/// offset counts from the end, and a negative length is an end position
/// rather than a count.
fn select(elements: Vec<BString>, offset: i64, length: Option<i64>) -> Vec<BString> {
    let total = elements.len() as i64;
    let start = if offset < 0 {
        (total + offset).max(0)
    } else {
        offset.min(total)
    };
    let end = match length {
        None => total,
        Some(length) if length < 0 => (total + length).max(start),
        Some(length) => (start + length).min(total),
    };
    elements
        .into_iter()
        .skip(start as usize)
        .take((end - start).max(0) as usize)
        .collect()
}

fn slice_characters(
    locale: &nsh_platform::Locale,
    text: &[u8],
    offset: i64,
    length: Option<i64>,
) -> BString {
    let boundaries = super::character_boundaries(locale, text);
    let count = boundaries.len() as i64 - 1;
    let start = if offset < 0 {
        (count + offset).max(0)
    } else {
        offset.min(count)
    };
    let end = match length {
        None => count,
        Some(length) if length < 0 => (count + length).max(start),
        Some(length) => (start + length).min(count),
    };
    BString::from(&text[boundaries[start as usize]..boundaries[end.max(start) as usize]])
}

/// `${name/pattern/replacement}` and its global, anchored, and
/// replacement-less spellings.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn substitute(
    shell: &mut Shell,
    parameter: &ParameterExpansion,
    name: &BStr,
    value: Value,
    context: Context,
) -> Result<Expansion, Error> {
    let units = operand_units(parameter);
    // The first byte of the pattern is always literal, so `${x///}`
    // replaces a slash rather than naming an empty pattern.
    let cut = separator_at(&units, b'/');
    let (mut pattern_units, replacement_units) = match cut {
        Some(at) => (&units[..at], &units[at + 1..]),
        None => (units.as_slice(), &units[units.len()..]),
    };
    let anchor = match pattern_units.first() {
        Some(WordUnit::Literal(b'#')) => Anchor::Start,
        Some(WordUnit::Literal(b'%')) => Anchor::End,
        _ => Anchor::None,
    };
    if anchor != Anchor::None {
        pattern_units = &pattern_units[1..];
    }
    let pattern =
        expand_units(shell, pattern_units, context.pattern_operand())?.pattern(trim_options(shell));
    let replacement = expand_units(shell, replacement_units, context.operand())?.bytes;
    let all = parameter.operation == ParameterOperation::SubstituteAll;

    if pattern.as_bytes().is_empty() {
        return value_expansion(shell, name, value, context);
    }
    map_value(shell, value, context, |locale, text| {
        replace(locale, text, &pattern, &replacement, all, anchor)
    })
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum Anchor {
    None,
    Start,
    End,
}

fn replace(
    locale: &nsh_platform::Locale,
    text: &[u8],
    pattern: &Pattern,
    replacement: &[u8],
    all: bool,
    anchor: Anchor,
) -> BString {
    let boundaries = super::character_boundaries(locale, text);
    let mut result = BString::new(Vec::new());
    if anchor == Anchor::End {
        let Some(start) = boundaries
            .iter()
            .copied()
            .find(|start| pattern.matches(locale, &text[*start..]))
        else {
            return BString::from(text);
        };
        result.extend_from_slice(&text[..start]);
        result.extend_from_slice(replacement);
        return result;
    }

    let mut at = 0;
    let mut replaced = false;
    while at < text.len() {
        let matched = (!replaced || all)
            .then(|| longest_match(locale, &boundaries, text, pattern, at))
            .flatten();
        match matched {
            Some(end) if end > at => {
                result.extend_from_slice(replacement);
                at = end;
                replaced = true;
            }
            Some(_) | None => {
                let next = character_end(locale, text, at);
                result.extend_from_slice(&text[at..next]);
                at = next;
            }
        }
        if anchor == Anchor::Start && !all {
            break;
        }
    }
    if replaced && at == 0 && text.is_empty() {
        return result;
    }
    result.extend_from_slice(&text[at.min(text.len())..]);
    result
}

fn longest_match(
    locale: &nsh_platform::Locale,
    boundaries: &[usize],
    text: &[u8],
    pattern: &Pattern,
    at: usize,
) -> Option<usize> {
    boundaries
        .iter()
        .copied()
        .rev()
        .filter(|end| *end >= at)
        .find(|end| pattern.matches(locale, &text[at..*end]))
}

/// `${name^pattern}`, `${name^^pattern}`, `${name,pattern}`, and
/// `${name,,pattern}`.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn change_case(
    shell: &mut Shell,
    parameter: &ParameterExpansion,
    value: Value,
    context: Context,
) -> Result<Expansion, Error> {
    let units = operand_units(parameter);
    let pattern = if units.is_empty() {
        Pattern::unquoted(BString::from("?"))
    } else {
        expand_units(shell, &units, context.pattern_operand())?.pattern(trim_options(shell))
    };
    let upper = matches!(
        parameter.operation,
        ParameterOperation::UpperFirst | ParameterOperation::UpperAll
    );
    let every = matches!(
        parameter.operation,
        ParameterOperation::UpperAll | ParameterOperation::LowerAll
    );
    map_value(shell, value, context, |locale, text| {
        recase(locale, text, &pattern, upper, every)
    })
}

fn recase(
    locale: &nsh_platform::Locale,
    text: &[u8],
    pattern: &Pattern,
    upper: bool,
    every: bool,
) -> BString {
    let mut result = BString::new(Vec::new());
    let mut at = 0;
    let mut first = true;
    while at < text.len() {
        let end = character_end(locale, text, at);
        let character = &text[at..end];
        if (every || first) && pattern.matches(locale, character) {
            result.extend_from_slice(&map_case(character, upper));
        } else {
            result.extend_from_slice(character);
        }
        first = false;
        at = end;
    }
    result
}

/// Map one character's case. A single byte follows ASCII rules, which is
/// all the C locale has; a complete UTF-8 character follows Unicode's
/// simple mappings.
fn map_case(character: &[u8], upper: bool) -> Vec<u8> {
    if character.len() == 1 {
        return vec![if upper {
            character[0].to_ascii_uppercase()
        } else {
            character[0].to_ascii_lowercase()
        }];
    }
    match core::str::from_utf8(character) {
        Ok(text) if upper => text.to_uppercase().into_bytes(),
        Ok(text) => text.to_lowercase().into_bytes(),
        Err(_) => character.to_vec(),
    }
}

/// `${name@operator}`, the nullary transformations.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn transform(
    shell: &mut Shell,
    parameter: &ParameterExpansion,
    name: &BStr,
    value: Value,
    context: Context,
) -> Result<Expansion, Error> {
    let operator = expand_units(shell, &operand_units(parameter), context.operand())?.bytes;
    /* A name with no value has nothing to transform, and Bash says so by
     * producing nothing at all -- not `''`, which is what quoting an
     * empty value would give and what `${empty@Q}` does produce. */
    if value.is_unset() {
        return Ok(Expansion::one(Field::from_bytes(
            b"",
            context.protects(),
            context.splits(),
            context.quoted,
        )));
    }
    match operator.as_slice() {
        /* `@a` reads the variable's declaration rather than its bytes.
         * It still maps over the value: every element shares the
         * array's attributes, so `${a[@]@a}` is one `a` per element
         * rather than one for the array. */
        b"a" => {
            let letters = attributes_of(shell, name);
            map_value(shell, value, context, |_, _| letters.clone())
        }
        /* `@A` prints the assignment that would recreate the name, so
         * unlike every other transform it needs the name itself. */
        b"A" => {
            let assignment = assignment_text(shell, name, &value, context);
            Ok(Expansion::one(Field::from_bytes(
                &assignment,
                context.protects(),
                context.splits(),
                context.quoted,
            )))
        }
        /* `@Q` quotes for a human to read back, and Bash always
         * reaches for quotation marks: `${x@Q}` on `x` is `'x'`, where
         * `printf %q` on the same bytes is a bare `x`. `@K` and `@k`
         * differ from it only for arrays, whose keys they keep. */
        b"Q" | b"K" | b"k" => map_value(shell, value, context, |locale, text| {
            crate::escape::bash::readable_quote(locale, BStr::new(text))
        }),
        b"U" => map_value(shell, value, context, |locale, text| {
            recase(locale, text, &any_character(), true, true)
        }),
        b"L" => map_value(shell, value, context, |locale, text| {
            recase(locale, text, &any_character(), false, true)
        }),
        b"u" => map_value(shell, value, context, |locale, text| {
            recase(locale, text, &any_character(), true, false)
        }),
        /* `@P` asks for the value rendered the way a prompt would be.
         * Prompt rendering is not part of this shell's Bash contract --
         * the dialect covers script and syntax compatibility, not the
         * interactive surface -- so there are no escapes to decode and
         * nothing re-reads the bytes. The transform is recognised, and
         * yields the value it was given. A value that happens to contain
         * `\w` or `$(...)` keeps those bytes rather than becoming a
         * directory name or a command, which is also the safer reading:
         * `@P` is otherwise a data-to-syntax path over a variable's
         * contents. */
        b"P" => map_value(shell, value, context, |_, text| BString::from(text)),
        _ => Err(shell.diagnostics().shell_error(b"Bad substitution")),
    }
}

/// `${name@A}`: `name='value'`, the assignment that would put the value
/// back.
///
/// An unset name has no assignment to print, which the caller has
/// already handled; an empty one prints `name=''`.
fn assignment_text(shell: &mut Shell, name: &BStr, value: &Value, context: Context) -> BString {
    let base = match super::split_subscript(name) {
        Some((base, _)) => base.to_owned(),
        None => name.to_owned(),
    };
    let text = super::value_bytes(shell, value.clone(), context);
    let mut assignment = base;
    assignment.push(b'=');
    assignment.extend_from_slice(&crate::escape::bash::readable_quote(
        &shell.locale,
        BStr::new(text.as_slice()),
    ));
    assignment
}

/// The attribute letters of the variable a parameter names.
///
/// The name may carry a subscript -- `${a[0]@a}` asks about `a` -- and
/// may be a positional or special parameter, which has no declaration
/// and therefore no letters.
fn attributes_of(shell: &mut Shell, name: &BStr) -> BString {
    let base = match super::split_subscript(name) {
        Some((base, _)) => base.to_owned(),
        None => name.to_owned(),
    };
    let base = BStr::new(base.as_slice());
    let target =
        crate::variables::nameref::read_name(shell, base).unwrap_or_else(|| base.to_owned());
    let target = match super::split_subscript(BStr::new(target.as_slice())) {
        Some((base, _)) => base.to_owned(),
        None => target,
    };
    crate::variables::special::attribute_letters(shell, BStr::new(target.as_slice()))
}

fn any_character() -> Pattern {
    Pattern::unquoted(BString::from("?"))
}

/// Apply one byte transformation to a value, element by element where
/// the value is a word list.
fn map_value(
    shell: &mut Shell,
    value: Value,
    context: Context,
    transform: impl Fn(&nsh_platform::Locale, &[u8]) -> BString,
) -> Result<Expansion, Error> {
    let elements = match &value {
        Value::At(words) if context.full => Some(words.clone()),
        Value::Star(words) if context.full && !context.quoted => Some(words.clone()),
        _ => None,
    };
    if let Some(words) = elements {
        return Ok(Expansion {
            fields: words
                .iter()
                .map(|word| {
                    let mapped = transform(&shell.locale, word);
                    Field::from_bytes(&mapped, context.protects(), !context.quoted, context.quoted)
                })
                .collect(),
        });
    }
    let text = super::value_bytes(shell, value, context);
    let mapped = transform(&shell.locale, &text);
    Ok(Expansion::one(Field::from_bytes(
        &mapped,
        context.protects(),
        context.splits(),
        context.quoted,
    )))
}

fn operand_units(parameter: &ParameterExpansion) -> Vec<WordUnit> {
    parameter
        .operand
        .as_deref()
        .map(ParsedWord::units)
        .unwrap_or_default()
}

fn expand_units(shell: &mut Shell, units: &[WordUnit], context: Context) -> Result<Field, Error> {
    let word = ParsedWord::from_units(units);
    Ok(expand_parts(shell, word.parts(), context)?.collapse())
}

/// The index of the first unquoted `separator` outside any nesting, with
/// the first unit exempt because a pattern's first byte is literal.
fn separator_at(units: &[WordUnit], separator: u8) -> Option<usize> {
    scan_units(units, separator, true, false)
}

/// The index of the unquoted `separator` that divides a `${x:a:b}`
/// operand, skipping the `:` a conditional expression owns.
fn boundary_at(units: &[WordUnit], separator: u8) -> Option<usize> {
    scan_units(units, separator, false, true)
}

fn scan_units(
    units: &[WordUnit],
    separator: u8,
    skip_first: bool,
    arithmetic: bool,
) -> Option<usize> {
    let mut quoted = false;
    let mut depth = 0usize;
    let mut conditionals = 0usize;
    for (index, unit) in units.iter().enumerate() {
        match unit {
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Open)) => quoted = true,
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Close)) => quoted = false,
            WordUnit::Literal(byte) if !quoted => match *byte {
                b'(' | b'[' if arithmetic => depth += 1,
                b')' | b']' if arithmetic => depth = depth.saturating_sub(1),
                b'?' if arithmetic => conditionals += 1,
                byte if byte == separator && depth == 0 && conditionals == 0 => {
                    if !(skip_first && index == 0) {
                        return Some(index);
                    }
                }
                byte if byte == separator && conditionals != 0 => conditionals -= 1,
                _ => {}
            },
            _ => {}
        }
    }
    None
}

fn arithmetic_operand(
    shell: &mut Shell,
    units: &[WordUnit],
    context: Context,
) -> Result<i64, Error> {
    let text = expand_units(shell, units, context.operand())?.bytes;
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(0);
    }
    crate::arithmetic::evaluate(shell, text.as_bstr())
}
