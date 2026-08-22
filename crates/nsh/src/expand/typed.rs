//! Expansion over the structural word representation.
//!
//! Each output byte carries the two facts needed by later phases: whether
//! quoting protects it from pattern syntax, and whether it came from an
//! unquoted expansion and is therefore eligible for IFS splitting.  Field
//! boundaries are values too, most notably for `"$@"`; no sentinel bytes are
//! inserted into the shell data.

// [spec:nsh:req:idiom.operation-modes]
use bstr::{BStr, BString, ByteSlice};
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};

use super::{ExpandedField, ExpandedFields, ExpansionMode};
use crate::context::Shell;
use crate::error::Error;
use crate::nodes::Node;
use crate::options::{OPTION_SPECS, ShellOption};
// [spec:nsh:def:idiom.shell-options]
use crate::pattern::Pattern;
use crate::variables::value::VariableValue;
use crate::word::{ParameterExpansion, ParameterOperation, ParsedWord, QuoteBoundary, WordPart};

mod field;
mod pathname;

use field::Field;
#[cfg(test)]
use field::FieldRegion;

#[derive(Clone, Debug)]
struct Expansion {
    fields: Vec<Field>,
}

impl Expansion {
    fn builder() -> Self {
        Self {
            fields: vec![Field::default()],
        }
    }

    fn none() -> Self {
        Self { fields: Vec::new() }
    }

    fn one(field: Field) -> Self {
        Self {
            fields: vec![field],
        }
    }

    /// Concatenate a shell expansion. The first resulting field joins the
    /// current last field and any additional fields retain their boundary.
    /// This is the prefix/suffix rule for `"pre$@post"` expressed directly.
    fn append(&mut self, mut other: Self) {
        if other.fields.is_empty() {
            return;
        }
        if self.fields.is_empty() {
            self.fields = other.fields;
            return;
        }
        let first = other.fields.remove(0);
        self.fields
            .last_mut()
            .expect("nonempty expansion")
            .append(first);
        self.fields.append(&mut other.fields);
    }

    fn preserve_empty(&mut self) {
        for field in &mut self.fields {
            field.anchor_empty();
        }
    }

    fn collapse(mut self) -> Field {
        let Some(mut first) = self.fields.drain(..1).next() else {
            return Field::default();
        };
        for field in self.fields {
            first.append(field);
        }
        first
    }
}

#[derive(Clone, Copy)]
struct Context {
    quoted: bool,
    full: bool,
    operand: bool,
    pattern: bool,
    tilde_at_start: bool,
    tilde_after_equal: bool,
    tilde_after_colon: bool,
}

impl Context {
    fn top(mode: ExpansionMode) -> Self {
        Self {
            quoted: mode.contains(ExpansionMode::QUOTED),
            full: mode.contains(ExpansionMode::SPLIT),
            operand: false,
            pattern: false,
            tilde_at_start: mode.contains(ExpansionMode::TILDE),
            tilde_after_equal: mode.contains(ExpansionMode::ASSIGNMENT_TILDE),
            tilde_after_colon: mode
                .intersects(ExpansionMode::ASSIGNMENT_TILDE | ExpansionMode::COLON_TILDE),
        }
    }

    fn quoted(self) -> Self {
        Self {
            quoted: true,
            tilde_at_start: false,
            tilde_after_equal: false,
            tilde_after_colon: false,
            ..self
        }
    }

    fn operand(self) -> Self {
        Self {
            operand: true,
            tilde_at_start: true,
            tilde_after_equal: false,
            // An operand remains part of the surrounding assignment word;
            // POSIX.1-2024 therefore keeps `:`-separated tilde prefixes
            // active inside `${parameter-word}`.
            tilde_after_colon: self.tilde_after_colon,
            ..self
        }
    }

    fn pattern_operand(self) -> Self {
        Self {
            quoted: false,
            full: false,
            operand: false,
            pattern: true,
            tilde_at_start: true,
            tilde_after_equal: false,
            tilde_after_colon: false,
        }
    }

    fn protects(self) -> bool {
        self.quoted
    }

    fn splits(self) -> bool {
        self.full && !self.quoted && !self.pattern
    }

    fn literal_splits(self) -> bool {
        self.operand && self.splits()
    }
}

