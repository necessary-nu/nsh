//! Bash's `shopt` namespace and read-only views of `set -o` state.

use bstr::{BStr, ByteSlice as _};

use super::{Dialect, OPTION_SPECS, ShellOption, ShellOptions};
// [spec:nsh:def:idiom.shell-options]

/// Names in this table are exposed only when Bash mode is active. Keep it
/// sorted so discovery, lookup, and output all have one deterministic order.
///
/// Most of these have behaviour; the rest are recorded and reported
/// without changing anything, which is what a name in [`INERT`] means.
/// Both kinds belong in one list because `shopt` has to answer "is this
/// a shell option name?" identically for both -- bash-completion sets
/// `progcomp` on the way in and gives up if the set is refused.
pub(crate) const NAMES: [&[u8]; 40] = [
    b"autocd",
    b"cdable_vars",
    b"cdspell",
    b"checkhash",
    b"checkjobs",
    b"checkwinsize",
    b"cmdhist",
    b"complete_fullquote",
    b"direxpand",
    b"dirspell",
    b"dotglob",
    b"execfail",
    b"expand_aliases",
    b"extdebug",
    b"extglob",
    b"extquote",
    b"failglob",
    b"force_fignore",
    b"globasciiranges",
    b"globstar",
    b"gnu_errfmt",
    b"histappend",
    b"histreedit",
    b"histverify",
    b"hostcomplete",
    b"huponexit",
    b"interactive_comments",
    b"lastpipe",
    b"lithist",
    b"localvar_inherit",
    b"localvar_unset",
    b"login_shell",
    b"mailwarn",
    b"no_empty_cmd_completion",
    b"nocaseglob",
    b"nocasematch",
    b"nullglob",
    b"progcomp",
    b"promptvars",
    b"sourcepath",
];

/// Names `shopt` accepts and remembers but which change nothing.
///
/// Recording them is not the same as implementing them, and the
/// difference is deliberate: a script that asks whether `progcomp` is on
/// gets the answer it set, and nothing pretends the completion machinery
/// exists. A name only belongs here while doing nothing is *safe* --
/// `xpg_echo` is absent for that reason, because silently ignoring it
/// would change what `echo` prints without saying so.
const INERT: [&[u8]; 32] = [
    b"autocd",
    b"cdable_vars",
    b"cdspell",
    b"checkhash",
    b"checkjobs",
    b"checkwinsize",
    b"cmdhist",
    b"complete_fullquote",
    b"direxpand",
    b"dirspell",
    b"execfail",
    b"extdebug",
    b"extquote",
    b"force_fignore",
    b"globasciiranges",
    b"gnu_errfmt",
    b"histappend",
    b"histreedit",
    b"histverify",
    b"hostcomplete",
    b"huponexit",
    b"interactive_comments",
    b"lastpipe",
    b"lithist",
    b"localvar_inherit",
    b"localvar_unset",
    b"login_shell",
    b"mailwarn",
    b"no_empty_cmd_completion",
    b"progcomp",
    b"promptvars",
    b"sourcepath",
];

/// Where an inert name's remembered bit lives.
fn inert_index(name: &BStr) -> Option<u32> {
    INERT
        .iter()
        .position(|candidate| *candidate == name.as_bytes())
        .map(|index| index as u32)
}

/// One Bash `shopt` name whose whole state is an on/off bit.
///
/// `expand_aliases` is deliberately not a member: its unset state is
/// distinct from off, because it follows the interactive default.
// [spec:nsh:req:compat.bash.expansion-globbing]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BashShopt {
    /// Include names beginning with `.` in pathname expansion.
    DotGlob,
    /// Recognise `?(…) *(…) +(…) @(…) !(…)` in patterns.
    ExtGlob,
    /// Fail the command when a pattern matches nothing.
    FailGlob,
    /// Let `**` cross directory boundaries.
    GlobStar,
    /// Match pathnames without regard to case.
    NoCaseGlob,
    /// Match `case` and `[[ ]]` patterns without regard to case.
    NoCaseMatch,
    /// Remove a pattern that matches nothing instead of keeping it.
    NullGlob,
}

