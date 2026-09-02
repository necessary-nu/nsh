//! Execution of Bash array and declaration assignments.
//!
//! The parser already produced [`BashArrayAssignment`]; this module turns
//! one into stored state. It sits between the parse tree and
//! [`crate::variables::arrays`], and owns exactly the part that needs the
//! expander: subscripts and element values are words, so they cannot be
//! resolved until the assignment actually runs.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::expand::{ExpandedField, ExpandedFields, ExpansionMode, expand_argument};
use crate::nodes::{
    BashArrayAssignment, BashArrayElement, BashArrayValue, BashAssignmentOperator, Node,
    SourceTokens, WordNode,
};
use crate::variables::arrays::{self, ArraySelector, CompoundElement, CompoundForm, ReadOnlyGuard};
use crate::word::{ParsedWord, WordUnit};

/// Apply one `a=(...)`, `a[i]=v`, or `a+=(...)` assignment.
///
/// `false` reports an assignment Bash refuses -- a list where one
/// element was named, or a write through a reference that already names
/// an element. The refusal has been reported and nothing was stored;
/// the caller turns it into the command's status.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn assign(
    shell: &mut Shell,
    assignment: &BashArrayAssignment,
    guard: ReadOnlyGuard,
) -> Result<bool, Error> {
    // An assignment to a Bash name reference reaches the variable it
    // names, so the reference is resolved before any subscript is.
    // [spec:nsh:req:compat.bash.functions-scoping]
    // [spec:nsh:req:compat.bash.arrays-declarations]
    if let Some(target) =
        crate::variables::nameref::refused_element_write(shell, assignment.name.as_bstr())
    {
        let mut message = b"`".to_vec();
        message.extend_from_slice(target.as_slice());
        message.extend_from_slice(b"': not a valid identifier");
        shell.diagnostics().shell_warning(&message);
        return Ok(false);
    }
    let Some(name) = crate::variables::nameref::element_base(shell, assignment.name.as_bstr())
    else {
        return Ok(true);
    };
    let append = assignment.operator == BashAssignmentOperator::Append;

    match &assignment.value {
        BashArrayValue::Word(word) => {
            let value = expand_value(shell, word)?;
            match &assignment.subscript {
                Some(subscript) => {
                    reject_empty_subscript(shell, BStr::new(name.as_slice()), subscript)?;
                    let selector =
                        element_selector(shell, BStr::new(name.as_slice()), assignment, subscript)?;
                    arrays::assign_element(
                        shell,
                        BStr::new(name.as_slice()),
                        &selector,
                        BStr::new(value.as_slice()),
                        append,
                        guard,
                    )?;
                    Ok(true)
                }
                // `a+=v` with no subscript appends to the zero element of
                // an array and concatenates onto a scalar -- without
                // making the scalar an array, which a written subscript
                // would.
                None if append => {
                    arrays::append_unsubscripted(
                        shell,
                        BStr::new(name.as_slice()),
                        BStr::new(value.as_slice()),
                        guard,
                    )?;
                    Ok(true)
                }
                None => {
                    arrays::assign_element(
                        shell,
                        BStr::new(name.as_slice()),
                        &ArraySelector::Index(0),
                        BStr::new(value.as_slice()),
                        false,
                        guard,
                    )?;
                    Ok(true)
                }
            }
        }
        // One element is a string, so a list has nothing to become
        // there: Bash refuses `a[i]=(1 2)` and `a[i]+=(1 2)` alike,
        // reports it, and leaves the array as it was.
        BashArrayValue::Compound(_) if assignment.subscript.is_some() => {
            let mut message = name.clone();
            message.extend_from_slice(b": cannot assign list to array member");
            shell.diagnostics().shell_warning(&message);
            Ok(false)
        }
        BashArrayValue::Compound(elements) => {
            let name = BStr::new(name.as_slice());
            let form = arrays::compound_form(
                shell,
                name,
                elements
                    .first()
                    .is_some_and(|element| element.subscript.is_some()),
            );
            let resolved = if form == CompoundForm::Pairs {
                pair_words(shell, elements)?
            } else {
                subscripted_elements(shell, elements)?
            };
            arrays::assign_compound(shell, name, resolved, form, append, guard)?;
            Ok(true)
        }
    }
}

