//! Bash-only parameter expansions and the pattern options they read.
//!
//! Every entry point here is reached only from Bash mode: the parser does
//! not produce these operations otherwise, and the option lookups return
//! the all-off value in POSIX mode.

use bstr::{BStr, BString, ByteSlice as _};

use super::{Context, Expansion, Field, Value, character_end, expand_parts, value_expansion};
use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::nodes::{FileRedirectionOperator, Node, Redirection, WordNode};
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

/// Bash's `$(<file)`: a command substitution whose body is nothing but an
/// input redirection reads the file rather than running a command.
///
/// The bytes become word data and nothing here parses them, which is what
/// keeps this an expansion rather than a second way into the evaluator.
// [dec:nsh:safety-trumps-compatibility]
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn file_substitution(
    shell: &mut Shell,
    command: Option<&Node>,
) -> Result<Option<BString>, Error> {
    let Some(target) = read_only_redirection(shell, command) else {
        return Ok(None);
    };
    let mut fields = crate::expand::ExpandedFields::new();
    let target = Node::Word(target);
    crate::expand::expand_argument(
        shell,
        &target,
        Some(&mut fields),
        crate::expand::ExpansionMode::TILDE,
    )?;
    debug_assert_eq!(fields.fields.len(), 1, "an unsplit expansion is one field");
    let pathname = fields.fields.remove(0).text;
    /* The open diagnostic is written by the redirection layer, so a
     * failure here needs no second one: the substitution answers with no
     * bytes and the status the shell would have taken from the command. */
    let Ok(Some(descriptor)) =
        crate::redirection::open_file_for_reading(shell, pathname.as_bstr(), false)
    else {
        shell.evaluation.command_substitution_status = crate::status::ExitStatus::FAILURE;
        return Ok(Some(BString::from(Vec::new())));
    };
    let content = nsh_platform::read_to_end(&descriptor).unwrap_or_default();
    shell.evaluation.command_substitution_status = crate::status::ExitStatus::SUCCESS;
    Ok(Some(BString::from(content)))
}

/// The redirection word of a `< word` body with nothing else in it.
fn read_only_redirection(shell: &Shell, command: Option<&Node>) -> Option<WordNode> {
    if shell.options.dialect() != Dialect::Bash {
        return None;
    }
    let Some(Node::Command(command)) = command else {
        return None;
    };
    if !command.arguments.is_empty() || !command.assignments.is_empty() {
        return None;
    }
    let [Redirection::File(redirection)] = command.redirections.as_slice() else {
        return None;
    };
    (redirection.operator == FileRedirectionOperator::Read
        && redirection.descriptor == LogicalDescriptor::STDIN)
        .then(|| redirection.target.clone())
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
    operation: ParameterOperation,
    context: Context,
) -> Result<Option<Expansion>, Error> {
    /* `${!a[@]}` lists subscripts, but `${!a[@]:2}` does not: an operator
     * turns the whole thing back into an indirection through `${a[@]}`.
     * Bash draws the line at the presence of an operator, so this does
     * too. */
    // [spec:nsh:req:compat.bash.expansion-globbing]
    if let Some((base, subscript)) = super::split_subscript(name)
        && matches!(subscript.as_ref() as &[u8], b"@" | b"*")
        && operation == ParameterOperation::Value
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
    // [spec:nsh:req:compat.bash.arrays-declarations]
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

/// Whether the value has bytes a transform can rewrite.
///
/// A name that holds an array has none without a subscript: `$A` reads
/// no element of an associative array, so `${A@Q}` has nothing to quote
/// and answers with nothing rather than with `''`.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn has_transformable_bytes(value: &Value) -> bool {
    match value {
        Value::Unset => false,
        Value::Variable(value) => value.scalar_ref().is_some(),
        Value::At(_) | Value::Star(_) => true,
    }
}

