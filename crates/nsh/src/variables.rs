//! Shell variables and function-local variable scopes.
//!
//! Rules: `docs/spec/port/src/var.md`.
//!
//! dash stores each variable as a pinned `NAME=value\0` allocation and
//! links raw pointers into a hash table and the `local` save stack. None of
//! those addresses are observable shell state. This implementation stores
//! the name and value separately in an owned ordered map; local scopes save
//! owned entries by name. The ordering remains bytewise, as documented in
//! `docs/divergences.md`.

use bstr::{BStr, BString, ByteSlice};
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};
use std::collections::BTreeMap;
use std::ffi::OsString;

use crate::context::Shell;
use crate::error::Error;
use crate::options::{OptionSet, ShellOption, options_changed};
// [spec:nsh:def:idiom.shell-options]

pub(crate) mod value;
use value::{BashAttributes, VariableKind, VariableValue};

pub(crate) mod arrays;

pub(crate) mod nameref;

pub(crate) mod call_stack;

pub(crate) mod declaration;

pub(crate) mod special;
pub(crate) use special::{
    continuation_prompt_value, default_ifs, default_path, history_size_value, ifs_is_set,
    ifs_value, mail_path_is_set, mail_path_value, mail_value, path_value, primary_prompt_value,
    trace_prompt_value,
};

/// Persistent attributes of a shell variable.
// [spec:nsh:def:idiom.variable-expansion-state]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct VariableAttributes {
    pub(crate) exported: bool,
    pub(crate) read_only: bool,
    pub(crate) fixed: bool,
}

impl VariableAttributes {
    pub(crate) const NONE: Self = Self {
        exported: false,
        read_only: false,
        fixed: false,
    };
    pub(crate) const EXPORTED: Self = Self {
        exported: true,
        ..Self::NONE
    };
    pub(crate) const READ_ONLY: Self = Self {
        read_only: true,
        ..Self::NONE
    };
    pub(crate) const FIXED: Self = Self {
        fixed: true,
        ..Self::NONE
    };
}

/// Which variables a declaration-style listing includes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum VariableSelection {
    Set,
    Exported,
    ReadOnly,
}

/// Whether a variable is unset, declared with a kind and nothing in it,
/// or owns a value.
///
/// The middle state is Bash's *invisible* variable, and it is the whole
/// of the difference between a declared array and an assigned empty one:
/// `declare -a z` spells itself back as `declare -a z`, where
/// `declare -a z=()` spells the empty list it was given. Both printers
/// -- `declare -p` and `${name[@]@A}` -- read one renderer, so the
/// distinction has to live here rather than in either of them.
// [spec:nsh:def:idiom.variable-expansion-state]
// [spec:nsh:req:compat.bash.arrays-declarations]
#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum VariableState {
    #[default]
    Unset,
    /// An array kind with no value yet. Only `-a` and `-A` reach here:
    /// a scalar declared without a value is simply `Unset`, which is
    /// what already makes `declare -i n` print `declare -i n`.
    Declared(VariableKind),
    Set(VariableValue),
}

const DEFAULT_IFS: &[u8] = b" \t\n";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Callback {
    None,
    Ifs,
    Mail,
    Path,
    Getopts,
    History,
    Locale,
    /// A name `special` recomputes on read and reacts to on assignment.
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    Special,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallbackPolicy {
    Run,
    Suppress,
}

/// Whether `set -a` marks the name a write creates.
///
/// Bash's `-a` marks an assignment that stores a *scalar*, and a
/// compound one is not: `set -a; x=1` exports `x` where
/// `set -a; z=(1)` exports nothing and `set -a; z[0]=5` exports
/// nothing. Every array-shaped write in this shell goes through
/// [`arrays::store`], so that is the one write that declines.
// [spec:nsh:req:compat.bash.arrays-declarations]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum AllExport {
    /// A scalar assignment, which the option marks.
    Marks,
    /// A structural value, which it does not reach.
    Declines,
}

#[derive(Clone, Debug)]
struct Variable {
    attributes: VariableAttributes,
    state: VariableState,
    /// Declaration attributes that do not already have a dash flag.
    bash_attributes: BashAttributes,
    callback: Callback,
    /// `$LINENO` is computed on read until a script assigns to it.
    dynamic_lineno: bool,
}

impl Variable {
    fn set(value: &[u8], attributes: VariableAttributes, callback: Callback) -> Self {
        Self {
            attributes,
            state: VariableState::Set(VariableValue::Scalar(BString::from(value))),
            bash_attributes: BashAttributes::new(),
            callback,
            dynamic_lineno: false,
        }
    }

