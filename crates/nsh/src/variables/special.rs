//! The variables the shell maintains on a script's behalf.
//!
//! The oldest of them are POSIX's, and the shell installs them before it
//! reads a line. [`super::initialize_variables`] enters `IFS`, `PATH`,
//! `MAIL`, `MAILPATH`, `PS1`, `PS2`, `PS4` and `HISTSIZE` as `FIXED`
//! entries carrying a [`Callback`], and the rest of the shell reads them
//! back through the one-line accessor named for each -- `ifs_value`,
//! `path_value`, `primary_prompt_value` and the others, over the single
//! lookup in `builtin_value`. Naming a reader for the variable rather
//! than for its caller is what makes the set of names the shell itself
//! depends on a list one can read.
//!
//! `ifs_is_set` and `mail_path_is_set` are separate from the readers
//! because for those two names unset and empty mean different things: an
//! unset `IFS` splits on the default and an empty one does not split at
//! all, and an empty `MAILPATH` still wins over `MAIL`. A caller given
//! only the value could not tell the cases apart. `default_ifs` and
//! `default_path` are what startup installs when nothing else supplies a
//! value, and they sit beside the readers so that what is written at
//! startup and what is read afterwards cannot drift.
//!
//! Three more kinds arrive with the Bash dialect, and they are not the
//! same thing as each other. *Facts* -- `BASH`, `BASH_VERSION`,
//! `OSTYPE`, `UID` -- are published once when the dialect turns on and
//! then behave like any other variable. *Clocks and
//! generators* -- `RANDOM`, `SRANDOM`, `SECONDS`, `EPOCHSECONDS`,
//! `EPOCHREALTIME`, `LINENO`, `BASHPID`, `BASH_SUBSHELL` -- have no
//! stored value worth trusting, so they are recomputed on the read that
//! asks for them. And `PIPESTATUS` and `DIRSTACK` are *published by
//! whatever moves them*, the way `call_stack` publishes `FUNCNAME`:
//! ordinary indexed arrays that every existing reader already
//! understands.
//!
//! Recomputation is driven from the read path rather than from a timer
//! because a shell has no timer: [`refresh`] runs on the same lookups
//! `$LINENO` already used, and the [`Callback::Special`] mark on the
//! entry is what says a name is one of these at all. A name that has
//! been unset loses the mark with the entry, which is how a script takes
//! `RANDOM` back for its own use.
//!
//! `RANDOM` deliberately diverges from Bash. Bash seeds it from an
//! assignment and will replay a sequence for anyone who knows the seed;
//! `[dec:nsh:safety-trumps-compatibility]` says not to import that, so
//! an assignment here re-seeds from the host's randomness and the
//! sequence is never reproducible. `SRANDOM` draws from the same place
//! on every read, which is what Bash's own already documents.
//! `docs/divergences.md` records both.

use bstr::{BStr, BString};
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};

use super::value::{VariableKind, VariableValue};
use super::{Callback, DEFAULT_IFS, Variable, VariableAttributes, VariableState, arrays};
use crate::context::Shell;
use crate::options::{Dialect, ShellOption};
use crate::status::ExitStatus;

/// What `$BASH_VERSION` answers, and the fields `$BASH_VERSINFO` splits
/// it into. The two must agree, so they are one table.
///
/// The value is the pinned Bash Reference Profile's own version, because
/// that is the contract this dialect implements: a script that narrows
/// itself on `$BASH_VERSION` must be told the release whose behaviour it
/// is about to observe. See `tests/surveys/oils/BASH_REFERENCE.toml`.
// [spec:nsh:req:compat.bash.reference-profile]
const VERSION_FIELDS: [&str; 5] = ["5", "3", "15", "1", "release"];

/// Facts about the host that an inherited environment may already
/// answer, and which the shell therefore must not overwrite.
const INHERITABLE: [&[u8]; 4] = [b"HOSTNAME", b"HOSTTYPE", b"MACHTYPE", b"OSTYPE"];