/// Resolve `${!ref…}` to the name the operators then apply to.
///
/// The reference's value is read as a string and has to spell a parameter
/// -- a name, a name with a subscript, or a special parameter. Bash
/// refuses anything else rather than inventing a variable, and so does
/// this: the step is name-to-value, never data-to-syntax.
// [dec:nsh:safety-trumps-compatibility]
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn indirect_target(
    shell: &mut Shell,
    reference: &BStr,
    value: Value,
) -> Result<BString, Error> {
    let target = match value {
        /* A name with no value at all is not a reference to anything, and
         * Bash says so rather than expanding to the empty name. */
        Value::Unset => {
            let mut message = BString::from(reference);
            message.extend_from_slice(b": invalid indirect expansion");
            return Err(shell.diagnostics().expansion_error_value(&message));
        }
        /* A name that holds an array has no scalar to read -- `$A` reads
         * no element of an associative array -- and there Bash yields
         * nothing quietly instead of reporting. */
        Value::Variable(value) => match value.scalar_owned() {
            Some(text) => text,
            None => return Ok(BString::default()),
        },
        Value::At(words) | Value::Star(words) => super::join_parameters(shell, &words),
    };
    if names_a_parameter(&shell.locale, target.as_bstr()) {
        return Ok(target);
    }
    let mut message = BString::from(reference);
    message.extend_from_slice(b": ");
    message.extend_from_slice(&target);
    message.extend_from_slice(b": invalid variable name");
    /* Bash reports the refusal, abandons the command and reads on; the
     * POSIX dialect has no `${!ref}` to reach this, so the boundary is
     * the dialect's either way. */
    // [spec:nsh:req:compat.bash.error-boundary]
    Err(shell.diagnostics().dialect_expansion_error(&message))
}

/// Whether this text spells a parameter an expansion may read.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn names_a_parameter(locale: &nsh_platform::Locale, text: &BStr) -> bool {
    let bytes: &[u8] = text.as_ref();
    let Some(&first) = bytes.first() else {
        return false;
    };
    if bytes.len() == 1 && (crate::syntax::is_special(first) || first == b'_') {
        return true;
    }
    if bytes.iter().all(u8::is_ascii_digit) {
        return true;
    }
    match subscript_reference(bytes) {
        Some(base) => is_plain_name(locale, base),
        None => is_plain_name(locale, bytes),
    }
}

fn is_plain_name(locale: &nsh_platform::Locale, bytes: &[u8]) -> bool {
    bytes
        .first()
        .is_some_and(|byte| crate::syntax::is_name(locale, *byte))
        && bytes[1..]
            .iter()
            .all(|byte| crate::syntax::is_in_name(locale, *byte))
}

/// The name in front of a well-formed `name[subscript]`.
///
/// The subscript has to be non-empty and to close on the last byte, with
/// its brackets nested and its quotes balanced -- an unterminated quote
/// runs past the `]` and leaves no subscript at all, which is how
/// `a[1"]` stops being a variable name.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn subscript_reference(bytes: &[u8]) -> Option<&[u8]> {
    let open = bytes.iter().position(|byte| *byte == b'[')?;
    if open == 0 {
        return None;
    }
    let close = subscript_end(bytes, open + 1)?;
    (close == bytes.len() - 1 && close > open + 1).then(|| &bytes[..open])
}

/// Where the `]` that closes a subscript opened before `from` sits.
fn subscript_end(bytes: &[u8], from: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut at = from;
    while at < bytes.len() {
        match bytes[at] {
            b'\\' => at += 1,
            quote @ (b'\'' | b'"') => {
                at = closing_quote(bytes, at + 1, quote)?;
            }
            b'[' => depth += 1,
            b']' if depth == 0 => return Some(at),
            b']' => depth -= 1,
            _ => {}
        }
        at += 1;
    }
    None
}

