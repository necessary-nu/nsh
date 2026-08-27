//! Bash brace expansion.
//!
//! Brace expansion runs before every other expansion and turns one word
//! into several, so it works on the structural word rather than on bytes:
//! a command substitution or a parameter expansion is one unit, and its
//! braces and commas are therefore never mistaken for the word's own.

use bstr::{BString, ByteSlice as _};

use crate::context::Shell;
use crate::error::Error;
use crate::options::Dialect;
use crate::word::{ParsedWord, QuoteBoundary, WordPart, WordUnit};

/// The most words one brace expansion may build.
///
/// Braces multiply — `{a,b}` twenty times over asks for a million words,
/// and nesting compounds it — so the count is charged against a budget
/// before anything is allocated rather than discovered afterwards. The
/// limit is far above any argument list a command could be given and far
/// below what would exhaust memory.
// [spec:nsh:req:compat.bash.expansion-globbing]
const WORD_LIMIT: usize = 65_536;

/// The budget ran out. Reported as an ordinary shell error, once, by the
/// entry point; the recursion carries only the fact.
struct TooManyWords;

/// Rewrite one word into the words its braces stand for.
///
/// An empty result means the word has no brace expression, which is the
/// case this has to leave untouched.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn expand(shell: &mut Shell, word: &ParsedWord) -> Result<Vec<ParsedWord>, Error> {
    if shell.options.dialect() != Dialect::Bash {
        return Ok(Vec::new());
    }
    let units = word.units();
    let mut budget = WORD_LIMIT;
    match expand_units(&units, &mut budget) {
        Ok(None) => Ok(Vec::new()),
        Ok(Some(words)) => Ok(words
            .iter()
            .map(|units| ParsedWord::from_units(units))
            .collect()),
        Err(TooManyWords) => Err(shell
            .diagnostics()
            .shell_error(b"brace expansion produces too many words")),
    }
}

/// Take `count` words out of the budget before they are built.
fn charge(budget: &mut usize, count: usize) -> Result<(), TooManyWords> {
    match budget.checked_sub(count) {
        Some(remaining) => {
            *budget = remaining;
            Ok(())
        }
        None => Err(TooManyWords),
    }
}

/// `Ok(None)` means the word has no brace expression, which is not the
/// same as one that stands for a single word: `{1..1}` expands to `1`.
fn expand_units(
    units: &[WordUnit],
    budget: &mut usize,
) -> Result<Option<Vec<Vec<WordUnit>>>, TooManyWords> {
    let mut from = 0;
    while let Some(open) = opening_brace(units, from) {
        let Some(close) = closing_brace(units, open) else {
            from = open + 1;
            continue;
        };
        let amble = &units[open + 1..close];
        let alternatives = match sequence(amble, budget)? {
            Some(alternatives) => Some(alternatives),
            None => comma_separated(amble, budget)?,
        };
        let Some(alternatives) = alternatives else {
            from = open + 1;
            continue;
        };
        let tails = expanded_or_self(&units[close + 1..], budget)?;
        let count = alternatives
            .len()
            .checked_mul(tails.len())
            .ok_or(TooManyWords)?;
        charge(budget, count)?;
        let mut result = Vec::with_capacity(count);
        for alternative in alternatives {
            for tail in &tails {
                let mut word = units[..open].to_vec();
                word.extend(alternative.iter().cloned());
                word.extend(tail.iter().cloned());
                result.push(word);
            }
        }
        return Ok(Some(result));
    }
    Ok(None)
}

fn expanded_or_self(
    units: &[WordUnit],
    budget: &mut usize,
) -> Result<Vec<Vec<WordUnit>>, TooManyWords> {
    match expand_units(units, budget)? {
        Some(words) => Ok(words),
        None => {
            charge(budget, 1)?;
            Ok(vec![units.to_vec()])
        }
    }
}

/// The next unquoted `{` at or after `from`.
fn opening_brace(units: &[WordUnit], from: usize) -> Option<usize> {
    let mut quoted = false;
    for (index, unit) in units.iter().enumerate() {
        match unit {
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Open(..))) => quoted = true,
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Close)) => quoted = false,
            WordUnit::Literal(b'{') if !quoted && index >= from => return Some(index),
            _ => {}
        }
    }
    None
}

/// The unquoted `}` that closes the brace at `open`, counting the braces
/// nested inside it.
fn closing_brace(units: &[WordUnit], open: usize) -> Option<usize> {
    let mut quoted = false;
    let mut depth = 0usize;
    for (index, unit) in units.iter().enumerate().skip(open + 1) {
        match unit {
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Open(..))) => quoted = true,
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Close)) => quoted = false,
            WordUnit::Literal(b'}') if !quoted && depth == 0 => return Some(index),
            WordUnit::Literal(b'}') if !quoted => depth -= 1,
            WordUnit::Literal(b'{') if !quoted => depth += 1,
            _ => {}
        }
    }
    None
}

/// Split an amble on its own commas, recursively expanding each part.
/// `None` means the amble has no comma, so the braces are ordinary text.
fn comma_separated(
    amble: &[WordUnit],
    budget: &mut usize,
) -> Result<Option<Vec<Vec<WordUnit>>>, TooManyWords> {
    let mut parts = Vec::new();
    let mut start = 0;
    let mut quoted = false;
    let mut depth = 0usize;
    for (index, unit) in amble.iter().enumerate() {
        match unit {
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Open(..))) => quoted = true,
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Close)) => quoted = false,
            WordUnit::Literal(b'{') if !quoted => depth += 1,
            WordUnit::Literal(b'}') if !quoted => depth = depth.saturating_sub(1),
            WordUnit::Literal(b',') if !quoted && depth == 0 => {
                parts.push(&amble[start..index]);
                start = index + 1;
            }
            _ => {}
        }
    }
    if parts.is_empty() {
        return Ok(None);
    }
    parts.push(&amble[start..]);
    let mut alternatives = Vec::new();
    for part in parts {
        alternatives.extend(expanded_or_self(part, budget)?);
    }
    Ok(Some(alternatives))
}

