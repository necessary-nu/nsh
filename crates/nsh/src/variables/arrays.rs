//! Indexed and associative array storage for the Bash dialect.
//!
//! The value model in [`super::value`] already stores sparse maps; this
//! module owns the *assignment* semantics layered on top of them --
//! which subscript spelling selects which element, when a plain name
//! acquires an array kind, and how `+=` differs between an element and a
//! whole compound value.
//!
//! Subscripts are resolved here rather than at the parser, because an
//! indexed subscript is an arithmetic expression whose value can depend
//! on the shell state at assignment time.

use bstr::{BStr, BString};

use super::value::{VariableKind, VariableValue};
use super::{AllExport, CallbackPolicy, VariableAttributes, VariableState, set_entry, valid_name};
use crate::context::Shell;
use crate::error::Error;

/// Which element of an array an assignment or read selects.
#[derive(Clone, Debug, Eq, PartialEq)]
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) enum ArraySelector {
    /// `a[expr]` against an indexed array, already evaluated.
    Index(u64),
    /// `a[key]` against an associative array.
    Key(BString),
    /// `a[@]` -- every element, each its own field.
    All,
    /// `a[*]` -- every element, joined by the first `IFS` byte.
    Joined,
    /// A subscript that names no element: `a[-5]` counted back past the
    /// start of the array.
    ///
    /// Bash reports it and the two sides then part company, which is why
    /// this is a selector rather than an error. A read yields nothing and
    /// the command it was written in still runs -- `argv.py "${a[-5]}"`
    /// prints an empty argument -- while an assignment or an `unset`
    /// through the same subscript refuses and answers 1. Only Bash mode
    /// produces one; the POSIX dialect raises where this is built.
    // [spec:nsh:req:compat.bash.error-boundary]
    Missing,
}

/// How the elements of `name=(...)` are read.
///
/// Bash 5.1 gave an associative array a second spelling for its compound
/// value -- `m=(k v k v)` -- and chooses between the two by the *first*
/// element alone. A leading `[key]=value` makes every element one of
/// those, and a bare word beside them is refused; a leading bare word
/// makes every element a key or a value, a written `[a]=1` among them,
/// which is then the literal key `[a]=1`.
// [spec:nsh:req:compat.bash.arrays-declarations]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CompoundForm {
    /// `[index]=value` against an indexed array, where a bare word takes
    /// the index after the last one assigned.
    Indexed,
    /// `[key]=value` against an associative array, where a bare word
    /// names no element at all.
    Keyed,
    /// `( key value ... )` against an associative array.
    Pairs,
}

impl CompoundForm {
    /// The kind of value the form builds.
    const fn kind(self) -> VariableKind {
        match self {
            Self::Indexed => VariableKind::Indexed,
            Self::Keyed | Self::Pairs => VariableKind::Associative,
        }
    }
}

/// Which of the three spellings a compound assignment to `name` is.
///
/// `subscripted` is whether the list opens with a `[...]=` element. A
/// name that holds no array yet, or holds a scalar, takes the value as
/// an indexed one, which is what makes `a=(x y)` an array.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn compound_form(shell: &Shell, name: &BStr, subscripted: bool) -> CompoundForm {
    let declared = super::value::variable_kind(shell, name);
    match declared.filter(|kind| *kind != VariableKind::Scalar) {
        Some(VariableKind::Associative) if subscripted => CompoundForm::Keyed,
        Some(VariableKind::Associative) => CompoundForm::Pairs,
        _ => CompoundForm::Indexed,
    }
}

/// What Bash says about a compound element an associative array cannot
/// place, so that the two callers that meet one word it the same way.
///
/// One reached through the command's own words abandons the list, and
/// one that arrived inside an operand the built-in had to read as text
/// is only reported -- but the sentence is Bash's either way.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn missing_subscript_message(name: &BStr, value: &BStr) -> BString {
    let mut message = BString::from(name);
    message.extend_from_slice(b": '");
    message.extend_from_slice(value.as_ref());
    message.extend_from_slice(b"': must use subscript when assigning associative array");
    message
}

/// Whether an existing read-only attribute refuses an assignment.
///
/// A declaration built-in sets `-r` before its own value lands, so
/// `declare -r a=(1)` would otherwise be refused by the attribute the
/// same command had just added. The declaration path therefore reports
/// the state the name held *before* it ran, which keeps
/// `readonly x=1; declare -a x=(2)` an error.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) enum ReadOnlyGuard {
    /// An ordinary assignment: a read-only name refuses it.
    Enforce,
    /// A declaration landing the value it was written with.
    Declaration,
}

/// Resolve a subscript's bytes against the kind the variable already has.
///
/// Bash decides this by the *target's* kind, not the subscript's shape:
/// `m[0]` is the key `"0"` in an associative array and the index `0` in
/// an indexed one. An unset name defaults to indexed, which is what
/// makes `a[3]=x` create an indexed array.
pub(crate) fn resolve_selector(
    shell: &mut Shell,
    name: &BStr,
    subscript: &BStr,
) -> Result<ArraySelector, Error> {
    if subscript == "@" {
        return Ok(ArraySelector::All);
    }
    if subscript == "*" {
        return Ok(ArraySelector::Joined);
    }

    let kind = super::value::variable_kind(shell, name).unwrap_or(VariableKind::Indexed);
    if kind == VariableKind::Associative {
        return Ok(ArraySelector::Key(subscript.to_owned()));
    }

    let index = crate::arithmetic::evaluate(shell, subscript)?;
    normalize_index(shell, name, index)
}

