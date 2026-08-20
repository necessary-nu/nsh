//! Typed shell-option identities, state, and command-line metadata.

// [spec:nsh:def:idiom.shell-options]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ShellOption {
    Errexit,
    NoGlob,
    IgnoreEof,
    Interactive,
    Monitor,
    NoExec,
    Stdin,
    Xtrace,
    Verbose,
    Vi,
    Emacs,
    NoClobber,
    AllExport,
    Notify,
    Nounset,
    NoLog,
    Pipefail,
    Debug,
    HashAll,
    NonLexicalControl,
    Bash,
}

impl ShellOption {
    pub(super) const fn mask(self) -> u32 {
        1 << self as u32
    }
}

/// A copyable set of enabled or explicitly mentioned shell options.
// [spec:nsh:def:idiom.shell-options]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct OptionSet(pub(super) u32);

impl OptionSet {
    pub(crate) const EMPTY: Self = Self(0);

    pub(crate) const fn contains(self, option: ShellOption) -> bool {
        self.0 & option.mask() != 0
    }

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

pub(crate) const OPTION_SPECS: [OptionSpec; 21] = [
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
}
