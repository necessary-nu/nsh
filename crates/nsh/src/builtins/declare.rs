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
use crate::variables::value::{BashAttribute, VariableKind};
use crate::variables::{VariableAttributes, add_attributes};

mod compound;

pub(crate) mod functions;

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
    /// `+a` or `+A`: the kind the invocation asks to take away, which
    /// Bash refuses for a name that has it.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    destroyed_kind: Option<VariableKind>,
    integer: Option<bool>,
    lowercase: Option<bool>,
    uppercase: Option<bool>,
    nameref: Option<bool>,
    trace: Option<bool>,
    /// `+r`, which Bash refuses for a name that is already read-only.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    read_only: Option<bool>,
    exported: Option<bool>,
    global: bool,
    print: bool,
    /// `-f`: print a function's source rather than declare a variable.
    functions: bool,
    /// `-F`: print only the name a function is filed under.
    function_names: bool,
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
            && self.read_only != Some(true)
            && !self.global
            && !self.functions
            && !self.function_names
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

    /* The function options are not declarations and never reach the
     * assignment path below: `declare -f x` prints what `x` already is
     * and must not bring an `x` into being. */
    if requested.functions || requested.function_names {
        return functions::run(shell, requested.function_names, operands);
    }

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
        /* `local -` names no variable: it saves the option set for the
         * duration of the function and restores it on return, which is
         * what `make_local_bytes` already does for the POSIX dialect.
         * Only `local` spells it -- `typeset -` is `not a valid
         * identifier` in the reference too. */
        // [spec:nsh:req:compat.bash.functions-scoping]
        if forced_local && *operand == "-" {
            crate::variables::make_local_bytes(shell, operand, VariableAttributes::NONE)?;
            continue;
        }
        let name = operand_name(operand);
        if apply(shell, args[0], &requested, operand, scope)?
            && crate::parser::is_valid_name(&shell.locale, name)
        {
            continue;
        }
        status = ExitStatus::FAILURE;
        /* One refused operand does not refuse the others, and a
         * structural value waiting for this command's attributes has to
         * be told which name was refused. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        shell.evaluation.refused_declarations.push(name.to_owned());
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
                /* `+a` asks for the kind to go rather than for an array;
                 * it does not make the operand a compound one, and Bash
                 * refuses it for a name that holds the kind. */
                b'a' if enable => requested.kind = Some(VariableKind::Indexed),
                b'A' if enable => requested.kind = Some(VariableKind::Associative),
                b'a' => requested.destroyed_kind = Some(VariableKind::Indexed),
                b'A' => requested.destroyed_kind = Some(VariableKind::Associative),
                b'i' => requested.integer = Some(enable),
                b'l' => requested.lowercase = Some(enable),
                b'u' => requested.uppercase = Some(enable),
                b'n' => requested.nameref = Some(enable),
                b't' => requested.trace = Some(enable),
                b'r' => requested.read_only = Some(enable),
                b'x' => requested.exported = Some(enable),
                b'g' => requested.global = enable,
                b'p' => requested.print = true,
                b'f' => requested.functions = true,
                b'F' => requested.function_names = true,
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
    builtin: &BStr,
    requested: &Requested,
    operand: &BStr,
    scope: Scope,
) -> Result<bool, Error> {
    let (name, value, append) = arrays::split_assignment_operand(operand);
    // A subscripted operand keeps its brackets out of the stored name.
    let written = operand_name(BStr::new(name.as_slice())).to_owned();
    let base = BStr::new(written.as_slice());
    /* An operand that does not name a variable is reported and skipped;
     * the operands beside it are unaffected, as they are in Bash. */
    if !crate::parser::is_valid_name(&shell.locale, base) {
        let mut message = builtin.to_vec();
        message.extend_from_slice(b": `");
        message.extend_from_slice(operand.as_ref());
        message.extend_from_slice(b"': not a valid identifier\n");
        shell.write_output(OutputDestination::Stderr, &message)?;
        return Ok(false);
    }

    // A reference may only be given the name of something to refer to.
    if requested.nameref == Some(true)
        && let Some(refusal) =
            refused_reference(shell, requested, base, value.as_deref().map(BStr::new))
    {
        if !refusal.is_empty() {
            let mut message = builtin.to_vec();
            message.extend_from_slice(b": ");
            message.extend_from_slice(&refusal);
            message.push(b'\n');
            shell.write_output(OutputDestination::Stderr, &message)?;
        }
        return Ok(false);
    }

    /* Every letter but `-n` reaches the variable a reference names, so
     * `declare -r rr` protects `rr`'s target rather than `rr`; `-n`
     * itself must not, because `declare -n rr=y` re-points `rr`. A
     * reference that reaches no variable declares nothing at all. */
    // [spec:nsh:req:compat.bash.functions-scoping]
    let base = if requested.nameref.is_some() {
        base.to_owned()
    } else {
        let Some(target) = nameref::declared_name(shell, base) else {
            return Ok(true);
        };
        target
    };
    let base = BStr::new(base.as_slice());

    /* An array's kind is how its value is held rather than a flag over
     * it, so Bash will not take it away: `declare +a` on an indexed
     * array reports and answers 1 with the variable exactly as it was,
     * before any other letter of the same command has landed and before
     * any value it carries is stored. `+a` on an *associative* array,
     * and either letter on a name that is neither, are ordinary. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    if requested.destroyed_kind.is_some()
        && requested.destroyed_kind == crate::variables::value::variable_kind(shell, base)
    {
        /* Bash names the operand it was given rather than the array a
         * reference led it to. */
        let mut message = builtin.to_vec();
        message.extend_from_slice(b": ");
        message.extend_from_slice(written.as_slice());
        message.extend_from_slice(b": cannot destroy array variables in this way\n");
        shell.write_output(OutputDestination::Stderr, &message)?;
        return Ok(false);
    }

    /* A read-only attribute cannot be taken back by any means, in
     * either shell -- that is what makes `readonly` worth anything --
     * so only the report and the status are Bash's own. Beside the kind
     * refusal because it has the same shape: it fires only for a name
     * that already carries the thing the letter would remove, and it
     * fires before any other letter of the same command has landed. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    if requested.read_only == Some(false)
        && crate::variables::variable_attributes(shell, base)
            .is_some_and(|attributes| attributes.read_only)
    {
        let mut message = builtin.to_vec();
        message.extend_from_slice(b": ");
        message.extend_from_slice(written.as_slice());
        message.extend_from_slice(b": readonly variable\n");
        shell.write_output(OutputDestination::Stderr, &message)?;
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
    /* Declaring a name whose value the read path recomputes is a read of
     * it: `declare SECONDS` is `declare -i SECONDS="0"` in the
     * reference, where the same name untouched prints
     * `declare -- SECONDS` with nothing beside it. */
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    crate::variables::special::refresh(shell, base);
    let was_exported = crate::variables::variable_attributes(shell, base)
        .is_some_and(|attributes| attributes.exported);

    if let Some(kind) = requested.kind {
        if !arrays::convertible(shell, base, kind) {
            arrays::reject_conversion(shell, builtin, base, kind)?;
            return Ok(false);
        }
        arrays::ensure_kind(
            shell,
            base,
            kind,
            VariableAttributes::NONE,
            arrays::ReadOnlyGuard::Enforce,
        )?;
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
        let value = BStr::new(value.as_slice());
        /* An operand that arrived as one word still spells a compound
         * value, and a name that is an array takes it as one. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        match compound::parenthesised(value).filter(|_| is_array(shell, base)) {
            Some(inner) => compound::assign(shell, base, inner)?,
            None => arrays::assign_text_target(shell, BStr::new(name.as_slice()), value, append)?,
        }
    }

    if requested.nameref == Some(true) {
        crate::variables::value::set_bash_attribute(shell, base, BashAttribute::Nameref, true);
    }

    let attributes = VariableAttributes {
        exported: requested.exported == Some(true)
            || exported_by_allexport(shell, requested, base, scope),
        read_only: requested.read_only == Some(true),
        fixed: false,
    };
    if attributes != VariableAttributes::NONE {
        add_attributes(shell, base, attributes);
    }
    // `+x` is the one attribute Bash lets a declaration take back.
    if requested.exported == Some(false) {
        crate::variables::value::clear_exported(shell, base);
    } else if !attributes.exported && !was_exported && is_array(shell, base) {
        /* A declaration that names an array is not exported by `set -a`,
         * and the value it just stored may have been: the plain `z=q` on
         * an array *is* exported there, and it is the same write. So the
         * option is taken back here rather than withheld from the write,
         * which cannot tell a declaration from an assignment. */
        // [spec:nsh:req:compat.bash.arrays-declarations]
        crate::variables::value::clear_exported(shell, base);
    }
    Ok(true)
}

