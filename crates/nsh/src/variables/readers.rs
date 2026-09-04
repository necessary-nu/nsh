//! The names POSIX defines that the shell itself reads back.
//!
//! [`super::initialize_variables`] enters `IFS`, `PATH`, `MAIL`,
//! `MAILPATH`, `PS1`, `PS2`, `PS4` and `HISTSIZE` before the shell reads
//! a line, and everything in the shell that needs one of them comes back
//! through the one-line accessor named for it here, over the single
//! lookup in [`builtin_value`]. Naming a reader for the variable rather
//! than for its caller is what makes the set of names the shell itself
//! depends on a list one can read.
//!
//! `ifs_is_set` and `mail_path_is_set` are separate from the readers
//! because for those two names unset and empty mean different things: an
//! unset `IFS` splits on the default and an empty one does not split at
//! all, and an empty `MAILPATH` still wins over `MAIL`. A caller given
//! only the value could not tell the cases apart.
//!
//! The defaults are what start-up installs when nothing else supplies a
//! value, and they sit beside the readers so that what is written at
//! startup and what is read afterwards cannot drift.
//! `special::publish_prompts` takes the two prompt defaults from here
//! for that reason: a Bash-mode shell that turns interactive after
//! start-up has to be given back the prompt start-up would have given
//! it.
//!
//! Nothing in this file knows the Bash dialect exists, which is the line
//! between it and [`super::special`].

use bstr::{BStr, BString};
use nsh_platform::NativeStrExt as _;

use super::{DEFAULT_IFS, Variable, VariableState};
use crate::context::Shell;

pub fn default_ifs() -> &'static BStr {
    BStr::new(DEFAULT_IFS)
}

pub fn default_path() -> BString {
    BString::from(nsh_platform::default_search_path().to_shell_bytes())
}

/// A shell running as root prompts with `#`, which is what POSIX asks
/// for and what tells the reader of a transcript which shell it was.
// [spec:posix:req:param.ps1-default]
pub fn default_primary_prompt() -> &'static BStr {
    if nsh_platform::effective_uid().is_root() {
        BStr::new(b"# ")
    } else {
        BStr::new(b"$ ")
    }
}

pub fn default_continuation_prompt() -> &'static BStr {
    BStr::new(b"> ")
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