/// The element a statement's `a[sub]=v` names.
///
/// An indexed subscript is an arithmetic expression, and Bash reads one
/// out of the *source* rather than out of the expanded word: `a['1']=z`
/// is an arithmetic syntax error there, exactly as `${a['1']}` is,
/// because the single quotes reach the evaluator. A parsed word cannot
/// answer that question -- `WordUnit::Literal` records that a byte was
/// quoted and not which quote it was written with -- so the bytes come
/// back off the run the node was parsed from, and
/// [`arrays::resolve_text_selector`] reads them the same way the
/// expansion side does.
///
/// An associative subscript is a key and keeps the word path, where an
/// assignment's tilde rule applies to it and its quotes come off:
/// `m['a b']=w` names the key `a b` in both shells. A compound
/// element's subscript is a word in Bash too -- `b=(['1']=x)` is
/// element one there -- so that path is untouched as well.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn element_selector(
    shell: &mut Shell,
    name: &BStr,
    assignment: &BashArrayAssignment,
    subscript: &WordNode,
) -> Result<ArraySelector, Error> {
    if let Some(source) = subscript_source(assignment) {
        let source = BStr::new(source.as_slice());
        if arrays::reads_as_arithmetic(shell, name, source) {
            return arrays::resolve_text_selector(shell, name, source);
        }
    }
    let expanded = expand_scalar(shell, subscript, ExpansionMode::ASSIGNMENT_TILDE)?;
    arrays::resolve_selector(shell, name, BStr::new(expanded.as_slice()))
}

/// The subscript's own bytes, cut out of the word the assignment was
/// parsed from.
///
/// `arg_part` builds the subscript from the word's *units*, and says why
/// in its own comment: the parts are this parser's reading of one word
/// the reader cut, so they carry no run of their own. That reading has
/// no room for the answer -- a unit records that a byte was quoted and
/// not which quote quoted it, and `a['1']` and `a["1"]` differ in
/// nothing else -- so the bytes are found again here, by the scan
/// `parameter_subscript` makes over the same syntax on the read side.
///
/// `None` for a node nothing parsed, or a word whose text does not open
/// with `name[`; the caller then falls back to the expanded word, which
/// is what every subscript used before.
// [spec:nsh:def:idiom.token-stream]
fn subscript_source(assignment: &BashArrayAssignment) -> Option<BString> {
    let text = assignment.tokens.text();
    /* The run is the word plus whatever blank separated it from what
     * came before, so `x=1; a[i]=2` hands this ` a[i]=2`. */
    let bytes: &[u8] = text.as_ref();
    let rest = bytes
        .trim_ascii_start()
        .strip_prefix(assignment.name.as_bstr().as_ref() as &[u8])?
        .strip_prefix(b"[")?;
    let mut depth = 1usize;
    let mut quote: Option<u8> = None;
    let mut escaped = false;
    for (at, byte) in rest.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match (quote, *byte) {
            (Some(b'\''), b'\'') | (Some(b'"'), b'"') => quote = None,
            (Some(b'"'), b'\\') | (None, b'\\') => escaped = true,
            (Some(_), _) => {}
            (None, b'\'' | b'"') => quote = Some(*byte),
            (None, b'[') => depth += 1,
            (None, b']') => {
                depth -= 1;
                if depth == 0 {
                    return Some(BString::from(&rest[..at]));
                }
            }
            (None, _) => {}
        }
    }
    None
}

/// Expand a `[subscript]=value` list, one element at a time.
///
/// `[k]=v` makes the value an assignment operand rather than an ordinary
/// word: it is not brace-expanded, not split, and its tildes follow the
/// assignment rule. An element with no subscript is a plain word and
/// splits, so `a=($x)` yields one element per field.
fn subscripted_elements(
    shell: &mut Shell,
    elements: &[BashArrayElement],
) -> Result<Vec<CompoundElement>, Error> {
    let mut resolved = Vec::with_capacity(elements.len());
    for element in elements {
        let append = element.operator == BashAssignmentOperator::Append;
        let Some(word) = &element.subscript else {
            for value in expand_fields(shell, &element.value)? {
                resolved.push(CompoundElement {
                    subscript: None,
                    value,
                    append,
                });
            }
            continue;
        };
        resolved.push(CompoundElement {
            subscript: Some(expand_scalar(shell, word, ExpansionMode::ASSIGNMENT_TILDE)?),
            value: expand_value(shell, &element.value)?,
            append,
        });
    }
    Ok(resolved)
}

