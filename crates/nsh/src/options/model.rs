//! Typed shell-option identities, state, and command-line metadata.

use bstr::BStr;

// [spec:nsh:def:idiom.shell-options]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
/// One option in the shell's `set -o` namespace.
pub enum ShellOption {
    /// Exit when an untested command fails.
    Errexit,
    /// Disable pathname expansion.
    NoGlob,
    /// Require repeated end-of-file requests in an interactive shell.
    IgnoreEof,
    /// Enable interactive parsing and diagnostics.
    Interactive,
    /// Enable job control.
    Monitor,
    /// Parse commands without executing them.
    NoExec,
    /// Read commands from standard input.
    Stdin,
    /// Trace expanded commands before executing them.
    Xtrace,
    /// Echo input lines as they are read.
    Verbose,
    /// Select vi-style line editing.
    Vi,
    /// Select emacs-style line editing.
    Emacs,
    /// Refuse to overwrite existing regular files with `>`.
    NoClobber,
    /// Export variables modified by assignments.
    AllExport,
    /// Report completed background jobs promptly.
    Notify,
    /// Diagnose expansion of unset parameters.
    Nounset,
    /// Suppress command-history recording.
    NoLog,
    /// Make a pipeline report the rightmost failing status.
    Pipefail,
    /// Enable implementation debugging behavior.
    Debug,
    /// Remember command locations.
    HashAll,
    /// Allow loop control to cross function boundaries.
    NonLexicalControl,
    /// Enable Bash Compatibility Mode.
    Bash,
    /// Let the Bash `ERR` trap reach functions, subshells, and command
    /// substitutions.
    Errtrace,
    /// Let the Bash `DEBUG` and `RETURN` traps reach functions, subshells,
    /// and command substitutions.
    Functrace,
}

impl ShellOption {
    /// Every shell option, in the stable order used by `set -o` reports.
    pub const ALL: [Self; 23] = [
        Self::Errexit,
        Self::NoGlob,
        Self::IgnoreEof,
        Self::Interactive,
        Self::Monitor,
        Self::NoExec,
        Self::Stdin,
        Self::Xtrace,
        Self::Verbose,
        Self::Vi,
        Self::Emacs,
        Self::NoClobber,
        Self::AllExport,
        Self::Notify,
        Self::Nounset,
        Self::NoLog,
        Self::Pipefail,
        Self::Debug,
        Self::HashAll,
        Self::NonLexicalControl,
        Self::Bash,
        Self::Errtrace,
        Self::Functrace,
    ];

    pub(super) const fn mask(self) -> u32 {
        1 << self as u32
    }

    /// The long name accepted by `set -o` and [`crate::Builder::option`].
    pub fn name(self) -> &'static BStr {
        BStr::new(OPTION_SPECS[self as usize].name)
    }

    /// The invocation/set letter, when this option has one.
    pub const fn letter(self) -> Option<u8> {
        OPTION_SPECS[self as usize].letter
    }

    /// Resolve a long `set -o` name without parsing an invocation.
    pub fn from_name(name: &BStr) -> Option<Self> {
        OPTION_SPECS
            .iter()
            .find(|spec| name == spec.name)
            .map(|spec| spec.option)
    }

    /// Resolve one option letter without parsing an invocation.
    pub fn from_letter(letter: u8) -> Option<Self> {
        OPTION_SPECS
            .iter()
            .find(|spec| spec.letter == Some(letter))
            .map(|spec| spec.option)
    }
}

/// A copyable set of enabled or explicitly mentioned shell options.
// [spec:nsh:def:idiom.shell-options]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct OptionSet(pub(super) u32);

impl OptionSet {
    pub(crate) const EMPTY: Self = Self(0);

    #[cfg(test)]
    pub(crate) const fn contains(self, option: ShellOption) -> bool {
        self.0 & option.mask() != 0
    }

    #[cfg(test)]
    pub(crate) fn set(&mut self, option: ShellOption, enabled: bool) {
        if enabled {
            self.0 |= option.mask();
        } else {
            self.0 &= !option.mask();
        }
    }
}

/// The user-facing spellings of one shell option.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct OptionSpec {
    pub(crate) option: ShellOption,
    pub(crate) name: &'static [u8],
    pub(crate) letter: Option<u8>,
}