/// Resolve a subscript that reached the shell as *text* rather than as a
/// parsed word.
///
/// `${A[k]}`, `unset -v 'A[k]'`, `declare -n r='A[k]'` and `(( A[k] ))`
/// all name an element with bytes no parser expanded. Bash reads those
/// bytes as a word: quotes come off, text outside single quotes expands,
/// and a blank is data rather than a field separator.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn resolve_text_selector(
    shell: &mut Shell,
    name: &BStr,
    subscript: &BStr,
) -> Result<ArraySelector, Error> {
    let resolved = if reads_as_arithmetic(shell, name, subscript) {
        arithmetic_word(shell, subscript)?
    } else {
        text_word(shell, subscript)?
    };
    resolve_selector(shell, name, BStr::new(resolved.as_slice()))
}

/// Whether a subscript's bytes are an arithmetic expression rather than
/// a key.
///
/// The target's kind decides, as it decides everything else about a
/// subscript, and an unset name defaults to indexed -- which is why
/// `${zz['1']}` reports in Bash for a name that does not exist. `@` and
/// `*` are neither: they select the whole array, and they do it only
/// when the *source* wrote them bare, so `${a[@]}` is every element and
/// `${a["@"]}` is arithmetic on the letter.
///
/// The POSIX dialect answers `false` for everything. It has no arrays,
/// and the one place it reaches a subscript at all is `unset a[i]`,
/// which dash refuses as a bad variable name; changing how those bytes
/// are read there would move a dialect this node is not about, and
/// measurably did before this test was added -- `unset "a['1']"` went
/// from doing nothing to ending the script.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn reads_as_arithmetic(shell: &Shell, name: &BStr, subscript: &BStr) -> bool {
    shell.options.dialect() == crate::options::Dialect::Bash
        && subscript != "@"
        && subscript != "*"
        && super::value::variable_kind(shell, name).unwrap_or(VariableKind::Indexed)
            != VariableKind::Associative
}

/// Read a subscript's bytes the way Bash reads an *index*: as the
/// contents of a double-quoted word.
///
/// Bash hands an indexed subscript to `expand_arith_string` with
/// `Q_DOUBLE_QUOTES` already set, so a single quote in it is an ordinary
/// byte that reaches the arithmetic evaluator and is rejected there --
/// `${a['1']}` is an arithmetic syntax error, not element one. The
/// evidence that it is *expansion* under double quotes rather than
/// quote-stripping is Bash's own diagnostic: `${a['$n']}` with `n=1`
/// reports the error token as `'1'`, so `$n` expanded inside the single
/// quotes and the quotes themselves survived.
///
/// Everything else follows from that. A backslash keeps its meaning only
/// before `$`, a backtick, a double quote or another backslash, so
/// `${a[\1]}` reports `\1`; a double quote is removed; `${a[""]}` is the
/// empty expression, which is zero, where `${a['']}` reports `''`.
///
/// [`text_word`] is the other reading and belongs to an associative
/// subscript, which is a key and not an expression: `m['a b']` is the
/// key `a b` in both shells.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn arithmetic_word(shell: &mut Shell, subscript: &BStr) -> Result<BString, Error> {
    const ACTIVE: &[u8] = b"'\"\\$`";
    if !subscript.iter().any(|byte| ACTIVE.contains(byte)) {
        return Ok(subscript.to_owned());
    }
    let mut resolved = BString::default();
    let mut expandable = BString::default();
    let mut at = 0;
    while at < subscript.len() {
        let byte = subscript[at];
        at += 1;
        match byte {
            // A single quote is a byte the evaluator will choke on, and
            // it must not reach the expander, which would read it as a
            // quote and eat what follows.
            b'\'' => {
                expand_run(shell, &mut expandable, &mut resolved)?;
                resolved.push(b'\'');
            }
            b'"' => {}
            b'\\' if at < subscript.len() => {
                let escaped = subscript[at];
                at += 1;
                if matches!(escaped, b'$' | b'`' | b'"' | b'\\') {
                    expandable.push(b'\\');
                    expandable.push(escaped);
                    continue;
                }
                /* The backslash means nothing here and both bytes are
                 * the expression's, so neither goes past the expander. */
                expand_run(shell, &mut expandable, &mut resolved)?;
                resolved.push(b'\\');
                resolved.push(escaped);
            }
            _ => expandable.push(byte),
        }
    }
    expand_run(shell, &mut expandable, &mut resolved)?;
    Ok(resolved)
}

/// Apply a word's quoting and expansion to text the parser never saw.
///
/// A subscript, and the value of an element in an operand a declaration
/// built-in was handed as one word, both arrive as bytes: quotes come
/// off, text outside single quotes expands, and a blank is data.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn text_word(shell: &mut Shell, subscript: &BStr) -> Result<BString, Error> {
    const ACTIVE: &[u8] = b"'\"\\$`";
    if !subscript.iter().any(|byte| ACTIVE.contains(byte)) {
        return Ok(subscript.to_owned());
    }
    let mut resolved = BString::default();
    let mut expandable = BString::default();
    let mut in_double_quotes = false;
    let mut at = 0;
    while at < subscript.len() {
        let byte = subscript[at];
        at += 1;
        match byte {
            b'\'' if !in_double_quotes => {
                expand_run(shell, &mut expandable, &mut resolved)?;
                while at < subscript.len() && subscript[at] != b'\'' {
                    resolved.push(subscript[at]);
                    at += 1;
                }
                at += 1;
            }
            b'"' => in_double_quotes = !in_double_quotes,
            b'\\' if at < subscript.len() => {
                let escaped = subscript[at];
                at += 1;
                /* Outside double quotes a backslash removes any byte's
                 * meaning; inside, only these four keep theirs, and the
                 * double-quoted expansion below already knows which. */
                if in_double_quotes || matches!(escaped, b'$' | b'`' | b'"' | b'\\') {
                    expandable.push(b'\\');
                }
                expandable.push(escaped);
            }
            _ => expandable.push(byte),
        }
    }
    expand_run(shell, &mut expandable, &mut resolved)?;
    Ok(resolved)
}