// [spec:nsh:sem:idiom.typed-expansion]
// [spec:nsh:req:idiom.parser-control-flow]
// [spec:dash:sem:expand.argstr-fn]
pub(super) fn expand_argument(
    shell: &mut Shell,
    word: &ParsedWord,
    output: Option<&mut ExpandedFields>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let context = Context::top(mode);
    let expanded = expand_parts(shell, word.parts(), context)?;

    if let Some(output) = output {
        let fields = if context.full {
            let mut split = split_fields(shell, expanded.fields);
            if !shell.options.enabled(ShellOption::NoGlob) {
                split = pathname::expand(shell, split);
            }
            split
        } else {
            vec![expanded.collapse()]
        };
        output.fields.extend(fields.into_iter().map(into_field));
    } else {
        let field = expanded.collapse();
        shell.expand.buffer.clear();
        shell.expand.buffer.extend_from_slice(&field.bytes);
    }

    Ok(())
}

// [spec:nsh:sem:idiom.typed-expansion]
pub(super) fn case_matches(
    shell: &mut Shell,
    word: &ParsedWord,
    value: &BStr,
) -> Result<bool, Error> {
    let context = Context {
        quoted: false,
        full: false,
        operand: false,
        pattern: true,
        tilde_at_start: true,
        tilde_after_equal: false,
        tilde_after_colon: false,
    };
    let pattern = expand_parts(shell, word.parts(), context)?
        .collapse()
        .pattern();
    Ok(pattern.matches(&shell.locale, value))
}

// Quote protection is metadata during expansion and is discarded only when
// the final owned field is materialized; no escape marker bytes are removed.
// [spec:dash:sem:expand.rmescapes-fn]
// [spec:posix:req:expand.quote-removal]
// [spec:posix:sem:expand.quote-removal-quoting-remembered]
// [spec:posix:syn:pattern.backslash-escape-with-shell-quoting]
// [spec:posix:syn:pattern.backslash-escape-without-shell-quoting]
// [spec:posix:req:pattern.escaping-follows-quoting-rules]
// [spec:posix:syn:pattern.trailing-backslash-unspecified]
// [spec:posix:req:pattern.quote-to-match-literally]
fn into_field(field: Field) -> ExpandedField {
    ExpandedField { text: field.bytes }
}

// The former helpers allocated fixed C buffers and threaded cursors through
// them. Structural words, owned fields, and Rust's integer formatting now
// implement the same observable expansion behavior without those mechanisms.
// [spec:dash:sem:expand.expari-fn]
// [spec:dash:sem:expand.expcmd-fn]
// [spec:dash:sem:shell.max-int-length-fn]
// [spec:posix:req:expand.arith-token-expansion]
fn expand_parts(
    shell: &mut Shell,
    parts: &[WordPart],
    context: Context,
) -> Result<Expansion, Error> {
    let mut result = Expansion::builder();
    let mut at = 0;
    let mut tilde = if context.tilde_at_start && !context.quoted {
        if context.operand && context.tilde_after_colon {
            TildePosition::Assignment
        } else {
            TildePosition::WordStart
        }
    } else {
        TildePosition::None
    };
    let mut assignment_equal_available = context.tilde_after_equal;

    while at < parts.len() {
        match &parts[at] {
            WordPart::Literal(bytes) => {
                append_literal(
                    shell,
                    &mut result,
                    bytes,
                    context,
                    at + 1 < parts.len(),
                    &mut tilde,
                    &mut assignment_equal_available,
                );
            }
            WordPart::Escaped(byte) => {
                result.append(Expansion::one(Field::from_bytes(
                    &[*byte],
                    true,
                    false,
                    context.quoted,
                )));
                tilde = TildePosition::None;
            }
            WordPart::Multibyte { bytes, escaped } => {
                result.append(Expansion::one(Field::from_bytes(
                    bytes,
                    context.protects() || *escaped,
                    context.literal_splits() && !escaped,
                    context.quoted,
                )));
                tilde = TildePosition::None;
            }
            WordPart::Quote(QuoteBoundary::Open) => {
                let close = matching_quote(parts, at);
                let inner = &parts[at + 1..close];
                if !is_empty_quoted_at(shell, inner, context) {
                    let mut quoted = expand_parts(shell, inner, context.quoted())?;
                    quoted.preserve_empty();
                    result.append(quoted);
                }
                at = close;
                tilde = TildePosition::None;
            }
            WordPart::Quote(QuoteBoundary::Close) => {}
            WordPart::Parameter(parameter) => {
                result.append(expand_parameter(shell, parameter, context)?);
                tilde = TildePosition::None;
            }
            WordPart::Command(command) => {
                let bytes = command_substitution(shell, command.as_deref())?;
                result.append(Expansion::one(Field::from_bytes(
                    &bytes,
                    context.protects(),
                    context.splits(),
                    context.quoted,
                )));
                tilde = TildePosition::None;
            }
            WordPart::Arithmetic(expression) => {
                let arithmetic_context = Context {
                    quoted: false,
                    full: false,
                    operand: false,
                    pattern: false,
                    tilde_at_start: false,
                    tilde_after_equal: false,
                    tilde_after_colon: false,
                };
                let expression =
                    expand_parts(shell, expression.parts(), arithmetic_context)?.collapse();
                let number = crate::arithmetic::evaluate(shell, expression.bytes.as_bstr())?;
                let rendered = number.to_string();
                result.append(Expansion::one(Field::from_bytes(
                    rendered.as_bytes(),
                    context.protects(),
                    context.splits(),
                    context.quoted,
                )));
                tilde = TildePosition::None;
            }
        }
        at += 1;
    }
    Ok(result)
}