    fn unset(attributes: VariableAttributes, callback: Callback) -> Self {
        Self {
            attributes,
            state: VariableState::Unset,
            bash_attributes: BashAttributes::new(),
            callback,
            dynamic_lineno: false,
        }
    }
}

enum LocalVariable {
    Options(OptionSet),
    /// The declaration created this name; remove it on return.
    Created(BString),
    /// Restore the complete previous entry on return.
    Saved {
        name: BString,
        previous: Variable,
    },
}

pub struct LocalVariableScopes {
    entries: Vec<LocalVariable>,
}

// [spec:posix:req:xcu.env.effects-confined-to-section]
// [spec:posix:req:xcu.env.eight-bit-transparency]
// [spec:posix:req:param.byte-values]
// [spec:posix:def:param.denotation]
// [spec:posix:def:param.set-state]
// [spec:posix:sem:param.variable-creation]
pub struct VariableTable {
    entries: BTreeMap<BString, Variable>,
    pub(crate) line_number: i32,
    locals: Vec<LocalVariableScopes>,
    /// Which local frame each running function body owns, innermost last.
    ///
    /// A declaration built-in pushes a frame of its own, so the frame on
    /// top is not the one whose restore a `local` must land in.
    // [spec:nsh:req:compat.bash.functions-scoping]
    function_frames: Vec<usize>,
    /// The call stack `FUNCNAME`, `BASH_SOURCE`, `BASH_LINENO` and
    /// `caller` report.
    // [spec:nsh:req:compat.bash.traps-introspection]
    pub(crate) call_stack: call_stack::CallStack,
    /// The generators, clocks and published facts of the Bash dialect.
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    pub(crate) special: special::SpecialState,
}

impl VariableTable {
    pub(crate) fn new() -> Self {
        Self {
            entries: BTreeMap::new(),
            line_number: 0,
            locals: Vec::new(),
            function_frames: Vec::new(),
            call_stack: call_stack::CallStack::new(),
            special: special::SpecialState::new(),
        }
    }

    #[inline]
    pub(crate) fn in_function(&self) -> bool {
        !self.locals.is_empty()
    }

    fn push_local(&mut self, local: LocalVariable) {
        self.locals
            .last_mut()
            .expect("mklocal runs inside a function")
            .entries
            .push(local);
    }
}

/// The name portion of `name` or `name=value`.
pub(crate) fn assignment_name(text: &BStr) -> &BStr {
    BStr::new(
        text.split_once_str(b"=")
            .map_or(text.as_bytes(), |(name, _)| name),
    )
}

fn valid_name(locale: &nsh_platform::Locale, name: &BStr) -> bool {
    let mut bytes = name.iter().copied();
    bytes
        .next()
        .is_some_and(|byte| byte == b'_' || locale.is_alpha(byte))
        && bytes.all(|byte| byte == b'_' || locale.is_alphanumeric(byte))
}

const LOCALE_CATEGORIES: [(nsh_platform::LocaleCategory, &[u8]); 6] = [
    (nsh_platform::LocaleCategory::Collate, b"LC_COLLATE"),
    (nsh_platform::LocaleCategory::Ctype, b"LC_CTYPE"),
    (nsh_platform::LocaleCategory::Messages, b"LC_MESSAGES"),
    (nsh_platform::LocaleCategory::Monetary, b"LC_MONETARY"),
    (nsh_platform::LocaleCategory::Numeric, b"LC_NUMERIC"),
    (nsh_platform::LocaleCategory::Time, b"LC_TIME"),
];

macro_rules! is_locale_variable {
    ($name:expr) => {
        $name == b"LC_ALL"
            || $name == b"LANG"
            || LOCALE_CATEGORIES
                .iter()
                .any(|(_, category_name)| $name == *category_name)
    };
}

// [spec:nsh:sem:shell-locale.selection]
// [spec:posix:req:xcurel.establish-locale]
fn selected_locale(shell: &Shell) -> std::io::Result<nsh_platform::Locale> {
    let nonempty = |name: &[u8]| {
        shell
            .variables
            .entries
            .get(BStr::new(name))
            .and_then(|var| {
                matches!(&var.state, VariableState::Set(_))
                    .then(|| var.scalar_owned())
                    .flatten()
                    .filter(|value| !value.is_empty())
            })
    };
    let selected: Vec<_> = LOCALE_CATEGORIES
        .iter()
        .map(|(category, name)| {
            let locale = nonempty(b"LC_ALL")
                .or_else(|| nonempty(name))
                .or_else(|| nonempty(b"LANG"))
                .unwrap_or_else(|| BString::from("C"));
            (*category, locale)
        })
        .collect();
    let overrides: Vec<_> = selected
        .iter()
        .map(|(category, name)| (*category, name.as_slice()))
        .collect();
    nsh_platform::Locale::new(b"C", &overrides)
}