/// Whether `set -a` exports the name this declaration is creating.
///
/// Bash's `-a` reaches a declaration that does not name an array:
/// `set -a; declare -i n` is `declare -ix n`, where `set -a; declare -a z`
/// is not exported at all. A declaration local to a function is not
/// reached either -- `local -i q` under `-a` stays `declare -i q` --
/// while `local x=1` *is* exported, because its value goes through the
/// ordinary assignment path and that is where the option is picked up.
// [spec:nsh:req:compat.bash.arrays-declarations]
fn exported_by_allexport(shell: &Shell, requested: &Requested, base: &BStr, scope: Scope) -> bool {
    requested.exported != Some(false)
        && scope == Scope::Global
        && !is_array(shell, base)
        && shell
            .options
            .enabled(crate::options::ShellOption::AllExport)
}

/// Why `-n` may not be given to this operand, as the text Bash reports.
///
/// `None` accepts it. An empty refusal is one Bash makes silently: an
/// integer conversion turns the value into a number, and a number is
/// never an identifier, so `declare -in r=x5` refuses with nothing
/// written where `declare -in r=5` reports the `5` it was handed.
///
/// With no value written it is the value the name already holds that
/// would be referred to, which is the case `is_valid_target` alone never
/// saw: `ref=1; typeset -n ref` leaves `ref` a plain scalar in Bash and
/// made it a reference here, and every transform over that state was
/// then an answer about a variable the reference cannot hold.
// [spec:nsh:req:compat.bash.functions-scoping]
fn refused_reference(
    shell: &Shell,
    requested: &Requested,
    base: &BStr,
    value: Option<&BStr>,
) -> Option<Vec<u8>> {
    let refusal = |text: &BStr| {
        let mut message = b"`".to_vec();
        message.extend_from_slice(text.as_ref());
        message.extend_from_slice(b"': invalid variable name for name reference");
        message
    };
    if let Some(value) = value {
        if !nameref::is_valid_target(shell, value) {
            return Some(refusal(value));
        }
        return (requested.integer == Some(true)).then(Vec::new);
    }
    if is_array(shell, base) {
        let mut message = base.to_vec();
        message.extend_from_slice(b": reference variable cannot be an array");
        return Some(message);
    }
    let held = crate::variables::value::variable_value(shell, base)?.scalar_ref()?;
    (!nameref::is_valid_target(shell, held)).then(|| refusal(held))
}

