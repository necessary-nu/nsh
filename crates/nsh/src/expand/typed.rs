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
use crate::options::{Dialect, OPTION_SPECS, ShellOption};
// [spec:nsh:def:idiom.shell-options]
use crate::pattern::Pattern;
use crate::variables::value::VariableValue;
use crate::word::{ParameterExpansion, ParameterOperation, ParsedWord, WordPart};

mod bash;
mod brace;
mod field;
mod pathname;
mod split;

use field::Field;
#[cfg(test)]
use field::FieldRegion;

#[derive(Clone, Debug)]
struct Expansion {
    fields: Vec<Field>,
}

impl Expansion {
    /// One empty field that survives field splitting.
    ///
    /// `""` writes a field the shell must keep even though it has no
    /// bytes. An expansion that produces *no* fields is a different
    /// thing and must stay distinguishable from this one.
    // [spec:posix:req:expand.quote-removal]
    fn anchored_empty() -> Self {
        let mut field = Field::default();
        field.anchor_empty();
        Self {
            fields: vec![field],
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
        if self.fields.is_empty() {
            return Field::default();
        }
        let mut first = self.fields.remove(0);
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
    let Some(output) = output else {
        let field = expand_parts(shell, word.parts(), context)?.collapse();
        shell.expand.buffer.clear();
        shell.expand.buffer.extend_from_slice(&field.bytes);
        return Ok(());
    };

    // Brace expansion precedes every other expansion and is the only one
    // that turns one word into several before they are expanded at all.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    let braced = if context.full {
        brace::expand(shell, word)?
    } else {
        Vec::new()
    };
    if braced.is_empty() {
        return expand_word(shell, word, output, context);
    }
    for word in &braced {
        expand_word(shell, word, output, context)?;
    }
    Ok(())
}

fn expand_word(
    shell: &mut Shell,
    word: &ParsedWord,
    output: &mut ExpandedFields,
    context: Context,
) -> Result<(), Error> {
    let expanded = expand_parts(shell, word.parts(), context)?;
    let fields = if context.full {
        let mut split = split::fields(shell, expanded.fields);
        if !shell.options.enabled(ShellOption::NoGlob) {
            let settings = pathname::settings(shell);
            split = pathname::expand(shell, split, &settings)?;
        }
        split
    } else {
        vec![expanded.collapse()]
    };
    output.fields.extend(fields.into_iter().map(into_field));
    Ok(())
}

/// Expand one word in a pattern position, retaining its quote bits.
///
/// `case` and `[[ ]]` ask the same question of a word, and the answer is
/// the pattern rather than a match: the regular-expression operator needs
/// the same bits to decide which bytes the shell already made literal.
// [spec:nsh:sem:idiom.typed-expansion]
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(super) fn conditional_pattern_of(
    shell: &mut Shell,
    word: &ParsedWord,
) -> Result<Pattern, Error> {
    let mut options = bash::match_options(shell);
    options.extended = shell.options.dialect() == Dialect::Bash;
    Ok(pattern_field(shell, word)?.pattern(options))
}

pub(super) fn pattern_of(shell: &mut Shell, word: &ParsedWord) -> Result<Pattern, Error> {
    let options = bash::match_options(shell);
    Ok(pattern_field(shell, word)?.pattern(options))
}

fn pattern_field(shell: &mut Shell, word: &ParsedWord) -> Result<Field, Error> {
    let context = Context {
        quoted: false,
        full: false,
        operand: false,
        pattern: true,
        tilde_at_start: true,
        tilde_after_equal: false,
        tilde_after_colon: false,
    };
    Ok(expand_parts(shell, word.parts(), context)?.collapse())
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
    /* No field is seeded. A part that expands to nothing -- `"$@"` with
     * no positional parameters, `"${a[@]}"` over an empty array -- has to
     * be able to leave the word with no fields at all, and a seeded field
     * would silently turn that into one empty field. */
    // [spec:posix:req:param.special-at-no-positional]
    let mut result = Expansion::none();
    let mut at = 0;
    let mut tilde = if context.tilde_at_start && !context.quoted {
        /* Where `:` separates tilde prefixes, it ends the first one too:
         * an assignment operand -- `${x-~:~}` inside one, or the value of
         * a `[k]=~:~` element, whose `name[k]=` the parser already took
         * off -- expands every colon-separated segment. */
        if context.tilde_after_colon {
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
            WordPart::Text {
                bytes,
                quoted: false,
            } => {
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
            /* A quoted run is data: it protects, it does not split, and
             * `''` is written text that keeps its empty field where an
             * expansion yielding nothing does not. Which quote made it so
             * is the node's run and not the tree's business. */
            // [spec:nsh:req:idiom.canonical-tree+1]
            WordPart::Text {
                bytes,
                quoted: true,
            } => {
                let mut quoted = if bytes.is_empty() {
                    Expansion::anchored_empty()
                } else {
                    let mut inner = Expansion::none();
                    let mut no_tilde = TildePosition::None;
                    let mut no_assignment = false;
                    append_literal(
                        shell,
                        &mut inner,
                        bytes,
                        context.quoted(),
                        at + 1 < parts.len(),
                        &mut no_tilde,
                        &mut no_assignment,
                    );
                    inner
                };
                quoted.preserve_empty();
                result.append(quoted);
                tilde = TildePosition::None;
            }
            WordPart::Parameter(parameter) => {
                let inner = quoting(context, parameter.quoted);
                let mut expanded = expand_parameter(shell, parameter, inner)?;
                if parameter.quoted {
                    expanded.preserve_empty();
                }
                result.append(expanded);
                tilde = TildePosition::None;
            }
            WordPart::Command { command, quoted } => {
                /* `$(list)` contributes the bytes the list wrote, and they
                 * are the list's data: unquoted, they split and they glob.
                 * Bash's `<(list)` and `>(list)` occupy the same lexical
                 * position and the same word part, but contribute a name the
                 * shell chose for a pipe the list is still using. That name
                 * is not data, so an `IFS` containing `/` leaves it whole. */
                // [spec:nsh:req:compat.bash.process-substitution]
                let inner = quoting(context, *quoted);
                let field = match command.as_deref() {
                    Some(Node::Bash(crate::nodes::BashNode::ProcessSubstitution(substitution))) => {
                        let name = crate::evaluation::bash_process_substitution::substitute(
                            shell,
                            substitution,
                        )?;
                        Field::from_bytes(&name, true, false, inner.quoted)
                    }
                    command => {
                        let bytes = command_substitution(shell, command)?;
                        Field::from_bytes(&bytes, inner.protects(), inner.splits(), inner.quoted)
                    }
                };
                let mut expanded = Expansion::one(field);
                if *quoted {
                    expanded.preserve_empty();
                }
                result.append(expanded);
                tilde = TildePosition::None;
            }
            WordPart::Arithmetic { expression, quoted } => {
                let arithmetic_context = Context {
                    quoted: false,
                    full: false,
                    operand: false,
                    pattern: false,
                    tilde_at_start: false,
                    tilde_after_equal: false,
                    tilde_after_colon: false,
                };
                let inner = quoting(context, *quoted);
                let expression =
                    expand_parts(shell, expression.parts(), arithmetic_context)?.collapse();
                let number = crate::arithmetic::evaluate(shell, expression.bytes.as_bstr())?;
                let rendered = number.to_string();
                let mut expanded = Expansion::one(Field::from_bytes(
                    rendered.as_bytes(),
                    inner.protects(),
                    inner.splits(),
                    inner.quoted,
                ));
                if *quoted {
                    expanded.preserve_empty();
                }
                result.append(expanded);
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

/// The context a part expands in, given whether the source quoted it.
///
/// Quoting used to be a pair of parts around a region and is a flag on
/// the part now, so the region walk is a question asked once per part.
// [spec:nsh:req:idiom.canonical-tree+1]
fn quoting(context: Context, quoted: bool) -> Context {
    if quoted { context.quoted() } else { context }
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

        /* The loop above is a state machine over three bytes and nothing
         * else reads a literal one at a time, so the bytes between them
         * are one contribution rather than one each. The field is the
         * same either way -- adjacent regions of equal quoting merge, and
         * an anchor inside a run sits on a byte quoting made unsplittable,
         * which is a byte splitting never asks about -- but a byte at a
         * time buys a field, four allocations and an anchor per byte of
         * the word, and the anchors then made appending quadratic. */
        let end = if TILDE_SIGNIFICANT.contains(&bytes[at]) {
            at + 1
        } else {
            bytes[at + 1..]
                .iter()
                .position(|byte| TILDE_SIGNIFICANT.contains(byte))
                .map_or(bytes.len(), |offset| at + 1 + offset)
        };
        result.append(Expansion::one(Field::from_bytes(
            &bytes[at..end],
            context.protects(),
            context.literal_splits(),
            context.quoted,
        )));
        let byte = bytes[end - 1];
        *tilde = if byte == b'=' && *assignment_equal_available {
            *assignment_equal_available = false;
            TildePosition::Assignment
        } else if byte == b':' && context.tilde_after_colon {
            TildePosition::Assignment
        } else {
            TildePosition::None
        };
        at = end;
    }
}

/// The literal bytes the tilde rules can tell apart.
///
/// `~` opens a prefix, and `=` and `:` are the separators after which
/// another may open. Every other byte closes any prefix under way and is
/// otherwise indistinguishable from its neighbours.
const TILDE_SIGNIFICANT: &[u8] = b"~=:";

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
    pub(super) fn is_unset(&self) -> bool {
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
            Self::At(words) | Self::Star(words) => {
                join_parameters(words, first_ifs_character(shell)).is_empty()
            }
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
// [spec:posix:req:exit.expansion-error]
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
        // [spec:nsh:req:compat.bash.error-boundary]
        return Err(shell.diagnostics().dialect_error(b"Bad substitution"));
    }

    let mut name = crate::variables::assignment_name(parameter.name.as_bstr()).to_owned();
    if parameter.indirect
        && let Some(expansion) =
            bash::indirect_names(shell, name.as_bstr(), parameter.operation, context)?
    {
        return Ok(expansion);
    }
    let mut value = read_value(shell, name.as_bstr())?;
    if parameter.indirect {
        /* A `declare -n` reference already reads through itself, so
         * Bash inverts `${!ref}` there: it answers with the *name* the
         * reference holds rather than dereferencing a second time. */
        // [spec:nsh:req:compat.bash.functions-scoping]
        if let Some(target) = crate::variables::nameref::reference_name(shell, name.as_bstr()) {
            return value_expansion(
                shell,
                name.as_bstr(),
                Value::Variable(VariableValue::Scalar(target)),
                context,
            );
        }
        // Otherwise `${!ref}` reads the name out of `ref` and expands
        // that. The text has to spell a parameter, and Bash refuses it
        // when it does not.
        // [spec:nsh:req:compat.bash.expansion-globbing]
        name = bash::indirect_target(shell, name.as_bstr(), value)?;
        value = read_value(shell, name.as_bstr())?;
    }
    /* An array with no elements answers `-`, `=`, `?` and `+` as an unset
     * name does, without the colon -- through a written `[@]` or `[*]`
     * subscript and through the bare name alike, since `$a` reads an
     * element that is not there either. `$@` and `$*` keep the POSIX
     * answer. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    let subscripted = split_subscript(name.as_bstr())
        .is_some_and(|(_, subscript)| subscript == "@" || subscript == "*");
    let empty_array = (subscripted
        && matches!(&value, Value::At(words) | Value::Star(words) if words.is_empty()))
        || matches!(&value, Value::Variable(stored)
            if stored.kind() != crate::variables::value::VariableKind::Scalar
                && crate::variables::arrays::elements(stored).is_empty());
    let unavailable =
        value.is_unset() || empty_array || (parameter.colon && value.is_empty(shell, context));

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
            // `${#a[@]}` counts elements; `${#a[0]}` counts characters.
            // The distinction is the subscript, not the value's kind.
            let whole_array = split_subscript(name.as_bstr())
                .is_some_and(|(_, subscript)| subscript == "@" || subscript == "*");
            let length = if whole_array {
                match &value {
                    Value::At(words) | Value::Star(words) => words.len(),
                    _ => value_length(shell, &value),
                }
            } else {
                value_length(shell, &value)
            };
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
            /* A word list is trimmed element by element; in Bash a
             * quoted `"${a[*]}"` is one too, and joins afterwards. */
            let operation = parameter.operation;
            bash::map_value(shell, value, context, |locale, text| {
                trim(locale, text, &pattern, operation)
            })
        }
        ParameterOperation::Substring
        | ParameterOperation::SubstituteFirst
        | ParameterOperation::SubstituteAll
        | ParameterOperation::UpperFirst
        | ParameterOperation::UpperAll
        | ParameterOperation::LowerFirst
        | ParameterOperation::LowerAll
        | ParameterOperation::Transform => {
            if value.is_unset() && shell.options.enabled(ShellOption::Nounset) {
                return Err(parameter_error(shell, name.as_bstr(), false, None));
            }
            match parameter.operation {
                ParameterOperation::Substring => {
                    bash::substring(shell, parameter, name.as_bstr(), value, context)
                }
                ParameterOperation::SubstituteFirst | ParameterOperation::SubstituteAll => {
                    bash::substitute(shell, parameter, name.as_bstr(), value, context)
                }
                ParameterOperation::Transform => {
                    bash::transform(shell, parameter, name.as_bstr(), value, context)
                }
                _ => bash::change_case(shell, parameter, value, context),
            }
        }
        ParameterOperation::Invalid => unreachable!(),
    }
}

/// Read one parameter, following a Bash name reference and an array
/// subscript where the name carries one.
// [spec:nsh:req:compat.bash.functions-scoping]
fn read_value(shell: &mut Shell, name: &BStr) -> Result<Value, Error> {
    // A Bash name reference reads the variable it points at; a circular
    // chain has nothing to read and behaves as unset.
    let target = crate::variables::nameref::read_name(shell, name);
    let Some(target) = target else {
        /* A whole-array subscript that resolves to nothing has no
         * elements, which is not the same as one empty field: quoted,
         * `"${ref[@]}"` contributes no argument at all. */
        return Ok(match split_subscript(name) {
            Some((_, subscript)) if subscript == "@" => Value::At(Vec::new()),
            Some((_, subscript)) if subscript == "*" => Value::Star(Vec::new()),
            _ => Value::Unset,
        });
    };
    match split_subscript(target.as_bstr()) {
        Some((base, subscript)) => {
            let base = base.to_owned();
            let subscript = subscript.to_owned();
            subscripted_value(shell, base.as_bstr(), subscript.as_bstr())
        }
        None => Ok(parameter_value(shell, target.as_bstr())),
    }
}

/// Split `a[expr]` into its name and subscript bytes.
///
/// Only a trailing `]` at the very end counts, so an ordinary name that
/// happens to contain a bracket is left alone.
fn split_subscript(name: &BStr) -> Option<(&BStr, &BStr)> {
    let bytes = name.as_ref() as &[u8];
    if bytes.last() != Some(&b']') {
        return None;
    }
    let open = bytes.iter().position(|byte| *byte == b'[')?;
    if open == 0 {
        return None;
    }
    Some((
        BStr::new(&bytes[..open]),
        BStr::new(&bytes[open + 1..bytes.len() - 1]),
    ))
}

/// Read one element, or every element, of an array-valued name.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn subscripted_value(shell: &mut Shell, base: &BStr, subscript: &BStr) -> Result<Value, Error> {
    use crate::variables::arrays::{self, ArraySelector};

    /* `a[]` has no expression in it, and an empty arithmetic expression is
     * 0 -- so without this the shell reads element zero of an array the
     * script never asked about. The test is on the written text, not on
     * what it expands to: `${a[$empty]}` names element 0 in Bash and here,
     * because there the script did write an expression. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    if subscript.is_empty() && shell.options.dialect() == Dialect::Bash {
        let mut message = BString::from(b"${".as_slice());
        message.extend_from_slice(base);
        message.extend_from_slice(b"[]}: bad substitution");
        return Err(shell.diagnostics().dialect_expansion_error(&message));
    }

    /* A read keeps its subscript as text, so its quoting and any expansion
     * written inside it are resolved here; the assignment side already
     * carries a word the expander has been through. */
    // [spec:nsh:req:compat.bash.functions-scoping]
    let selector = if shell.options.dialect() == Dialect::Bash {
        arrays::resolve_text_selector(shell, base, subscript)?
    } else {
        arrays::resolve_selector(shell, base, subscript)?
    };
    let Some(stored) = crate::variables::value::variable_value(shell, base).cloned() else {
        /* A whole-array read of a name that holds nothing is an array of no
         * elements, not an unset scalar: it contributes no fields and `set
         * -u` has nothing to complain about. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        return Ok(match selector {
            ArraySelector::All => Value::At(Vec::new()),
            ArraySelector::Joined => Value::Star(Vec::new()),
            ArraySelector::Missing | ArraySelector::Index(_) | ArraySelector::Key(_) => {
                Value::Unset
            }
        });
    };
    Ok(match selector {
        ArraySelector::All => Value::At(arrays::elements(&stored)),
        ArraySelector::Joined => Value::Star(arrays::elements(&stored)),
        ArraySelector::Index(index) => match stored.indexed(index) {
            Some(element) => Value::Variable(VariableValue::Scalar(element.to_owned())),
            // An indexed read of a scalar sees it as element zero.
            None if index == 0 => match stored.scalar_ref() {
                Some(element) => Value::Variable(VariableValue::Scalar(element.to_owned())),
                None => Value::Unset,
            },
            None => Value::Unset,
        },
        ArraySelector::Key(key) => match stored.associative(BStr::new(key.as_slice())) {
            Some(element) => Value::Variable(VariableValue::Scalar(element.to_owned())),
            None => Value::Unset,
        },
        /* Reported by the resolver and expanded to nothing. The command
         * the subscript was written in still runs, with one fewer field
         * than it asked for. */
        // [spec:nsh:req:compat.bash.error-boundary]
        ArraySelector::Missing => Value::Unset,
    })
}