fn builtin(name: &[u8], value: Option<&[u8]>, callback: Callback) -> (BString, Variable) {
    let var = match value {
        Some(value) => Variable::set(value, VariableAttributes::FIXED, callback),
        None => Variable::unset(VariableAttributes::FIXED, callback),
    };
    (BString::from(name), var)
}

// [spec:dash:sem:var.initvar-fn]
// [spec:posix:def:param.shell-variables]
// [spec:posix:def:param.home]
// [spec:posix:def:param.ifs]
// [spec:posix:req:param.ifs-unset]
// [spec:posix:req:param.ifs-initial-value]
// [spec:posix:req:param.lang]
// [spec:posix:req:param.lc-all]
// [spec:posix:req:param.lc-collate]
// [spec:posix:req:param.lc-ctype]
// [spec:posix:req:param.lc-messages]
// [spec:posix:req:param.lineno]
// [spec:posix:req:param.nlspath]
// [spec:posix:def:param.path]
// [spec:posix:req:param.ppid]
// [spec:posix:req:param.ps1-default]
pub fn initialize_variables(shell: &mut Shell) {
    let prompt = special::default_primary_prompt();
    let continuation = special::default_continuation_prompt();
    let default_path = nsh_platform::default_search_path().to_shell_bytes();
    let mut entries = [
        builtin(b"IFS", Some(DEFAULT_IFS), Callback::Ifs),
        builtin(b"MAIL", None, Callback::Mail),
        builtin(b"MAILPATH", None, Callback::Mail),
        builtin(b"PATH", Some(&default_path), Callback::Path),
        builtin(b"PS1", Some(prompt.as_ref()), Callback::None),
        builtin(b"PS2", Some(continuation.as_ref()), Callback::None),
        builtin(b"PS4", Some(b"+ "), Callback::None),
        builtin(b"OPTIND", Some(b"1"), Callback::Getopts),
        builtin(b"LINENO", Some(b"0"), Callback::None),
        builtin(b"TERM", None, Callback::None),
        builtin(b"HISTSIZE", None, Callback::History),
        builtin(b"LC_ALL", None, Callback::Locale),
        builtin(b"LC_COLLATE", None, Callback::Locale),
        builtin(b"LC_CTYPE", None, Callback::Locale),
        builtin(b"LC_NUMERIC", None, Callback::Locale),
        builtin(b"LANG", None, Callback::Locale),
    ];
    entries[8].1.dynamic_lineno = true;
    for (name, var) in entries {
        shell.variables.entries.entry(name).or_insert(var);
    }
}

impl Shell {
    // [spec:nsh:sem:shell-locale.invalid-selection]
    // [spec:posix:req:param.variable-environment-initialization]
    // [spec:posix:def:sh.environment-variables]
    // [spec:posix:req:sh.envvar-env]
    // [spec:posix:req:sh.envvar-fcedit]
    // [spec:posix:req:sh.envvar-histfile]
    // [spec:posix:req:sh.envvar-histsize]
    // [spec:posix:sem:sh.envvar-home]
    // [spec:posix:sem:sh.envvar-lang-and-lc-all]
    // [spec:posix:sem:sh.envvar-lc-collate]
    // [spec:posix:sem:sh.envvar-lc-ctype]
    // [spec:posix:req:sh.envvar-lc-messages]
    // [spec:posix:req:sh.envvar-mail]
    // [spec:posix:req:sh.envvar-mailcheck]
    // [spec:posix:req:sh.envvar-mailpath]
    // [spec:posix:sem:sh.envvar-nlspath]
    // [spec:posix:sem:sh.envvar-path]
    // [spec:posix:req:sh.envvar-pwd]
    pub(crate) fn initialize_variable_state(
        &mut self,
        pairs: &[(BString, BString)],
    ) -> Result<(), Error> {
        initialize_variables(self);

        for (name, value) in pairs {
            let name = BStr::new(name.as_slice());
            if !is_locale_variable!(name) {
                continue;
            }
            let entry = self
                .variables
                .entries
                .entry(name.to_owned())
                .or_insert_with(|| Variable {
                    attributes: VariableAttributes::NONE,
                    state: VariableState::Unset,
                    bash_attributes: BashAttributes::new(),
                    callback: Callback::Locale,
                    dynamic_lineno: false,
                });
            entry.attributes.exported = true;
            entry.state = VariableState::Set(VariableValue::Scalar(value.clone()));
            entry.callback = Callback::Locale;
            entry.dynamic_lineno = false;
        }
        if let Ok(locale) = selected_locale(self) {
            self.locale = locale;
        }
        self.import_environment_pairs(pairs)?;

        set_bytes(
            self,
            BStr::new(b"IFS"),
            Some(default_ifs()),
            VariableAttributes::NONE,
        )?;
        set_bytes(
            self,
            BStr::new(b"OPTIND"),
            Some(BStr::new(b"1")),
            VariableAttributes::NONE,
        )?;
        let parent_pid = nsh_platform::parent_process_id()
            .map_or_else(|| "0".to_owned(), |process| process.to_string());
        set_bytes(
            self,
            BStr::new(b"PPID"),
            Some(BStr::new(parent_pid.as_bytes())),
            VariableAttributes::NONE,
        )?;

        let pwd = lookup_bytes(self, BStr::new(b"PWD"));
        let valid_pwd = pwd.as_ref().filter(|path| {
            if !nsh_platform::shell_path_is_absolute(path) {
                return false;
            }
            let (Ok(path), Ok(dot)) = (path.try_to_path_buf(), b"."[..].try_to_path_buf()) else {
                return false;
            };
            nsh_platform::path_is_same_file(&path, &dot)
        });
        match valid_pwd {
            Some(path) => crate::working_directory::update_current_directory(
                self,
                crate::working_directory::DirectoryUpdate::New(BStr::new(path)),
                false,
            ),
            None => crate::working_directory::update_current_directory(
                self,
                crate::working_directory::DirectoryUpdate::Unknown,
                false,
            ),
        }
    }