/// Expand one run of subscript text that no single quote protected.
fn expand_run(shell: &mut Shell, run: &mut BString, resolved: &mut BString) -> Result<(), Error> {
    if run.is_empty() {
        return Ok(());
    }
    let expanded = crate::parser::expand_string(shell, BStr::new(run.as_slice()))?;
    resolved.extend_from_slice(&expanded);
    run.clear();
    Ok(())
}

/// Bash counts a negative subscript back from the highest set index, so
/// `a[-1]` is the last element rather than an error.
///
/// Counting back past the start is reported by both dialects and answered
/// differently. POSIX raises, because an expansion in error ends a
/// non-interactive shell. Bash writes the same diagnostic and hands back
/// [`ArraySelector::Missing`], because there the failure belongs to the
/// subscript and not to the command: the read carries on with nothing.
// [spec:nsh:req:compat.bash.error-boundary]
fn normalize_index(shell: &mut Shell, name: &BStr, index: i64) -> Result<ArraySelector, Error> {
    if index >= 0 {
        return Ok(ArraySelector::Index(index.unsigned_abs()));
    }
    let highest = super::value::variable_value(shell, name)
        .and_then(|value| value.indexed_keys())
        .and_then(|keys| keys.last().copied())
        .unwrap_or(0);
    let resolved = i128::from(highest) + 1 + i128::from(index);
    if resolved < 0 {
        let mut message = name.to_vec();
        message.extend_from_slice(b": bad array subscript");
        if shell.options.dialect() == crate::options::Dialect::Bash {
            shell.diagnostics().shell_warning(&message);
            /* `set -e` ends a Bash script at a reported failure even where
             * the command it was reported in would have succeeded, so the
             * one place that reports without raising has to raise there. */
            if shell.options.enabled(crate::options::ShellOption::Errexit) {
                return Err(Error::abandoned(shell.evaluation.diagnostic_line));
            }
            return Ok(ArraySelector::Missing);
        }
        return Err(shell.diagnostics().shell_error(&message));
    }
    Ok(ArraySelector::Index(u64::try_from(resolved).unwrap_or(0)))
}

/// Whether `name` may be given `kind`.
///
/// Bash reshapes a scalar into either array kind -- that is what
/// `x=v; declare -a x` relies on -- but refuses to turn an indexed array
/// into an associative one or the reverse: the elements mean something
/// different under the other kind, so `declare -A` on an indexed name
/// fails and leaves the value it already holds alone.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn convertible(shell: &Shell, name: &BStr, kind: VariableKind) -> bool {
    match super::value::variable_kind(shell, name) {
        Some(current) => current == kind || current == VariableKind::Scalar,
        None => true,
    }
}

/// Give `name` an array kind without disturbing a value it already holds.
///
/// Converting a scalar keeps its bytes as element zero, matching Bash's
/// `x=v; declare -a x` promotion.
pub(crate) fn ensure_kind(
    shell: &mut Shell,
    name: &BStr,
    kind: VariableKind,
    attributes: VariableAttributes,
    guard: ReadOnlyGuard,
) -> Result<(), Error> {
    reject_bad_name(shell, name)?;
    reject_read_only(shell, name, guard)?;

    if super::value::variable_kind(shell, name) == Some(kind) {
        return Ok(());
    }
    match super::value::variable_value(shell, name).cloned() {
        Some(value) => store(shell, name, convert(value, kind), attributes, guard),
        /* A declaration with nothing to store leaves Bash's *invisible*
         * variable: the kind is known and there is no value, so
         * `declare -a z` spells `declare -a z` where `declare -a z=()`
         * spells the list it was handed. The empty value still goes
         * through `store`, because a brand-new name picks up its export
         * state and callbacks there; the state is corrected after. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        None => {
            store(shell, name, VariableValue::empty(kind), attributes, guard)?;
            if let Some(entry) = shell.variables.entries.get_mut(name) {
                entry.state = VariableState::Declared(kind);
            }
            Ok(())
        }
    }
}

fn convert(value: VariableValue, kind: VariableKind) -> VariableValue {
    let mut converted = VariableValue::empty(kind);
    match (&value, &mut converted) {
        (VariableValue::Scalar(bytes), _) => converted.assign_scalar(BStr::new(bytes.as_slice())),
        (VariableValue::Indexed(values), VariableValue::Associative(target)) => {
            for (index, element) in values {
                target.insert(BString::from(index.to_string()), element.clone());
            }
        }
        (VariableValue::Associative(values), VariableValue::Indexed(target)) => {
            for (position, element) in values.values().enumerate() {
                target.insert(position as u64, element.clone());
            }
        }
        _ => {}
    }
    converted
}

/// `a[i]=v`, `m[k]=v`, and their `+=` forms.
pub(crate) fn assign_element(
    shell: &mut Shell,
    name: &BStr,
    selector: &ArraySelector,
    value: &BStr,
    append: bool,
    guard: ReadOnlyGuard,
) -> Result<(), Error> {
    reject_bad_name(shell, name)?;
    reject_read_only(shell, name, guard)?;
    reject_empty_key(shell, name, selector)?;

    let kind = match selector {
        ArraySelector::Key(_) => VariableKind::Associative,
        _ => VariableKind::Indexed,
    };
    let mut current = match super::value::variable_value(shell, name).cloned() {
        Some(value) if value.kind() != VariableKind::Scalar => value,
        Some(scalar) => convert(scalar, kind),
        None => VariableValue::empty(kind),
    };

    // `+=` on one element concatenates that element's bytes; the whole
    // value is never re-read as a scalar here.
    let appended;
    let element = if append {
        let existing = existing_element(&current, selector).unwrap_or_default();
        appended = append_element(shell, name, BStr::new(existing.as_slice()), value)?;
        BStr::new(appended.as_slice())
    } else {
        value
    };

    match selector {
        ArraySelector::Index(index) => {
            current.set_indexed(*index, element);
        }
        ArraySelector::Key(key) => {
            current.set_associative(BStr::new(key.as_slice()), element);
        }
        /* Already reported by `normalize_index`, which cannot raise: a
         * read through the same subscript is reported there and then
         * carries on. Only the write has an error left to take. */
        ArraySelector::Missing => return Err(Error::abandoned(shell.evaluation.diagnostic_line)),
        ArraySelector::All | ArraySelector::Joined => {
            let mut message = name.to_vec();
            message.extend_from_slice(b": cannot assign to a whole-array subscript");
            return Err(shell.diagnostics().shell_error(&message));
        }
    }
    store(shell, name, current, VariableAttributes::NONE, guard)
}

