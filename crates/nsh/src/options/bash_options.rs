//! Bash's `shopt` namespace and read-only views of `set -o` state.

use bstr::{BStr, ByteSlice as _};

use super::{Dialect, OPTION_SPECS, ShellOption, ShellOptions};
// [spec:nsh:def:idiom.shell-options]

/// Names in this table are exposed only when Bash mode is active. Keep it
/// sorted so discovery, lookup, and output all have one deterministic order.
pub(crate) const NAMES: [&[u8]; 1] = [b"expand_aliases"];

pub(super) struct BashOptions {
    /// `None` selects Bash's default: on for interactive shells, off for
    /// non-interactive shells. `Some` records an explicit `shopt -s/-u`.
    expand_aliases: Option<bool>,
}

impl BashOptions {
    pub(super) const fn new() -> Self {
        Self {
            expand_aliases: None,
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

    pub(crate) fn bash_option(&self, name: &BStr) -> Option<bool> {
        match name.as_bytes() {
            b"expand_aliases" => Some(self.alias_expansion_enabled(Dialect::Bash)),
            _ => None,
        }
    }

    pub(crate) fn set_bash_option(&mut self, name: &BStr, on: bool) -> bool {
        match name.as_bytes() {
            b"expand_aliases" => {
                self.bash_options.expand_aliases = Some(on);
                true
            }
            _ => false,
        }
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

        assert!(options.set_bash_option(BStr::new(b"expand_aliases"), false));
        assert!(!options.alias_expansion_enabled(Dialect::Bash));
        assert!(options.alias_expansion_enabled(Dialect::Posix));
    }

    // [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
    #[test]
    fn option_name_views() {
        let options = ShellOptions::new();
        assert_eq!(NAMES, [b"expand_aliases" as &[u8]]);
        assert_eq!(
            options.bash_option(BStr::new(b"expand_aliases")),
            Some(false)
        );
        assert_eq!(options.bash_option(BStr::new(b"nullglob")), None);
        assert_eq!(options.shell_option(BStr::new(b"bash")), Some(false));
    }
}
