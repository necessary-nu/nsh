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
use value::{BashAttributes, VariableValue};

pub(crate) mod arrays;

pub(crate) mod nameref;

pub(crate) mod call_stack;

pub(crate) mod special;

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

/// Whether a variable is unset or owns a value of a particular kind.
#[derive(Clone, Debug, Default, Eq, PartialEq)]
enum VariableState {
    #[default]
    Unset,
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

pub fn default_ifs() -> &'static BStr {
    BStr::new(DEFAULT_IFS)
}

pub fn default_path() -> BString {
    BString::from(nsh_platform::default_search_path().to_shell_bytes())
}

fn builtin_value(shell: &Shell, name: &[u8]) -> BString {
    shell
        .variables
        .entries
        .get(BStr::new(name))
        .and_then(Variable::scalar_owned)
        .unwrap_or_default()
}

pub fn ifs_value(shell: &Shell) -> BString {
    builtin_value(shell, b"IFS")
}

pub fn ifs_is_set(shell: &Shell) -> bool {
    shell
        .variables
        .entries
        .get(BStr::new(b"IFS"))
        .is_some_and(|var| matches!(&var.state, VariableState::Set(_)))
}

pub fn mail_value(shell: &Shell) -> BString {
    builtin_value(shell, b"MAIL")
}

pub fn mail_path_value(shell: &Shell) -> BString {
    builtin_value(shell, b"MAILPATH")
}

pub fn path_value(shell: &Shell) -> BString {
    builtin_value(shell, b"PATH")
}

pub fn primary_prompt_value(shell: &Shell) -> BString {
    builtin_value(shell, b"PS1")
}

pub fn continuation_prompt_value(shell: &Shell) -> BString {
    builtin_value(shell, b"PS2")
}

pub fn trace_prompt_value(shell: &Shell) -> BString {
    builtin_value(shell, b"PS4")
}

pub fn history_size_value(shell: &Shell) -> BString {
    builtin_value(shell, b"HISTSIZE")
}