    fn import_environment_pairs(&mut self, pairs: &[(BString, BString)]) -> Result<(), Error> {
        for (name, value) in pairs {
            let name = BStr::new(name.as_slice());
            let value = BStr::new(value.as_slice());
            if !is_locale_variable!(name) && valid_name(&self.locale, name) {
                set_bytes(self, name, Some(value), VariableAttributes::EXPORTED)?;
            }
        }
        Ok(())
    }

    pub(crate) fn unwind_local_variables(&mut self) {
        self.variables.function_frames.clear();
        while !self.variables.locals.is_empty() {
            pop_local_scope(self);
        }
    }
}

// [spec:dash:sem:var.changelocale-fn]
// [spec:dash:sem:var.varfunc-fn]
// [spec:nsh:sem:shell-locale.invalid-selection]
fn run_callback(shell: &mut Shell, name: &BStr, callback: Callback, value: Option<&BStr>) {
    let effective = value.unwrap_or_else(|| BStr::new(b""));
    match callback {
        Callback::None => {}
        Callback::Ifs => {
            let effective_ifs = if ifs_is_set(shell) {
                effective
            } else {
                default_ifs()
            };
            crate::expand::update_ifs_cache(shell, effective_ifs);
        }
        Callback::Mail => crate::mail::reset_mail_state(&mut shell.mail, effective),
        Callback::Path => crate::execution::update_search_path(
            &mut shell.interrupt_deferral,
            &mut shell.commands,
            effective,
        ),
        Callback::Getopts => crate::options::reset_getopts(shell, effective),
        Callback::History => crate::editor::set_history_size(shell, effective),
        Callback::Special => special::assigned(shell, name, value),
        Callback::Locale => {
            // Locale selection depends on the complete variable table, not
            // merely on the entry that triggered this callback.
            if let Ok(locale) = selected_locale(shell) {
                shell.locale = locale;
                let ifs = if ifs_is_set(shell) {
                    ifs_value(shell)
                } else {
                    default_ifs().to_owned()
                };
                crate::expand::update_ifs_cache(shell, BStr::new(ifs.as_slice()));
            }
        }
    }
}

