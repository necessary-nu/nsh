//! Execution of Bash `[[ ... ]]` conditional expressions.
//!
//! `[[` is a keyword, not a command, and that is the whole reason this is
//! not the `test` built-in with different spelling: its operands are never
//! split into fields, its right-hand side may be a pattern or a regular
//! expression rather than a string, and its connectives short-circuit in
//! the parse tree instead of in an argument vector. The file operators do
//! come from `test`, because those really are the same questions.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::expand::{ExpandedFields, ExpansionMode, expand_argument};
use crate::nodes::{BashConditional, BashConditionalExpr, Node, WordNode};
use crate::options::ShellOption;
use crate::regex::Regex;
use crate::status::ExitStatus;
use crate::variables::arrays::{self, CompoundElement};

/// What one conditional expression evaluated to.
///
/// `Invalid` is Bash's third answer: an operand it could not evaluate is
/// neither true nor false, it is status 2 with a diagnostic already
/// written, and it must not be inverted by `!` or absorbed by `||`.
#[derive(Clone, Copy, Eq, PartialEq)]
enum Truth {
    True,
    False,
    Invalid,
}

impl Truth {
    const fn from(value: bool) -> Self {
        if value { Self::True } else { Self::False }
    }
}

/// Run one `[[ ... ]]` command.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(crate) fn evaluate(shell: &mut Shell, command: &BashConditional) -> Result<ExitStatus, Error> {
    Ok(match expression(shell, &command.expression)? {
        Truth::True => ExitStatus::SUCCESS,
        Truth::False => ExitStatus::FAILURE,
        Truth::Invalid => ExitStatus::ERROR,
    })
}

fn expression(shell: &mut Shell, node: &BashConditionalExpr) -> Result<Truth, Error> {
    match node {
        BashConditionalExpr::Empty => Ok(invalid(shell, b"[[: expected a conditional expression")),
        BashConditionalExpr::Word(word) => Ok(Truth::from(!operand(shell, word)?.is_empty())),
        BashConditionalExpr::Unary { operator, operand } => {
            unary(shell, operator.as_bstr(), operand)
        }
        BashConditionalExpr::Binary {
            left,
            operator,
            right,
        } => binary(shell, left, operator.as_bstr(), right),
        BashConditionalExpr::Not(inner) => Ok(match expression(shell, inner)? {
            Truth::True => Truth::False,
            Truth::False => Truth::True,
            Truth::Invalid => Truth::Invalid,
        }),
        BashConditionalExpr::And(left, right) => match expression(shell, left)? {
            Truth::True => expression(shell, right),
            other => Ok(other),
        },
        BashConditionalExpr::Or(left, right) => match expression(shell, left)? {
            Truth::False => expression(shell, right),
            other => Ok(other),
        },
        BashConditionalExpr::Group(inner) => expression(shell, inner),
    }
}

// [spec:nsh:def:idiom.logical-descriptors]
fn unary(shell: &mut Shell, operator: &BStr, word: &WordNode) -> Result<Truth, Error> {
    let value = operand(shell, word)?;
    let value = BStr::new(value.as_slice());
    let bytes: &[u8] = operator.as_ref();
    // Bash spells "the file exists" `-a` inside `[[ ]]`, where `-a` cannot
    // also be the connective it is in `test`.
    if bytes == b"-a" {
        return Ok(Truth::from(crate::builtins::test::file_exists(value)));
    }
    if bytes == b"-o" {
        let enabled =
            ShellOption::from_name(value).is_some_and(|option| shell.options.enabled(option));
        return Ok(Truth::from(enabled));
    }
    if bytes == b"-v" {
        return Ok(Truth::from(is_set(shell, value)));
    }
    match crate::builtins::test::unary_test(shell, operator, value) {
        Some(result) => Ok(Truth::from(result?)),
        // `-N` and `-R` are recognised by the parser so that a script using
        // them is not a syntax error; neither question has an answer yet.
        None => Ok(Truth::False),
    }
}

