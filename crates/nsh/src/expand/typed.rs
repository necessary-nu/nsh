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

use super::{ExpansionMode, arglist, strlist};
use crate::context::Shell;
use crate::error::Error;
use crate::nodes::Node;
use crate::options::{OPTION_SPECS, ShellOption};
// [spec:nsh:def:idiom.shell-options]
use crate::pmatch::Pattern;
use crate::word::{ParameterExpansion, ParameterOperation, ParsedWord, QuoteBoundary, WordPart};

mod pathname;

#[derive(Clone, Debug, Default)]
struct Field {
    bytes: BString,
    quoted: Vec<bool>,
    splittable: Vec<bool>,
    preserve_empty: bool,
}

impl Field {
    fn from_bytes(bytes: &[u8], quoted: bool, splittable: bool, preserve_empty: bool) -> Self {
        let bytes = bytes
            .iter()
            .copied()
            .filter(|byte| *byte != 0)
            .collect::<Vec<_>>();
        let len = bytes.len();
        Self {
            bytes: BString::from(bytes),
            quoted: vec![quoted; len],
            splittable: vec![splittable; len],
            preserve_empty,
        }
    }

    fn append(&mut self, mut other: Self) {
        self.bytes.append(&mut other.bytes);
        self.quoted.append(&mut other.quoted);
        self.splittable.append(&mut other.splittable);
        self.preserve_empty |= other.preserve_empty;
    }

    fn slice(&self, range: std::ops::Range<usize>) -> Self {
        Self {
            bytes: BString::from(&self.bytes[range.clone()]),
            quoted: self.quoted[range.clone()].to_vec(),
            splittable: self.splittable[range].to_vec(),
            preserve_empty: false,
        }
    }

    fn pattern(&self) -> Pattern {
        Pattern::new(self.bytes.clone(), self.quoted.clone())
    }
}

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
            field.preserve_empty = true;
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
            tilde_after_colon: false,
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
pub(super) fn expand_argument(
    sh: &mut Shell,
    word: &ParsedWord,
    output: Option<&mut arglist>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let context = Context::top(mode);
    let expanded = expand_parts(sh, word.parts(), context)?;

    if let Some(output) = output {
        let fields = if context.full {
            let mut split = split_fields(sh, expanded.fields);
            if !sh.options.enabled(ShellOption::NoGlob) {
                split = pathname::expand(sh, split);
            }
            split
        } else {
            vec![expanded.collapse()]
        };
        output.list.extend(fields.into_iter().map(terminated));
    } else {
        let field = expanded.collapse();
        sh.expand.buffer.clear();
        sh.expand.buffer.extend_from_slice(&field.bytes);
        sh.expand.buffer.push(0);
    }

    super::ifsfree(&mut sh.expand);
    Ok(())
}

// [spec:nsh:sem:idiom.typed-expansion]
pub(super) fn case_matches(sh: &mut Shell, word: &ParsedWord, value: &BStr) -> Result<bool, Error> {
    let context = Context {
        quoted: false,
        full: false,
        operand: false,
        pattern: true,
        tilde_at_start: true,
        tilde_after_equal: false,
        tilde_after_colon: false,
    };
    let pattern = expand_parts(sh, word.parts(), context)?
        .collapse()
        .pattern();
    Ok(pattern.matches(&sh.locale, value))
}

fn terminated(mut field: Field) -> strlist {
    field.bytes.push(0);
    strlist { text: field.bytes }
}

