//! The variables the Bash dialect maintains on a script's behalf.
//!
//! The names POSIX defines and this shell reads back for itself are in
//! [`super::readers`]; nothing in this file is one of them. What is here
//! arrives with the dialect, and it is four kinds rather than one.
//!
//! *Facts* -- `BASH`, `BASH_VERSION`, `OSTYPE`, `UID` -- are published
//! once when the dialect turns on and then behave like any other
//! variable. *Clocks and
//! generators* -- `RANDOM`, `SRANDOM`, `SECONDS`, `EPOCHSECONDS`,
//! `EPOCHREALTIME`, `LINENO`, `BASHPID`, `BASH_SUBSHELL` -- have no
//! stored value worth trusting, so they are recomputed on the read that
//! asks for them. And `PIPESTATUS` and `DIRSTACK` are *published by
//! whatever moves them*, the way `call_stack` publishes `FUNCNAME`:
//! ordinary indexed arrays that every existing reader already
//! understands.
//!
//! *State the shell already keeps* is the third kind and the largest:
//! `OLDPWD`, `OPTERR`, `HISTCMD`, `_`, `BASH_COMMAND`, `BASH_ARGV0`,
//! `BASH_MONOSECONDS` and `BASH_EXECUTION_STRING` are all names for
//! something this shell had before it had the name. Each is wired to
//! whatever holds that state rather than seeded with a value, because
//! the two are not the same claim: `${BASH_COMMAND}` reading empty says
//! the shell is running a command with no text, and `BASH_COMMAND`
//! being unset says the shell has not told you.
//!
//! `BASH_ARGC` and `BASH_ARGV` are a fourth kind and the odd one: they
//! are a *stack*, and two unrelated things push onto it. The reference
//! *installs* a bottom frame of the shell's own arguments on the first
//! read that asks for them, and it then stands however the parameters
//! move afterwards; separately, a call that has arguments to record
//! pushes a frame of its own and drops it on return. So they are
//! published empty like an array, reached from the read path like a
//! clock, written once like a fact -- and moved by `call_stack`'s pushes
//! like `FUNCNAME`.
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

use super::readers::{default_continuation_prompt, default_primary_prompt, path_value};
use super::value::{VariableKind, VariableValue};
use super::{Callback, Variable, VariableAttributes, VariableState, arrays};
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

/// What `OPTERR` holds, which is the reference's own default and the
/// value `getopts` reads to decide whether to say anything.
const DIAGNOSE_BAD_OPTIONS: &str = "1";

/// Facts about the host that an inherited environment may already
/// answer, and which the shell therefore must not overwrite.
///
/// `TERM` and `SHELL` are here for a slightly different reason from the
/// other four: they are not facts about the host but statements about
/// the session, and whoever started the shell knows more about both than
/// the shell does. An inherited empty string counts as an answer --
/// `TERM= bash -c 'declare -p TERM'` is `declare -x TERM=""` in the
/// reference, not `dumb`.
const INHERITABLE: [&[u8]; 6] = [
    b"HOSTNAME",
    b"HOSTTYPE",
    b"MACHTYPE",
    b"OSTYPE",
    b"SHELL",
    b"TERM",
];

/// What `TERM` says when nothing else does, which is what the reference
/// says: a terminal with no capabilities at all.
const UNKNOWN_TERMINAL: &str = "dumb";

/// The highest value `$RANDOM` produces, as Bash documents it.
const RANDOM_MODULUS: u64 = 32_768;

/// The two call-stack names whose value a read installs rather than
/// recomputes, count first.
const CALL_ARGUMENT_NAMES: [&[u8]; 2] = [b"BASH_ARGC", b"BASH_ARGV"];

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
    /// Whether `PS1` and `PS2` are currently on the table on the shell's
    /// own behalf. True to begin with because
    /// [`super::initialize_variables`] enters both before the shell reads
    /// a line, and only a change of answer is acted on, so a script's own
    /// `PS1=` is never taken back.
    prompts_entered: bool,
    /// Whether the shell's own arguments have been pushed onto
    /// `BASH_ARGC` and `BASH_ARGV`. See [`install_call_arguments`]: the
    /// reference pushes rather than computes, so this happens once.
    call_arguments_published: bool,
    /// The `BASH_ARGV` stack, outermost frame first, each frame holding
    /// that call's words in the order they were written.
    ///
    /// The bottom frame is the install's; every frame above it belongs to
    /// a call in progress and is dropped when that call returns. Kept
    /// here rather than derived from `call_stack` because the bottom one
    /// belongs to no call at all.
    call_arguments: Vec<Vec<BString>>,
    /// The run of tokens `$BASH_COMMAND` spells back.
    ///
    /// The run and not the text: keeping one is an `Arc` clone and two
    /// offsets, so the write every command pays for is that, and the
    /// bytes are assembled only if something reads the name. `None`
    /// until the first command runs, which is what makes
    /// `declare -p BASH_COMMAND` print no value in a shell that has not
    /// run one.
    current_command: Option<crate::nodes::SourceTokens>,
}