impl BashShopt {
    const ALL: [Self; 7] = [
        Self::DotGlob,
        Self::ExtGlob,
        Self::FailGlob,
        Self::GlobStar,
        Self::NoCaseGlob,
        Self::NoCaseMatch,
        Self::NullGlob,
    ];

    const fn name(self) -> &'static [u8] {
        match self {
            Self::DotGlob => b"dotglob",
            Self::ExtGlob => b"extglob",
            Self::FailGlob => b"failglob",
            Self::GlobStar => b"globstar",
            Self::NoCaseGlob => b"nocaseglob",
            Self::NoCaseMatch => b"nocasematch",
            Self::NullGlob => b"nullglob",
        }
    }

    const fn mask(self) -> u16 {
        1 << self as u16
    }

    fn from_name(name: &BStr) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|option| option.name() == name.as_bytes())
    }
}

pub(super) struct BashOptions {
    /// `None` selects Bash's default: on for interactive shells, off for
    /// non-interactive shells. `Some` records an explicit `shopt -s/-u`.
    expand_aliases: Option<bool>,
    /// The plain on/off `shopt` names, as one bit each.
    flags: u16,
    /// The [`INERT`] names, as one bit each.
    inert: u32,
}

/// The inert names Bash has on in a fresh shell.
///
/// A default matters even for a name that does nothing: `$BASHOPTS` is
/// the list of what is on, and a script that reads it expects to find
/// the usual set rather than an empty string.
/// `progcomp` and `hostcomplete` are recognised but start *off*, unlike
/// Bash. Both announce that programmable completion is available, and
/// bash-completion reads them as a licence to load itself; this shell
/// has no `complete` built-in, so reporting them on would advertise a
/// facility that is not there.
const INERT_DEFAULTS: [&[u8]; 8] = [
    b"cmdhist",
    b"complete_fullquote",
    b"extquote",
    b"force_fignore",
    b"globasciiranges",
    b"interactive_comments",
    b"promptvars",
    b"sourcepath",
];

/// The default bits, computed once so the option table stays a `const`
/// constructor.
const fn default_inert() -> u32 {
    let mut bits = 0u32;
    let mut wanted = 0;
    while wanted < INERT_DEFAULTS.len() {
        let mut candidate = 0;
        while candidate < INERT.len() {
            if const_bytes_equal(INERT[candidate], INERT_DEFAULTS[wanted]) {
                bits |= 1 << candidate;
            }
            candidate += 1;
        }
        wanted += 1;
    }
    bits
}

const fn const_bytes_equal(left: &[u8], right: &[u8]) -> bool {
    if left.len() != right.len() {
        return false;
    }
    let mut at = 0;
    while at < left.len() {
        if left[at] != right[at] {
            return false;
        }
        at += 1;
    }
    true
}

impl BashOptions {
    pub(super) const fn new() -> Self {
        Self {
            expand_aliases: None,
            flags: 0,
            inert: default_inert(),
        }
    }
}

impl ShellOptions {
    /// Whether alias substitution is enabled for the parser entry being read.
    /// POSIX mode retains nsh's established unconditional alias behavior;
    /// Bash mode follows `expand_aliases` and its interactive default.
    pub(crate) fn alias_expansion_enabled(&self, dialect: Dialect) -> bool {
        match dialect {
            Dialect::Posix => true,
            Dialect::Bash => self
                .bash_options
                .expand_aliases
                .unwrap_or_else(|| self.enabled(ShellOption::Interactive)),
        }
    }