/// Expand a `( key value ... )` list, one field per element.
///
/// Bash's key/value form neither splits nor globs -- `p="a b"; m=($p)`
/// is the single key `a b`, and `m=(* v)` is the key `*` -- and a
/// `[k]=v` element is not an element here at all: it is the word it was
/// written as, brackets and operator included, which is why
/// `m=(foo [a]=1)` stores `[foo]="[a]=1"`. So the element is put back
/// together from the pieces the parser cut it into and expanded once,
/// the way an assignment's right-hand side is.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn pair_words(
    shell: &mut Shell,
    elements: &[BashArrayElement],
) -> Result<Vec<CompoundElement>, Error> {
    let mut words = Vec::with_capacity(elements.len());
    for element in elements {
        let word = match &element.subscript {
            Some(subscript) => rejoin(subscript, element.operator, &element.value),
            None => element.value.clone(),
        };
        words.push(CompoundElement {
            subscript: None,
            value: expand_value(shell, &word)?,
            append: false,
        });
    }
    Ok(words)
}

/// Put a `[subscript]=value` element back together as the one word it
/// was written as, so that the key/value form can read it as text.
fn rejoin(subscript: &WordNode, operator: BashAssignmentOperator, value: &WordNode) -> WordNode {
    let literal = |byte| WordUnit::Literal {
        byte,
        quoted: false,
    };
    let mut units = vec![literal(b'[')];
    units.extend(subscript.word.units());
    units.push(literal(b']'));
    if operator == BashAssignmentOperator::Append {
        units.push(literal(b'+'));
    }
    units.push(literal(b'='));
    units.extend(value.word.units());
    WordNode {
        tokens: SourceTokens::none(),
        word: ParsedWord::from_units(&units),
    }
}

/// Apply one structural assignment written in a simple command's prefix.
///
/// `temporary` says the binding lasts for one command. An environment
/// has no spelling for a list, so Bash passes the compound's text
/// instead; an element is not a variable at all, so Bash reports the
/// binding and runs the command without it. `false` reports a refusal
/// that abandons the command.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn assign_prefix(
    shell: &mut Shell,
    assignment: &BashArrayAssignment,
    temporary: bool,
) -> Result<bool, Error> {
    if !temporary {
        return assign(shell, assignment, ReadOnlyGuard::Enforce);
    }
    if let Some(subscript) = &assignment.subscript {
        let mut message = BString::from(assignment.name.as_bstr());
        message.push(b'[');
        message.extend_from_slice(subscript.word.as_bstr().as_ref());
        message.extend_from_slice(b"]: cannot bind an array element");
        shell.diagnostics().shell_warning(&message);
        return Ok(true);
    }
    let BashArrayValue::Compound(_) = &assignment.value else {
        return assign(shell, assignment, ReadOnlyGuard::Enforce);
    };
    let mut binding = BString::from(assignment.name.as_bstr());
    binding.push(b'=');
    binding.extend_from_slice(&value_text(&assignment.value));
    crate::variables::make_local_bytes(
        shell,
        BStr::new(binding.as_slice()),
        crate::variables::VariableAttributes::EXPORTED,
    )?;
    Ok(true)
}

/// A declaration built-in's structural operand, held until the built-in
/// has applied its attributes.
///
/// `declare -A m=([k]=v)` is associative only because `-A` was seen
/// first, so the compound value cannot land while the command line is
/// still being assembled. The built-in is handed the operand's bare name
/// in its place, and the assignment runs once the built-in has returned.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) struct Declaration<'a> {
    assignment: &'a BashArrayAssignment,
    /// Whether the name was already read-only when the command started.
    was_read_only: bool,
}