/// The highest value `$RANDOM` produces, as Bash documents it.
const RANDOM_MODULUS: u64 = 32_768;

/// The per-shell state behind the generators and clocks.
///
/// `published` is not a cache: publishing writes into the variable table,
/// which a script may then overwrite, so re-publishing on every option
/// change would silently undo a script's own assignment.
pub(crate) struct SpecialState {
    published: bool,
    random: u64,
    /// Monotonic seconds when `SECONDS` last had its origin set.
    seconds_origin: f64,
    /// What `SECONDS` was assigned at that origin.
    seconds_base: i64,
}

impl SpecialState {
    pub(crate) const fn new() -> Self {
        Self {
            published: false,
            random: 0,
            seconds_origin: 0.0,
            seconds_base: 0,
        }
    }

    /// One step of SplitMix64, which is enough of a generator for a
    /// value a script may not predict and small enough to read.
    fn next_random(&mut self) -> u64 {
        self.random = self.random.wrapping_add(0x9E37_79B9_7F4A_7C15);
        let mut z = self.random;
        z = (z ^ (z >> 30)).wrapping_mul(0xBF58_476D_1CE4_E5B9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94D0_49BB_1331_11EB);
        (z ^ (z >> 31)) % RANDOM_MODULUS
    }

    fn reseed(&mut self) {
        self.random = nsh_platform::facts::entropy_seed();
    }
}

/// Publish the Bash-only variables once the dialect selects them.
///
/// Called from the completed-option-change path, so a `set -o bash`
/// midway through a script publishes exactly what starting in Bash mode
/// would have.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn dialect_changed(shell: &mut Shell) {
    if shell.options.dialect() != Dialect::Bash || shell.variables.special.published {
        return;
    }
    shell.variables.special.published = true;
    shell.variables.special.reseed();
    shell.variables.special.seconds_origin = nsh_platform::facts::monotonic_seconds();
    publish(shell);
}

/// The release string `$BASH_VERSION` carries, assembled from the same
/// fields `$BASH_VERSINFO` publishes so the two can never disagree.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn version_text() -> String {
    let mut version = String::new();
    for (position, field) in VERSION_FIELDS.iter().take(3).enumerate() {
        if position > 0 {
            version.push('.');
        }
        version.push_str(field);
    }
    version.push('(');
    version.push_str(VERSION_FIELDS[3]);
    version.push_str(")-");
    version.push_str(VERSION_FIELDS[4]);
    version
}