    /// Whether one plain `shopt` name is on. Bash's `shopt` namespace does
    /// not exist in POSIX mode, so every member reads as off there.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) const fn shopt(&self, option: BashShopt) -> bool {
        self.bash_options.flags & option.mask() != 0
    }

    pub(crate) fn bash_option(&self, name: &BStr) -> Option<bool> {
        if name.as_bytes() == b"expand_aliases" {
            return Some(self.alias_expansion_enabled(Dialect::Bash));
        }
        if let Some(option) = BashShopt::from_name(name) {
            return Some(self.shopt(option));
        }
        inert_index(name).map(|index| self.bash_options.inert & (1 << index) != 0)
    }

    pub(crate) fn set_bash_option(&mut self, name: &BStr, on: bool) -> bool {
        if name.as_bytes() == b"expand_aliases" {
            self.bash_options.expand_aliases = Some(on);
            return true;
        }
        if let Some(option) = BashShopt::from_name(name) {
            if on {
                self.bash_options.flags |= option.mask();
            } else {
                self.bash_options.flags &= !option.mask();
            }
            return true;
        }
        let Some(index) = inert_index(name) else {
            return false;
        };
        if on {
            self.bash_options.inert |= 1 << index;
        } else {
            self.bash_options.inert &= !(1 << index);
        }
        true
    }

    /// `$BASHOPTS`: the `shopt` names currently on, in table order.
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    pub(crate) fn enabled_bash_options(&self) -> Vec<&'static [u8]> {
        NAMES
            .into_iter()
            .filter(|name| self.bash_option(BStr::new(name)) == Some(true))
            .collect()
    }

    /// `$SHELLOPTS`: the `set -o` names currently on, in table order.
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    pub(crate) fn enabled_shell_options(&self) -> Vec<&'static [u8]> {
        self.shell_options()
            .filter_map(|(name, on)| on.then_some(name))
            .collect()
    }

    pub(crate) fn shell_options(&self) -> impl Iterator<Item = (&'static [u8], bool)> + '_ {
        OPTION_SPECS
            .iter()
            .map(|spec| (spec.name, self.enabled(spec.option)))
    }

    pub(crate) fn shell_option(&self, name: &BStr) -> Option<bool> {
        self.shell_options()
            .find_map(|(candidate, on)| (candidate == name.as_bytes()).then_some(on))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
    #[test]
    fn alias_defaults_by_dialect() {
        let mut options = ShellOptions::new();
        assert!(options.alias_expansion_enabled(Dialect::Posix));
        assert!(!options.alias_expansion_enabled(Dialect::Bash));

        options.set(ShellOption::Interactive, true);
        assert!(options.alias_expansion_enabled(Dialect::Bash));

        assert!(options.set_bash_option(BStr::new(b"extglob"), true));
        assert!(options.shopt(BashShopt::ExtGlob));
        assert!(!options.shopt(BashShopt::NullGlob));
        assert!(options.set_bash_option(BStr::new(b"extglob"), false));
        assert!(!options.shopt(BashShopt::ExtGlob));
        assert!(options.set_bash_option(BStr::new(b"expand_aliases"), false));
        assert!(!options.alias_expansion_enabled(Dialect::Bash));
        assert!(options.alias_expansion_enabled(Dialect::Posix));
    }

    // [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
    #[test]
    fn option_name_views() {
        let mut options = ShellOptions::new();
        assert!(NAMES.contains(&(b"expand_aliases" as &[u8])));
        assert!(NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        assert!(INERT.iter().all(|name| NAMES.contains(name)));
        assert_eq!(
            options.bash_option(BStr::new(b"expand_aliases")),
            Some(false)
        );
        assert_eq!(options.bash_option(BStr::new(b"nullglob")), Some(false));
        assert_eq!(options.bash_option(BStr::new(b"nosuchopt")), None);
        assert_eq!(options.bash_option(BStr::new(b"progcomp")), Some(false));
        assert!(options.set_bash_option(BStr::new(b"progcomp"), true));
        assert_eq!(options.bash_option(BStr::new(b"progcomp")), Some(true));
        assert!(
            options
                .enabled_bash_options()
                .contains(&(b"progcomp" as &[u8]))
        );
        assert_eq!(options.shell_option(BStr::new(b"bash")), Some(false));
    }
}