/// `name+=value`, written with no subscript at all.
///
/// The distinction the selector cannot carry: a *written* subscript
/// promotes a scalar to an array -- `v=str; v[0]=new` gives
/// `declare -a v` in Bash -- while an unsubscripted `+=` does not.
/// `v=a; v+=b` is `declare -- v="ab"`, and a name that does not exist yet
/// becomes a scalar rather than an array of one element.
///
/// A name that already holds an array, or carries `-i`, takes the append
/// at element zero, which is what [`assign_element`] does; only the
/// scalar case needed telling apart.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn append_unsubscripted(
    shell: &mut Shell,
    name: &BStr,
    value: &BStr,
    guard: ReadOnlyGuard,
) -> Result<(), Error> {
    /* Element zero for an indexed array; for an associative one that is
     * the *key* `0`, which is a key like any other and is what Bash
     * writes. */
    let selector = match super::value::variable_value(shell, name).map(VariableValue::kind) {
        Some(VariableKind::Indexed) => Some(ArraySelector::Index(0)),
        Some(VariableKind::Associative) => Some(ArraySelector::Key(BString::from("0"))),
        Some(VariableKind::Scalar) | None => None,
    };
    if let Some(selector) = selector {
        return assign_element(shell, name, &selector, value, true, guard);
    }
    reject_bad_name(shell, name)?;
    reject_read_only(shell, name, guard)?;
    let existing = super::lookup_bytes(shell, name).unwrap_or_default();
    let appended = append_element(shell, name, BStr::new(existing.as_slice()), value)?;
    /* `store` rather than `set_bytes`, because the guard has to reach the
     * write: `readonly r+=bar` declares `r` and assigns it in one command,
     * and the declaration is allowed to write the name it just marked. */
    store(
        shell,
        name,
        VariableValue::Scalar(appended),
        VariableAttributes::NONE,
        guard,
    )
}

/// Combine an element's bytes with what `+=` appends to them.
///
/// `declare -i` makes the two numbers rather than strings, so `i=1;
/// i+=' 2 '` is 3 where the same line without the attribute is `1 2 `.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn append_element(
    shell: &mut Shell,
    name: &BStr,
    existing: &BStr,
    value: &BStr,
) -> Result<BString, Error> {
    if super::value::bash_attributes(shell, name)
        .is_some_and(|declared| declared.contains(super::value::BashAttribute::Integer))
    {
        let left = arithmetic_value(shell, existing)?;
        let right = arithmetic_value(shell, value)?;
        return Ok(BString::from(left.wrapping_add(right).to_string()));
    }
    let mut combined = existing.to_owned();
    combined.extend_from_slice(value);
    Ok(combined)
}

/// An integer variable reads empty text as zero rather than refusing it.
fn arithmetic_value(shell: &mut Shell, text: &BStr) -> Result<i64, Error> {
    if text.iter().all(u8::is_ascii_whitespace) {
        return Ok(0);
    }
    crate::arithmetic::evaluate(shell, text)
}

fn existing_element(value: &VariableValue, selector: &ArraySelector) -> Option<BString> {
    match selector {
        ArraySelector::Index(index) => value.indexed(*index).map(BStr::to_owned),
        ArraySelector::Key(key) => value
            .associative(BStr::new(key.as_slice()))
            .map(BStr::to_owned),
        ArraySelector::Missing | ArraySelector::All | ArraySelector::Joined => None,
    }
}