fn publish(shell: &mut Shell) {
    let version = version_text();
    let machine = nsh_platform::facts::machine_type();
    let host = nsh_platform::host_name()
        .map(|name| BString::from(name.to_shell_bytes()))
        .unwrap_or_default();
    let scalars: [(&[u8], BString); 9] = [
        (b"BASH", shell_path(shell)),
        (b"BASH_VERSION", BString::from(version.as_str())),
        (b"HOSTNAME", host),
        (
            b"HOSTTYPE",
            BString::from(nsh_platform::facts::hardware_type()),
        ),
        (b"MACHTYPE", BString::from(machine)),
        (
            b"OSTYPE",
            BString::from(nsh_platform::facts::operating_system_type()),
        ),
        (
            b"UID",
            BString::from(nsh_platform::real_uid().as_raw().to_string()),
        ),
        (
            b"EUID",
            BString::from(nsh_platform::effective_uid().as_raw().to_string()),
        ),
        (b"SHLVL", BString::from(shell_level(shell).to_string())),
    ];
    for (name, value) in scalars {
        let name = BStr::new(name);
        /* A host fact the environment already carries belongs to
         * whoever set it: `HOSTNAME=x sh -c 'echo $HOSTNAME'` must
         * print `x`. The shell's own facts -- the version, the
         * identities, the nesting depth -- are not negotiable that way
         * and are written whatever arrived. */
        if INHERITABLE.contains(&(name.as_ref() as &[u8]))
            && super::lookup_bytes(shell, name).is_some()
        {
            continue;
        }
        let exported = name == "SHLVL";
        drop(super::set_bytes(
            shell,
            name,
            Some(BStr::new(value.as_slice())),
            if exported {
                VariableAttributes::EXPORTED
            } else {
                VariableAttributes::NONE
            },
        ));
    }

    let mut versinfo: Vec<BString> = VERSION_FIELDS.iter().map(|f| BString::from(*f)).collect();
    versinfo.push(BString::from(machine));
    store_array(shell, BStr::new(b"BASH_VERSINFO"), &versinfo);

    let groups: Vec<BString> = nsh_platform::supplementary_groups()
        .unwrap_or_default()
        .into_iter()
        .map(|group| BString::from(group.as_raw().to_string()))
        .collect();
    store_array(shell, BStr::new(b"GROUPS"), &groups);

    /* Read before the marks below overwrite the entry: what arrived in
     * the environment is a request, and the mark is what makes the name
     * answer for the option table from here on. */
    import_shell_options(shell);

    for name in [
        b"RANDOM".as_slice(),
        b"SRANDOM",
        b"SECONDS",
        b"EPOCHSECONDS",
        b"EPOCHREALTIME",
        b"BASHPID",
        b"BASH_SUBSHELL",
    ] {
        mark_dynamic(shell, BStr::new(name), VariableAttributes::NONE);
    }
    /* Bash marks the two option listings read-only, and so does this.
     * The mark was withheld until 2026-08-23 because an assignment to a
     * read-only name ended a non-interactive shell here, so importing it
     * would have turned `SHELLOPTS=x` -- a line Bash tolerates with
     * status 1 -- into an aborted script. Bash mode now takes Bash's
     * boundary, so the mark says what Bash's says: the assignment is
     * refused, the shell answers 1 and reads on. */
    // [spec:nsh:req:compat.bash.error-boundary]
    for name in [b"SHELLOPTS".as_slice(), b"BASHOPTS"] {
        mark_dynamic(shell, BStr::new(name), VariableAttributes::READ_ONLY);
    }
    mark_published_facts(shell);
    publish_directory_stack(shell);
}

/// Give the published facts the attributes Bash publishes them with.
///
/// The attributes are observable well beyond a listing. A read-only
/// `UID` is what makes `UID=0` fail, which a script testing
/// `[ "$UID" = 0 ]` after something tried to set it is relying on; an
/// integer `OPTIND` is what makes `OPTIND=abc` zero rather than `abc`
/// and `OPTIND+=1` arithmetic rather than concatenation.
///
/// The names are the answer to a `declare -p` diff of the two shells'
/// start-up sets rather than a list transcribed by hand, which is what
/// the node asked for and what turned its six names into eight:
/// `BASHPID`, `OPTIND`, `RANDOM` and `SRANDOM` carry `-i` in the
/// reference and appear in no read-only listing, so nothing that read
/// `readonly -p` would have found them.
///
/// Last in `publish`, and after the value in every case: a name marked
/// read-only before the shell seeds it refuses the shell's own seed,
/// which is the reason `mark_dynamic` orders itself the same way.
// [spec:nsh:req:compat.bash.builtins-special-variables]
fn mark_published_facts(shell: &mut Shell) {
    use super::value::BashAttribute;

    for name in [
        b"BASHPID".as_slice(),
        b"EUID",
        b"OPTIND",
        b"PPID",
        b"RANDOM",
        b"SRANDOM",
        b"UID",
    ] {
        super::value::set_bash_attribute(shell, BStr::new(name), BashAttribute::Integer, true);
    }
    /* `BASH_VERSINFO` is read-only and not an integer: it is an array,
     * and `declare -ar` is what the reference prints for it. */
    for name in [b"BASH_VERSINFO".as_slice(), b"EUID", b"PPID", b"UID"] {
        super::add_attributes(shell, BStr::new(name), VariableAttributes::READ_ONLY);
    }
}

