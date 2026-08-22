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
use crate::expand::{ExpandedFields, ExpansionMode, expand_argument};
use crate::nodes::{BashArrayAssignment, BashArrayValue, BashAssignmentOperator, Node, WordNode};
use crate::variables::arrays::{self, ArraySelector, CompoundElement};

/// Apply one `a=(...)`, `a[i]=v`, or `a+=(...)` assignment.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn assign(shell: &mut Shell, assignment: &BashArrayAssignment) -> Result<(), Error> {
    // An assignment to a Bash name reference reaches the variable it
    // names, so the reference is resolved before any subscript is.
    // [spec:nsh:req:compat.bash.functions-scoping]
    let Some(name) = crate::variables::nameref::element_base(shell, assignment.name.as_bstr())
    else {
        return Ok(());
    };
    let append = assignment.operator == BashAssignmentOperator::Append;

    match &assignment.value {
        BashArrayValue::Word(word) => {
            let value = expand_scalar(shell, word)?;
            match &assignment.subscript {
                Some(subscript) => {
                    let subscript = expand_scalar(shell, subscript)?;
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
                    )
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
                    )
                }
            }
        }
        BashArrayValue::Compound(elements) => {
            let mut resolved = Vec::with_capacity(elements.len());
            for element in elements {
                let subscript = match &element.subscript {
                    Some(word) => Some(expand_scalar(shell, word)?),
                    None => None,
                };
                // An unquoted element word splits, so `a=($x)` yields one
                // element per field rather than a single joined element.
                for value in expand_fields(shell, &element.value)? {
                    resolved.push(CompoundElement {
                        subscript: subscript.clone(),
                        value,
                        append: element.operator == BashAssignmentOperator::Append,
                    });
                }
            }
            arrays::assign_compound(shell, BStr::new(name.as_slice()), resolved, append)
        }
    }
}

/// Expand a word to exactly one field, as an assignment right-hand side.
fn expand_scalar(shell: &mut Shell, word: &WordNode) -> Result<BString, Error> {
    let mut fields = ExpandedFields::new();
    let node = Node::Word(word.clone());
    expand_argument(
        shell,
        &node,
        Some(&mut fields),
        ExpansionMode::ASSIGNMENT_TILDE,
    )?;
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