pub(crate) const OPTION_SPECS: [OptionSpec; 23] = [
    OptionSpec {
        option: ShellOption::Errexit,
        name: b"errexit",
        letter: Some(b'e'),
    },
    OptionSpec {
        option: ShellOption::NoGlob,
        name: b"noglob",
        letter: Some(b'f'),
    },
    OptionSpec {
        option: ShellOption::IgnoreEof,
        name: b"ignoreeof",
        letter: Some(b'I'),
    },
    OptionSpec {
        option: ShellOption::Interactive,
        name: b"interactive",
        letter: Some(b'i'),
    },
    OptionSpec {
        option: ShellOption::Monitor,
        name: b"monitor",
        letter: Some(b'm'),
    },
    OptionSpec {
        option: ShellOption::NoExec,
        name: b"noexec",
        letter: Some(b'n'),
    },
    OptionSpec {
        option: ShellOption::Stdin,
        name: b"stdin",
        letter: Some(b's'),
    },
    OptionSpec {
        option: ShellOption::Xtrace,
        name: b"xtrace",
        letter: Some(b'x'),
    },
    OptionSpec {
        option: ShellOption::Verbose,
        name: b"verbose",
        letter: Some(b'v'),
    },
    OptionSpec {
        option: ShellOption::Vi,
        name: b"vi",
        letter: Some(b'V'),
    },
    OptionSpec {
        option: ShellOption::Emacs,
        name: b"emacs",
        letter: Some(b'E'),
    },
    OptionSpec {
        option: ShellOption::NoClobber,
        name: b"noclobber",
        letter: Some(b'C'),
    },
    OptionSpec {
        option: ShellOption::AllExport,
        name: b"allexport",
        letter: Some(b'a'),
    },
    OptionSpec {
        option: ShellOption::Notify,
        name: b"notify",
        letter: Some(b'b'),
    },
    OptionSpec {
        option: ShellOption::Nounset,
        name: b"nounset",
        letter: Some(b'u'),
    },
    OptionSpec {
        option: ShellOption::NoLog,
        name: b"nolog",
        letter: None,
    },
    OptionSpec {
        option: ShellOption::Pipefail,
        name: b"pipefail",
        letter: None,
    },
    OptionSpec {
        option: ShellOption::Debug,
        name: b"debug",
        letter: None,
    },
    OptionSpec {
        option: ShellOption::HashAll,
        name: b"hashall",
        letter: Some(b'h'),
    },
    OptionSpec {
        option: ShellOption::NonLexicalControl,
        name: b"nonlexicalctrl",
        letter: None,
    },
    OptionSpec {
        option: ShellOption::Bash,
        name: b"bash",
        letter: None,
    },
    /* `-E` and `-T` are Bash's letters for these and are claimed by the
     * dialect-gated arm in `options::set_option`, not by this table: `-E`
     * is already `emacs` in the POSIX dialect and the table is what that
     * dialect reads. */
    OptionSpec {
        option: ShellOption::Errtrace,
        name: b"errtrace",
        letter: None,
    },
    OptionSpec {
        option: ShellOption::Functrace,
        name: b"functrace",
        letter: None,
    },
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn option_state_is_isolated() {
        let mut state = OptionSet::EMPTY;
        state.set(ShellOption::Errexit, true);
        assert!(state.contains(ShellOption::Errexit));
        assert!(!state.contains(ShellOption::NoGlob));
        state.set(ShellOption::Errexit, false);
        assert!(!state.contains(ShellOption::Errexit));
    }

    #[test]
    fn metadata_spellings_are_unique() {
        for (index, spec) in OPTION_SPECS.iter().enumerate() {
            assert!(
                OPTION_SPECS[..index]
                    .iter()
                    .all(|other| other.name != spec.name)
            );
            if let Some(letter) = spec.letter {
                assert!(
                    OPTION_SPECS[..index]
                        .iter()
                        .all(|other| other.letter != Some(letter))
                );
            }
        }
    }

    #[test]
    fn public_order_matches_metadata() {
        for (index, option) in ShellOption::ALL.into_iter().enumerate() {
            assert_eq!(option, OPTION_SPECS[index].option);
        }
    }
}