pub fn mail_path_is_set(shell: &Shell) -> bool {
    shell
        .variables
        .entries
        .get(BStr::new(b"MAILPATH"))
        .is_some_and(|var| matches!(&var.state, VariableState::Set(_)))
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
    let prompt: &[u8] = if nsh_platform::effective_uid().is_root() {
        b"# "
    } else {
        b"$ "
    };
    let default_path = nsh_platform::default_search_path().to_shell_bytes();
    let mut entries = [
        builtin(b"IFS", Some(DEFAULT_IFS), Callback::Ifs),
        builtin(b"MAIL", None, Callback::Mail),
        builtin(b"MAILPATH", None, Callback::Mail),
        builtin(b"PATH", Some(&default_path), Callback::Path),
        builtin(b"PS1", Some(prompt), Callback::None),
        builtin(b"PS2", Some(b"> "), Callback::None),
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
) -> Result<(), Error> {
    if !valid_name(&shell.locale, name) {
        let mut message = name.to_vec();
        message.extend_from_slice(b": bad variable name");
        return Err(shell.diagnostics().shell_error(&message));
    }
    if shell.options.enabled(ShellOption::AllExport) {
        attributes.exported = true;
    }

    let existing = shell.variables.entries.get(name).cloned();
    let Some(old) = existing else {
        if value.is_none() && attributes == VariableAttributes::NONE {
            return Ok(());
        }
        let callback = if is_locale_variable!(name) {
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
                callback,
                dynamic_lineno: false,
            },
        );
        if callback_policy == CallbackPolicy::Run {
            run_callback(shell, name, callback, value);
        }
        return Ok(());
    };

    if old.attributes.read_only && guard == arrays::ReadOnlyGuard::Enforce {
        let mut message = name.to_vec();
        message.extend_from_slice(b": is read only");
        // [spec:nsh:req:compat.bash.error-boundary]
        return Err(shell.diagnostics().dialect_error(&message));
    }

    if value.is_some() || attributes != VariableAttributes::NONE {
        attributes.exported |= old.attributes.exported;
        attributes.read_only |= old.attributes.read_only;
        attributes.fixed |= old.attributes.fixed;
    } else if old.attributes.fixed {
        attributes = VariableAttributes::FIXED;
    } else {
        shell.variables.entries.remove(name);
        if old.callback == Callback::Locale {
            run_callback(shell, name, old.callback, None);
        }
        return Ok(());
    }

    let callback = old.callback;
    let bash_attributes = old.bash_attributes;
    let mut state = old.state;
    match (&mut state, value) {
        (VariableState::Set(existing), Some(value)) => existing.assign_scalar(value),
        (state @ VariableState::Unset, Some(value)) => {
            *state = VariableState::Set(VariableValue::Scalar(value.to_owned()));
        }
        (state, None) => *state = VariableState::Unset,
    }
    let callback_value = match &state {
        VariableState::Unset => None,
        VariableState::Set(value) => value.scalar_owned(),
    };
    shell.variables.entries.insert(
        name.to_owned(),
        Variable {
            attributes,
            state,
            bash_attributes,
            callback,
            dynamic_lineno: false,
        },
    );
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
    set_bytes(shell, name, None, VariableAttributes::NONE)
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
// [spec:dash:sem:var.listvars-fn]
pub fn environment(shell: &Shell) -> std::io::Result<Vec<(OsString, OsString)>> {
    shell
        .variables
        .entries
        .iter()
        .filter(|(_, var)| var.attributes.exported && matches!(&var.state, VariableState::Set(_)))
        .map(|(name, var)| {
            let value = var.scalar().unwrap_or_else(|| BStr::new(b""));
            Ok((name.try_to_os_string()?, value.try_to_os_string()?))
        })
        .collect()
}

// [spec:dash:sem:var.showvars-fn]
pub(crate) fn show_vars(
    shell: &mut Shell,
    prefix: &BStr,
    selection: VariableSelection,
) -> Result<(), Error> {
    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let locale = shell.locale.clone();
    let records: Vec<Vec<u8>> = shell
        .variables
        .entries
        .iter()
        .filter(|(_, var)| match selection {
            VariableSelection::Set => matches!(&var.state, VariableState::Set(_)),
            VariableSelection::Exported => var.attributes.exported,
            VariableSelection::ReadOnly => var.attributes.read_only,
        })
        .map(|(name, var)| {
            let mut record = Vec::new();
            record.extend_from_slice(prefix);
            if !prefix.is_empty() {
                record.push(b' ');
            }
            record.extend_from_slice(name);
            if let Some(value) = var.scalar() {
                record.push(b'=');
                // Bash's listing reaches for `$'...'` where single quotes
                // could not carry the bytes; POSIX's never does.
                // [spec:nsh:req:compat.bash.builtins-special-variables]
                let quoted = if bash {
                    crate::escape::bash::readable_quote(&locale, value)
                } else {
                    crate::escape::shell_quote(value)
                };
                record.extend_from_slice(&quoted);
            }
            record.push(b'\n');
            record
        })
        .collect();
    for record in records {
        shell.write_output(crate::output::OutputDestination::Stdout, &record)?;
    }
    Ok(())
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
mod tests {
    use super::*;
    use crate::test_support::lock;

    // [spec:nsh:sem:shell-locale.selection/test]
    #[test]
    fn an_empty_locale_assignment_is_not_an_unset() {
        let _guard = lock();
        let mut shell = Shell::builder().env([("LC_ALL", "C")]).build().unwrap();
        set_bytes(
            &mut shell,
            BStr::new(b"LC_ALL"),
            Some(BStr::new(b"")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(
            lookup_bytes(&mut shell, BStr::new(b"LC_ALL"))
                .as_ref()
                .map(|value| value.as_slice()),
            Some(&b""[..])
        );
        assert!(
            environment(&shell)
                .unwrap()
                .iter()
                .any(|(name, value)| name.to_shell_bytes() == b"LC_ALL" && value.is_empty())
        );

        unset_bytes(&mut shell, BStr::new(b"LC_ALL")).unwrap();
        assert_eq!(lookup_bytes(&mut shell, BStr::new(b"LC_ALL")), None);
        assert!(
            environment(&shell)
                .unwrap()
                .iter()
                .all(|(name, _)| name.to_shell_bytes() != b"LC_ALL")
        );
    }

    // [spec:dash:sem:var.lookupvar-fn/test]
    #[test]
    fn lineno_survives_a_shell_move() {
        let _guard = lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        initialize_variables(&mut shell);
        shell.variables.line_number = 41;
        assert_eq!(
            lookup_bytes(&mut shell, BStr::new(b"LINENO"))
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"41".as_slice())
        );
        let mut moved = shell;
        moved.variables.line_number = 42;
        assert_eq!(
            lookup_bytes(&mut moved, BStr::new(b"LINENO"))
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"42".as_slice())
        );
    }

    // [spec:dash:sem:var.setvar-fn/test]
    // [spec:posix:req:builtin.getopts.env-optind/test]
    #[test]
    fn set_and_unset_variable() {
        let _guard = lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        set_bytes(
            &mut shell,
            BStr::new(b"Tsetvar"),
            Some(BStr::new(b"hello")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(
            lookup_bytes(&mut shell, BStr::new(b"Tsetvar"))
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"hello".as_slice())
        );
        unset_bytes(&mut shell, BStr::new(b"Tsetvar")).unwrap();
        assert_eq!(lookup_bytes(&mut shell, BStr::new(b"Tsetvar")), None);

        initialize_variables(&mut shell);
        shell.options.positional_parameters.option_index = 7;
        shell.options.positional_parameters.option_offset = Some(3);

        set_integer_bytes(
            &mut shell,
            BStr::new(b"OPTIND"),
            8,
            VariableAttributes::NONE,
            CallbackPolicy::Suppress,
        )
        .unwrap();
        assert_eq!(shell.options.positional_parameters.option_index, 7);
        assert_eq!(shell.options.positional_parameters.option_offset, Some(3));

        set_bytes(
            &mut shell,
            BStr::new(b"OPTIND"),
            Some(BStr::new(b"1")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(shell.options.positional_parameters.option_index, 1);
        assert_eq!(shell.options.positional_parameters.option_offset, None);
        assert_eq!(
            variable_attributes(&shell, BStr::new(b"OPTIND")),
            Some(VariableAttributes::FIXED),
        );
    }

    // [spec:dash:sem:var.poplocalvars-fn/test]
    #[test]
    fn a_frame_restores_in_reverse_order() {
        let _guard = lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        set_bytes(
            &mut shell,
            BStr::new(b"Tframe"),
            Some(BStr::new(b"one")),
            VariableAttributes::NONE,
        )
        .unwrap();
        let stop = push_local_scope(&mut shell, true);
        make_local_bytes(
            &mut shell,
            BStr::new(b"Tframe=two"),
            VariableAttributes::NONE,
        )
        .unwrap();
        make_local_bytes(
            &mut shell,
            BStr::new(b"Tframe=three"),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(
            lookup_bytes(&mut shell, BStr::new(b"Tframe"))
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"three".as_slice())
        );
        unwind_local_scopes(&mut shell, stop);
        assert_eq!(
            lookup_bytes(&mut shell, BStr::new(b"Tframe"))
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"one".as_slice())
        );
    }

    #[test]
    fn environment_is_owned_and_sorted() {
        let _guard = lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        set_bytes(
            &mut shell,
            BStr::new(b"ZED"),
            Some(BStr::new(b"z")),
            VariableAttributes::EXPORTED,
        )
        .unwrap();
        set_bytes(
            &mut shell,
            BStr::new(b"ALPHA"),
            Some(BStr::new(b"a")),
            VariableAttributes::EXPORTED,
        )
        .unwrap();
        let environment: Vec<(Vec<u8>, Vec<u8>)> = environment(&shell)
            .unwrap()
            .iter()
            .map(|(name, value)| (name.to_shell_bytes(), value.to_shell_bytes()))
            .collect();
        assert_eq!(
            environment,
            [
                (b"ALPHA".to_vec(), b"a".to_vec()),
                (b"ZED".to_vec(), b"z".to_vec()),
            ]
        );
    }
}