fn closing_quote(bytes: &[u8], from: usize, quote: u8) -> Option<usize> {
    let mut at = from;
    while at < bytes.len() {
        if bytes[at] == b'\\' && quote == b'"' {
            at += 2;
            continue;
        }
        if bytes[at] == quote {
            return Some(at);
        }
        at += 1;
    }
    None
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
    /* `${x:}` has no expression to evaluate and Bash rejects it, where
     * `${x: }` is the expression ` `, which is zero, and `${x::}` is a
     * slice whose two empty expressions are both zero. */
    if cut.is_none() && offset_units.is_empty() {
        return Err(shell
            .diagnostics()
            .expansion_error_value(b"bad substitution"));
    }
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
            let subscripts = if positional {
                None
            } else {
                array_subscripts(shell, name)
            };
            let selected = select(shell, all, subscripts.as_deref(), offset, length)?;
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

/// The subscripts an indexed array's elements are stored under.
///
/// A slice of one counts subscripts rather than surviving elements, so
/// the holes an `unset` left still take up room. An associative array
/// has no order to count with, and neither have the positional
/// parameters, so both answer `None` and are sliced by position.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn array_subscripts(shell: &mut Shell, name: &BStr) -> Option<Vec<u64>> {
    let base = super::split_subscript(name).map_or(name, |(base, _)| base);
    let target =
        crate::variables::nameref::read_name(shell, base).unwrap_or_else(|| base.to_owned());
    let target = BStr::new(target.as_slice());
    let target = super::split_subscript(target).map_or(target, |(base, _)| base);
    crate::variables::value::variable_value(shell, target)?.indexed_keys()
}

/// Cut one offset/length pair out of a list of elements.
///
/// `subscripts` is what the elements are stored under where they are
/// stored sparsely: the offset is measured in subscripts, so
/// `${a[@]:5}` starts at the first element whose subscript is at least
/// 5, while the length that follows counts elements. A negative offset
/// counts back from one past the highest subscript, and one that
/// reaches past the start selects nothing rather than clamping. Bash
/// refuses a negative length here, where the same spelling on a string
/// would name an end position.
fn select(
    shell: &mut Shell,
    elements: Vec<BString>,
    subscripts: Option<&[u64]>,
    offset: i64,
    length: Option<i64>,
) -> Result<Vec<BString>, Error> {
    let total = elements.len() as i64;
    let bound = match subscripts {
        Some(keys) => keys.last().map_or(0, |last| *last as i64 + 1),
        None => total,
    };
    let first_subscript = if offset < 0 {
        let start = bound + offset;
        if start < 0 {
            return Ok(Vec::new());
        }
        start
    } else {
        offset
    };
    let start = match subscripts {
        Some(keys) => keys
            .iter()
            .position(|key| *key as i64 >= first_subscript)
            .map_or(total, |position| position as i64),
        None => first_subscript.min(total),
    };
    let count = match length {
        None => total - start,
        Some(length) if length < 0 => {
            let mut message = length.to_string().into_bytes();
            message.extend_from_slice(b": substring expression < 0");
            return Err(shell.diagnostics().expansion_error_value(&message));
        }
        Some(length) => length.min(total - start),
    };
    Ok(elements
        .into_iter()
        .skip(start as usize)
        .take(count.max(0) as usize)
        .collect())
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
     * empty value would give and what `${empty@Q}` does produce. The two
     * transforms that read the declaration rather than the value are the
     * exceptions: `@a` answers for a name that holds nothing at all, and
     * `@A` answers for one that is set but has no scalar to read. */
    let available = match operator.as_slice() {
        b"a" => true,
        b"A" => !value.is_unset(),
        _ => has_transformable_bytes(&value),
    };
    if !available {
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
        // [spec:nsh:req:compat.bash.error-boundary]
        _ => Err(shell.diagnostics().dialect_error(b"Bad substitution")),
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
///
/// A quoted `"${a[*]}"` is a word list too, and Bash applies the
/// operator to each element *before* joining them: `"${a[*]#-}"` strips
/// one prefix per element rather than one from the joined string. The
/// join is the last step, so the operator never sees a separator.
// [spec:nsh:req:compat.bash.arrays-declarations]
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn map_value(
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
    /* The POSIX dialect has no `[*]`, and dash joins `$*` before the
     * operator runs, so the order stays the other way round there. */
    if shell.options.dialect() == Dialect::Bash
        && let Value::At(words) | Value::Star(words) = &value
    {
        let mapped = words
            .iter()
            .map(|word| transform(&shell.locale, word))
            .collect::<Vec<_>>();
        return Ok(Expansion::one(Field::from_bytes(
            &super::join_parameters(shell, &mapped),
            context.protects(),
            context.splits(),
            context.quoted,
        )));
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