fn binary(
    shell: &mut Shell,
    left: &WordNode,
    operator: &BStr,
    right: &WordNode,
) -> Result<Truth, Error> {
    let bytes: &[u8] = operator.as_ref();
    if matches!(bytes, b"=" | b"==" | b"!=") {
        let subject = operand(shell, left)?;
        let pattern = crate::expand::conditional_pattern(shell, right)?;
        let matched = pattern.matches(&shell.locale, &subject);
        return Ok(Truth::from(matched == (bytes != b"!=")));
    }
    if bytes == b"=~" {
        let subject = operand(shell, left)?;
        return regex_match(shell, BStr::new(subject.as_slice()), right);
    }
    if matches!(bytes, b"<" | b">") {
        let left = operand(shell, left)?;
        let right = operand(shell, right)?;
        let order = shell.locale.collate(&left, &right);
        return Ok(Truth::from(if bytes == b"<" {
            order == std::cmp::Ordering::Less
        } else {
            order == std::cmp::Ordering::Greater
        }));
    }
    if matches!(bytes, b"-nt" | b"-ot" | b"-ef") {
        let left = operand(shell, left)?;
        let right = operand(shell, right)?;
        let compared = crate::builtins::test::file_comparison(
            operator,
            BStr::new(left.as_slice()),
            BStr::new(right.as_slice()),
        );
        return Ok(compared.map_or(Truth::False, Truth::from));
    }
    arithmetic_comparison(shell, left, operator, right)
}

/// `-eq` and friends compare arithmetic *expressions* inside `[[ ]]`, so
/// `[[ 1+2 -eq 3 ]]` is true and an unset name is zero.
fn arithmetic_comparison(
    shell: &mut Shell,
    left: &WordNode,
    operator: &BStr,
    right: &WordNode,
) -> Result<Truth, Error> {
    let left = operand(shell, left)?;
    let right = operand(shell, right)?;
    let (Some(left), Some(right)) = (
        number(shell, BStr::new(left.as_slice()))?,
        number(shell, BStr::new(right.as_slice()))?,
    ) else {
        return Ok(Truth::Invalid);
    };
    let name: &[u8] = operator.as_ref();
    Ok(Truth::from(match name {
        b"-eq" => left == right,
        b"-ne" => left != right,
        b"-lt" => left < right,
        b"-le" => left <= right,
        b"-gt" => left > right,
        b"-ge" => left >= right,
        _ => return Ok(invalid(shell, b"[[: unknown conditional operator")),
    }))
}

/// Evaluate one arithmetic operand, reporting rather than raising so that
/// `[[ ]]` answers with status 2 the way Bash does.
fn number(shell: &mut Shell, text: &BStr) -> Result<Option<i64>, Error> {
    match crate::arithmetic::evaluate(shell, text) {
        Ok(value) => Ok(Some(value)),
        Err(error) if error.status() == ExitStatus::ERROR => Ok(None),
        Err(error) => Err(error),
    }
}

fn regex_match(shell: &mut Shell, subject: &BStr, word: &WordNode) -> Result<Truth, Error> {
    let pattern = crate::expand::conditional_pattern(shell, word)?;
    let regex = match Regex::compile(pattern.as_bytes(), pattern.quote_bits()) {
        Ok(regex) => regex,
        Err(message) => {
            let mut text = BString::from(&b"[["[..]);
            text.extend_from_slice(b": ");
            text.extend_from_slice(&message);
            return Ok(invalid(shell, &text));
        }
    };
    let found = regex.search(&shell.locale, subject);
    let matched = found.is_some();
    let elements = found.map_or_else(Vec::new, |captures| {
        captures
            .groups
            .into_iter()
            .map(|span| CompoundElement {
                subscript: None,
                value: span.map_or_else(BString::default, |(start, end)| {
                    BString::from(&subject[start..end])
                }),
                append: false,
            })
            .collect()
    });
    arrays::assign_compound(shell, BStr::new(b"BASH_REMATCH"), elements, false)?;
    Ok(Truth::from(matched))
}

/// Expand one operand: no field splitting and no pathname expansion, which
/// is the whole difference between `[[ $x = y ]]` and `[ $x = y ]`.
fn operand(shell: &mut Shell, word: &WordNode) -> Result<BString, Error> {
    let mut fields = ExpandedFields::new();
    let node = Node::Word(word.clone());
    expand_argument(shell, &node, Some(&mut fields), ExpansionMode::TILDE)?;
    Ok(fields
        .fields
        .into_iter()
        .next()
        .map(|field| field.as_bstr().to_owned())
        .unwrap_or_default())
}

fn is_set(shell: &mut Shell, name: &BStr) -> bool {
    let base = match name.iter().position(|byte| *byte == b'[') {
        Some(open) if name.last() == Some(&b']') => BStr::new(&name[..open]),
        _ => name,
    };
    crate::variables::lookup_bytes(shell, base).is_some()
}

fn invalid(shell: &mut Shell, message: &[u8]) -> Truth {
    shell.diagnostics().shell_warning(message);
    Truth::Invalid
}