/// Expand one command argument, holding back a declaration operand.
///
/// The name is what the built-in has to see: it lives inside the node
/// rather than among the words, so dropping the node outright would
/// leave `declare -A` with nothing to apply `-A` to.
///
/// `assignment_operands` says the command is a declaration built-in
/// whose assignment-shaped words are assignments: those expand unsplit
/// and with an assignment's tilde rules, and every other argument is an
/// ordinary word.
// [spec:posix:req:cmd.simple-declaration-utility-expansion]
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn expand_command_argument<'a>(
    shell: &mut Shell,
    argument: &'a Node,
    fields: &mut ExpandedFields,
    assignment_operands: bool,
    held: &mut Vec<Declaration<'a>>,
) -> Result<(), Error> {
    let Node::Bash(crate::nodes::BashNode::ArrayAssignment(assignment)) = argument else {
        let assignment_word = assignment_operands
            && matches!(argument, Node::Word(word) if word.word.is_assignment(&shell.locale));
        let mode = if assignment_word {
            ExpansionMode::ASSIGNMENT_TILDE
        } else {
            ExpansionMode::SPLIT | ExpansionMode::TILDE
        };
        return expand_argument(shell, argument, Some(fields), mode);
    };
    let name = assignment.name.as_bstr();
    let was_read_only = crate::variables::variable_attributes(shell, name)
        .is_some_and(|attributes| attributes.read_only);
    fields.fields.push(ExpandedField::from_bytes(name.as_ref()));
    held.push(Declaration {
        assignment,
        was_read_only,
    });
    Ok(())
}

/// Whether the built-in that ran accepts a subscripted operand.
///
/// `export` and `readonly` name variables rather than elements, so Bash
/// refuses `export a[7]=8` outright: the element is not assigned and the
/// command fails. `declare`, `typeset` and `local` do accept one.
// [spec:nsh:req:compat.bash.arrays-declarations]
#[derive(Clone, Copy, Eq, PartialEq)]
pub(crate) enum SubscriptedOperand {
    Accepted,
    Refused,
}