/// Whether `name` holds an array, which decides how `(…)` is read.
fn is_array(shell: &Shell, name: &BStr) -> bool {
    matches!(
        crate::variables::value::variable_kind(shell, name),
        Some(VariableKind::Indexed | VariableKind::Associative)
    )
}

/// The name an operand declares, without its subscript or its value.
///
/// A subscript comes off only when it is closed: `a[2]=3` declares `a`,
/// where `a[` declares nothing and keeps its bracket, so the two are
/// never mistaken for one another when a refusal is recorded by name.
fn operand_name(operand: &BStr) -> &BStr {
    let bytes: &[u8] = operand.as_ref();
    let name = bytes.find_byte(b'=').map_or(bytes, |at| &bytes[..at]);
    if name.last() != Some(&b']') {
        return BStr::new(name);
    }
    BStr::new(name.find_byte(b'[').map_or(name, |at| &name[..at]))
}

/// `declare -p`, and the bare `declare` listing.
fn print(shell: &mut Shell, requested: &Requested, operands: &[&BStr]) -> Result<Flow, Error> {
    /* A bare `declare` -- no options, no operands -- is Bash's own
     * `set`: it lists `name=value`, not the `declare -- name="value"`
     * line that `-p` prints. */
    if !requested.print && operands.is_empty() && requested.is_bare() {
        return crate::variables::show_vars(
            shell,
            BStr::new(b""),
            crate::variables::VariableSelection::Set,
            None,
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
    let named = !operands.is_empty();
    for name in names {
        let name = BStr::new(name.as_slice());
        /* Asking for a name by name is a *read*, and asking for the
         * listing is not: `declare -p SECONDS` is `declare -i
         * SECONDS="0"` in the reference where `declare -p` in the same
         * fresh shell prints `declare -- SECONDS` with nothing beside
         * it. The asymmetry is Bash's, and it is what makes
         * `declare -p BASHPID` name the running process rather than the
         * seed the shell published. */
        // [spec:nsh:req:compat.bash.builtins-special-variables]
        if named {
            crate::variables::special::refresh(shell, name);
        }
        let Some(rendered) = crate::variables::declaration::declaration_line(shell, name) else {
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
    if requested.read_only == Some(true) && !attributes.read_only {
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