fn set_entry(
    shell: &mut Shell,
    name: &BStr,
    value: Option<&BStr>,
    mut attributes: VariableAttributes,
    callback_policy: CallbackPolicy,
    guard: arrays::ReadOnlyGuard,
    all_export: AllExport,
) -> Result<(), Error> {
    if !valid_name(&shell.locale, name) {
        let mut message = name.to_vec();
        message.extend_from_slice(b": bad variable name");
        return Err(shell.diagnostics().shell_error(&message));
    }
    if all_export == AllExport::Marks && shell.options.enabled(ShellOption::AllExport) {
        attributes.exported = true;
    }

    /* What the decision below needs from an entry already present, read out of
     * it rather than cloned with it. The value a variable holds is the
     * expensive part of one and no arm here reads it: each drops it,
     * overwrites it in place, or leaves it alone. */
    let existing = shell
        .variables
        .entries
        .get(name)
        .map(|old| (old.attributes, old.callback));
    let Some((old_attributes, callback)) = existing else {
        if value.is_none() && attributes == VariableAttributes::NONE {
            return Ok(());
        }
        let fresh_callback = if is_locale_variable!(name) {
            Callback::Locale
        } else {
            Callback::None
        };
        shell.variables.entries.insert(
            name.to_owned(),
            Variable {
                attributes,
                state: value.map_or(VariableState::Unset, |value| {
                    VariableState::Set(VariableValue::Scalar(value.to_owned()))
                }),
                bash_attributes: BashAttributes::new(),
                callback: fresh_callback,
                dynamic_lineno: false,
            },
        );
        if callback_policy == CallbackPolicy::Run {
            run_callback(shell, name, fresh_callback, value);
        }
        return Ok(());
    };

    if old_attributes.read_only && guard == arrays::ReadOnlyGuard::Enforce {
        let mut message = name.to_vec();
        message.extend_from_slice(b": is read only");
        // [spec:nsh:req:compat.bash.error-boundary]
        return Err(shell.diagnostics().dialect_error(&message));
    }

    if value.is_some() || attributes != VariableAttributes::NONE {
        attributes.exported |= old_attributes.exported;
        attributes.read_only |= old_attributes.read_only;
        attributes.fixed |= old_attributes.fixed;
    } else if old_attributes.fixed {
        attributes = VariableAttributes::FIXED;
    } else {
        /* Bash's `unset` on a name the running body made local leaves an
         * invisible local behind rather than taking the entry away. The
         * values were never in question either way -- the frame holds
         * the caller's and restores it on return -- so only a
         * declaration printer can see the difference, which is why the
         * entry has to survive at all. Everything the declaration
         * carried goes with the value: `local -i pv=1; unset pv` is
         * `declare -- pv` there and not `declare -i pv`, and
         * `local -a pv=(1)` leaves no kind either. */
        // [spec:nsh:req:compat.bash.functions-scoping]
        let keep = shell.options.dialect() == crate::options::Dialect::Bash
            && declaration::is_local(shell, name);
        if keep {
            shell.variables.entries.insert(
                name.to_owned(),
                Variable::unset(VariableAttributes::NONE, callback),
            );
        } else {
            shell.variables.entries.remove(name);
        }
        if keep || callback == Callback::Locale {
            run_callback(shell, name, callback, None);
        }
        return Ok(());
    }

    /* The entry keeps its slot: only its attributes and the value it holds
     * change, so both are written where it stands. Rebuilding the whole
     * variable and inserting it over itself cost a copy of the name and a copy
     * of the value already there, on every assignment, for a slot that was
     * never going to move. */
    let mut callback_value = None;
    if let Some(entry) = shell.variables.entries.get_mut(name) {
        entry.attributes = attributes;
        entry.dynamic_lineno = false;
        /* A plain `name=value` on a declared array writes its zero element
         * and makes the name visible: `declare -a z; z=q` is
         * `declare -a z=([0]="q")` in Bash, not a scalar. */
        /* Taken out of the slot and put back into it: what the value owns
         * moves, where reading it out of a clone copied it. */
        let state = core::mem::take(&mut entry.state);
        entry.state = match (state, value) {
            (VariableState::Set(mut existing), Some(value)) => {
                existing.assign_scalar(value);
                VariableState::Set(existing)
            }
            (VariableState::Declared(kind), Some(value)) => {
                let mut fresh = VariableValue::empty(kind);
                fresh.assign_scalar(value);
                VariableState::Set(fresh)
            }
            (VariableState::Unset, Some(value)) => {
                VariableState::Set(VariableValue::Scalar(value.to_owned()))
            }
            (_, None) => VariableState::Unset,
        };
        /* Only a callback that reads it: the value is owned out of the table
         * to say it, and nothing but a callback asks. */
        if callback_policy == CallbackPolicy::Run && callback != Callback::None {
            callback_value = match &entry.state {
                VariableState::Unset | VariableState::Declared(_) => None,
                VariableState::Set(value) => value.scalar_owned(),
            };
        }
    }
    if callback_policy == CallbackPolicy::Run {
        run_callback(
            shell,
            name,
            callback,
            callback_value
                .as_ref()
                .map(|value| BStr::new(value.as_slice())),
        );
    }
    Ok(())
}

/// Read a variable through the owned-byte interface used throughout nsh.
// [spec:dash:sem:var.lookupvar-fn]
pub(crate) fn lookup_bytes(shell: &mut Shell, name: &BStr) -> Option<BString> {
    special::refresh(shell, name);
    shell
        .variables
        .entries
        .get(name)
        .and_then(Variable::scalar_owned)
}