/// Refuse the empty key, which names no element of an associative array.
///
/// An indexed array has no such refusal to make: `a[""]` is the
/// arithmetic expression `""`, which is zero, and both shells write
/// element zero for it. `m[""]` is a key, and the empty one is the key
/// Bash does not have -- it reports the subscript and abandons the
/// list, which is what a written `m[]` already does here.
// [spec:nsh:req:compat.bash.arrays-declarations]
// [spec:nsh:req:compat.bash.error-boundary]
fn reject_empty_key(shell: &mut Shell, name: &BStr, selector: &ArraySelector) -> Result<(), Error> {
    if !matches!(selector, ArraySelector::Key(key) if key.is_empty()) {
        return Ok(());
    }
    let mut message = BString::from(name);
    message.extend_from_slice(b"[\"\"]: bad array subscript");
    Err(shell.diagnostics().dialect_error(&message))
}

/// Read a `( key value key value )` list as `[key]=value` elements.
///
/// An odd list leaves its last key holding nothing, which is what
/// `m=(foo)` stores. A key that expanded to nothing names no element,
/// and this is the one place where that is not fatal: Bash reports the
/// subscript, drops the pair and keeps reading the list.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn key_value_pairs(shell: &mut Shell, elements: Vec<CompoundElement>) -> Vec<CompoundElement> {
    let mut pairs = Vec::with_capacity(elements.len().div_ceil(2));
    let mut elements = elements.into_iter();
    while let Some(key) = elements.next() {
        let value = elements.next().map(|element| element.value);
        if key.value.is_empty() {
            shell
                .diagnostics()
                .shell_warning(b"'': bad array subscript");
            continue;
        }
        pairs.push(CompoundElement {
            subscript: Some(key.value),
            value: value.unwrap_or_default(),
            append: false,
        });
    }
    pairs
}

/// One element of a compound assignment, after expansion.
pub(crate) struct CompoundElement {
    pub(crate) subscript: Option<BString>,
    pub(crate) value: BString,
    pub(crate) append: bool,
}

/// `a=(...)` and `a+=(...)`.
///
/// Unsubscripted elements continue from the highest index assigned so
/// far, so `a=(x [5]=y z)` puts `z` at 6 rather than at 1.
///
/// `form` is what the caller expanded the elements for: it decides
/// whether a bare word is the next index, one half of a key/value pair,
/// or the element an associative array refuses.
pub(crate) fn assign_compound(
    shell: &mut Shell,
    name: &BStr,
    elements: Vec<CompoundElement>,
    form: CompoundForm,
    append: bool,
    guard: ReadOnlyGuard,
) -> Result<(), Error> {
    reject_bad_name(shell, name)?;
    reject_read_only(shell, name, guard)?;

    let kind = form.kind();
    let elements = match form {
        CompoundForm::Pairs => key_value_pairs(shell, elements),
        CompoundForm::Indexed | CompoundForm::Keyed => elements,
    };

    let mut current = if append {
        match super::value::variable_value(shell, name).cloned() {
            Some(value) if value.kind() != VariableKind::Scalar => value,
            Some(scalar) => convert(scalar, kind),
            None => VariableValue::empty(kind),
        }
    } else {
        VariableValue::empty(kind)
    };

    let mut next = if append {
        current
            .indexed_keys()
            .and_then(|keys| keys.last().map(|index| index + 1))
            .unwrap_or(0)
    } else {
        0
    };

    /* Bash reads a `+=` element of an *associative* compound assignment
     * against the value its key held before the assignment began rather
     * than against the one the elements before it have built:
     * `m=([k]=z)` then `m=([k]+=1 [k]+=2)` is `z2` there, where both
     * elements append to `z`. An appending assignment has no separate
     * before -- its elements build on the array that is still in place,
     * and `m+=([k]+=1 [k]+=2)` is `z12` in both shells -- and an indexed
     * array reads the running value either way. Measured against the
     * pinned 5.3.15; without the snapshot this shell loses the `z`. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    let previous = (kind == VariableKind::Associative && !append).then(|| {
        /* A name that holds nothing yet -- an array declared and never
         * assigned among them -- has an empty base rather than no
         * snapshot: falling back to the running value here would let
         * `m=([k]+=1 [k]+=2)` read the `1` it just wrote. */
        super::value::variable_value(shell, name)
            .cloned()
            .unwrap_or_else(|| VariableValue::empty(kind))
    });

    for element in elements {
        /* A bare word names no key, and an associative array reading
         * `[key]=value` elements has nowhere to put it. Bash keeps the
         * elements before it and abandons the list there. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        // [spec:nsh:req:compat.bash.error-boundary]
        if form == CompoundForm::Keyed && element.subscript.is_none() {
            let message = missing_subscript_message(name, BStr::new(element.value.as_slice()));
            store(shell, name, current, VariableAttributes::NONE, guard)?;
            return Err(shell.diagnostics().dialect_error(&message));
        }
        /* A written `[]=` names no element whatever the array's kind:
         * `declare -a a=([""]=x)` is refused by Bash where the same
         * subscript in a statement -- `a[""]=x` -- is the expression
         * `""` and writes element zero. The elements before it are kept,
         * as they are for the refusal above. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        // [spec:nsh:req:compat.bash.error-boundary]
        if element
            .subscript
            .as_ref()
            .is_some_and(|subscript| subscript.is_empty())
        {
            let mut message = BString::from(name);
            message.extend_from_slice(b"[]: bad array subscript");
            store(shell, name, current, VariableAttributes::NONE, guard)?;
            return Err(shell.diagnostics().dialect_error(&message));
        }
        let selector = match &element.subscript {
            /* A pair's key was never a written subscript, so nothing in
             * it is syntax: `m=(* v)` stores the key `*`, where the
             * subscript `m[*]` names the whole array. */
            // [spec:nsh:req:compat.bash.arrays-declarations]
            Some(key) if form == CompoundForm::Pairs => ArraySelector::Key(key.clone()),
            // A subscript is an expression, and Bash evaluates it
            // against the array the preceding elements have already
            // built: in `a=([0]=1 [a[0]]=x)` the second subscript reads
            // the element the first one wrote. Publishing what the loop
            // holds so far is what makes it visible.
            Some(subscript) => {
                store(
                    shell,
                    name,
                    current.clone(),
                    VariableAttributes::NONE,
                    guard,
                )?;
                resolve_selector(shell, name, BStr::new(subscript.as_slice()))?
            }
            None => ArraySelector::Index(next),
        };
        let value = BStr::new(element.value.as_slice());
        let combined;
        let value = if element.append {
            let base = previous.as_ref().unwrap_or(&current);
            let mut existing = existing_element(base, &selector).unwrap_or_default();
            existing.extend_from_slice(value);
            combined = existing;
            BStr::new(combined.as_slice())
        } else {
            value
        };
        match &selector {
            ArraySelector::Index(index) => {
                current.set_indexed(*index, value);
                next = index + 1;
            }
            ArraySelector::Key(key) => {
                current.set_associative(BStr::new(key.as_slice()), value);
            }
            /* Already reported by `normalize_index`, which cannot raise: a
             * read through the same subscript is reported there and then
             * carries on. Only the write has an error left to take. */
            ArraySelector::Missing => {
                return Err(Error::abandoned(shell.evaluation.diagnostic_line));
            }
            ArraySelector::All | ArraySelector::Joined => {
                let mut message = name.to_vec();
                message.extend_from_slice(b": cannot assign to a whole-array subscript");
                return Err(shell.diagnostics().shell_error(&message));
            }
        }
    }
    store(shell, name, current, VariableAttributes::NONE, guard)
}