/// `$DIRSTACK`: the directory stack as an ordinary indexed array.
///
/// Published rather than computed, for the reason `PIPESTATUS` is: an
/// indexed array already answers `${DIRSTACK[@]}`, `${#DIRSTACK[@]}` and
/// `${DIRSTACK[1]}`. It is republished by everything that can move the
/// stack -- `pushd`, `popd`, `dirs -c` and `cd` -- because entry zero is
/// the current directory itself.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn publish_directory_stack(shell: &mut Shell) {
    if shell.options.dialect() != Dialect::Bash || !shell.variables.special.published {
        return;
    }
    let mut entries = vec![shell.working_directory.logical.clone().unwrap_or_default()];
    entries.extend(shell.directory_stack.below().iter().cloned());
    store_array(shell, BStr::new(b"DIRSTACK"), &entries);
}

/// `$BASH`: the path this shell was started from.
///
/// A name with a separator in it is made absolute; a bare one is looked
/// up in `PATH`, which is what makes `$BASH` runnable in a script that
/// was itself started by name.
// [spec:nsh:req:compat.bash.builtins-special-variables]
fn shell_path(shell: &mut Shell) -> BString {
    let Some(name) = shell.options.invocation_name.clone() else {
        return BString::default();
    };
    if nsh_platform::shell_path_has_separator(BStr::new(name.as_slice())) {
        return name
            .as_slice()
            .try_to_path_buf()
            .and_then(|path| nsh_platform::absolute_path(&path))
            .map_or_else(
                |_| name.clone(),
                |path| BString::from(path.to_shell_bytes()),
            );
    }
    let path = path_value(shell);
    let mut cursor = crate::execution::PathCursor::literal(BStr::new(path.as_slice()));
    while let Some(candidate) = cursor.advance(BStr::new(name.as_slice())) {
        let Ok(native) = candidate.path.as_slice().try_to_path_buf() else {
            continue;
        };
        if nsh_platform::effective_access(&native, nsh_platform::AccessMode::EXEC_OK) {
            return candidate.path;
        }
    }
    name
}

/// Turn on the `set -o` options an inherited `SHELLOPTS` names.
///
/// `export SHELLOPTS` is how Bash carries `set -x` into a child, and the
/// child half of that is this: the value is read once at startup and
/// every name in it that the option table knows is switched on.
///
/// Three names are refused. The dialect is `argv[0]`'s to choose and the
/// invocation's shape is the command line's, so an environment variable
/// may not decide either -- and a shell that let one do so could be
/// handed `interactive` by whatever set the environment.
// [spec:nsh:req:compat.bash.builtins-special-variables]
fn import_shell_options(shell: &mut Shell) {
    let name = BStr::new(b"SHELLOPTS");
    if !super::variable_attributes(shell, name).is_some_and(|attributes| attributes.exported) {
        return;
    }
    let Some(value) = super::lookup_bytes(shell, name) else {
        return;
    };
    let requested: Vec<BString> = value
        .split(|byte| *byte == b':')
        .map(BString::from)
        .collect();
    for requested in requested {
        let Some(option) = ShellOption::from_name(BStr::new(requested.as_slice())) else {
            continue;
        };
        if matches!(
            option,
            ShellOption::Bash | ShellOption::Interactive | ShellOption::Stdin
        ) {
            continue;
        }
        shell.options.set(option, true);
    }
}

/// Keep the two option listings current in the variable table.
///
/// They are recomputed on the read path, but an exported `SHELLOPTS`
/// leaves through the environment rather than through a read, so the
/// stored bytes have to be right at the moment an option changes.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn options_changed(shell: &mut Shell) {
    if !shell.variables.special.published {
        return;
    }
    for name in [b"SHELLOPTS".as_slice(), b"BASHOPTS"] {
        refresh(shell, BStr::new(name));
    }
}

/// `$SHLVL` counts shell invocations, so it continues the value the
/// environment carried in rather than starting over.
fn shell_level(shell: &mut Shell) -> i64 {
    super::lookup_bytes(shell, BStr::new(b"SHLVL"))
        .and_then(|value| std::str::from_utf8(&value).ok()?.trim().parse::<i64>().ok())
        .unwrap_or(0)
        .saturating_add(1)
        .max(1)
}

