//! Bash's `shopt` namespace and read-only views of `set -o` state.

use bstr::{BStr, ByteSlice as _};

use super::{Dialect, OPTION_SPECS, ShellOption, ShellOptions};
// [spec:nsh:def:idiom.shell-options]

/// Names in this table are exposed only when Bash mode is active. Keep it
/// sorted so discovery, lookup, and output all have one deterministic order.
pub(crate) const NAMES: [&[u8]; 8] = [
    b"dotglob",
    b"expand_aliases",
    b"extglob",
    b"failglob",
    b"globstar",
    b"nocaseglob",
    b"nocasematch",
    b"nullglob",
];

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
}

impl BashOptions {
    pub(super) const fn new() -> Self {
        Self {
            expand_aliases: None,
            flags: 0,
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
        BashShopt::from_name(name).map(|option| self.shopt(option))
    }

    pub(crate) fn set_bash_option(&mut self, name: &BStr, on: bool) -> bool {
        if name.as_bytes() == b"expand_aliases" {
            self.bash_options.expand_aliases = Some(on);
            return true;
        }
        let Some(option) = BashShopt::from_name(name) else {
            return false;
        };
        if on {
            self.bash_options.flags |= option.mask();
        } else {
            self.bash_options.flags &= !option.mask();
        }
        true
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
        let options = ShellOptions::new();
        assert_eq!(NAMES[1], b"expand_aliases" as &[u8]);
        assert!(NAMES.windows(2).all(|pair| pair[0] < pair[1]));
        assert_eq!(
            options.bash_option(BStr::new(b"expand_aliases")),
            Some(false)
        );
        assert_eq!(options.bash_option(BStr::new(b"nullglob")), Some(false));
        assert_eq!(options.bash_option(BStr::new(b"nosuchopt")), None);
        assert_eq!(options.shell_option(BStr::new(b"bash")), Some(false));
    }
}