// [spec:dash:sem:var.setvar-fn]
// [spec:nsh:req:compat.bash.functions-scoping]
pub(crate) fn set_bytes(
    shell: &mut Shell,
    name: &BStr,
    value: Option<&BStr>,
    attributes: VariableAttributes,
) -> Result<(), Error> {
    let Some(value) = value else {
        return crate::error::with_interrupts_deferred(shell, |shell| {
            set_entry(
                shell,
                name,
                None,
                attributes,
                CallbackPolicy::Run,
                arrays::ReadOnlyGuard::Enforce,
                AllExport::Marks,
            )
        });
    };
    /* A Bash name reference sends the value somewhere else entirely, and
     * a declaration attribute reshapes it before it is stored. Neither is
     * reachable without `declare`, so a name that carries no Bash
     * attribute pays one map probe and takes the ordinary path. */
    if nameref::assign_through(shell, name, value, attributes)? {
        return Ok(());
    }
    let converted = nameref::declared_value(shell, name, value)?;
    let stored = converted.as_ref().map_or(value, |text| BStr::new(text));
    crate::error::with_interrupts_deferred(shell, |shell| {
        set_entry(
            shell,
            name,
            Some(stored),
            attributes,
            CallbackPolicy::Run,
            arrays::ReadOnlyGuard::Enforce,
            AllExport::Marks,
        )
    })
}

// [spec:dash:sem:var.setvareq-fn]
pub(crate) fn set_assignment_bytes(
    shell: &mut Shell,
    assignment: &BStr,
    attributes: VariableAttributes,
) -> Result<(), Error> {
    match assignment.split_once_str(b"=") {
        Some((name, value)) => {
            set_bytes(shell, BStr::new(name), Some(BStr::new(value)), attributes)
        }
        None => set_bytes(shell, assignment, None, attributes),
    }
}

// [spec:dash:sem:var.setvarint-fn]
pub(crate) fn set_integer_bytes(
    shell: &mut Shell,
    name: &BStr,
    value: i64,
    attributes: VariableAttributes,
    callback_policy: CallbackPolicy,
) -> Result<i64, Error> {
    let text = value.to_string();
    crate::error::with_interrupts_deferred(shell, |shell| {
        set_entry(
            shell,
            name,
            Some(BStr::new(text.as_bytes())),
            attributes,
            callback_policy,
            arrays::ReadOnlyGuard::Enforce,
            AllExport::Marks,
        )
    })?;
    Ok(value)
}

// [spec:dash:sem:var.lookupvarint-fn]
// [spec:posix:req:builtin.set.opt-u-nounset]
pub(crate) fn lookup_integer_bytes(shell: &mut Shell, name: &BStr) -> Result<i64, Error> {
    let value = match lookup_bytes(shell, name) {
        Some(value) => value,
        None if shell.options.enabled(ShellOption::Nounset) => {
            let mut message = name.to_vec();
            message.extend_from_slice(b": parameter not set");
            return Err(shell.diagnostics().shell_error(&message));
        }
        None => BString::default(),
    };
    crate::number::parse_integer(&mut shell.diagnostics(), BStr::new(&value), 0)
}