/// Assign to a target named by text, which may carry a subscript.
///
/// `printf -v 'a[k]'` and `declare 'a[k]=v'` both name an element with
/// bytes no parser split into a name and a subscript, so the brackets
/// are read here rather than by each caller.
// [spec:nsh:req:compat.bash.arrays-declarations]
/// Split a declaration built-in's operand into `name`, the value it was
/// given, and whether the operator was `+=`.
///
/// The operand reached the built-in as one expanded word, so `+=` in it
/// is not the parser's assignment operator -- `dyn=x; typeset s${dyn}+=v`
/// spells `sx+=v` only after expansion, and Bash reads it there. Nothing
/// else may take a trailing `+`: `a+b=c` is not a name and has to stay
/// not a name.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn split_assignment_operand(operand: &BStr) -> (BString, Option<BString>, bool) {
    let bytes: &[u8] = operand.as_ref();
    let Some(at) = bytes.iter().position(|byte| *byte == b'=') else {
        return (operand.to_owned(), None, false);
    };
    let append = at > 0 && bytes[at - 1] == b'+';
    let name_end = if append { at - 1 } else { at };
    (
        BString::from(&bytes[..name_end]),
        Some(BString::from(&bytes[at + 1..])),
        append,
    )
}

pub(crate) fn assign_text_target(
    shell: &mut Shell,
    name: &BStr,
    value: &BStr,
    append: bool,
) -> Result<(), Error> {
    let bytes: &[u8] = name.as_ref();
    let subscript = match bytes.iter().position(|byte| *byte == b'[') {
        Some(open) if bytes.last() == Some(&b']') => {
            Some((open, &bytes[open + 1..bytes.len() - 1]))
        }
        _ => None,
    };
    let Some((open, subscript)) = subscript else {
        if append {
            return append_unsubscripted(shell, name, value, ReadOnlyGuard::Enforce);
        }
        return super::set_bytes(shell, name, Some(value), VariableAttributes::NONE);
    };
    let base = BString::from(&bytes[..open]);
    let base = BStr::new(base.as_slice());
    let selector = resolve_text_selector(shell, base, BStr::new(subscript))?;
    assign_element(
        shell,
        base,
        &selector,
        value,
        append,
        ReadOnlyGuard::Enforce,
    )
}

/// `unset a[i]` removes one element; `unset a` is the ordinary path.
pub(crate) fn unset_element(
    shell: &mut Shell,
    name: &BStr,
    selector: &ArraySelector,
) -> Result<(), Error> {
    reject_read_only(shell, name, ReadOnlyGuard::Enforce)?;
    let Some(mut current) = super::value::variable_value(shell, name).cloned() else {
        return Ok(());
    };
    match selector {
        ArraySelector::Index(index) => {
            current.unset_indexed(*index);
        }
        ArraySelector::Key(key) => {
            current.unset_associative(BStr::new(key.as_slice()));
        }
        /* Already reported by `normalize_index`, which cannot raise: a
         * read through the same subscript is reported there and then
         * carries on. Only the write has an error left to take. */
        ArraySelector::Missing => return Err(Error::abandoned(shell.evaluation.diagnostic_line)),
        // `unset a[@]` clears every element but keeps the declaration.
        ArraySelector::All | ArraySelector::Joined => {
            current = VariableValue::empty(current.kind());
        }
    }
    store(
        shell,
        name,
        current,
        VariableAttributes::NONE,
        ReadOnlyGuard::Enforce,
    )
}