// [spec:posix:def:expand.tilde-prefix]
// [spec:posix:def:expand.tilde-prefix-in-assignment]
#[derive(Clone, Copy, Eq, PartialEq)]
enum TildePosition {
    None,
    WordStart,
    Assignment,
}

fn matching_quote(parts: &[WordPart], open: usize) -> usize {
    let mut depth = 1;
    for (offset, part) in parts[open + 1..].iter().enumerate() {
        match part {
            WordPart::Quote(QuoteBoundary::Open) => depth += 1,
            WordPart::Quote(QuoteBoundary::Close) => {
                depth -= 1;
                if depth == 0 {
                    return open + offset + 1;
                }
            }
            _ => {}
        }
    }
    parts.len()
}

fn is_empty_quoted_at(shell: &Shell, parts: &[WordPart], context: Context) -> bool {
    context.full
        && shell.options.positional_parameters.parameter_count == 0
        && matches!(
            parts,
            [WordPart::Parameter(ParameterExpansion {
                name,
                operation: ParameterOperation::Value,
                ..
            })] if name.as_slice() == b"@"
        )
}

// [spec:posix:sem:expand.tilde-no-further-expansion]
// [spec:posix:req:expand.tilde-result-quoted]
// [spec:dash:sem:expand.chtodest-fn]
// [spec:dash:sem:expand.mbtodest-fn]
// [spec:dash:sem:expand.memtodest-fn]
// [spec:dash:sem:expand.strtodest-fn]
fn append_literal(
    shell: &mut Shell,
    result: &mut Expansion,
    bytes: &[u8],
    context: Context,
    has_following_parts: bool,
    tilde: &mut TildePosition,
    assignment_equal_available: &mut bool,
) {
    let mut at = 0;
    while at < bytes.len() {
        if *tilde != TildePosition::None && bytes[at] == b'~' && !context.quoted {
            let end = bytes[at + 1..]
                .iter()
                .position(|byte| {
                    *byte == b'/' || (*byte == b':' && *tilde == TildePosition::Assignment)
                })
                .map_or(bytes.len(), |offset| at + 1 + offset);
            if !(end == bytes.len() && has_following_parts)
                && let Some(home) = tilde_home(shell, &bytes[at + 1..end])
            {
                result.append(Expansion::one(Field::from_bytes(&home, true, false, false)));
                at = end;
                *tilde = TildePosition::None;
                continue;
            }
        }

        let byte = bytes[at];
        result.append(Expansion::one(Field::from_bytes(
            &[byte],
            context.protects(),
            context.literal_splits(),
            context.quoted,
        )));
        *tilde = if byte == b'=' && *assignment_equal_available {
            *assignment_equal_available = false;
            TildePosition::Assignment
        } else if byte == b':' && context.tilde_after_colon {
            TildePosition::Assignment
        } else {
            TildePosition::None
        };
        at += 1;
    }
}

