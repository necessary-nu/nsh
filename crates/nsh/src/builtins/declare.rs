//! Bash's `declare` and `typeset` declaration builtin.
//!
//! Two jobs share one command: giving a name an attribute (`-a`, `-A`,
//! `-i`, `-r`, `-x`, ...) and printing what a name already carries
//! (`-p`). Assignment operands are handled here rather than by the
//! ordinary assignment path because the attribute has to exist before
//! the value lands -- `declare -A m=([k]=v)` is only associative because
//! `-A` was seen first.

use bstr::{BStr, BString, ByteSlice as _};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::output::OutputDestination;
use crate::status::ExitStatus;
use crate::variables::arrays;
use crate::variables::nameref::{self, LocalValue};
use crate::variables::value::{BashAttribute, VariableKind, VariableValue};
use crate::variables::{VariableAttributes, add_attributes, set_bytes};

/// Which entry a declaration operand creates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Scope {
    /// The shell's own variable table.
    Global,
    /// A name the running function's return restores.
    Local,
}

/// Which attributes a single invocation turns on or off.
#[derive(Clone, Copy, Default)]
struct Requested {
    kind: Option<VariableKind>,
    integer: Option<bool>,
    lowercase: Option<bool>,
    uppercase: Option<bool>,
    nameref: Option<bool>,
    trace: Option<bool>,
    read_only: bool,
    exported: Option<bool>,
    global: bool,
    print: bool,
    functions: bool,
}

impl Requested {
    /// Whether the invocation asked for nothing at all, which is the
    /// listing form rather than a declaration.
    fn is_bare(&self) -> bool {
        self.kind.is_none()
            && self.integer.is_none()
            && self.lowercase.is_none()
            && self.uppercase.is_none()
            && self.nameref.is_none()
            && self.trace.is_none()
            && self.exported.is_none()
            && !self.read_only
            && !self.global
            && !self.functions
    }
}

// [spec:nsh:req:compat.bash.arrays-declarations]
// [spec:nsh:req:compat.bash.functions-scoping]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let (requested, operands) = match parse(args) {
        Ok(parsed) => parsed,
        Err(letter) => {
            let mut message = b"declare: -".to_vec();
            message.push(letter);
            message.extend_from_slice(b": invalid option\n");
            shell.write_output(OutputDestination::Stderr, &message)?;
            return Ok(Flow::Done(ExitStatus::ERROR));
        }
    };

    if requested.print || operands.is_empty() {
        return print(shell, &requested, operands);
    }

    /* Bash's `local` is this built-in restricted to the running function,
     * and a bare `declare` in a function body is local already unless
     * `-g` sends it to the shell's own table. */
    let forced_local = args.first().is_some_and(|name| *name == "local");
    let scope = if !requested.global && (forced_local || nameref::in_function_scope(shell)) {
        Scope::Local
    } else {
        Scope::Global
    };

    let mut status = ExitStatus::SUCCESS;
    for operand in operands {
        if !apply(shell, &requested, operand, scope)? || !valid_operand(shell, operand) {
            status = ExitStatus::FAILURE;
        }
    }
    Ok(Flow::Done(status))
}