/// Replace a name's whole value with one the shell built itself.
///
/// `read -a` and `mapfile` do not assign element by element -- the array
/// they produce is the record they read, entire -- so they land it in
/// one write rather than as a compound assignment that would have to
/// re-derive the subscripts it already knows.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn assign_value(
    shell: &mut Shell,
    name: &BStr,
    value: VariableValue,
) -> Result<(), Error> {
    reject_bad_name(shell, name)?;
    reject_read_only(shell, name, ReadOnlyGuard::Enforce)?;
    store(
        shell,
        name,
        value,
        VariableAttributes::NONE,
        ReadOnlyGuard::Enforce,
    )
}

/// Every element in subscript order, for `${a[@]}` and friends.
pub(crate) fn elements(value: &VariableValue) -> Vec<BString> {
    match value {
        VariableValue::Scalar(bytes) => vec![bytes.clone()],
        VariableValue::Indexed(values) => values.values().cloned().collect(),
        VariableValue::Associative(values) => values.values().cloned().collect(),
    }
}

/// Every subscript in order, for `${!a[@]}`.
pub(crate) fn keys(value: &VariableValue) -> Vec<BString> {
    match value {
        VariableValue::Scalar(_) => vec![BString::from("0")],
        VariableValue::Indexed(_) => value
            .indexed_keys()
            .unwrap_or_default()
            .into_iter()
            .map(|index| BString::from(index.to_string()))
            .collect(),
        VariableValue::Associative(_) => value.associative_keys().unwrap_or_default(),
    }
}

fn reject_bad_name(shell: &mut Shell, name: &BStr) -> Result<(), Error> {
    if valid_name(&shell.locale, name) {
        return Ok(());
    }
    let mut message = name.to_vec();
    message.extend_from_slice(b": bad variable name");
    Err(shell.diagnostics().shell_error(&message))
}

fn reject_read_only(shell: &mut Shell, name: &BStr, guard: ReadOnlyGuard) -> Result<(), Error> {
    if guard == ReadOnlyGuard::Declaration {
        return Ok(());
    }
    let read_only = super::variable_attributes(shell, name).is_some_and(|attrs| attrs.read_only);
    if !read_only {
        return Ok(());
    }
    let mut message = name.to_vec();
    message.extend_from_slice(b": is read only");
    // [spec:nsh:req:compat.bash.error-boundary]
    Err(shell.diagnostics().dialect_error(&message))
}