// [spec:posix:req:expand.tilde-home]
// [spec:posix:req:expand.tilde-login-name]
// [spec:posix:req:expand.tilde-replacement-pathname]
// [spec:dash:sem:expand.exptilde-fn]
fn tilde_home(shell: &mut Shell, user: &[u8]) -> Option<Vec<u8>> {
    if user.is_empty() {
        crate::variables::lookup_bytes(shell, BStr::new(b"HOME")).map(|home| home.to_vec())
    } else {
        let user = user.try_to_os_string().ok()?;
        nsh_platform::named_user_home(&user).map(|home| home.to_shell_bytes())
    }
}

#[derive(Clone)]
enum Value {
    Unset,
    Variable(VariableValue),
    At(Vec<BString>),
    Star(Vec<BString>),
}

impl Value {
    fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    fn is_empty(&self, shell: &Shell, context: Context) -> bool {
        match self {
            Self::Unset => true,
            Self::Variable(value) => value.scalar_ref().is_none_or(|bytes| bytes.is_empty()),
            Self::At(words) if context.full => {
                words.is_empty() || (words.len() == 1 && words[0].is_empty())
            }
            Self::Star(words) if context.full && !context.quoted => {
                words.is_empty() || (words.len() == 1 && words[0].is_empty())
            }
            Self::At(words) | Self::Star(words) => join_parameters(shell, words).is_empty(),
        }
    }
}

// The parameter operation and its scan direction are represented in the AST;
// no integer selector or shared scan cursor crosses this boundary.
// [spec:dash:sem:expand.cvtnum-fn]
// [spec:dash:sem:expand.evalvar-fn]
// [spec:dash:sem:expand.scanleft-fn]
// [spec:posix:req:expand.param-simple]
// [spec:posix:req:expand.param-word-expansion]
// [spec:posix:req:expand.param-colon-effect]
// [spec:posix:req:expand.param-use-default]
// [spec:posix:req:expand.param-assign-default]
// [spec:posix:req:expand.param-error-if-unset]
// [spec:posix:req:expand.param-use-alternative]
// [spec:dash:sem:expand.scanright-fn]
// [spec:dash:sem:expand.subevalvar-fn]
// [spec:dash:sem:expand.varunset-fn]
// [spec:dash:sem:expand.varvalue-fn]
fn expand_parameter(
    shell: &mut Shell,
    parameter: &ParameterExpansion,
    context: Context,
) -> Result<Expansion, Error> {
    if parameter.operation == ParameterOperation::Invalid {
        return Err(shell.diagnostics().shell_error(b"Bad substitution"));
    }

    let name = crate::variables::assignment_name(parameter.name.as_bstr()).to_owned();
    let value = parameter_value(shell, name.as_bstr());
    let unavailable = value.is_unset() || (parameter.colon && value.is_empty(shell, context));

    match parameter.operation {
        ParameterOperation::Value => value_expansion(shell, name.as_bstr(), value, context),
        ParameterOperation::Default if unavailable => operand_expansion(shell, parameter, context),
        ParameterOperation::Default => value_expansion(shell, name.as_bstr(), value, context),
        ParameterOperation::Alternate if unavailable => Ok(empty_value(context)),
        ParameterOperation::Alternate => operand_expansion(shell, parameter, context),
        ParameterOperation::Error if unavailable => {
            let message = operand_expansion(
                shell,
                parameter,
                Context {
                    full: false,
                    ..context
                },
            )?
            .collapse()
            .bytes;
            let custom_message = parameter
                .operand
                .as_deref()
                .filter(|operand| !operand.is_empty())
                .map(|_| message.as_slice());
            Err(parameter_error(
                shell,
                name.as_bstr(),
                parameter.colon,
                custom_message,
            ))
        }
        ParameterOperation::Error => value_expansion(shell, name.as_bstr(), value, context),
        ParameterOperation::Assign if unavailable => {
            // Assignment operands are first reduced to a scalar, then the
            // assigned value is expanded in the surrounding context.  This
            // is observable for `${v="$@"}`: parameter boundaries join for
            // the stored value and ordinary splitting applies afterward.
            let assigned = operand_expansion(
                shell,
                parameter,
                Context {
                    full: false,
                    ..context
                },
            )?
            .collapse()
            .bytes;
            crate::variables::set_bytes(
                shell,
                name.as_bstr(),
                Some(assigned.as_bstr()),
                crate::variables::VariableAttributes::NONE,
            )?;
            value_expansion(
                shell,
                name.as_bstr(),
                Value::Variable(VariableValue::Scalar(assigned)),
                context,
            )
        }
        ParameterOperation::Assign => value_expansion(shell, name.as_bstr(), value, context),
        ParameterOperation::Length => {
            if value.is_unset() && shell.options.enabled(ShellOption::Nounset) {
                return Err(parameter_error(shell, name.as_bstr(), false, None));
            }
            let length = value_length(shell, &value);
            Ok(Expansion::one(Field::from_bytes(
                length.to_string().as_bytes(),
                context.protects(),
                context.splits(),
                context.quoted,
            )))
        }
        ParameterOperation::RemoveSmallestSuffix
        | ParameterOperation::RemoveLargestSuffix
        | ParameterOperation::RemoveSmallestPrefix
        | ParameterOperation::RemoveLargestPrefix => {
            if value.is_unset() {
                if shell.options.enabled(ShellOption::Nounset) {
                    return Err(parameter_error(shell, name.as_bstr(), false, None));
                }
                return Ok(empty_value(context));
            }
            let pattern = pattern_operand(shell, parameter, context)?;
            let positional_words = match &value {
                Value::At(words) if context.full => Some(words),
                Value::Star(words) if context.full && !context.quoted => Some(words),
                _ => None,
            };
            if let Some(words) = positional_words {
                return Ok(Expansion {
                    fields: words
                        .iter()
                        .map(|word| {
                            let trimmed = trim(&shell.locale, word, &pattern, parameter.operation);
                            Field::from_bytes(
                                &trimmed,
                                context.protects(),
                                !context.quoted,
                                context.quoted,
                            )
                        })
                        .collect(),
                });
            }
            let bytes = value_bytes(shell, value, context);
            let trimmed = trim(&shell.locale, &bytes, &pattern, parameter.operation);
            Ok(Expansion::one(Field::from_bytes(
                &trimmed,
                context.protects(),
                context.splits(),
                context.quoted,
            )))
        }
        ParameterOperation::Invalid => unreachable!(),
    }
}