// [spec:dash:sem:var.unsetvar-fn]
pub(crate) fn unset_bytes(shell: &mut Shell, name: &BStr) -> Result<(), Error> {
    /* `unset` is not an assignment, and Bash's `set -a` does not reach
     * one: `set -a; unset zz` leaves nothing behind there, where the
     * option's mark makes the attributes non-empty and so brings an
     * entry into being that `declare -p` then lists and the next
     * declaration of the same name inherits. dash creates that entry --
     * `set -a; unset zz; export -p` names `zz` -- so the option keeps
     * its reach in the POSIX dialect, which is why the write below
     * cannot simply stop consulting it. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    let all_export = if shell.options.dialect() == crate::options::Dialect::Bash {
        AllExport::Declines
    } else {
        AllExport::Marks
    };
    crate::error::with_interrupts_deferred(shell, |shell| {
        set_entry(
            shell,
            name,
            None,
            VariableAttributes::NONE,
            CallbackPolicy::Run,
            arrays::ReadOnlyGuard::Enforce,
            all_export,
        )
    })
}

pub(crate) fn add_attributes(
    shell: &mut Shell,
    name: &BStr,
    attributes: VariableAttributes,
) -> bool {
    let Some(var) = shell.variables.entries.get_mut(name) else {
        return false;
    };
    var.attributes.exported |= attributes.exported;
    var.attributes.read_only |= attributes.read_only;
    var.attributes.fixed |= attributes.fixed;
    true
}

pub(crate) fn variable_attributes(shell: &Shell, name: &BStr) -> Option<VariableAttributes> {
    shell.variables.entries.get(name).map(|var| var.attributes)
}

/// Build the exported environment as owned native name/value pairs.
///
/// An array has no environment spelling, so an exported name that holds
/// one contributes nothing -- not even its first element, which is what
/// Bash's `export PYTHONPATH; PYTHONPATH=(x)` shows. Only the Bash
/// dialect can produce such a value at all.
// [spec:dash:sem:var.listvars-fn]
// [spec:nsh:req:compat.bash.arrays-declarations]
pub fn environment(shell: &Shell) -> std::io::Result<Vec<(OsString, OsString)>> {
    shell
        .variables
        .entries
        .iter()
        .filter(|(_, var)| {
            var.attributes.exported
                && matches!(&var.state, VariableState::Set(value)
                    if value.kind() == value::VariableKind::Scalar)
        })
        .map(|(name, var)| {
            let value = var.scalar().unwrap_or_else(|| BStr::new(b""));
            Ok((name.try_to_os_string()?, value.try_to_os_string()?))
        })
        .collect()
}

/// List the variables a selection names, in the dialect's own form.
///
/// `export -p` and `readonly -p` print declarations in Bash -- the same
/// `declare -p` line, out of the same renderer, so an exported array is
/// `declare -ax xa=([0]="1")` there and not its first element -- and
/// `export NAME='value'` in the POSIX dialect, which is dash's form and
/// is the one `[spec:posix:req:builtin.readonly.p-output-reinput]` asks
/// to be readable back. A bare `set` prints `name=value` in both.
///
/// `kind` narrows the listing to one array kind, which is the only other
/// thing Bash does with `readonly -a` and `readonly -A`: the letters are
/// not attributes there, so with no operand they can only select.
// [spec:dash:sem:var.showvars-fn]
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn show_vars(
    shell: &mut Shell,
    prefix: &BStr,
    selection: VariableSelection,
    kind: Option<VariableKind>,
) -> Result<(), Error> {
    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let declarations = bash && selection != VariableSelection::Set;
    let locale = shell.locale.clone();
    let selected: Vec<BString> = shell
        .variables
        .entries
        .iter()
        .filter(|(_, var)| match selection {
            VariableSelection::Set => matches!(&var.state, VariableState::Set(_)),
            VariableSelection::Exported => var.attributes.exported,
            VariableSelection::ReadOnly => var.attributes.read_only,
        })
        .map(|(name, _)| name.clone())
        .collect();
    let mut records: Vec<Vec<u8>> = Vec::new();
    for name in selected {
        let name = BStr::new(name.as_slice());
        if kind.is_some() && value::variable_kind(shell, name) != kind {
            continue;
        }
        let mut record = Vec::new();
        if declarations {
            let Some(line) = declaration::declaration_line(shell, name) else {
                continue;
            };
            record.extend_from_slice(&line);
        } else {
            record.extend_from_slice(prefix);
            if !prefix.is_empty() {
                record.push(b' ');
            }
            record.extend_from_slice(name.as_ref());
            record.extend_from_slice(&listed_value(shell, name, bash, &locale));
        }
        record.push(b'\n');
        records.push(record);
    }
    for record in records {
        shell.write_output(crate::output::OutputDestination::Stdout, &record)?;
    }
    Ok(())
}

/// The `=value` a `name=value` listing writes, in the dialect's own
/// form.
///
/// The two dialects disagree twice, and both are Bash's doing. Bash
/// quotes only what would not read back as itself -- `x=1` bare against
/// dash's `x='1'` -- and it spells an array as the compound assignment
/// that would rebuild it, out of the same renderer `declare -p` uses:
/// `z=([0]="1" [1]="2")`, where reading element zero and calling it the
/// variable gave `z='1'` and gave a bare `m` for an associative array
/// with no `"0"` key. dash has no arrays and quotes unconditionally,
/// and `[spec:posix:req:builtin.readonly.p-output-reinput]` is what
/// asks it to.
// [spec:nsh:req:compat.bash.arrays-declarations]
// [spec:nsh:req:compat.bash.builtins-special-variables]
// [spec:dash:sem:var.showvars-fn]
fn listed_value(shell: &Shell, name: &BStr, bash: bool, locale: &nsh_platform::Locale) -> Vec<u8> {
    let mut text = Vec::new();
    if !bash {
        if let Some(value) = shell.variables.entries.get(name).and_then(Variable::scalar) {
            text.push(b'=');
            text.extend_from_slice(&crate::escape::shell_quote(value));
        }
        return text;
    }
    let Some(value) = value::variable_value(shell, name) else {
        return text;
    };
    let VariableValue::Scalar(scalar) = value else {
        text.extend_from_slice(&declaration::declaration_value(shell, value));
        return text;
    };
    text.push(b'=');
    text.extend_from_slice(&crate::escape::bash::listed_quote(
        locale,
        BStr::new(scalar.as_slice()),
    ));
    text
}

// [spec:dash:sem:var.mklocal-fn]
pub(crate) fn make_local_bytes(
    shell: &mut Shell,
    assignment: &BStr,
    attributes: VariableAttributes,
) -> Result<(), Error> {
    crate::error::with_interrupts_deferred(shell, |shell| {
        if assignment == b"-" {
            let saved = shell.options.state;
            shell.variables.push_local(LocalVariable::Options(saved));
            return Ok(());
        }

        let name = assignment_name(assignment).to_owned();
        if let Some(previous) = shell.variables.entries.get(&name).cloned() {
            shell.variables.push_local(LocalVariable::Saved {
                name: name.clone(),
                previous,
            });
            if assignment.contains(&b'=') {
                set_assignment_bytes(shell, assignment, attributes)?;
            }
        } else {
            let mut attributes = attributes;
            attributes.fixed = true;
            if assignment.contains(&b'=') {
                set_assignment_bytes(shell, assignment, attributes)?;
            } else {
                set_bytes(shell, BStr::new(name.as_slice()), None, attributes)?;
            }
            shell.variables.push_local(LocalVariable::Created(name));
        }
        Ok(())
    })
}

// [spec:dash:sem:var.pushlocalvars-fn]
pub fn push_local_scope(shell: &mut Shell, push: bool) -> usize {
    let top = shell.variables.locals.len();
    if push {
        crate::error::with_interrupts_deferred(shell, |shell| {
            shell.variables.locals.push(LocalVariableScopes {
                entries: Vec::new(),
            });
        });
    }
    top
}

// [spec:dash:sem:var.poplocalvars-fn]
fn pop_local_scope(shell: &mut Shell) {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let mut frame = shell
            .variables
            .locals
            .pop()
            .expect("poplocalvars runs on a pushed frame");
        while let Some(local) = frame.entries.pop() {
            match local {
                LocalVariable::Options(saved) => {
                    shell.options.state = saved;
                    if let Err(error) = options_changed(shell) {
                        shell.status = error.status();
                    }
                }
                LocalVariable::Created(name) => {
                    let callback = shell
                        .variables
                        .entries
                        .remove(&name)
                        .map_or(Callback::None, |var| var.callback);
                    if callback == Callback::Locale {
                        run_callback(shell, BStr::new(name.as_slice()), callback, None);
                    }
                }
                LocalVariable::Saved { name, previous } => {
                    let callback = previous.callback;
                    let value = previous.scalar_owned();
                    shell.variables.entries.insert(name.clone(), previous);
                    run_callback(
                        shell,
                        BStr::new(name.as_slice()),
                        callback,
                        value.as_ref().map(|value| BStr::new(value.as_slice())),
                    );
                }
            }
        }
    });
}

// [spec:dash:sem:var.unwindlocalvars-fn]
pub fn unwind_local_scopes(shell: &mut Shell, stop: usize) {
    while shell.variables.locals.len() > stop {
        pop_local_scope(shell);
    }
}

impl Shell {
    /// Read a shell variable. `$LINENO` is refreshed before the borrow is
    /// returned, which is why this method takes `&mut self`.
    pub fn var(&mut self, name: &BStr) -> Option<&BStr> {
        special::refresh(self, name);
        self.variables.entries.get(name).and_then(Variable::scalar)
    }

    /// Assign a shell variable with normal script-assignment semantics.
    pub fn set_var(&mut self, name: &BStr, value: &BStr) -> Result<(), Error> {
        set_bytes(self, name, Some(value), VariableAttributes::NONE)
    }

    /// Unset a shell variable and report whether it had a value.
    pub fn unset_var(&mut self, name: &BStr) -> Result<bool, Error> {
        let was_set = lookup_bytes(self, name).is_some();
        unset_bytes(self, name)?;
        Ok(was_set)
    }

    /// Return every set variable as owned name/value pairs in name order.
    pub fn variables(&mut self) -> Vec<(BString, BString)> {
        self.variables
            .entries
            .iter()
            .filter_map(|(name, var)| Some((name.clone(), var.scalar_owned()?)))
            .collect()
    }
}

#[cfg(test)]
mod tests;