fn parse<'a>(args: &'a [&'a BStr]) -> Result<(Requested, &'a [&'a BStr]), u8> {
    let mut requested = Requested::default();
    let mut at = 1;
    while at < args.len() {
        let arg: &[u8] = args[at].as_ref();
        let (enable, letters) = match arg.split_first() {
            Some((b'-', rest)) if !rest.is_empty() => (true, rest),
            Some((b'+', rest)) if !rest.is_empty() => (false, rest),
            _ => break,
        };
        if letters == b"-" {
            at += 1;
            break;
        }
        for letter in letters {
            match letter {
                b'a' => requested.kind = Some(VariableKind::Indexed),
                b'A' => requested.kind = Some(VariableKind::Associative),
                b'i' => requested.integer = Some(enable),
                b'l' => requested.lowercase = Some(enable),
                b'u' => requested.uppercase = Some(enable),
                b'n' => requested.nameref = Some(enable),
                b't' => requested.trace = Some(enable),
                b'r' => requested.read_only = enable,
                b'x' => requested.exported = Some(enable),
                b'g' => requested.global = enable,
                b'p' => requested.print = true,
                b'f' | b'F' => requested.functions = true,
                other => return Err(*other),
            }
        }
        at += 1;
    }
    Ok((requested, &args[at..]))
}

/// Apply one operand, which is either `name` or `name=value`.
///
/// Order matters twice over. An attribute that reshapes a value -- `-i`,
/// `-u`, `-l` -- has to exist before the value is stored, while `-r` and
/// `-x` must wait until after it, or `declare -r x=1` would refuse its
/// own assignment. And `-n` is cleared first because `declare -n ref=y`
/// re-points the reference rather than writing through it.
// [spec:nsh:req:compat.bash.functions-scoping]
fn apply(
    shell: &mut Shell,
    requested: &Requested,
    operand: &BStr,
    scope: Scope,
) -> Result<bool, Error> {
    let bytes: &[u8] = operand.as_ref();
    let (name, value) = match bytes.find_byte(b'=') {
        Some(at) => (
            BStr::new(&bytes[..at]).to_owned(),
            Some(BStr::new(&bytes[at + 1..]).to_owned()),
        ),
        None => (operand.to_owned(), None),
    };
    // A subscripted operand keeps its brackets out of the stored name.
    let base = match name.find_byte(b'[') {
        Some(at) if name.last() == Some(&b']') => BStr::new(&name[..at]).to_owned(),
        _ => name.clone(),
    };
    let base = BStr::new(base.as_slice());

    // A reference may only be given the name of something to refer to.
    if requested.nameref == Some(true)
        && let Some(value) = &value
        && !nameref::is_valid_target(shell, BStr::new(value.as_slice()))
    {
        return Ok(false);
    }

    if scope == Scope::Local {
        let held = if value.is_some() {
            LocalValue::Assigned
        } else {
            LocalValue::Discard
        };
        nameref::make_local(shell, base, held);
    }
    // A declaration records the name even with nothing to store in it,
    // so the attributes below have an entry to live on.
    nameref::ensure_entry(shell, base);

    if let Some(kind) = requested.kind {
        arrays::ensure_kind(shell, base, kind, VariableAttributes::NONE)?;
    }

    for (attribute, enabled) in [
        (BashAttribute::Integer, requested.integer),
        (BashAttribute::Lowercase, requested.lowercase),
        (BashAttribute::Uppercase, requested.uppercase),
        (BashAttribute::Trace, requested.trace),
    ] {
        if let Some(enabled) = enabled {
            crate::variables::value::set_bash_attribute(shell, base, attribute, enabled);
        }
    }
    if requested.nameref.is_some() {
        nameref::clear_reference(shell, base);
    }

    if let Some(value) = value {
        assign(
            shell,
            BStr::new(name.as_slice()),
            base,
            BStr::new(value.as_slice()),
        )?;
    }

    if requested.nameref == Some(true) {
        crate::variables::value::set_bash_attribute(shell, base, BashAttribute::Nameref, true);
    }

    let attributes = VariableAttributes {
        exported: requested.exported == Some(true),
        read_only: requested.read_only,
        fixed: false,
    };
    if attributes != VariableAttributes::NONE {
        add_attributes(shell, base, attributes);
    }
    // `+x` is the one attribute Bash lets a declaration take back.
    if requested.exported == Some(false) {
        crate::variables::value::clear_exported(shell, base);
    }
    Ok(true)
}

fn assign(shell: &mut Shell, name: &BStr, base: &BStr, value: &BStr) -> Result<(), Error> {
    let bytes: &[u8] = name.as_ref();
    match bytes.find_byte(b'[') {
        Some(at) if bytes.last() == Some(&b']') => {
            let subscript = BStr::new(&bytes[at + 1..bytes.len() - 1]).to_owned();
            let selector =
                arrays::resolve_text_selector(shell, base, BStr::new(subscript.as_slice()))?;
            arrays::assign_element(
                shell,
                base,
                &selector,
                value,
                false,
                arrays::ReadOnlyGuard::Enforce,
            )
        }
        _ => set_bytes(shell, base, Some(value), VariableAttributes::NONE),
    }
}

fn valid_operand(shell: &mut Shell, operand: &BStr) -> bool {
    let bytes: &[u8] = operand.as_ref();
    let name = bytes.find_byte(b'=').map_or(bytes, |at| &bytes[..at]);
    let name = name.find_byte(b'[').map_or(name, |at| &name[..at]);
    crate::parser::is_valid_name(&shell.locale, BStr::new(name))
}

/// `declare -p`, and the bare `declare` listing.
fn print(shell: &mut Shell, requested: &Requested, operands: &[&BStr]) -> Result<Flow, Error> {
    if requested.functions {
        // Function listing is the `-f` slice's own work; declaring it
        // unsupported is more honest than printing an empty list.
        return Ok(Flow::Done(ExitStatus::SUCCESS));
    }

    /* A bare `declare` -- no options, no operands -- is Bash's own
     * `set`: it lists `name=value`, not the `declare -- name="value"`
     * line that `-p` prints. */
    if !requested.print && operands.is_empty() && requested.is_bare() {
        return crate::variables::show_vars(
            shell,
            BStr::new(b""),
            crate::variables::VariableSelection::Set,
        )
        .map(|()| Flow::Done(ExitStatus::SUCCESS));
    }

    let names: Vec<BString> = if operands.is_empty() {
        // A listing with no operands reports only the names the
        // requested attributes select, as `declare -pn` does.
        crate::variables::value::declared_names(shell)
            .into_iter()
            .filter(|name| selected(shell, requested, BStr::new(name.as_slice())))
            .collect()
    } else {
        operands.iter().map(|name| (*name).to_owned()).collect()
    };

    let mut status = ExitStatus::SUCCESS;
    for name in names {
        let name = BStr::new(name.as_slice());
        let Some(rendered) = render(shell, name) else {
            let mut message = b"declare: ".to_vec();
            message.extend_from_slice(name.as_ref());
            message.extend_from_slice(b": not found\n");
            shell.write_output(OutputDestination::Stderr, &message)?;
            status = ExitStatus::FAILURE;
            continue;
        };
        let mut line = rendered;
        line.push(b'\n');
        shell.write_output(OutputDestination::Stdout, &line)?;
    }
    Ok(Flow::Done(status))
}

/// Whether an attribute-filtered listing includes `name`.
// [spec:nsh:req:compat.bash.functions-scoping]
fn selected(shell: &Shell, requested: &Requested, name: &BStr) -> bool {
    let attributes = crate::variables::variable_attributes(shell, name).unwrap_or_default();
    let bash = crate::variables::value::bash_attributes(shell, name).unwrap_or_default();
    if requested.read_only && !attributes.read_only {
        return false;
    }
    if requested.exported == Some(true) && !attributes.exported {
        return false;
    }
    for (attribute, wanted) in [
        (BashAttribute::Integer, requested.integer),
        (BashAttribute::Lowercase, requested.lowercase),
        (BashAttribute::Uppercase, requested.uppercase),
        (BashAttribute::Nameref, requested.nameref),
        (BashAttribute::Trace, requested.trace),
    ] {
        if wanted == Some(true) && !bash.contains(attribute) {
            return false;
        }
    }
    match requested.kind {
        Some(kind) => crate::variables::value::variable_kind(shell, name) == Some(kind),
        None => true,
    }
}

fn render(shell: &Shell, name: &BStr) -> Option<BString> {
    let double_quote = |value: &BStr| crate::escape::bash::declaration_quote(&shell.locale, value);
    // A name that was declared without a value still has a declaration
    // to print; only a name with no entry at all is missing.
    let attributes = crate::variables::variable_attributes(shell, name)?;
    let value = crate::variables::value::variable_value(shell, name);
    let bash = crate::variables::value::bash_attributes(shell, name).unwrap_or_default();

    let mut flags = Vec::new();
    match value.map(VariableValue::kind) {
        Some(VariableKind::Indexed) => flags.push(b'a'),
        Some(VariableKind::Associative) => flags.push(b'A'),
        Some(VariableKind::Scalar) | None => {}
    }
    for (attribute, letter) in [
        (BashAttribute::Integer, b'i'),
        (BashAttribute::Lowercase, b'l'),
        (BashAttribute::Nameref, b'n'),
        (BashAttribute::Trace, b't'),
        (BashAttribute::Uppercase, b'u'),
    ] {
        if bash.contains(attribute) {
            flags.push(letter);
        }
    }
    if attributes.read_only {
        flags.push(b'r');
    }
    if attributes.exported {
        flags.push(b'x');
    }
    if flags.is_empty() {
        flags.push(b'-');
    }

    let mut line = BString::from("declare -");
    line.extend_from_slice(&flags);
    line.push(b' ');
    line.extend_from_slice(name.as_ref());
    let Some(value) = value else {
        return Some(line);
    };
    match value.kind() {
        VariableKind::Scalar => {
            if let Some(scalar) = value.scalar_ref() {
                line.extend_from_slice(b"=");
                line.extend_from_slice(&double_quote(scalar));
            }
        }
        VariableKind::Indexed | VariableKind::Associative => {
            line.extend_from_slice(b"=(");
            let keys = arrays::keys(value);
            let elements = arrays::elements(value);
            for (position, (key, element)) in keys.iter().zip(elements.iter()).enumerate() {
                if position > 0 {
                    line.push(b' ');
                }
                line.push(b'[');
                line.extend_from_slice(key);
                line.extend_from_slice(b"]=");
                line.extend_from_slice(&double_quote(BStr::new(element.as_slice())));
            }
            /* Bash pads the closing paren for associative arrays only, and
             * only when there is an element to pad away from: an empty one
             * prints `=()` like an indexed array. */
            if value.kind() == VariableKind::Associative && !keys.is_empty() {
                line.push(b' ');
            }
            line.push(b')');
        }
    }
    Some(line)
}