/// Give `name` a value that the read path recomputes, creating the entry
/// if a script has not already claimed the name for itself.
///
/// The attributes land after the value, because a name published
/// read-only would otherwise refuse the shell's own seed.
fn mark_dynamic(shell: &mut Shell, name: &BStr, attributes: VariableAttributes) {
    drop(super::set_bytes(
        shell,
        name,
        Some(BStr::new(b"0")),
        VariableAttributes::NONE,
    ));
    if let Some(entry) = shell.variables.entries.get_mut(name) {
        entry.callback = Callback::Special;
        entry.attributes.read_only |= attributes.read_only;
    }
}

fn store_array(shell: &mut Shell, name: &BStr, elements: &[BString]) {
    let mut value = VariableValue::empty(VariableKind::Indexed);
    for (index, element) in elements.iter().enumerate() {
        value.set_indexed(index as u64, BStr::new(element.as_slice()));
    }
    drop(arrays::store(
        shell,
        name,
        value,
        VariableAttributes::NONE,
        arrays::ReadOnlyGuard::Declaration,
    ));
}

/// `${PIPESTATUS[@]}`: the status of every command in the pipeline that
/// just finished.
///
/// Published rather than computed for the same reason the call stack is:
/// an ordinary indexed array answers `${PIPESTATUS[@]}`, `${#PIPESTATUS[@]}`
/// and `${PIPESTATUS[1]}` without the expansion pipeline knowing that
/// this module exists.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn set_pipeline_status(shell: &mut Shell, statuses: &[ExitStatus]) {
    if shell.options.dialect() != Dialect::Bash {
        return;
    }
    let elements: Vec<BString> = statuses
        .iter()
        .map(|status| BString::from(status.code().to_string()))
        .collect();
    store_array(shell, BStr::new(b"PIPESTATUS"), &elements);
}

/// Record a pipeline stage this shell ran itself.
///
/// `shopt -s lastpipe` keeps the final stage here, so the job never held
/// it and the array `wait_for_job` published is one member short.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn append_pipeline_status(shell: &mut Shell, status: ExitStatus) {
    if shell.options.dialect() != Dialect::Bash {
        return;
    }
    let name = BStr::new(b"PIPESTATUS");
    let mut elements: Vec<BString> = super::value::variable_value(shell, name)
        .map(arrays::elements)
        .unwrap_or_default();
    elements.push(BString::from(status.code().to_string()));
    store_array(shell, name, &elements);
}

/// Recompute the value of a name whose stored bytes are stale by
/// construction, immediately before a reader borrows it.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn refresh(shell: &mut Shell, name: &BStr) {
    let Some(entry) = shell.variables.entries.get(name) else {
        return;
    };
    if entry.dynamic_lineno {
        if matches!(&entry.state, VariableState::Set(_)) {
            let line = shell.variables.line_number.to_string();
            write_back(shell, name, BStr::new(line.as_bytes()));
        }
        return;
    }
    if entry.callback != Callback::Special {
        return;
    }
    let Some(value) = compute(shell, name) else {
        return;
    };
    write_back(shell, name, BStr::new(value.as_slice()));
}

fn compute(shell: &mut Shell, name: &BStr) -> Option<BString> {
    let text = match name.as_ref() as &[u8] {
        b"RANDOM" => shell.variables.special.next_random().to_string(),
        /* Bash's own `SRANDOM` is 32 bits straight from the host and
         * carries no sequence a script could rejoin, so unlike `RANDOM`
         * there is nothing here to diverge from: every read is a fresh
         * draw and an assignment is not a seed.
         * [dec:nsh:safety-trumps-compatibility] */
        b"SRANDOM" => (nsh_platform::facts::entropy_seed() as u32).to_string(),
        b"BASHPID" => nsh_platform::current_process_id().to_string(),
        b"SECONDS" => {
            let elapsed =
                nsh_platform::facts::monotonic_seconds() - shell.variables.special.seconds_origin;
            shell
                .variables
                .special
                .seconds_base
                .saturating_add(elapsed as i64)
                .to_string()
        }
        b"EPOCHSECONDS" => nsh_platform::facts::wall_clock().0.to_string(),
        b"EPOCHREALTIME" => {
            let (seconds, nanos) = nsh_platform::facts::wall_clock();
            format!("{seconds}.{:06}", nanos / 1_000)
        }
        b"BASH_SUBSHELL" => shell.shell_level.to_string(),
        b"SHELLOPTS" => return Some(joined(&shell.options.enabled_shell_options())),
        b"BASHOPTS" => return Some(joined(&shell.options.enabled_bash_options())),
        _ => return None,
    };
    Some(BString::from(text))
}