/// `{first..last}` and `{first..last..step}` over integers or over
/// single characters.
fn sequence(
    amble: &[WordUnit],
    budget: &mut usize,
) -> Result<Option<Vec<Vec<WordUnit>>>, TooManyWords> {
    let Some(text) = literal_text(amble) else {
        return Ok(None);
    };
    let mut terms = text.split_str("..");
    let (Some(first), Some(last)) = (terms.next(), terms.next()) else {
        return Ok(None);
    };
    let step = terms.next();
    if terms.next().is_some() {
        return Ok(None);
    }
    let step = match step {
        None => None,
        Some(step) => match parse_integer(step) {
            Some(step) => Some(step),
            None => return Ok(None),
        },
    };
    let Some(items) = numeric_sequence(first, last, step, budget)?
        .or(character_sequence(first, last, step, budget)?)
    else {
        return Ok(None);
    };
    Ok(Some(
        items
            .into_iter()
            .map(|item| item.iter().copied().map(WordUnit::Literal).collect())
            .collect(),
    ))
}

/// How many terms a `{first..last..step}` sequence has, charged against
/// the budget before any of them is built.
fn sequence_length(
    start: i64,
    end: i64,
    step: i64,
    budget: &mut usize,
) -> Result<usize, TooManyWords> {
    let span = end.checked_sub(start).ok_or(TooManyWords)?.unsigned_abs();
    let stride = step.unsigned_abs().max(1);
    let count = usize::try_from(span / stride + 1).map_err(|_| TooManyWords)?;
    charge(budget, count)?;
    Ok(count)
}

fn numeric_sequence(
    first: &[u8],
    last: &[u8],
    step: Option<i64>,
    budget: &mut usize,
) -> Result<Option<Vec<BString>>, TooManyWords> {
    let (Some(start), Some(end)) = (parse_integer(first), parse_integer(last)) else {
        return Ok(None);
    };
    let width = padded_width(first, last);
    let step = increment(step, start, end);
    let count = sequence_length(start, end, step, budget)?;
    let mut items = Vec::with_capacity(count);
    let mut value = start;
    for _ in 0..count {
        items.push(render_integer(value, width));
        value = value.saturating_add(step);
    }
    Ok(Some(items))
}

fn character_sequence(
    first: &[u8],
    last: &[u8],
    step: Option<i64>,
    budget: &mut usize,
) -> Result<Option<Vec<BString>>, TooManyWords> {
    if first.len() != 1
        || last.len() != 1
        || !first[0].is_ascii_alphabetic()
        || !last[0].is_ascii_alphabetic()
    {
        return Ok(None);
    }
    let start = i64::from(first[0]);
    let end = i64::from(last[0]);
    let step = increment(step, start, end);
    let count = sequence_length(start, end, step, budget)?;
    let mut items = Vec::with_capacity(count);
    let mut value = start;
    for _ in 0..count {
        items.push(BString::from(vec![u8::try_from(value).unwrap_or(b'?')]));
        value += step;
    }
    Ok(Some(items))
}

fn increment(step: Option<i64>, start: i64, end: i64) -> i64 {
    let magnitude = step.map_or(1, i64::abs).max(1);
    if start <= end { magnitude } else { -magnitude }
}

/// The field width a zero-padded operand asks for. `{01..10}` counts to
/// `10` in two digits, and the wider operand wins.
fn padded_width(first: &[u8], last: &[u8]) -> usize {
    let padded = |term: &[u8]| {
        let digits = term.strip_prefix(b"-").unwrap_or(term);
        digits.len() > 1 && digits.first() == Some(&b'0')
    };
    if padded(first) || padded(last) {
        first.len().max(last.len())
    } else {
        0
    }
}

fn render_integer(value: i64, width: usize) -> BString {
    let digits = value.unsigned_abs().to_string();
    let sign: &[u8] = if value < 0 { b"-" } else { b"" };
    let zeros = width.saturating_sub(digits.len() + sign.len());
    let mut text = BString::from(sign);
    text.extend(std::iter::repeat_n(b'0', zeros));
    text.extend_from_slice(digits.as_bytes());
    text
}

fn parse_integer(text: &[u8]) -> Option<i64> {
    let (sign, digits) = match text.first() {
        Some(b'-') => (-1, &text[1..]),
        Some(b'+') => (1, &text[1..]),
        _ => (1, text),
    };
    if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
        return None;
    }
    let magnitude = digits.iter().try_fold(0i64, |value, byte| {
        value.checked_mul(10)?.checked_add(i64::from(byte - b'0'))
    })?;
    Some(sign * magnitude)
}

/// The amble's bytes, when every unit is an unquoted literal. A sequence
/// expression has no expansions and no quoting in it.
fn literal_text(units: &[WordUnit]) -> Option<BString> {
    units
        .iter()
        .map(|unit| match unit {
            WordUnit::Literal(byte) => Some(*byte),
            WordUnit::Part(_) => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(BString::from)
}
