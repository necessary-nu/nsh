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
use crate::nodes::{BashArrayAssignment, BashArrayValue, BashAssignmentOperator, Node, WordNode};
use crate::variables::arrays::{self, ArraySelector, CompoundElement, ReadOnlyGuard};

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
                    let subscript =
                        expand_scalar(shell, subscript, ExpansionMode::ASSIGNMENT_TILDE)?;
                    let selector = arrays::resolve_selector(
                        shell,
                        BStr::new(name.as_slice()),
                        BStr::new(subscript.as_slice()),
                    )?;
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
                // `a+=v` with no subscript appends to the zero element,
                // which for a scalar is ordinary string concatenation.
                None => {
                    let selector = ArraySelector::Index(0);
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
            let mut resolved = Vec::with_capacity(elements.len());
            for element in elements {
                let subscript = match &element.subscript {
                    Some(word) => {
                        Some(expand_scalar(shell, word, ExpansionMode::ASSIGNMENT_TILDE)?)
                    }
                    None => None,
                };
                // `[k]=v` makes the value an assignment operand rather
                // than an ordinary word: it is not brace-expanded, not
                // split, and its tildes follow the assignment rule. An
                // element with no subscript is a plain word and splits,
                // so `a=($x)` yields one element per field.
                if subscript.is_some() {
                    resolved.push(CompoundElement {
                        subscript,
                        value: expand_value(shell, &element.value)?,
                        append: element.operator == BashAssignmentOperator::Append,
                    });
                    continue;
                }
                for value in expand_fields(shell, &element.value)? {
                    resolved.push(CompoundElement {
                        subscript: None,
                        value,
                        append: element.operator == BashAssignmentOperator::Append,
                    });
                }
            }
            arrays::assign_compound(shell, BStr::new(name.as_slice()), resolved, append, guard)?;
            Ok(true)
        }
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