/// The colon-separated spelling both option listings use.
fn joined(names: &[&'static [u8]]) -> BString {
    let mut text = BString::default();
    for name in names {
        if !text.is_empty() {
            text.push(b':');
        }
        text.extend_from_slice(name);
    }
    text
}

/// Land a recomputed value without disturbing anything else the entry
/// carries -- and without running the assignment path, which would clear
/// the very mark that says the name is recomputed.
fn write_back(shell: &mut Shell, name: &BStr, value: &BStr) {
    if let Some(entry) = shell.variables.entries.get_mut(name) {
        match &mut entry.state {
            VariableState::Set(current) => current.assign_scalar(value),
            /* A recomputed name is never declared as an array, so the
             * declared state takes the same fresh scalar as the unset
             * one rather than an empty list of the declared kind. */
            state @ (VariableState::Unset | VariableState::Declared(_)) => {
                *state = VariableState::Set(VariableValue::Scalar(value.to_owned()));
            }
        }
    }
}

/// What an assignment to one of these names means.
///
/// `SECONDS=n` moves the origin, which is the only way a script can
/// restart the clock. `RANDOM=n` is accepted and discarded: Bash would
/// make the sequence replayable from `n`, and
/// `[dec:nsh:safety-trumps-compatibility]` says a generator anything
/// security-adjacent might reach must not be seedable by the data it is
/// generating for.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn assigned(shell: &mut Shell, name: &BStr, value: Option<&BStr>) {
    match name.as_ref() as &[u8] {
        b"SECONDS" => {
            shell.variables.special.seconds_base = value
                .and_then(|text| std::str::from_utf8(text).ok()?.trim().parse::<i64>().ok())
                .unwrap_or(0);
            shell.variables.special.seconds_origin = nsh_platform::facts::monotonic_seconds();
        }
        b"RANDOM" => shell.variables.special.reseed(),
        _ => {}
    }
}

/// Whether `name` currently holds a value, which `test -v` asks.
///
/// The name may carry a subscript, and `a[0]` is a different question
/// from `a`: Bash's `-v` is about the element when one is named.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(crate) fn is_assigned(shell: &mut Shell, name: &BStr) -> bool {
    let bytes: &[u8] = name.as_ref();
    let Some(open) = bytes.iter().position(|byte| *byte == b'[') else {
        // A name declared with `-a` or `-A` holds nothing until an
        // element exists: `typeset -a a; test -v a` is false in Bash and
        // stays false until an element is written.
        if let Some(value) = super::value::variable_value(shell, name) {
            return value.kind() == super::value::VariableKind::Scalar
                || !arrays::elements(value).is_empty();
        }
        return super::lookup_bytes(shell, name).is_some();
    };
    if bytes.last() != Some(&b']') {
        return false;
    }
    let base = BString::from(&bytes[..open]);
    let subscript = BString::from(&bytes[open + 1..bytes.len() - 1]);
    let base = BStr::new(base.as_slice());
    let Ok(selector) = arrays::resolve_text_selector(shell, base, BStr::new(subscript.as_slice()))
    else {
        return false;
    };
    let Some(value) = super::value::variable_value(shell, base) else {
        return false;
    };
    match selector {
        arrays::ArraySelector::Index(index) => value.indexed(index).is_some(),
        arrays::ArraySelector::Key(key) => value.associative(BStr::new(key.as_slice())).is_some(),
        // A subscript that named no element selects nothing to be set.
        arrays::ArraySelector::Missing => false,
        arrays::ArraySelector::All | arrays::ArraySelector::Joined => {
            !arrays::elements(value).is_empty()
        }
    }
}

