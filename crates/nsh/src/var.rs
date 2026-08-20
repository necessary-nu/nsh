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
use core::ffi::c_int;
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};
use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::Write as _;

use crate::context::Shell;
use crate::error::Error;
use crate::options::{OptionSet, ShellOption, options_changed};
// [spec:nsh:def:idiom.shell-options]

pub(crate) mod value;
use value::{BashAttributes, VariableValue};

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
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum CallbackPolicy {
    Run,
    Suppress,
}

// [spec:dash:def:var.var]
#[derive(Clone, Debug)]
struct Var {
    attributes: VariableAttributes,
    state: VariableState,
    /// Declaration attributes that do not already have a dash flag.
    bash_attributes: BashAttributes,
    callback: Callback,
    /// `$LINENO` is computed on read until a script assigns to it.
    dynamic_lineno: bool,
}

impl Var {
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

// [spec:dash:def:var.localvar]
enum LocalVar {
    Options(OptionSet),
    /// The declaration created this name; remove it on return.
    Created(BString),
    /// Restore the complete previous entry on return.
    Saved {
        name: BString,
        previous: Var,
    },
}

// [spec:dash:def:var.localvar-list]
pub struct LocalVarList {
    entries: Vec<LocalVar>,
}

// [spec:posix:req:xcu.env.effects-confined-to-section]
// [spec:posix:req:xcu.env.eight-bit-transparency]
// [spec:posix:req:param.byte-values]
// [spec:posix:def:param.denotation]
// [spec:posix:def:param.set-state]
// [spec:posix:sem:param.variable-creation]
pub struct VarTable {
    tab: BTreeMap<BString, Var>,
    pub(crate) lineno: c_int,
    locals: Vec<LocalVarList>,
}

impl VarTable {
    pub(crate) fn new() -> Self {
        Self {
            tab: BTreeMap::new(),
            lineno: 0,
            locals: Vec::new(),
        }
    }

    #[inline]
    pub(crate) fn in_function(&self) -> bool {
        !self.locals.is_empty()
    }

    fn push_local(&mut self, local: LocalVar) {
        self.locals
            .last_mut()
            .expect("mklocal runs inside a function")
            .entries
            .push(local);
    }

    fn refresh_lineno(&mut self, name: &BStr) {
        if name == b"LINENO" {
            if let Some(var) = self.tab.get_mut(name) {
                if var.dynamic_lineno && matches!(&var.state, VariableState::Set(_)) {
                    var.state = VariableState::Set(VariableValue::Scalar(BString::from(
                        self.lineno.to_string().as_bytes(),
                    )));
                }
            }
        }
    }
}

pub fn defifs() -> &'static BStr {
    BStr::new(DEFAULT_IFS)
}

pub fn defpath() -> BString {
    BString::from(nsh_platform::default_search_path().to_shell_bytes())
}

fn builtin_value(sh: &Shell, name: &[u8]) -> BString {
    sh.vars
        .tab
        .get(BStr::new(name))
        .and_then(Var::scalar_owned)
        .unwrap_or_default()
}

pub fn ifsval(sh: &Shell) -> BString {
    builtin_value(sh, b"IFS")
}

pub fn ifsset(sh: &Shell) -> bool {
    sh.vars
        .tab
        .get(BStr::new(b"IFS"))
        .is_some_and(|var| matches!(&var.state, VariableState::Set(_)))
}

pub fn mailval(sh: &Shell) -> BString {
    builtin_value(sh, b"MAIL")
}

pub fn mpathval(sh: &Shell) -> BString {
    builtin_value(sh, b"MAILPATH")
}

pub fn pathval(sh: &Shell) -> BString {
    builtin_value(sh, b"PATH")
}

pub fn ps1val(sh: &Shell) -> BString {
    builtin_value(sh, b"PS1")
}

pub fn ps2val(sh: &Shell) -> BString {
    builtin_value(sh, b"PS2")
}

pub fn ps4val(sh: &Shell) -> BString {
    builtin_value(sh, b"PS4")
}

pub fn histsizeval(sh: &Shell) -> BString {
    builtin_value(sh, b"HISTSIZE")
}

pub fn mpathset(sh: &Shell) -> bool {
    sh.vars
        .tab
        .get(BStr::new(b"MAILPATH"))
        .is_some_and(|var| matches!(&var.state, VariableState::Set(_)))
}

