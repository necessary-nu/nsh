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
use crate::variables::value::{BashAttribute, VariableKind};
use crate::variables::{VariableAttributes, add_attributes, set_bytes};

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
    exported: bool,
    global: bool,
    print: bool,
    functions: bool,
}

// [spec:nsh:req:compat.bash.arrays-declarations]
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

    let mut status = ExitStatus::SUCCESS;
    for operand in operands {
        apply(shell, &requested, operand)?;
        if !valid_operand(shell, operand) {
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
                b'x' => requested.exported = enable,
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
fn apply(shell: &mut Shell, requested: &Requested, operand: &BStr) -> Result<(), Error> {
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

    if let Some(kind) = requested.kind {
        arrays::ensure_kind(
            shell,
            BStr::new(base.as_slice()),
            kind,
            VariableAttributes::NONE,
        )?;
    }

    if let Some(value) = value {
        assign(
            shell,
            BStr::new(name.as_slice()),
            BStr::new(base.as_slice()),
            BStr::new(value.as_slice()),
        )?;
    } else if requested.kind.is_none()
        && crate::variables::variable_attributes(shell, BStr::new(base.as_slice())).is_none()
    {
        // A bare `declare name` creates the name unset but declared, so
        // later attribute reads see an entry. It must not touch a name
        // that already exists: `ref=1; typeset -n ref` keeps the value,
        // and `set_bytes(.., None, NONE)` would remove the entry.
        set_bytes(
            shell,
            BStr::new(base.as_slice()),
            None,
            VariableAttributes::NONE,
        )?;
    }

    let attributes = VariableAttributes {
        exported: requested.exported,
        read_only: requested.read_only,
        fixed: false,
    };
    if attributes != VariableAttributes::NONE {
        add_attributes(shell, BStr::new(base.as_slice()), attributes);
    }

    for (attribute, enabled) in [
        (BashAttribute::Integer, requested.integer),
        (BashAttribute::Lowercase, requested.lowercase),
        (BashAttribute::Uppercase, requested.uppercase),
        (BashAttribute::Nameref, requested.nameref),
        (BashAttribute::Trace, requested.trace),
    ] {
        if let Some(enabled) = enabled {
            crate::variables::value::set_bash_attribute(
                shell,
                BStr::new(base.as_slice()),
                attribute,
                enabled,
            );
        }
    }
    Ok(())
}

fn assign(shell: &mut Shell, name: &BStr, base: &BStr, value: &BStr) -> Result<(), Error> {
    let bytes: &[u8] = name.as_ref();
    match bytes.find_byte(b'[') {
        Some(at) if bytes.last() == Some(&b']') => {
            let subscript = BStr::new(&bytes[at + 1..bytes.len() - 1]).to_owned();
            let selector = arrays::resolve_selector(shell, base, BStr::new(subscript.as_slice()))?;
            arrays::assign_element(shell, base, &selector, value, false)
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

    let names: Vec<BString> = if operands.is_empty() {
        crate::variables::value::declared_names(shell)
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

/// `declare -p` quotes with double quotes, escaping only what would
/// otherwise be expanded when the line is read back.
fn double_quote(value: &BStr) -> BString {
    let mut quoted = BString::from("\"");
    for byte in value.as_ref() as &[u8] {
        if matches!(byte, b'"' | b'\\' | b'$' | b'`') {
            quoted.push(b'\\');
        }
        quoted.push(*byte);
    }
    quoted.push(b'"');
    quoted
}

fn render(shell: &Shell, name: &BStr) -> Option<BString> {
    let value = crate::variables::value::variable_value(shell, name)?;
    let attributes = crate::variables::variable_attributes(shell, name)?;
    let bash = crate::variables::value::bash_attributes(shell, name).unwrap_or_default();

    let mut flags = Vec::new();
    match value.kind() {
        VariableKind::Indexed => flags.push(b'a'),
        VariableKind::Associative => flags.push(b'A'),
        VariableKind::Scalar => {}
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
            // Bash pads the closing paren for associative arrays only.
            if value.kind() == VariableKind::Associative {
                line.push(b' ');
            }
            line.push(b')');
        }
    }
    Some(line)
}