fn expand_parts(sh: &mut Shell, parts: &[WordPart], context: Context) -> Result<Expansion, Error> {
    let mut result = Expansion::builder();
    let mut at = 0;
    let mut tilde = if context.tilde_at_start && !context.quoted {
        TildePosition::WordStart
    } else {
        TildePosition::None
    };
    let mut assignment_equal_available = context.tilde_after_equal;

    while at < parts.len() {
        match &parts[at] {
            WordPart::Literal(bytes) => {
                append_literal(
                    sh,
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
                if !is_empty_quoted_at(sh, inner, context) {
                    let mut quoted = expand_parts(sh, inner, context.quoted())?;
                    quoted.preserve_empty();
                    result.append(quoted);
                }
                at = close;
                tilde = TildePosition::None;
            }
            WordPart::Quote(QuoteBoundary::Close) => {}
            WordPart::Parameter(parameter) => {
                result.append(expand_parameter(sh, parameter, context)?);
                tilde = TildePosition::None;
            }
            WordPart::Command(command) => {
                let bytes = command_substitution(sh, command.as_deref())?;
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
                    expand_parts(sh, expression.parts(), arithmetic_context)?.collapse();
                let number = crate::arith_yacc::arith(sh, expression.bytes.as_bstr())?;
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

fn is_empty_quoted_at(sh: &Shell, parts: &[WordPart], context: Context) -> bool {
    context.full
        && sh.options.shellparam.nparam == 0
        && matches!(
            parts,
            [WordPart::Parameter(ParameterExpansion {
                name,
                operation: ParameterOperation::Value,
                ..
            })] if name.as_slice() == b"@"
        )
}

fn append_literal(
    sh: &mut Shell,
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
                && let Some(home) = tilde_home(sh, &bytes[at + 1..end])
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

fn tilde_home(sh: &mut Shell, user: &[u8]) -> Option<Vec<u8>> {
    if user.is_empty() {
        crate::var::lookup_bytes(sh, BStr::new(b"HOME")).map(|home| home.to_vec())
    } else {
        let user = user.try_to_os_string().ok()?;
        nsh_platform::named_user_home(&user).map(|home| home.to_shell_bytes())
    }
}

#[derive(Clone)]
enum Value {
    Unset,
    Scalar(BString),
    At(Vec<BString>),
    Star(Vec<BString>),
}

impl Value {
    fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    fn is_empty(&self) -> bool {
        match self {
            Self::Unset => true,
            Self::Scalar(bytes) => bytes.is_empty(),
            Self::At(words) | Self::Star(words) => words.is_empty(),
        }
    }
}

fn expand_parameter(
    sh: &mut Shell,
    parameter: &ParameterExpansion,
    context: Context,
) -> Result<Expansion, Error> {
    if parameter.operation == ParameterOperation::Invalid {
        return Err(sh.sh_error_value(b"Bad substitution"));
    }

    let name = crate::var::varname(parameter.name.as_bstr()).to_owned();
    let value = parameter_value(sh, name.as_bstr());
    let unavailable = value.is_unset() || (parameter.colon && value.is_empty());

    match parameter.operation {
        ParameterOperation::Value => value_expansion(sh, name.as_bstr(), value, context),
        ParameterOperation::Default if unavailable => operand_expansion(sh, parameter, context),
        ParameterOperation::Default => value_expansion(sh, name.as_bstr(), value, context),
        ParameterOperation::Alternate if unavailable => Ok(empty_value(context)),
        ParameterOperation::Alternate => operand_expansion(sh, parameter, context),
        ParameterOperation::Error if unavailable => {
            let message = operand_expansion(
                sh,
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
                sh,
                name.as_bstr(),
                parameter.colon,
                custom_message,
            ))
        }
        ParameterOperation::Error => value_expansion(sh, name.as_bstr(), value, context),
        ParameterOperation::Assign if unavailable => {
            let expanded = operand_expansion(sh, parameter, context)?;
            let assigned = expanded.clone().collapse().bytes;
            crate::var::set_bytes(sh, name.as_bstr(), Some(assigned.as_bstr()), 0)?;
            Ok(expanded)
        }
        ParameterOperation::Assign => value_expansion(sh, name.as_bstr(), value, context),
        ParameterOperation::Length => {
            if value.is_unset() && sh.options.enabled(ShellOption::Nounset) {
                return Err(parameter_error(sh, name.as_bstr(), false, None));
            }
            let length = value_length(sh, &value, context);
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
                if sh.options.enabled(ShellOption::Nounset) {
                    return Err(parameter_error(sh, name.as_bstr(), false, None));
                }
                return Ok(empty_value(context));
            }
            let pattern = pattern_operand(sh, parameter, context)?;
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
                            let trimmed = trim(&sh.locale, word, &pattern, parameter.operation);
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
            let bytes = value_bytes(sh, value, context);
            let trimmed = trim(&sh.locale, &bytes, &pattern, parameter.operation);
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

fn parameter_value(sh: &mut Shell, name: &BStr) -> Value {
    match name.first().copied() {
        Some(b'$') if name.len() == 1 => Value::Scalar(BString::from(sh.root_pid.to_string())),
        Some(b'?') if name.len() == 1 => Value::Scalar(BString::from(sh.status.to_string())),
        Some(b'#') if name.len() == 1 => {
            Value::Scalar(BString::from(sh.options.shellparam.nparam.to_string()))
        }
        Some(b'!') if name.len() == 1 => match sh.backgndpid {
            Some(pid) => Value::Scalar(BString::from(pid.to_string())),
            None => Value::Unset,
        },
        Some(b'-') if name.len() == 1 => {
            let mut flags = BString::new(Vec::new());
            for spec in OPTION_SPECS.iter().rev() {
                if sh.options.enabled(spec.option)
                    && let Some(letter) = spec.letter
                {
                    flags.push(letter);
                }
            }
            Value::Scalar(flags)
        }
        Some(b'@') if name.len() == 1 => Value::At(sh.options.shellparam.words()),
        Some(b'*') if name.len() == 1 => Value::Star(sh.options.shellparam.words()),
        Some(first) if first.is_ascii_digit() => {
            let Some(index) = decimal_index(name) else {
                return Value::Unset;
            };
            if index == 0 {
                sh.options
                    .arg0()
                    .map(BStr::to_owned)
                    .map(Value::Scalar)
                    .unwrap_or(Value::Unset)
            } else {
                sh.options
                    .shellparam
                    .words()
                    .get(index - 1)
                    .cloned()
                    .map(Value::Scalar)
                    .unwrap_or(Value::Unset)
            }
        }
        _ => crate::var::lookup_bytes(sh, name)
            .map(Value::Scalar)
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
    sh: &mut Shell,
    name: &BStr,
    value: Value,
    context: Context,
) -> Result<Expansion, Error> {
    match value {
        Value::Unset => {
            if sh.options.enabled(ShellOption::Nounset) {
                Err(parameter_error(sh, name, false, None))
            } else {
                Ok(empty_value(context))
            }
        }
        Value::Scalar(bytes) => Ok(Expansion::one(Field::from_bytes(
            &bytes,
            context.protects(),
            context.splits(),
            context.quoted,
        ))),
        Value::At(words) if context.full => {
            if words.is_empty() {
                return Ok(Expansion::none());
            }
            Ok(Expansion {
                fields: words
                    .iter()
                    .map(|word| {
                        Field::from_bytes(word, context.protects(), !context.quoted, context.quoted)
                    })
                    .collect(),
            })
        }
        Value::Star(words) if context.full && !context.quoted => {
            if words.is_empty() {
                return Ok(Expansion::none());
            }
            Ok(Expansion {
                fields: words
                    .iter()
                    .map(|word| Field::from_bytes(word, false, true, false))
                    .collect(),
            })
        }
        Value::At(words) | Value::Star(words) => {
            let joined = join_parameters(sh, &words);
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

fn join_parameters(sh: &Shell, words: &[BString]) -> BString {
    let separator = first_ifs_character(sh);
    let mut joined = BString::new(Vec::new());
    for (index, word) in words.iter().enumerate() {
        if index != 0 {
            joined.extend_from_slice(separator);
        }
        joined.extend_from_slice(word);
    }
    joined
}

fn first_ifs_character(sh: &Shell) -> &[u8] {
    let ifs = effective_ifs(sh);
    if ifs.is_empty() {
        return b"";
    }
    let width = character_end(&sh.locale, ifs, 0);
    &ifs[..width]
}

fn operand_expansion(
    sh: &mut Shell,
    parameter: &ParameterExpansion,
    context: Context,
) -> Result<Expansion, Error> {
    match parameter.operand.as_deref() {
        Some(word) => expand_parts(sh, word.parts(), context.operand()),
        None => Ok(empty_value(context)),
    }
}

fn pattern_operand(
    sh: &mut Shell,
    parameter: &ParameterExpansion,
    context: Context,
) -> Result<Pattern, Error> {
    let field = match parameter.operand.as_deref() {
        Some(word) => expand_parts(sh, word.parts(), context.pattern_operand())?.collapse(),
        None => Field::default(),
    };
    Ok(field.pattern())
}

fn parameter_error(
    sh: &mut Shell,
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
    if sh.eval.inps4 != 0 {
        sh.sh_error_value(&message)
    } else {
        sh.expansion_error_value(&message)
    }
}

fn value_bytes(sh: &Shell, value: Value, context: Context) -> BString {
    match value {
        Value::Unset => BString::new(Vec::new()),
        Value::Scalar(bytes) => bytes,
        Value::At(words) | Value::Star(words) => {
            if context.full && !context.quoted {
                let mut joined = BString::new(Vec::new());
                for word in words {
                    joined.extend_from_slice(&word);
                }
                joined
            } else {
                join_parameters(sh, &words)
            }
        }
    }
}

fn value_length(sh: &Shell, value: &Value, context: Context) -> usize {
    match value {
        Value::Unset => 0,
        Value::Scalar(bytes) => character_count(&sh.locale, bytes),
        Value::At(words) | Value::Star(words) => {
            let values = words
                .iter()
                .map(|word| character_count(&sh.locale, word))
                .sum::<usize>();
            let separator_count = words.len().saturating_sub(1);
            let separator_width = character_count(&sh.locale, first_ifs_character(sh));
            values + separator_count * separator_width
        }
    }
}

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

fn command_substitution(sh: &mut Shell, command: Option<&Node>) -> Result<BString, Error> {
    let mut result = crate::eval::backcmd { fd: None, jp: None };
    let mut output = BString::new(Vec::new());
    let mut buffer = [0u8; 128];

    crate::error::INTOFF(sh);
    crate::eval::evalbackcmd(sh, command, &mut result)?;
    while let Some(fd) = result.fd.as_ref() {
        let count = loop {
            match nsh_platform::read_once(fd, &mut buffer) {
                Ok(count) => break count,
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                    if let Some(error) = crate::error::poll_interrupt(sh) {
                        return Err(error);
                    }
                }
                Err(_) => break 0,
            }
        };
        if count == 0 {
            break;
        }
        output.extend(buffer[..count].iter().copied().filter(|byte| *byte != 0));
    }
    if result.fd.take().is_some() {
        sh.eval.back_exitstatus = crate::jobs::waitforjob(sh, result.jp)?;
    }
    crate::error::INTON(sh);

    while output.last() == Some(&b'\n') {
        output.pop();
    }
    Ok(output)
}

fn effective_ifs(sh: &Shell) -> &[u8] {
    sh.ifs
        .ncifs
        .strip_suffix(&[0])
        .unwrap_or(sh.ifs.ncifs.as_slice())
}

fn split_fields(sh: &Shell, fields: Vec<Field>) -> Vec<Field> {
    let ifs = ifs_characters(&sh.locale, effective_ifs(sh));
    fields
        .into_iter()
        .flat_map(|field| split_field(&sh.locale, field, &ifs))
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
    if !field.splittable.get(at).copied().unwrap_or(false) {
        return None;
    }
    let end = character_end(locale, &field.bytes, at);
    field.splittable[at..end]
        .iter()
        .all(|eligible| *eligible)
        .then(|| {
            ifs.iter()
                .find(|separator| separator.bytes.as_slice() == &field.bytes[at..end])
                .map(|separator| (separator, end))
        })
        .flatten()
}

fn split_field(locale: &nsh_platform::Locale, field: Field, ifs: &[IfsCharacter]) -> Vec<Field> {
    if ifs.is_empty() || !field.splittable.iter().any(|eligible| *eligible) {
        return if field.bytes.is_empty() && !field.preserve_empty {
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
            } else if at > start {
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
    } else if result.is_empty() && field.preserve_empty {
        result.push(Field {
            preserve_empty: true,
            ..Field::default()
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // [spec:nsh:sem:idiom.typed-expansion/test]
    fn typed_field_masks() {
        let field = Field::from_bytes(b"a*b", true, false, true);
        assert_eq!(field.bytes, BString::from("a*b"));
        assert_eq!(field.quoted, vec![true; 3]);
        assert_eq!(field.splittable, vec![false; 3]);
        assert!(field.preserve_empty);
    }
}