impl SpecialState {
    pub(crate) const fn new() -> Self {
        Self {
            published: false,
            random: 0,
            seconds_origin: 0.0,
            seconds_base: 0,
            prompts_entered: true,
            call_arguments_published: false,
            call_arguments: Vec::new(),
            current_command: None,
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
    let scalars: [(&[u8], BString); 13] = [
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
        (b"TERM", BString::from(UNKNOWN_TERMINAL)),
        (b"SHELL", login_shell()),
        /* `getopts`' own error switch, and the reference writes `1` over
         * whatever the environment carried: `OPTERR=0 bash -c ...` reads
         * back `declare -x OPTERR="1"`, keeping only the export mark the
         * import gave it. So it is not `INHERITABLE`. */
        (b"OPTERR", BString::from(DIAGNOSE_BAD_OPTIONS)),
        /* `$_` before any command has run is the name the shell was
         * invoked under, which is where the reference starts it too. */
        (b"_", argument_zero(shell)),
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
        /* State the shell keeps elsewhere, answered from wherever it is
         * kept rather than seeded here. All four are invisible in the
         * reference's listing and answer a named lookup, which is what
         * the mark plus a declared entry gives them. */
        b"BASH_MONOSECONDS",
        b"HISTCMD",
        b"BASH_COMMAND",
        b"BASH_ARGV0",
    ] {
        let name = BStr::new(name);
        mark_dynamic(shell, name, VariableAttributes::NONE);
        /* A clock holds nothing until something reads it. The reference
         * prints `declare -i BASHPID` in a fresh listing and
         * `declare -i BASHPID="1868669"` once `$BASHPID` has been read,
         * so a seeded `0` was not merely early: `declare -p` spelled it
         * back as though the shell's pid were zero. The two option
         * listings below keep their seed, because they are facts rather
         * than clocks and the reference prints them with a value. */
        // [spec:nsh:req:compat.bash.builtins-special-variables]
        if let Some(entry) = shell.variables.entries.get_mut(name) {
            entry.state = VariableState::Declared(VariableKind::Scalar);
        }
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
    publish_previous_directory(shell);
    mark_published_facts(shell);
    publish_directory_stack(shell);
    publish_call_frames(shell);
}

/// `$OLDPWD`: where `cd -` goes back to, which is nowhere yet.
///
/// The name is exported and holds nothing until a `cd` moves the shell,
/// which `working_directory::update_current_directory` is what writes.
/// This is the name before that, which the reference publishes as
/// `declare -x OLDPWD` with no value at all.
///
/// An inherited value is discarded rather than kept, which is the
/// reference's own answer and not an oversight here:
/// `OLDPWD=/xx bash -c 'declare -p OLDPWD'` prints `declare -x OLDPWD`.
/// A directory this shell has never been in is not one `cd -` may go
/// back to.
// [spec:nsh:req:compat.bash.names.ordinary-state]
fn publish_previous_directory(shell: &mut Shell) {
    let name = BStr::new(b"OLDPWD");
    drop(super::set_bytes(
        shell,
        name,
        Some(BStr::new(b"")),
        VariableAttributes::EXPORTED,
    ));
    if let Some(entry) = shell.variables.entries.get_mut(name) {
        entry.state = VariableState::Declared(VariableKind::Scalar);
    }
}

/// The bytes `$BASH_COMMAND` answers with, less what ended the command.
///
/// A run reaches as far as the separator that closed it, so the newline
/// or `;` after the last word is in it and the reference's answer has no
/// such thing. Trailing blanks and separators come off; nothing else
/// does, because the rest is the command as it was written.
// [spec:nsh:req:compat.bash.names.ordinary-state]
fn running_command_text(shell: &Shell) -> Option<BString> {
    let text = shell.variables.special.current_command.as_ref()?.written();
    let end = text
        .iter()
        .rposition(|byte| !matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | b';' | b'&'))
        .map_or(0, |last| last + 1);
    Some(BString::from(&text[..end]))
}

/// Whether `$OPTERR` is asking `getopts` to report a bad option.
///
/// Zero is the only value that silences it: the reference treats the name
/// as a switch and not as a level, so `OPTERR=x` reports and `OPTERR=00`
/// does not.
// [spec:nsh:req:compat.bash.names.ordinary-state]
pub(crate) fn opterr_reports(shell: &Shell) -> bool {
    let value = shell
        .variables
        .entries
        .get(BStr::new(b"OPTERR"))
        .and_then(Variable::scalar_owned);
    let Some(value) = value else {
        return true;
    };
    std::str::from_utf8(&value)
        .ok()
        .and_then(|text| text.trim().parse::<i64>().ok())
        != Some(0)
}

/// `$SHELL`: the shell this account is meant to run.
///
/// The password entry's login shell, and not this program. Established
/// rather than assumed: a copy of the pinned Bash run from a directory
/// of its own answers `declare -- SHELL="/bin/bash"`, and so does the
/// same binary run through `exec -a totallyother`, on a host whose
/// `getent passwd` says `/bin/bash`. So the value follows neither `$0`
/// nor the path the binary was found at.
///
/// The name means "which shell does this user use", which is the
/// question a script asking `$SHELL -c` and an editor spawning a shell
/// are both asking. An account whose entry names no shell falls back to
/// the host's own answer, as the reference does.
// [spec:nsh:req:compat.bash.names.environment-facts]
fn login_shell() -> BString {
    let shell = nsh_platform::login_shell()
        .unwrap_or_else(|| nsh_platform::fallback_shell().to_os_string());
    BString::from(shell.to_shell_bytes())
}

/// `$0` as the shell currently answers it, which `BASH_ARGV0` both
/// reports and rewrites.
fn argument_zero(shell: &Shell) -> BString {
    shell
        .options
        .argument_zero()
        .map(BStr::to_owned)
        .unwrap_or_default()
}

/// `$BASH_EXECUTION_STRING`: the argument `-c` was given.
///
/// Published from the start-up request rather than from `publish`,
/// because the request is not known when the dialect is applied and
/// because the name exists only for that one invocation shape: a shell
/// reading standard input or a script file has no such string and the
/// reference publishes no such name.
// [spec:nsh:req:compat.bash.names.ordinary-state]
pub(crate) fn set_execution_string(shell: &mut Shell, text: &BStr) {
    if shell.options.dialect() != Dialect::Bash || !shell.variables.special.published {
        return;
    }
    drop(super::set_bytes(
        shell,
        BStr::new(b"BASH_EXECUTION_STRING"),
        Some(text),
        VariableAttributes::NONE,
    ));
}

/// Remember the command about to run, for `$BASH_COMMAND`.
///
/// Called where the `DEBUG` action is raised, because that is the moment
/// the reference means by "the command currently being executed": a
/// `DEBUG` action reads the command it was raised for, and a read from
/// anywhere else reads whatever is running at the time.
///
/// A command inside a trap action does not move it. The reference's own
/// name for the thing it publishes here is
/// `the_printed_command_except_trap`, and the exception is observable:
/// `trap 'echo $BASH_COMMAND' DEBUG` prints the command that raised the
/// action and not the `echo` printing it.
// [spec:nsh:req:compat.bash.names.ordinary-state]
pub(crate) fn record_command(shell: &mut Shell, tokens: &crate::nodes::SourceTokens) {
    if shell.options.dialect() != Dialect::Bash || shell.traps.bash.action_is_running() {
        return;
    }
    shell.variables.special.current_command = Some(tokens.clone());
}

/// The five names that describe the call in progress.
///
/// Three of them are `variables::call_stack`'s and were only entered on
/// a push, so a shell that had never called anything did not have them
/// at all; the reference has all three from the moment it starts. The
/// other two are entered empty here and filled by the read that asks
/// for them.
// [spec:nsh:req:compat.bash.names.call-stack]
fn publish_call_frames(shell: &mut Shell) {
    super::call_stack::refresh(shell);
    for name in CALL_ARGUMENT_NAMES {
        let name = BStr::new(name);
        store_array(shell, name, &[]);
        if let Some(entry) = shell.variables.entries.get_mut(name) {
            entry.callback = Callback::Special;
        }
    }
}

/// Write the two names out from [`SpecialState::call_arguments`].
///
/// `BASH_ARGC` runs innermost frame first and `BASH_ARGV` runs
/// innermost *argument* first, so the stack is walked backwards and each
/// frame's own words are reversed inside it: a shell started `-s a b c`
/// with nothing called gives `([0]="3")` and `([0]="c" [1]="b" [2]="a")`.
///
/// A name a script has unset is not written back. The mark went with the
/// entry, which is how a script takes one of these back for its own use;
/// writing here anyway would put the name back on the table at the next
/// call the script made, which is not the shell's to decide.
fn store_call_arguments(shell: &mut Shell) {
    if !CALL_ARGUMENT_NAMES
        .iter()
        .all(|name| shell.variables.entries.contains_key(BStr::new(*name)))
    {
        return;
    }
    let mut counts = Vec::with_capacity(shell.variables.special.call_arguments.len());
    let mut words = Vec::new();
    for frame in shell.variables.special.call_arguments.iter().rev() {
        counts.push(BString::from(frame.len().to_string()));
        words.extend(frame.iter().rev().cloned());
    }
    for (name, elements) in CALL_ARGUMENT_NAMES.into_iter().zip([counts, words]) {
        let name = BStr::new(name);
        store_array(shell, name, &elements);
        if let Some(entry) = shell.variables.entries.get_mut(name) {
            entry.callback = Callback::Special;
        }
    }
}

/// Install the frame the reference has before anything is called: the
/// shell's own positional parameters.
///
/// Once, and never again. The install is a *push* and not a computation,
/// and that is observable: after it, `set -- x y z` leaves
/// `${BASH_ARGV[@]}` spelling the arguments the shell started with.
///
/// Unconditional, because one of its two callers is `shopt -s extdebug`,
/// which the reference lets install from inside a function -- measured on
/// the pinned 5.3.15: `f(){ shopt -s extdebug; }; set -- x y; f` leaves
/// `BASH_ARGC=([0]="0")`, which is `f`'s own empty parameter list and not
/// the shell's `x y`. The read path applies the function gate itself.
// [spec:nsh:req:compat.bash.names.call-stack]
pub(crate) fn install_call_arguments(shell: &mut Shell) {
    if shell.options.dialect() != Dialect::Bash || shell.variables.special.call_arguments_published
    {
        return;
    }
    shell.variables.special.call_arguments_published = true;
    let words = shell.options.positional_parameters.words();
    shell.variables.special.call_arguments.push(words);
    store_call_arguments(shell);
}

/// `BASH_ARGC` and `BASH_ARGV`, on the first read that asks for them.
///
/// The reference fills them from the shell's own positional parameters on
/// the first read taken with no *function* frame in progress -- its own
/// source calls that mimicking the behaviour it had before
/// `shopt -s extdebug` existed -- and they then stand however the
/// parameters move afterwards. A read taken inside a function fills
/// nothing at all, which is why the reference answers `()` for both
/// there.
///
/// The gate is a function frame and not any frame. A dot script is not
/// one: measured on the pinned 5.3.15, `. lib.sh` at the top level
/// installs the shell's parameters under the frame the dot script itself
/// pushes, while the same `. lib.sh` from inside a function installs
/// nothing.
// [spec:nsh:req:compat.bash.names.call-stack]
fn publish_call_arguments(shell: &mut Shell) {
    if shell.variables.call_stack.function_depth() > 0 {
        return;
    }
    install_call_arguments(shell);
}

/// Push a call's own arguments, for a call that has any to push.
///
/// The frame goes on top of whatever the install left, so the two
/// compose rather than replacing one another, and the install is what
/// this reaches for first: the reference's push walks the same two names
/// a read does, so it installs before it pushes.
///
/// Answers whether it pushed, so the caller knows whether it owes a pop.
/// Only the Bash dialect has these names, and a dialect that changes
/// mid-script must not leave a frame owing a pop that was never taken.
// [spec:nsh:req:compat.bash.names.call-stack]
pub(crate) fn push_call_arguments(shell: &mut Shell, words: Vec<BString>) -> bool {
    if shell.options.dialect() != Dialect::Bash {
        return false;
    }
    publish_call_arguments(shell);
    shell.variables.special.call_arguments.push(words);
    store_call_arguments(shell);
    true
}

/// Drop the frame [`push_call_arguments`] pushed, as the call returns.
// [spec:nsh:req:compat.bash.names.call-stack]
pub(crate) fn pop_call_arguments(shell: &mut Shell) {
    shell.variables.special.call_arguments.pop();
    store_call_arguments(shell);
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
        b"HISTCMD",
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

/// Follow an option change through the names that depend on one.
///
/// The two option listings are recomputed on the read path, but an
/// exported `SHELLOPTS` leaves through the environment rather than
/// through a read, so the stored bytes have to be right at the moment an
/// option changes. The two prompts are here because whether the
/// reference has those names at all is decided by the invocation.
// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn options_changed(shell: &mut Shell) {
    if !shell.variables.special.published {
        return;
    }
    for name in [b"SHELLOPTS".as_slice(), b"BASHOPTS"] {
        refresh(shell, BStr::new(name));
    }
    publish_prompts(shell);
}

/// `PS1` and `PS2` belong to a shell somebody is watching.
///
/// A non-interactive reference has neither name -- not even when the
/// environment supplied one, which it drops -- and this shell had both
/// unconditionally, because [`super::initialize_variables`] is shared
/// with the POSIX dialect and dash's `set` listing carries them. So Bash
/// mode withholds the two rather than never entering them, which leaves
/// the `FIXED` slot each was entered in standing and holding nothing,
/// the way `TERM`'s slot stands empty until something fills it.
///
/// Followed rather than settled once, because interactivity belongs to
/// the invocation and not to the dialect: this shell kept dash's `set -o
/// interactive`, which the reference refuses, so a Bash-mode script can
/// still ask for a prompt after startup and has to be given one. Leaving
/// Bash mode gives them back for the same reason -- the POSIX dialect is
/// judged against dash, which has both. Only a change of answer is acted
/// on and a restore writes only into a name holding nothing, so a
/// script's own `PS1=` is never taken back.
// [spec:nsh:req:compat.bash.names.only-what-the-reference-has]
fn publish_prompts(shell: &mut Shell) {
    let entered =
        shell.options.dialect() != Dialect::Bash || shell.options.enabled(ShellOption::Interactive);
    if entered == shell.variables.special.prompts_entered {
        return;
    }
    shell.variables.special.prompts_entered = entered;
    for (name, default) in [
        (b"PS1".as_slice(), default_primary_prompt()),
        (b"PS2".as_slice(), default_continuation_prompt()),
    ] {
        let name = BStr::new(name);
        if !entered {
            drop(super::unset_bytes(shell, name));
        } else if super::lookup_bytes(shell, name).is_none() {
            drop(super::set_bytes(
                shell,
                name,
                Some(default),
                VariableAttributes::NONE,
            ));
        }
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
    if CALL_ARGUMENT_NAMES.contains(&(name.as_ref() as &[u8])) {
        publish_call_arguments(shell);
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
            /* The read is what gives `SECONDS` the integer attribute, and
             * it is the only name whose letters a read changes:
             * `declare -p | grep SECONDS` is `declare -- SECONDS` in a
             * fresh reference and `declare -i SECONDS` once `$SECONDS`
             * has been read. `BASHPID`, `RANDOM`, `SRANDOM` and `OPTIND`
             * carry `-i` from the start and `EPOCHSECONDS`,
             * `EPOCHREALTIME`, `LINENO` and `BASH_SUBSHELL` never carry
             * it, read or not. */
            // [spec:nsh:req:compat.bash.builtins-special-variables]
            super::value::set_bash_attribute(
                shell,
                name,
                super::value::BashAttribute::Integer,
                true,
            );
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
        /* A clock exactly like `EPOCHSECONDS`, off the monotonic source
         * rather than the wall clock, which is the whole of what the
         * name says. */
        b"BASH_MONOSECONDS" => (nsh_platform::facts::monotonic_seconds() as i64).to_string(),
        /* The number the newest history entry carries, and `0` where
         * there is no history -- which is every non-interactive shell,
         * here and in the reference. */
        b"HISTCMD" => crate::editor::history_mut(shell)
            .and_then(|history| history.newest())
            .map_or(0, |event| event.number)
            .to_string(),
        b"BASH_ARGV0" => return Some(argument_zero(shell)),
        b"BASH_COMMAND" => return running_command_text(shell),
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
        /* `BASH_ARGV0=zed` makes `$0` answer `zed`, which is the only
         * way a script can rename itself. */
        // [spec:nsh:req:compat.bash.names.ordinary-state]
        b"BASH_ARGV0" => {
            if let Some(value) = value {
                shell.options.set_arg0(value);
            }
        }
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
    let bracket = bytes.iter().position(|byte| *byte == b'[');
    /* Asking is a read, and for one name the read is what makes the
     * answer true: `test -v BASH_ARGC` is false in a reference that has
     * never looked at it and true afterwards, because looking is what
     * pushes the shell's arguments onto it. */
    // [spec:nsh:req:compat.bash.names.call-stack]
    refresh(shell, BStr::new(&bytes[..bracket.unwrap_or(bytes.len())]));
    let Some(open) = bracket else {
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