fn parameter_value(shell: &mut Shell, name: &BStr) -> Value {
    match name.first().copied() {
        Some(b'$') if name.len() == 1 => Value::Variable(VariableValue::Scalar(BString::from(
            shell.root_pid.to_string(),
        ))),
        Some(b'?') if name.len() == 1 => Value::Variable(VariableValue::Scalar(BString::from(
            shell.status.to_string(),
        ))),
        Some(b'#') if name.len() == 1 => Value::Variable(VariableValue::Scalar(BString::from(
            shell
                .options
                .positional_parameters
                .parameter_count
                .to_string(),
        ))),
        Some(b'!') if name.len() == 1 => match shell.background_process {
            Some(pid) => Value::Variable(VariableValue::Scalar(BString::from(pid.to_string()))),
            None => Value::Unset,
        },
        Some(b'-') if name.len() == 1 => {
            let mut flags = BString::new(Vec::new());
            for spec in OPTION_SPECS.iter().rev() {
                if shell.options.enabled(spec.option)
                    && let Some(letter) = spec.letter
                {
                    flags.push(letter);
                }
            }
            Value::Variable(VariableValue::Scalar(flags))
        }
        Some(b'@') if name.len() == 1 => Value::At(shell.options.positional_parameters.words()),
        Some(b'*') if name.len() == 1 => Value::Star(shell.options.positional_parameters.words()),
        Some(first) if first.is_ascii_digit() => {
            let Some(index) = decimal_index(name) else {
                return Value::Unset;
            };
            if index == 0 {
                shell
                    .options
                    .argument_zero()
                    .map(BStr::to_owned)
                    .map(VariableValue::Scalar)
                    .map(Value::Variable)
                    .unwrap_or(Value::Unset)
            } else {
                shell
                    .options
                    .positional_parameters
                    .words()
                    .get(index - 1)
                    .cloned()
                    .map(VariableValue::Scalar)
                    .map(Value::Variable)
                    .unwrap_or(Value::Unset)
            }
        }
        _ => crate::variables::value::variable_value_owned(shell, name)
            .map(Value::Variable)
            .unwrap_or(Value::Unset),
    }
}

fn decimal_index(name: &BStr) -> Option<usize> {
    name.iter().try_fold(0usize, |value, byte| {
        byte.is_ascii_digit().then(|| {
            value
                .saturating_mul(10)
                .saturating_add((byte - b'0') as usize)
        })
    })
}

