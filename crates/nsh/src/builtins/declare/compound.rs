//! Compound values in an operand a declaration built-in was handed as
//! one word.
//!
//! `declare -a 'x=(1 2 3)'` and `typeset -a "$code"` reach the built-in
//! as a single argument: the parser saw a quoted word, not a compound
//! assignment, so the parentheses are still text when the built-in runs.
//! Bash reads them anyway, and only where the name is an array -- a
//! plain `declare 'x=(1 2)'` keeps its bytes.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::variables::arrays::{self, CompoundElement, CompoundForm, ReadOnlyGuard};

/// The text of a compound value, without its parentheses.
///
/// `None` is every other value, including one that merely starts with a
/// parenthesis: the whole operand has to be the bracketed list.
pub(super) fn parenthesised(value: &BStr) -> Option<&BStr> {
    let bytes: &[u8] = value.as_ref();
    let inner = bytes.strip_prefix(b"(")?.strip_suffix(b")")?;
    Some(BStr::new(inner))
}

/// Assign the elements a compound operand spells out.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(super) fn assign(shell: &mut Shell, name: &BStr, inner: &BStr) -> Result<(), Error> {
    let words = split(inner);
    let opens_subscripted = words
        .first()
        .is_some_and(|word| split_element(BStr::new(word.as_slice())).0.is_some());
    let form = arrays::compound_form(shell, name, opens_subscripted);
    let mut elements = Vec::new();
    for word in words {
        let (subscript, append, value) = split_element(BStr::new(word.as_slice()));
        let subscript = match subscript {
            Some(text) => Some(arrays::text_word(shell, text)?),
            None => None,
        };
        elements.push(CompoundElement {
            subscript,
            value: arrays::text_word(shell, value)?,
            append,
        });
    }
    /* An operand read as text is not the command's own words, and Bash
     * treats a refused element in it more gently: `declare -A
     * "m=([k]=v x)"` reports `x`, keeps `[k]=v` and answers 0, where
     * the same list written out abandons the command list. Truncating
     * here is what leaves the refusal to the report alone. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    if form == CompoundForm::Keyed
        && let Some(at) = elements
            .iter()
            .position(|element| element.subscript.is_none())
    {
        let message =
            arrays::missing_subscript_message(name, BStr::new(elements[at].value.as_slice()));
        shell.diagnostics().shell_warning(&message);
        elements.truncate(at);
    }
    arrays::assign_compound(
        shell,
        name,
        elements,
        form,
        false,
        ReadOnlyGuard::Declaration,
    )
}

/// Split a compound value into its element words.
///
/// Blanks separate elements, and quoted ones belong to the element they
/// are written in. The quotes themselves are kept: whether they protect
/// their contents is decided when the element is expanded.
fn split(inner: &BStr) -> Vec<BString> {
    let bytes: &[u8] = inner.as_ref();
    let mut words: Vec<BString> = Vec::new();
    let mut current = BString::default();
    let mut started = false;
    let mut quote = None;
    let mut at = 0;
    while at < bytes.len() {
        let byte = bytes[at];
        at += 1;
        match byte {
            b'\\' if quote != Some(b'\'') && at < bytes.len() => {
                current.push(byte);
                current.push(bytes[at]);
                at += 1;
                started = true;
            }
            b'\'' | b'"' if quote.is_none() => {
                quote = Some(byte);
                current.push(byte);
                started = true;
            }
            _ if quote == Some(byte) => {
                quote = None;
                current.push(byte);
            }
            b' ' | b'\t' | b'\n' if quote.is_none() => {
                if started {
                    words.push(core::mem::take(&mut current));
                    started = false;
                }
            }
            _ => {
                current.push(byte);
                started = true;
            }
        }
    }
    if started {
        words.push(current);
    }
    words
}

/// Split one element into its `[subscript]`, its operator and its value.
///
/// The middle answer is whether the operator was `+=`, which the text
/// form has to read for itself: `declare -A "m=([k]+=1 [k]+=2)"` is the
/// same assignment as the one the parser cuts up, and dropping the `+`
/// left this shell with an element it could not place at all.
fn split_element(word: &BStr) -> (Option<&BStr>, bool, &BStr) {
    let bytes: &[u8] = word.as_ref();
    if bytes.first() != Some(&b'[') {
        return (None, false, word);
    }
    let mut depth = 0usize;
    for (at, byte) in bytes.iter().enumerate() {
        match byte {
            b'[' => depth += 1,
            b']' => {
                depth -= 1;
                if depth == 0 {
                    let subscript = Some(BStr::new(&bytes[1..at]));
                    return match (bytes.get(at + 1), bytes.get(at + 2)) {
                        (Some(b'='), _) => (subscript, false, BStr::new(&bytes[at + 2..])),
                        (Some(b'+'), Some(b'=')) => (subscript, true, BStr::new(&bytes[at + 3..])),
                        _ => (None, false, word),
                    };
                }
            }
            _ => {}
        }
    }
    (None, false, word)
}