/// Write a whole structural value back, going through `set_entry` first
/// so a brand-new name picks up export state and locale callbacks
/// exactly as a scalar assignment would.
///
/// `allexport` is the one thing it does not pick up. Bash's `-a` marks
/// an assignment that stores a scalar, and this is every write that
/// stores something else: `set -a; z=(1)`, `set -a; z[0]=5` and
/// `set -a; declare -a z` all leave the name unexported there.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(super) fn store(
    shell: &mut Shell,
    name: &BStr,
    value: VariableValue,
    attributes: VariableAttributes,
    guard: ReadOnlyGuard,
) -> Result<(), Error> {
    let seeded = value.scalar_owned().unwrap_or_default();
    crate::error::with_interrupts_deferred(shell, |shell| {
        set_entry(
            shell,
            name,
            Some(BStr::new(seeded.as_slice())),
            attributes,
            CallbackPolicy::Run,
            guard,
            AllExport::Declines,
        )
    })?;
    let entry = shell
        .variables
        .entries
        .get_mut(name)
        .expect("set_entry inserted the name");
    entry.state = VariableState::Set(value);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock;
    use crate::variables::value::variable_value;

    fn shell() -> Shell {
        Shell::new(crate::streams::Streams::INHERIT)
    }

    /// `a[3]=x` on an unset name creates an indexed array, not a scalar,
    /// and leaves the lower indices genuinely absent rather than empty.
    #[test]
    // [spec:nsh:req:compat.bash.arrays-declarations/test]
    fn element_assignment_creates_a_sparse_indexed_array() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tarr1");

        assign_element(
            shell,
            name,
            &ArraySelector::Index(3),
            BStr::new("x"),
            false,
            ReadOnlyGuard::Enforce,
        )
        .unwrap();

        let value = variable_value(shell, name).expect("the name exists");
        assert_eq!(value.kind(), VariableKind::Indexed);
        assert_eq!(value.indexed_keys(), Some(vec![3]));
        assert_eq!(value.indexed(0), None);
    }

    /// Unsubscripted elements continue past an explicit subscript, so
    /// `a=(x [5]=y z)` puts `z` at 6 rather than reusing index 1.
    #[test]
    // [spec:nsh:req:compat.bash.arrays-declarations/test]
    fn explicit_subscripts_move_the_append_cursor() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tarr2");

        let elements = vec![
            CompoundElement {
                subscript: None,
                value: BString::from("x"),
                append: false,
            },
            CompoundElement {
                subscript: Some(BString::from("5")),
                value: BString::from("y"),
                append: false,
            },
            CompoundElement {
                subscript: None,
                value: BString::from("z"),
                append: false,
            },
        ];
        assign_compound(
            shell,
            name,
            elements,
            CompoundForm::Indexed,
            false,
            ReadOnlyGuard::Enforce,
        )
        .unwrap();

        let value = variable_value(shell, name).expect("the name exists");
        assert_eq!(value.indexed_keys(), Some(vec![0, 5, 6]));
        assert_eq!(value.indexed(6), Some(BStr::new("z")));
    }

    /// `+=` on one element concatenates that element; `+=` on a compound
    /// value appends new elements after the highest index.
    #[test]
    // [spec:nsh:req:compat.bash.arrays-declarations/test]
    fn append_distinguishes_element_from_whole_value() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tarr3");

        assign_element(
            shell,
            name,
            &ArraySelector::Index(0),
            BStr::new("one"),
            false,
            ReadOnlyGuard::Enforce,
        )
        .unwrap();
        assign_element(
            shell,
            name,
            &ArraySelector::Index(0),
            BStr::new("X"),
            true,
            ReadOnlyGuard::Enforce,
        )
        .unwrap();
        assert_eq!(
            variable_value(shell, name).and_then(|value| value.indexed(0)),
            Some(BStr::new("oneX"))
        );

        assign_compound(
            shell,
            name,
            vec![CompoundElement {
                subscript: None,
                value: BString::from("two"),
                append: false,
            }],
            CompoundForm::Indexed,
            true,
            ReadOnlyGuard::Enforce,
        )
        .unwrap();
        let value = variable_value(shell, name).expect("the name exists");
        assert_eq!(value.indexed_keys(), Some(vec![0, 1]));
        assert_eq!(value.indexed(1), Some(BStr::new("two")));
    }

    /// A negative subscript counts back from the highest set index.
    #[test]
    // [spec:nsh:req:compat.bash.arrays-declarations/test]
    fn negative_subscripts_count_from_the_end() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tarr4");

        for index in 0..3 {
            assign_element(
                shell,
                name,
                &ArraySelector::Index(index),
                BStr::new("v"),
                false,
                ReadOnlyGuard::Enforce,
            )
            .unwrap();
        }
        let selector = resolve_selector(shell, name, BStr::new("-1")).unwrap();
        assert_eq!(selector, ArraySelector::Index(2));
    }

    /// An associative target reads its subscript literally, so `m[0]` is
    /// the key "0" rather than an arithmetic index.
    #[test]
    // [spec:nsh:req:compat.bash.arrays-declarations/test]
    fn associative_subscripts_are_literal_keys() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tarr5");

        ensure_kind(
            shell,
            name,
            VariableKind::Associative,
            VariableAttributes::NONE,
            ReadOnlyGuard::Enforce,
        )
        .unwrap();
        let selector = resolve_selector(shell, name, BStr::new("1+1")).unwrap();
        assert_eq!(selector, ArraySelector::Key(BString::from("1+1")));

        assign_element(
            shell,
            name,
            &selector,
            BStr::new("v"),
            false,
            ReadOnlyGuard::Enforce,
        )
        .unwrap();
        assert_eq!(
            variable_value(shell, name).and_then(|value| value.associative(BStr::new("1+1"))),
            Some(BStr::new("v"))
        );
    }

    /// Promoting a scalar keeps its bytes as element zero.
    #[test]
    // [spec:nsh:req:compat.bash.arrays-declarations/test]
    fn declaring_an_array_preserves_a_scalar_value() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tarr6");

        super::super::set_bytes(shell, name, Some(BStr::new("v")), VariableAttributes::NONE)
            .unwrap();
        ensure_kind(
            shell,
            name,
            VariableKind::Indexed,
            VariableAttributes::NONE,
            ReadOnlyGuard::Enforce,
        )
        .unwrap();

        let value = variable_value(shell, name).expect("the name exists");
        assert_eq!(value.kind(), VariableKind::Indexed);
        assert_eq!(value.indexed(0), Some(BStr::new("v")));
    }

    /// A declaration lands the value it was written with past the
    /// read-only attribute the same command just added, while a name
    /// that arrived read-only still refuses one.
    #[test]
    // [spec:nsh:req:compat.bash.arrays-declarations/test]
    fn a_declaration_passes_its_own_read_only_flag() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tarr8");
        let element = || {
            vec![CompoundElement {
                subscript: None,
                value: BString::from("v"),
                append: false,
            }]
        };

        super::super::set_bytes(shell, name, None, VariableAttributes::READ_ONLY).unwrap();
        assign_compound(
            shell,
            name,
            element(),
            CompoundForm::Indexed,
            false,
            ReadOnlyGuard::Declaration,
        )
        .unwrap();

        assert_eq!(
            variable_value(shell, name).and_then(|value| value.indexed(0)),
            Some(BStr::new("v"))
        );
        assert!(
            super::super::variable_attributes(shell, name)
                .expect("the name exists")
                .read_only
        );
        assert!(
            assign_compound(
                shell,
                name,
                element(),
                CompoundForm::Indexed,
                false,
                ReadOnlyGuard::Enforce,
            )
            .is_err()
        );
    }

    /// `unset a[i]` removes one element and leaves the rest in place.
    #[test]
    // [spec:nsh:req:compat.bash.arrays-declarations/test]
    fn unsetting_an_element_keeps_the_array() {
        let _g = lock();
        let shell = &mut shell();
        let name = BStr::new("Tarr7");

        for index in 0..3 {
            assign_element(
                shell,
                name,
                &ArraySelector::Index(index),
                BStr::new("v"),
                false,
                ReadOnlyGuard::Enforce,
            )
            .unwrap();
        }
        unset_element(shell, name, &ArraySelector::Index(1)).unwrap();

        let value = variable_value(shell, name).expect("the name exists");
        assert_eq!(value.indexed_keys(), Some(vec![0, 2]));
    }
}