fn value_expansion(
    shell: &mut Shell,
    name: &BStr,
    value: Value,
    context: Context,
) -> Result<Expansion, Error> {
    match value {
        Value::Unset => {
            if shell.options.enabled(ShellOption::Nounset) {
                Err(parameter_error(shell, name, false, None))
            } else {
                Ok(empty_value(context))
            }
        }
        Value::Variable(value) => Ok(Expansion::one(Field::from_bytes(
            value.scalar_ref().unwrap_or_else(|| BStr::new(b"")),
            context.protects(),
            context.splits(),
            context.quoted,
        ))),
        Value::At(words) if context.full => {
            if words.is_empty() {
                return Ok(Expansion::none());
            }
            let last = words.len() - 1;
            Ok(Expansion {
                fields: words
                    .iter()
                    .enumerate()
                    .map(|(index, word)| {
                        Field::from_bytes(
                            word,
                            context.protects(),
                            !context.quoted,
                            context.quoted || index < last,
                        )
                    })
                    .collect(),
            })
        }
        Value::Star(words) if context.full && !context.quoted => {
            if words.is_empty() {
                return Ok(Expansion::none());
            }
            if effective_ifs(shell).is_empty() {
                return Ok(Expansion {
                    fields: words
                        .iter()
                        .map(|word| Field::from_bytes(word, false, true, false))
                        .collect(),
                });
            }
            let joined = join_parameters(shell, &words);
            Ok(Expansion::one(Field::from_bytes(
                &joined, false, true, false,
            )))
        }
        Value::At(words) | Value::Star(words) => {
            let joined = join_parameters(shell, &words);
            Ok(Expansion::one(Field::from_bytes(
                &joined,
                context.protects(),
                context.splits(),
                context.quoted,
            )))
        }
    }
}

fn empty_value(context: Context) -> Expansion {
    Expansion::one(Field::from_bytes(b"", false, false, context.quoted))
}

fn join_parameters(shell: &Shell, words: &[BString]) -> BString {
    let separator = first_ifs_character(shell);
    let mut joined = BString::new(Vec::new());
    for (index, word) in words.iter().enumerate() {
        if index != 0 {
            joined.extend_from_slice(separator);
        }
        joined.extend_from_slice(word);
    }
    joined
}

fn first_ifs_character(shell: &Shell) -> &[u8] {
    let ifs = effective_ifs(shell);
    if ifs.is_empty() {
        return b"";
    }
    let width = character_end(&shell.locale, ifs, 0);
    &ifs[..width]
}

fn operand_expansion(
    shell: &mut Shell,
    parameter: &ParameterExpansion,
    context: Context,
) -> Result<Expansion, Error> {
    match parameter.operand.as_deref() {
        Some(word) => expand_parts(shell, word.parts(), context.operand()),
        None => Ok(empty_value(context)),
    }
}

fn pattern_operand(
    shell: &mut Shell,
    parameter: &ParameterExpansion,
    context: Context,
) -> Result<Pattern, Error> {
    let field = match parameter.operand.as_deref() {
        Some(word) => expand_parts(shell, word.parts(), context.pattern_operand())?.collapse(),
        None => Field::default(),
    };
    Ok(field.pattern())
}

fn parameter_error(
    shell: &mut Shell,
    name: &BStr,
    colon: bool,
    expanded_message: Option<&[u8]>,
) -> Error {
    let mut message = BString::from(name);
    message.extend_from_slice(b": ");
    if let Some(expanded_message) = expanded_message {
        message.extend_from_slice(expanded_message);
    } else {
        message.extend_from_slice(b"parameter not set");
        if colon {
            message.extend_from_slice(b" or null");
        }
    }
    if shell.evaluation.expanding_trace_prompt {
        shell.diagnostics().shell_error(&message)
    } else {
        shell.diagnostics().expansion_error_value(&message)
    }
}

fn value_bytes(shell: &Shell, value: Value, context: Context) -> BString {
    match value {
        Value::Unset => BString::new(Vec::new()),
        Value::Variable(value) => value.scalar_owned().unwrap_or_default(),
        Value::At(words) | Value::Star(words) => {
            if context.full && !context.quoted {
                let mut joined = BString::new(Vec::new());
                for word in words {
                    joined.extend_from_slice(&word);
                }
                joined
            } else {
                join_parameters(shell, &words)
            }
        }
    }
}

