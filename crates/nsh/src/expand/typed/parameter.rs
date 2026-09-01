//! `${name}` and its operators: what a parameter's value is, and what the
//! expansion does to it.
//!
//! The other five siblings here are what happens to a *word*: braces,
//! tildes, fields, patterns, splitting. This is what happens to a *name* --
//! reading it, following a subscript into an array, deciding whether it
//! counts as unset or merely empty, and then applying whichever of the
//! dozen operators the word spelled.
//!
//! Moved here unchanged. `typed.rs` had every sibling in a file of its own
//! and this, the largest of them, still in the middle of it.

use super::*;

#[derive(Clone)]
pub(super) enum Value {
    Unset,
    Variable(VariableValue),
    At(Vec<BString>),
    Star(Vec<BString>),
}

impl Value {
    pub(super) fn is_unset(&self) -> bool {
        matches!(self, Self::Unset)
    }

    pub(super) fn is_empty(&self, shell: &Shell, context: Context) -> bool {
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
pub(super) fn expand_parameter(
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
pub(super) fn split_subscript(name: &BStr) -> Option<(&BStr, &BStr)> {
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
pub(super) fn value_expansion(
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
pub(super) fn join_parameters(words: &[BString], separator: &[u8]) -> BString {
    let mut joined = BString::new(Vec::new());
    for (index, word) in words.iter().enumerate() {
        if index != 0 {
            joined.extend_from_slice(separator);
        }
        joined.extend_from_slice(word);
    }
    joined
}

pub(super) fn first_ifs_character(shell: &Shell) -> &[u8] {
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

pub(super) fn value_bytes(shell: &Shell, value: Value, context: Context) -> BString {
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