/// The name portion of `name` or `name=value`.
pub(crate) fn varname(text: &BStr) -> &BStr {
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
fn selected_locale(sh: &Shell) -> std::io::Result<nsh_platform::Locale> {
    let nonempty = |name: &[u8]| {
        sh.vars.tab.get(BStr::new(name)).and_then(|var| {
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

fn builtin(name: &[u8], value: Option<&[u8]>, callback: Callback) -> (BString, Var) {
    let var = match value {
        Some(value) => Var::set(value, VariableAttributes::FIXED, callback),
        None => Var::unset(VariableAttributes::FIXED, callback),
    };
    (BString::from(name), var)
}

// [spec:dash:def:var.initvar-fn]
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
pub fn initvar(sh: &mut Shell) {
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
        sh.vars.tab.entry(name).or_insert(var);
    }
}

/// Where a new shell's exported variables come from.
pub(crate) enum EnvSource<'a> {
    Process,
    Explicit(&'a [(BString, BString)]),
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
    pub(crate) fn initialize_variable_state(&mut self, env: EnvSource<'_>) -> Result<(), Error> {
        initvar(self);
        let process_env;
        let pairs = match env {
            EnvSource::Process => {
                process_env = nsh_platform::process_environment()
                    .into_iter()
                    .map(|(name, value)| {
                        (
                            BString::from(name.to_shell_bytes()),
                            BString::from(value.to_shell_bytes()),
                        )
                    })
                    .collect::<Vec<_>>();
                process_env.as_slice()
            }
            EnvSource::Explicit(pairs) => pairs,
        };

        for (name, value) in pairs {
            let name = BStr::new(name.as_slice());
            if !is_locale_variable!(name) {
                continue;
            }
            let entry = self.vars.tab.entry(name.to_owned()).or_insert_with(|| Var {
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
            Some(defifs()),
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
            Some(path) => crate::cd::setpwd_inner(self, crate::cd::Pwd::New(BStr::new(path)), 0),
            None => crate::cd::setpwd_inner(self, crate::cd::Pwd::Unknown, 0),
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
        while !self.vars.locals.is_empty() {
            poplocalvars(self);
        }
    }
}

// [spec:nsh:sem:shell-locale.invalid-selection]
fn run_callback(sh: &mut Shell, callback: Callback, name: &BStr, value: Option<&BStr>) {
    let effective = value.unwrap_or_else(|| BStr::new(b""));
    match callback {
        Callback::None => {}
        Callback::Ifs => {
            let effective_ifs = if ifsset(sh) { effective } else { defifs() };
            crate::expand::changeifs_bytes(sh, effective_ifs);
        }
        Callback::Mail => crate::mail::changemail(sh, effective),
        Callback::Path => crate::exec::changepath(sh, effective),
        Callback::Getopts => crate::options::getoptsreset(sh, effective),
        Callback::History => crate::histedit::sethistsize(sh, effective),
        Callback::Locale => {
            // Locale selection depends on the complete variable table, not
            // merely on the entry that triggered this callback.
            if let Ok(locale) = selected_locale(sh) {
                sh.locale = locale;
                let ifs = if ifsset(sh) {
                    ifsval(sh)
                } else {
                    defifs().to_owned()
                };
                crate::expand::changeifs_bytes(sh, BStr::new(ifs.as_slice()));
            }
        }
    }
}

fn set_entry(
    sh: &mut Shell,
    name: &BStr,
    value: Option<&BStr>,
    mut attributes: VariableAttributes,
    callback_policy: CallbackPolicy,
) -> Result<(), Error> {
    if !valid_name(&sh.locale, name) {
        let mut message = name.to_vec();
        message.extend_from_slice(b": bad variable name");
        return Err(sh.sh_error_value(&message));
    }
    if sh.options.enabled(ShellOption::AllExport) {
        attributes.exported = true;
    }

    let existing = sh.vars.tab.get(name).cloned();
    let Some(old) = existing else {
        if value.is_none() && attributes == VariableAttributes::NONE {
            return Ok(());
        }
        let callback = if is_locale_variable!(name) {
            Callback::Locale
        } else {
            Callback::None
        };
        sh.vars.tab.insert(
            name.to_owned(),
            Var {
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
            run_callback(sh, callback, name, value);
        }
        return Ok(());
    };

    if old.attributes.read_only {
        let mut message = name.to_vec();
        message.extend_from_slice(b": is read only");
        return Err(sh.sh_error_value(&message));
    }

    if value.is_some() || attributes != VariableAttributes::NONE {
        attributes.exported |= old.attributes.exported;
        attributes.read_only |= old.attributes.read_only;
        attributes.fixed |= old.attributes.fixed;
    } else if old.attributes.fixed {
        attributes = VariableAttributes::FIXED;
    } else {
        sh.vars.tab.remove(name);
        if old.callback == Callback::Locale {
            run_callback(sh, old.callback, name, None);
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
    sh.vars.tab.insert(
        name.to_owned(),
        Var {
            attributes,
            state,
            bash_attributes,
            callback,
            dynamic_lineno: false,
        },
    );
    if callback_policy == CallbackPolicy::Run {
        run_callback(
            sh,
            callback,
            name,
            callback_value
                .as_ref()
                .map(|value| BStr::new(value.as_slice())),
        );
    }
    Ok(())
}

/// Read a variable through the owned-byte interface used throughout nsh.
// [spec:dash:def:var.lookupvar-fn]
// [spec:dash:sem:var.lookupvar-fn]
pub(crate) fn lookup_bytes(sh: &mut Shell, name: &BStr) -> Option<BString> {
    sh.vars.refresh_lineno(name);
    sh.vars.tab.get(name).and_then(Var::scalar_owned)
}

// [spec:dash:def:var.setvar-fn]
// [spec:dash:sem:var.setvar-fn]
pub(crate) fn set_bytes(
    sh: &mut Shell,
    name: &BStr,
    value: Option<&BStr>,
    attributes: VariableAttributes,
) -> Result<(), Error> {
    crate::error::with_interrupts_deferred(sh, |sh| {
        set_entry(sh, name, value, attributes, CallbackPolicy::Run)
    })
}

// [spec:dash:def:var.setvareq-fn]
// [spec:dash:sem:var.setvareq-fn]
pub(crate) fn set_assignment_bytes(
    sh: &mut Shell,
    assignment: &BStr,
    attributes: VariableAttributes,
) -> Result<(), Error> {
    match assignment.split_once_str(b"=") {
        Some((name, value)) => set_bytes(sh, BStr::new(name), Some(BStr::new(value)), attributes),
        None => set_bytes(sh, assignment, None, attributes),
    }
}

pub(crate) fn setvarint_bytes(
    sh: &mut Shell,
    name: &BStr,
    value: i64,
    attributes: VariableAttributes,
    callback_policy: CallbackPolicy,
) -> Result<i64, Error> {
    let text = value.to_string();
    crate::error::with_interrupts_deferred(sh, |sh| {
        set_entry(
            sh,
            name,
            Some(BStr::new(text.as_bytes())),
            attributes,
            callback_policy,
        )
    })?;
    Ok(value)
}

// [spec:dash:def:var.lookupvarint-fn]
// [spec:dash:sem:var.lookupvarint-fn]
// [spec:posix:req:builtin.set.opt-u-nounset]
pub(crate) fn lookupvarint_bytes(sh: &mut Shell, name: &BStr) -> Result<i64, Error> {
    let value = match lookup_bytes(sh, name) {
        Some(value) => value,
        None if sh.options.enabled(ShellOption::Nounset) => {
            let mut message = name.to_vec();
            message.extend_from_slice(b": parameter not set");
            return Err(sh.sh_error_value(&message));
        }
        None => BString::default(),
    };
    crate::mystring::parse_integer(sh, BStr::new(&value), 0)
}

pub(crate) fn unset_bytes(sh: &mut Shell, name: &BStr) -> Result<(), Error> {
    set_bytes(sh, name, None, VariableAttributes::NONE)
}

pub(crate) fn add_attributes(sh: &mut Shell, name: &BStr, attributes: VariableAttributes) -> bool {
    let Some(var) = sh.vars.tab.get_mut(name) else {
        return false;
    };
    var.attributes.exported |= attributes.exported;
    var.attributes.read_only |= attributes.read_only;
    var.attributes.fixed |= attributes.fixed;
    true
}

pub(crate) fn variable_attributes(sh: &Shell, name: &BStr) -> Option<VariableAttributes> {
    sh.vars.tab.get(name).map(|var| var.attributes)
}

/// Build the exported environment as owned native name/value pairs.
pub fn environment(sh: &Shell) -> std::io::Result<Vec<(OsString, OsString)>> {
    sh.vars
        .tab
        .iter()
        .filter(|(_, var)| var.attributes.exported && matches!(&var.state, VariableState::Set(_)))
        .map(|(name, var)| {
            let value = var.scalar().unwrap_or_else(|| BStr::new(b""));
            Ok((name.try_to_os_string()?, value.try_to_os_string()?))
        })
        .collect()
}

// [spec:dash:def:var.showvars-fn]
// [spec:dash:sem:var.showvars-fn]
pub(crate) fn show_vars(sh: &mut Shell, prefix: &BStr, selection: VariableSelection) {
    let records: Vec<Vec<u8>> = sh
        .vars
        .tab
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
                record.extend_from_slice(&crate::mystring::single_quote(value));
            }
            record.push(b'\n');
            record
        })
        .collect();
    for record in records {
        let _ = sh.io.stdout().write_all(&record);
    }
}

// [spec:dash:def:var.mklocal-fn]
// [spec:dash:sem:var.mklocal-fn]
pub(crate) fn make_local_bytes(
    sh: &mut Shell,
    assignment: &BStr,
    attributes: VariableAttributes,
) -> Result<(), Error> {
    crate::error::with_interrupts_deferred(sh, |sh| {
        if assignment == b"-" {
            let saved = sh.options.state;
            sh.vars.push_local(LocalVar::Options(saved));
            return Ok(());
        }

        let name = varname(assignment).to_owned();
        if let Some(previous) = sh.vars.tab.get(&name).cloned() {
            sh.vars.push_local(LocalVar::Saved {
                name: name.clone(),
                previous,
            });
            if assignment.contains(&b'=') {
                set_assignment_bytes(sh, assignment, attributes)?;
            }
        } else {
            let mut attributes = attributes;
            attributes.fixed = true;
            if assignment.contains(&b'=') {
                set_assignment_bytes(sh, assignment, attributes)?;
            } else {
                set_bytes(sh, BStr::new(name.as_slice()), None, attributes)?;
            }
            sh.vars.push_local(LocalVar::Created(name));
        }
        Ok(())
    })
}

// [spec:dash:def:var.pushlocalvars-fn]
// [spec:dash:sem:var.pushlocalvars-fn]
pub fn pushlocalvars(sh: &mut Shell, push: c_int) -> usize {
    let top = sh.vars.locals.len();
    if push != 0 {
        crate::error::with_interrupts_deferred(sh, |sh| {
            sh.vars.locals.push(LocalVarList {
                entries: Vec::new(),
            });
        });
    }
    top
}

fn poplocalvars(sh: &mut Shell) {
    crate::error::with_interrupts_deferred(sh, |sh| {
        let mut frame = sh
            .vars
            .locals
            .pop()
            .expect("poplocalvars runs on a pushed frame");
        while let Some(local) = frame.entries.pop() {
            match local {
                LocalVar::Options(saved) => {
                    sh.options.state = saved;
                    if let Err(error) = options_changed(sh) {
                        sh.status = error.status();
                    }
                }
                LocalVar::Created(name) => {
                    let callback = sh
                        .vars
                        .tab
                        .remove(&name)
                        .map_or(Callback::None, |var| var.callback);
                    if callback == Callback::Locale {
                        run_callback(sh, callback, BStr::new(name.as_slice()), None);
                    }
                }
                LocalVar::Saved { name, previous } => {
                    let callback = previous.callback;
                    let value = previous.scalar_owned();
                    sh.vars.tab.insert(name.clone(), previous);
                    run_callback(
                        sh,
                        callback,
                        BStr::new(name.as_slice()),
                        value.as_ref().map(|value| BStr::new(value.as_slice())),
                    );
                }
            }
        }
    });
}

// [spec:dash:def:var.unwindlocalvars-fn]
// [spec:dash:sem:var.unwindlocalvars-fn]
pub fn unwindlocalvars(sh: &mut Shell, stop: usize) {
    while sh.vars.locals.len() > stop {
        poplocalvars(sh);
    }
}

impl Shell {
    /// Read a shell variable. `$LINENO` is refreshed before the borrow is
    /// returned, which is why this method takes `&mut self`.
    pub fn var(&mut self, name: &BStr) -> Option<&BStr> {
        self.vars.refresh_lineno(name);
        self.vars.tab.get(name).and_then(Var::scalar)
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
    pub fn vars(&mut self) -> Vec<(BString, BString)> {
        self.vars
            .tab
            .iter()
            .filter_map(|(name, var)| Some((name.clone(), var.scalar_owned()?)))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::lock;

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
        initvar(&mut shell);
        shell.vars.lineno = 41;
        assert_eq!(
            lookup_bytes(&mut shell, BStr::new(b"LINENO"))
                .as_ref()
                .map(|value| value.as_slice()),
            Some(b"41".as_slice())
        );
        let mut moved = shell;
        moved.vars.lineno = 42;
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

        initvar(&mut shell);
        shell.options.shellparam.optind = 7;
        shell.options.shellparam.optoff = 3;

        setvarint_bytes(
            &mut shell,
            BStr::new(b"OPTIND"),
            8,
            VariableAttributes::NONE,
            CallbackPolicy::Suppress,
        )
        .unwrap();
        assert_eq!(shell.options.shellparam.optind, 7);
        assert_eq!(shell.options.shellparam.optoff, 3);

        set_bytes(
            &mut shell,
            BStr::new(b"OPTIND"),
            Some(BStr::new(b"1")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(shell.options.shellparam.optind, 1);
        assert_eq!(shell.options.shellparam.optoff, -1);
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
        let stop = pushlocalvars(&mut shell, 1);
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
        unwindlocalvars(&mut shell, stop);
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