// [spec:posix:req:expand.param-string-length]
fn value_length(shell: &Shell, value: &Value) -> usize {
    match value {
        Value::Unset => 0,
        Value::Variable(value) => value
            .scalar_ref()
            .map_or(0, |bytes| character_count(&shell.locale, bytes)),
        Value::At(words) | Value::Star(words) => {
            let values = words
                .iter()
                .map(|word| character_count(&shell.locale, word))
                .sum::<usize>();
            let separator_count = words.len().saturating_sub(1);
            let separator_width = character_count(&shell.locale, first_ifs_character(shell));
            values + separator_count * separator_width
        }
    }
}

// [spec:posix:req:expand.param-substring-common]
// [spec:posix:req:expand.param-remove-smallest-suffix]
// [spec:posix:req:expand.param-remove-largest-suffix]
// [spec:posix:req:expand.param-remove-smallest-prefix]
// [spec:posix:req:expand.param-remove-largest-prefix]
fn trim(
    locale: &nsh_platform::Locale,
    value: &[u8],
    pattern: &Pattern,
    operation: ParameterOperation,
) -> BString {
    let boundaries = character_boundaries(locale, value);
    let cut = match operation {
        ParameterOperation::RemoveSmallestSuffix => boundaries
            .iter()
            .rev()
            .copied()
            .find(|at| pattern.matches(locale, &value[*at..]))
            .map(|at| (0, at)),
        ParameterOperation::RemoveLargestSuffix => boundaries
            .iter()
            .copied()
            .find(|at| pattern.matches(locale, &value[*at..]))
            .map(|at| (0, at)),
        ParameterOperation::RemoveSmallestPrefix => boundaries
            .iter()
            .copied()
            .find(|at| pattern.matches(locale, &value[..*at]))
            .map(|at| (at, value.len())),
        ParameterOperation::RemoveLargestPrefix => boundaries
            .iter()
            .rev()
            .copied()
            .find(|at| pattern.matches(locale, &value[..*at]))
            .map(|at| (at, value.len())),
        _ => unreachable!("only trimming operations reach trim"),
    };
    cut.map_or_else(
        || BString::from(value),
        |(start, end)| BString::from(&value[start..end]),
    )
}

fn character_count(locale: &nsh_platform::Locale, bytes: &[u8]) -> usize {
    character_boundaries(locale, bytes).len().saturating_sub(1)
}

fn character_boundaries(locale: &nsh_platform::Locale, bytes: &[u8]) -> Vec<usize> {
    let mut boundaries = vec![0];
    let mut at = 0;
    while at < bytes.len() {
        at = character_end(locale, bytes, at);
        boundaries.push(at);
    }
    boundaries
}

fn character_end(locale: &nsh_platform::Locale, bytes: &[u8], at: usize) -> usize {
    if at >= bytes.len() {
        return bytes.len();
    }
    let width = locale
        .multibyte_len(&bytes[at..])
        .filter(|width| *width > 0 && at + width <= bytes.len())
        .unwrap_or(1);
    at + width
}

// [spec:posix:req:expand.cmdsub-semantics]
// [spec:posix:req:expand.cmdsub-no-reexpansion]
// [spec:dash:sem:expand.expbackq-fn]
fn command_substitution(shell: &mut Shell, command: Option<&Node>) -> Result<BString, Error> {
    let mut result = crate::evaluation::CommandSubstitution {
        descriptor: None,
        job_id: None,
    };
    let mut output = crate::error::with_interrupts_deferred(shell, |shell| {
        let mut output = BString::new(Vec::new());
        let mut buffer = [0u8; 128];

        crate::evaluation::evaluate_command_substitution(shell, command, &mut result)?;
        while let Some(descriptor) = result.descriptor.as_ref() {
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
            output.extend(buffer[..count].iter().copied().filter(|byte| *byte != 0));
        }
        if result.descriptor.take().is_some() {
            shell.evaluation.command_substitution_status =
                crate::jobs::wait_for_job(shell, result.job_id)?;
        }
        Ok::<_, Error>(output)
    })?;

    if let Some(error) = crate::error::poll_interrupt(shell.interrupt_context()) {
        return Err(error);
    }

    while output.last() == Some(&b'\n') {
        output.pop();
    }
    Ok(output)
}