/// Give a forked subshell its own generator stream.
///
/// Bash's children continue the parent's sequence, which makes two
/// subshells of one shell produce the same numbers. Re-seeding costs
/// nothing here and removes that surprise.
pub(crate) fn fork_child(shell: &mut Shell) {
    if shell.options.dialect() != Dialect::Bash {
        return;
    }
    shell.variables.special.reseed();
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::ShellOption;
    use crate::test_support::lock;
    use crate::variables::lookup_bytes;

    fn bash_shell() -> Shell {
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        shell.options.set(ShellOption::Bash, true);
        dialect_changed(&mut shell);
        shell
    }

    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn facts_are_published_once_the_dialect_selects_them() {
        let _guard = lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        dialect_changed(&mut shell);
        assert_eq!(lookup_bytes(&mut shell, BStr::new(b"BASH_VERSION")), None);

        let mut shell = bash_shell();
        let version = lookup_bytes(&mut shell, BStr::new(b"BASH_VERSION")).expect("version");
        assert!(version.starts_with(b"5."));
        assert_eq!(
            lookup_bytes(&mut shell, BStr::new(b"OSTYPE")),
            Some(BString::from(nsh_platform::facts::operating_system_type()))
        );
        assert_eq!(
            lookup_bytes(&mut shell, BStr::new(b"BASH_SUBSHELL")),
            Some(BString::from("0"))
        );
    }

    /// A generated value changes between reads, and an assignment does
    /// not make the sequence replayable.
    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn random_is_not_seedable_from_a_script() {
        let _guard = lock();
        let mut shell = bash_shell();
        let name = BStr::new(b"RANDOM");
        let draws: Vec<BString> = (0..8)
            .filter_map(|_| lookup_bytes(&mut shell, name))
            .collect();
        assert_eq!(draws.len(), 8);
        assert!(draws.iter().any(|value| value != &draws[0]));
        for value in &draws {
            let number: u64 = std::str::from_utf8(value).unwrap().parse().unwrap();
            assert!(number < RANDOM_MODULUS);
        }

        super::super::set_bytes(
            &mut shell,
            name,
            Some(BStr::new(b"42")),
            VariableAttributes::NONE,
        )
        .unwrap();
        let first = lookup_bytes(&mut shell, name).unwrap();
        super::super::set_bytes(
            &mut shell,
            name,
            Some(BStr::new(b"42")),
            VariableAttributes::NONE,
        )
        .unwrap();
        let second = lookup_bytes(&mut shell, name).unwrap();
        assert!(
            first != second || draws.iter().any(|value| value != &first),
            "a seed must not replay a sequence"
        );
    }

    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn seconds_restarts_from_an_assignment() {
        let _guard = lock();
        let mut shell = bash_shell();
        let name = BStr::new(b"SECONDS");
        super::super::set_bytes(
            &mut shell,
            name,
            Some(BStr::new(b"100")),
            VariableAttributes::NONE,
        )
        .unwrap();
        assert_eq!(
            lookup_bytes(&mut shell, name),
            Some(BString::from("100")),
            "the clock restarts where the assignment put it"
        );
    }

    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn pipeline_status_is_an_ordinary_indexed_array() {
        let _guard = lock();
        let mut shell = bash_shell();
        set_pipeline_status(
            &mut shell,
            &[
                ExitStatus::from(0),
                ExitStatus::from(1),
                ExitStatus::from(0),
            ],
        );
        let value = super::super::value::variable_value(&shell, BStr::new(b"PIPESTATUS"))
            .expect("PIPESTATUS")
            .clone();
        assert_eq!(value.kind(), VariableKind::Indexed);
        assert_eq!(
            arrays::elements(&value),
            vec![BString::from("0"), BString::from("1"), BString::from("0")]
        );
    }
}