/// Land every held operand now that the declaration's attributes exist.
///
/// `accepted_all` is whether the built-in reported success. When it did
/// not, the operands it refused *by name* are the only ones held back:
/// Bash applies each operand on its own, so `declare a[ a[2]=3 ]=Y`
/// stores the middle one and reports the other two. A failure with no
/// named operand behind it -- an unknown option, say -- refuses them
/// all, because none of them was ever reached.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn apply_declarations(
    shell: &mut Shell,
    held: &[Declaration<'_>],
    subscripts: SubscriptedOperand,
    accepted_all: bool,
) -> Result<(), Error> {
    let refused = core::mem::take(&mut shell.evaluation.refused_declarations);
    let kind = shell.evaluation.declared_kind.take();
    if !accepted_all && refused.is_empty() {
        return Ok(());
    }
    for declaration in held {
        if refused
            .iter()
            .any(|name| BStr::new(name.as_slice()) == declaration.assignment.name.as_bstr())
        {
            continue;
        }
        if subscripts == SubscriptedOperand::Refused
            && let Some(subscript) = &declaration.assignment.subscript
        {
            let mut message = b"export: `".to_vec();
            message.extend_from_slice(declaration.assignment.name.as_bstr().as_ref());
            message.push(b'[');
            message.extend_from_slice(subscript.word.as_bstr().as_ref());
            message.extend_from_slice(b"]': not a valid identifier");
            shell.diagnostics().shell_warning(&message);
            shell.status = crate::status::ExitStatus::FAILURE;
            continue;
        }
        // A name the command itself just made read-only is still waiting
        // for the value it was written with; one that arrived read-only
        // refuses it, as an ordinary assignment would.
        let guard = if declaration.was_read_only {
            ReadOnlyGuard::Enforce
        } else {
            ReadOnlyGuard::Declaration
        };
        /* `readonly -A m=([a b]=1)` is associative only because `-A` was
         * seen, and the letter reaches the name here rather than in the
         * built-in: `export` and `readonly` were handed the bare name
         * and cannot tell an operand that carries a compound value from
         * one that does not. Bash consults the letter only for the
         * former, which is why `readonly -A m` is `declare -r m` there. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        if let Some(kind) = kind
            && matches!(declaration.assignment.value, BashArrayValue::Compound(_))
        {
            let name = declaration.assignment.name.as_bstr();
            arrays::ensure_kind(
                shell,
                name,
                kind,
                crate::variables::VariableAttributes::NONE,
                guard,
            )?;
        }
        if !assign(shell, declaration.assignment, guard)? {
            shell.status = crate::status::ExitStatus::FAILURE;
        }
    }
    Ok(())
}

/// The assignment as it was written, for `set -x` and for the string
/// Bash hands a command prefix that binds a compound value.
///
/// The text comes from the parse tree rather than from an expansion, so
/// rendering it cannot run a command substitution the assignment itself
/// is about to run.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn assignment_text(assignment: &BashArrayAssignment) -> BString {
    let mut text = BString::from(assignment.name.as_bstr());
    if let Some(subscript) = &assignment.subscript {
        text.push(b'[');
        text.extend_from_slice(subscript.word.as_bstr().as_ref());
        text.push(b']');
    }
    if assignment.operator == BashAssignmentOperator::Append {
        text.push(b'+');
    }
    text.push(b'=');
    text.extend_from_slice(&value_text(&assignment.value));
    text
}

/// The right-hand side of one array assignment as it was written.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn value_text(value: &BashArrayValue) -> BString {
    let mut text = BString::default();
    match value {
        BashArrayValue::Word(word) => text.extend_from_slice(word.word.as_bstr().as_ref()),
        BashArrayValue::Compound(elements) => {
            text.push(b'(');
            for (position, element) in elements.iter().enumerate() {
                if position > 0 {
                    text.push(b' ');
                }
                if let Some(subscript) = &element.subscript {
                    text.push(b'[');
                    text.extend_from_slice(subscript.word.as_bstr().as_ref());
                    text.extend_from_slice(b"]=");
                }
                text.extend_from_slice(element.value.word.as_bstr().as_ref());
            }
            text.push(b')');
        }
    }
    text
}

/// Expand an assignment's right-hand side to exactly one field.
///
/// The parser has already taken `name[i]=` off the front, so what is
/// left is the value alone: its tildes are the ones an assignment
/// expands, at the start of the word and after each `:`, and nothing
/// splits or globs.
fn expand_value(shell: &mut Shell, word: &WordNode) -> Result<BString, Error> {
    expand_scalar(
        shell,
        word,
        ExpansionMode::TILDE | ExpansionMode::COLON_TILDE,
    )
}

/// Expand a word to exactly one field under `mode`.
/// Refuse `a[]=v`, which names no element.
///
/// Written emptiness only: the word `$empty` is an expression that
/// evaluates to 0, and Bash assigns element zero for it. `a[]` is not an
/// expression at all, and taking it as 0 would write over an element the
/// script never named. Bash reports and carries on with status 1, which
/// is what the dialect boundary gives.
// [spec:nsh:req:compat.bash.arrays-declarations]
// [spec:nsh:req:compat.bash.error-boundary]
fn reject_empty_subscript(
    shell: &mut Shell,
    name: &BStr,
    subscript: &WordNode,
) -> Result<(), Error> {
    if !subscript.word.parts().is_empty() {
        return Ok(());
    }
    let mut message = BString::from(name);
    message.extend_from_slice(b"[]: bad array subscript");
    Err(shell.diagnostics().dialect_error(&message))
}

fn expand_scalar(
    shell: &mut Shell,
    word: &WordNode,
    mode: ExpansionMode,
) -> Result<BString, Error> {
    let mut fields = ExpandedFields::new();
    let node = Node::Word(word.clone());
    expand_argument(shell, &node, Some(&mut fields), mode)?;
    Ok(fields
        .fields
        .into_iter()
        .next()
        .map(|field| field.as_bstr().to_owned())
        .unwrap_or_default())
}

/// Expand a word with splitting and pathname expansion, as a list element.
fn expand_fields(shell: &mut Shell, word: &WordNode) -> Result<Vec<BString>, Error> {
    let mut fields = ExpandedFields::new();
    let node = Node::Word(word.clone());
    expand_argument(
        shell,
        &node,
        Some(&mut fields),
        ExpansionMode::SPLIT | ExpansionMode::TILDE,
    )?;
    Ok(fields
        .fields
        .into_iter()
        .map(|field| field.as_bstr().to_owned())
        .collect())
}