fn effective_ifs(shell: &Shell) -> &[u8] {
    &shell.ifs.bytes
}

// Split eligibility lives beside each byte, so truncating a parallel linked
// list of regions is no longer an operation the implementation can forget.
// [spec:dash:sem:expand.removerecordregions-fn]
fn split_fields(shell: &Shell, fields: Vec<Field>) -> Vec<Field> {
    let ifs = ifs_characters(&shell.locale, effective_ifs(shell));
    fields
        .into_iter()
        .flat_map(|field| split_field(&shell.locale, field, &ifs))
        .collect()
}

struct IfsCharacter {
    bytes: BString,
    whitespace: bool,
}

fn ifs_characters(locale: &nsh_platform::Locale, ifs: &[u8]) -> Vec<IfsCharacter> {
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

fn split_field(locale: &nsh_platform::Locale, field: Field, ifs: &[IfsCharacter]) -> Vec<Field> {
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
            while next < field.bytes.len() {
                let Some((following, end)) = separator_at(locale, &field, ifs, next) else {
                    break;
                };
                if !following.whitespace {
                    break;
                }
                next = end;
            }
            let following_nonwhite = (next < field.bytes.len())
                .then(|| separator_at(locale, &field, ifs, next))
                .flatten()
                .filter(|(following, _)| !following.whitespace);
            if let Some((_, end)) = following_nonwhite {
                result.push(field.slice(start..at));
                next = end;
                while next < field.bytes.len() {
                    let Some((following, end)) = separator_at(locale, &field, ifs, next) else {
                        break;
                    };
                    if !following.whitespace {
                        break;
                    }
                    next = end;
                }
                start = next;
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
        while next < field.bytes.len() {
            let Some((following, end)) = separator_at(locale, &field, ifs, next) else {
                break;
            };
            if !following.whitespace {
                break;
            }
            next = end;
        }
        start = next;
        at = next;
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

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:sem:idiom.typed-expansion/test]
    // [spec:nsh:def:idiom.variable-expansion-state/test]
    #[test]
    fn typed_field_masks() {
        let field = Field::from_bytes(b"a*b", true, false, true);
        assert_eq!(field.bytes, BString::from("a*b"));
        assert_eq!(
            field.regions,
            vec![FieldRegion {
                start: 0,
                end: 3,
                quoted: true,
                splittable: false,
            }]
        );
        assert!(field.empty_anchors.is_empty());

        let empty = Field::from_bytes(b"", true, false, true);
        assert_eq!(empty.empty_anchors, vec![0]);

        let indexed = Value::Variable(VariableValue::empty(
            crate::variables::value::VariableKind::Indexed,
        ));
        let associative = Value::Variable(VariableValue::empty(
            crate::variables::value::VariableKind::Associative,
        ));

        assert!(!indexed.is_unset());
        let shell = Shell::builder().build().unwrap();
        assert!(indexed.is_empty(&shell, Context::top(ExpansionMode::SPLIT)));
        assert!(matches!(
            indexed,
            Value::Variable(VariableValue::Indexed(_))
        ));
        assert!(matches!(
            associative,
            Value::Variable(VariableValue::Associative(_))
        ));
    }

    #[test]
    fn expanded_backslash_quotes_pattern_byte() {
        let locale = nsh_platform::Locale::c().unwrap();
        let pattern = Field::from_bytes(b"\\*", false, false, false).pattern();

        assert!(pattern.matches(&locale, b"*"));
        assert!(!pattern.matches(&locale, b"anything"));
    }

    #[test]
    fn field_metadata_stays_sparse() {
        let mut field = Field::from_bytes(&vec![b'x'; 131_072], false, true, false);
        assert_eq!(field.regions.len(), 1);

        field.append(Field::from_bytes(b"quoted", true, false, false));
        field.append(Field::from_bytes(b"-tail", true, false, false));
        assert_eq!(field.regions.len(), 2);

        let slice = field.slice(131_068..131_080);
        assert_eq!(slice.bytes, BString::from("xxxxquoted-t"));
        assert_eq!(
            slice.regions,
            [
                FieldRegion {
                    start: 0,
                    end: 4,
                    quoted: false,
                    splittable: true,
                },
                FieldRegion {
                    start: 4,
                    end: 12,
                    quoted: true,
                    splittable: false,
                },
            ]
        );
    }
}