// [spec:posix:def:param.special-parameters]
// [spec:posix:def:param.positional-definition]
// [spec:posix:req:param.positional-zero-not-positional]
// [spec:posix:req:param.special-at]
// [spec:posix:req:param.special-asterisk]
// [spec:posix:req:param.special-hash]
// [spec:posix:req:param.special-question]
// [spec:posix:sem:param.special-question-assignment]
// [spec:posix:req:param.special-hyphen]
// [spec:posix:req:param.special-dollar]
// [spec:posix:req:param.special-bang]
// [spec:posix:req:param.special-zero]
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

// [spec:posix:req:param.positional-decimal-digits]
fn decimal_index(name: &BStr) -> Option<usize> {
    name.iter().try_fold(0usize, |value, byte| {
        byte.is_ascii_digit().then(|| {
            value
                .saturating_mul(10)
                .saturating_add((byte - b'0') as usize)
        })
    })
}

// [spec:posix:req:param.special-at-double-quotes]
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
        Value::At(words) if context.full && context.quoted => {
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
        /* Unquoted, `$@` and `$*` are the same expansion: the words are
         * joined with IFS and the result is split like any other. Giving
         * `$@` a field per word instead kept an empty positional as an
         * empty field, where splitting would have dropped it -- so
         * `set -- '' b; echo $@` printed a leading space that Bash, dash
         * and POSIX do not. Found by the differential fuzz target on
         * 2026-09-01; the quoted form, where the two do differ, is the
         * arm above. */
        Value::At(words) | Value::Star(words) if context.full && !context.quoted => {
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
            let joined = join_parameters(&words, first_ifs_character(shell));
            Ok(Expansion::one(Field::from_bytes(
                &joined, false, true, false,
            )))
        }
        /* A context that cannot hold more than one field -- an
         * assignment, a here-document -- still has to join what `$@`
         * produced. `$*` joins with IFS, because choosing the separator
         * is what `$*` is for; `$@` joins with a space, because it never
         * named a separator and the fields it made were the point. dash
         * uses IFS for both, so `IFS=x; v="$@"` differs there; POSIX
         * leaves the case unspecified and a space is the answer that
         * does not silently rewrite the values. */
        Value::At(words) => {
            let joined = join_parameters(&words, b" ");
            Ok(Expansion::one(Field::from_bytes(
                &joined,
                context.protects(),
                context.splits(),
                context.quoted,
            )))
        }
        Value::Star(words) => {
            let joined = join_parameters(&words, first_ifs_character(shell));
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

/// Join what `$@` or `$*` produced into the one field its context can
/// hold. The separator is the caller's because the two do not share one:
/// see the `Value::At` arm above.
fn join_parameters(words: &[BString], separator: &[u8]) -> BString {
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
    let Some(word) = parameter.operand.as_deref() else {
        return Ok(empty_value(context));
    };
    let expanded = expand_parts(shell, word.parts(), context.operand())?;
    /* `${x-}` and `${x:-}` name the empty string, which is a value; only a
     * `[@]` or `[*]` expansion of nothing is *no* field. An operand made
     * of no parts at all must therefore still produce one. */
    // [spec:posix:req:expand.param-use-default]
    if expanded.fields.is_empty() {
        return Ok(empty_value(context));
    }
    Ok(expanded)
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
    Ok(field.pattern(bash::trim_options(shell)))
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
                join_parameters(&words, first_ifs_character(shell))
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
    // `$(<file)` never reaches a child: it is a file read wearing command
    // syntax.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    let mut output = match bash::file_substitution(shell, command)? {
        Some(content) => content,
        None => run_command_substitution(shell, command)?,
    };
    while output.last() == Some(&b'\n') {
        output.pop();
    }
    Ok(output)
}

fn run_command_substitution(shell: &mut Shell, command: Option<&Node>) -> Result<BString, Error> {
    let mut result = crate::evaluation::CommandSubstitution {
        descriptor: None,
        job_id: None,
    };
    let output = crate::error::with_interrupts_deferred(shell, |shell| {
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
    Ok(output)
}

fn effective_ifs(shell: &Shell) -> &[u8] {
    &shell.ifs.bytes
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
        assert_eq!(field.empty_anchors, vec![3]);

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
        let pattern = Field::from_bytes(b"\\*", false, false, false)
            .pattern(crate::pattern::PatternOptions::NONE);

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

        /* Sparse is also what a long word costs. A quoted run appended a
         * byte at a time left an anchor at every byte boundary and then
         * rescanned every anchor already recorded on the next append, so
         * reading a 200,000-byte word took seconds where dash takes
         * milliseconds. The metadata is the shape that gives that away
         * and a clock on a shared machine is not: what a run records must
         * not grow with how long the run is. */
        let mut shell = Shell::builder().build().unwrap();
        let mut recorded = |length: usize| {
            let mut expansion = Expansion::none();
            let mut tilde = TildePosition::None;
            let mut assignment_equal_available = false;
            append_literal(
                &mut shell,
                &mut expansion,
                &vec![b'a'; length],
                Context::top(ExpansionMode::SPLIT).quoted(),
                false,
                &mut tilde,
                &mut assignment_equal_available,
            );
            let run = expansion.collapse();
            (run.bytes.len(), run.regions.len(), run.empty_anchors)
        };

        assert_eq!(recorded(16), (16, 1, vec![16]));
        assert_eq!(recorded(200_000), (200_000, 1, vec![200_000]));
    }
}
