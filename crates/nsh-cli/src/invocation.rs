//! Byte-preserving parsing of the process invocation.

use bstr::{BStr, BString};

use nsh::{ShellOption, Startup};

#[derive(Debug)]
pub(super) enum ParseError {
    Invocation(Vec<u8>),
    Output,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(super) enum Input {
    Command(BString),
    CommandThenStdin(BString),
    Script(BString),
    Stdin,
}

#[derive(Clone, Debug)]
pub(super) struct OptionState {
    values: Vec<(ShellOption, bool)>,
}

impl OptionState {
    fn new() -> Self {
        Self {
            values: ShellOption::ALL
                .into_iter()
                .map(|option| (option, false))
                .collect(),
        }
    }

    pub(super) fn enabled(&self, option: ShellOption) -> bool {
        self.values
            .iter()
            .find(|(candidate, _)| *candidate == option)
            .is_some_and(|(_, enabled)| *enabled)
    }

    fn set(&mut self, option: ShellOption, enabled: bool) {
        if let Some((_, value)) = self
            .values
            .iter_mut()
            .find(|(candidate, _)| *candidate == option)
        {
            *value = enabled;
        }
        if enabled {
            let counterpart = match option {
                ShellOption::Vi => Some(ShellOption::Emacs),
                ShellOption::Emacs => Some(ShellOption::Vi),
                _ => None,
            };
            if let Some(counterpart) = counterpart
                && let Some((_, value)) = self
                    .values
                    .iter_mut()
                    .find(|(candidate, _)| *candidate == counterpart)
            {
                *value = false;
            }
        }
    }

    fn report(&self, enabled_form: bool) -> Vec<u8> {
        let mut output = Vec::new();
        if enabled_form {
            output.extend_from_slice(b"Current option settings\n");
            for option in ShellOption::ALL {
                let name = option.name();
                output.extend_from_slice(name);
                output.resize(output.len() + 16usize.saturating_sub(name.len()), b' ');
                output.extend_from_slice(if self.enabled(option) {
                    b"on\n"
                } else {
                    b"off\n"
                });
            }
        } else {
            for option in ShellOption::ALL {
                output.extend_from_slice(if self.enabled(option) {
                    b"set -o "
                } else {
                    b"set +o "
                });
                output.extend_from_slice(option.name());
                output.push(b'\n');
            }
        }
        output
    }
}

#[derive(Clone, Debug)]
pub(super) struct Invocation {
    pub(super) invocation_name: BString,
    pub(super) arg0: BString,
    pub(super) parameters: Vec<BString>,
    pub(super) options: OptionState,
    pub(super) login: bool,
    pub(super) input: Input,
}

impl Invocation {
    // [spec:dash:def:options.procargs-fn]
    // [spec:dash:sem:options.procargs-fn]
    // [spec:posix:req:sh.option-o-without-option-argument]
    // [spec:posix:req:sh.option-c]
    // [spec:posix:req:sh.option-i]
    // [spec:posix:req:sh.option-s]
    // [spec:posix:req:sh.option-s-assumed]
    // [spec:posix:req:sh.operand-hyphen]
    // [spec:posix:req:sh.operand-argument]
    // [spec:posix:req:sh.operand-command-file]
    // [spec:posix:req:sh.special-parameter-0]
    // [spec:posix:req:sh.operand-command-name]
    // [spec:posix:req:sh.operand-command-string]
    // [spec:nsh:req:compat.bash.selection]
    // [spec:nsh:req:idiom.shell-entrypoint]
    pub(super) fn parse(
        argv: &[Vec<u8>],
        stdin_is_terminal: bool,
        stderr_is_terminal: bool,
        mut write_report: impl FnMut(&[u8]) -> std::io::Result<()>,
    ) -> Result<Self, ParseError> {
        let invocation_name = argv.first().cloned().unwrap_or_else(|| b"sh".to_vec());
        let mut login = invocation_name.first() == Some(&b'-');
        let mut options = OptionState::new();
        let mut explicit = Vec::new();
        if bash_invocation(&invocation_name) {
            options.set(ShellOption::Bash, true);
        }

        let mut command = false;
        let mut next = 1usize;
        while let Some(word) = argv.get(next) {
            next += 1;
            let enabled = match word.first() {
                Some(b'-') => {
                    if word.len() == 1 || word.as_slice() == b"--" {
                        break;
                    }
                    true
                }
                Some(b'+') => false,
                _ => {
                    next -= 1;
                    break;
                }
            };

            for &letter in &word[1..] {
                match letter {
                    b'c' => command = true,
                    b'l' => login = true,
                    b'o' => {
                        if let Some(name) = argv.get(next) {
                            next += 1;
                            let Some(option) = ShellOption::from_name(BStr::new(name)) else {
                                let mut message = b"Illegal option -o ".to_vec();
                                message.extend_from_slice(name);
                                return Err(ParseError::Invocation(message));
                            };
                            options.set(option, enabled);
                            if !explicit.contains(&option) {
                                explicit.push(option);
                            }
                        } else {
                            write_report(&options.report(enabled))
                                .map_err(|_| ParseError::Output)?;
                        }
                    }
                    letter => {
                        let Some(option) = ShellOption::from_letter(letter) else {
                            let mut message = b"Illegal option -".to_vec();
                            message.push(letter);
                            return Err(ParseError::Invocation(message));
                        };
                        options.set(option, enabled);
                        if !explicit.contains(&option) {
                            explicit.push(option);
                        }
                    }
                }
            }
        }

        if next >= argv.len() {
            if command {
                return Err(ParseError::Invocation(b"-c requires an argument".to_vec()));
            }
            options.set(ShellOption::Stdin, true);
        }

        if !explicit.contains(&ShellOption::Interactive)
            && options.enabled(ShellOption::Stdin)
            && stdin_is_terminal
            && stderr_is_terminal
        {
            options.set(ShellOption::Interactive, true);
        }
        if !explicit.contains(&ShellOption::Monitor) {
            // [spec:nsh:req:compat.smoosh.interactive-job-prompt]
            // Forced interactivity over a pipe does not create the terminal
            // process group that monitor mode requires. An explicit `-m`
            // remains distinct and is left enabled.
            let monitor = if options.enabled(ShellOption::Stdin) && !stdin_is_terminal {
                false
            } else {
                options.enabled(ShellOption::Interactive)
            };
            options.set(ShellOption::Monitor, monitor);
        }

        let remaining = &argv[next..];
        let mut arg0 = BString::from(invocation_name.as_slice());
        let (input, parameters) = if command {
            let command = BString::from(remaining[0].as_slice());
            let mut parameter_start = 1;
            if let Some(name) = remaining.get(parameter_start) {
                arg0 = BString::from(name.as_slice());
                parameter_start += 1;
            }
            let input = if options.enabled(ShellOption::Stdin) {
                Input::CommandThenStdin(command)
            } else {
                Input::Command(command)
            };
            (input, owned_words(&remaining[parameter_start..]))
        } else if options.enabled(ShellOption::Stdin) {
            (Input::Stdin, owned_words(remaining))
        } else {
            arg0 = BString::from(remaining[0].as_slice());
            (Input::Script(arg0.clone()), owned_words(&remaining[1..]))
        };

        Ok(Self {
            invocation_name: BString::from(invocation_name),
            arg0,
            parameters,
            options,
            login,
            input,
        })
    }

    pub(super) fn startup(&self) -> Startup {
        let startup = match &self.input {
            Input::Command(command) => Startup::command(command.clone()),
            Input::CommandThenStdin(command) => Startup::command_then_stdin(command.clone()),
            Input::Script(path) => Startup::script(path.clone()),
            Input::Stdin => Startup::standard_input(),
        };
        startup.login(self.login)
    }
}

fn owned_words(words: &[Vec<u8>]) -> Vec<BString> {
    words
        .iter()
        .map(|word| BString::from(word.as_slice()))
        .collect()
}

pub(super) fn bash_invocation(raw_arg0: &[u8]) -> bool {
    let basename = raw_arg0
        .rsplit(|byte| *byte == b'/')
        .next()
        .unwrap_or_default();
    matches!(basename, b"bash" | b"-bash")
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(argv: &[&[u8]]) -> (Invocation, Vec<u8>) {
        let argv: Vec<Vec<u8>> = argv.iter().map(|word| word.to_vec()).collect();
        let mut report = Vec::new();
        let invocation = Invocation::parse(&argv, false, false, |bytes| {
            report.extend_from_slice(bytes);
            Ok(())
        })
        .expect("valid invocation");
        (invocation, report)
    }

    // [spec:nsh:req:compat.bash.selection/test]
    #[test]
    fn only_exact_bash_basenames_infer_mode() {
        for inferred in [
            b"bash".as_slice(),
            b"-bash",
            b"/bin/bash",
            b"relative/-bash",
        ] {
            assert!(bash_invocation(inferred), "{inferred:?}");
        }
        for ordinary in [b"nsh".as_slice(), b"mybash", b"bash/", b""] {
            assert!(!bash_invocation(ordinary), "{ordinary:?}");
        }
    }

    #[test]
    fn explicit_mode_overrides_inference() {
        let (invocation, _) = parse(&[b"/bin/bash", b"+o", b"bash", b"-c", b":"]);
        assert!(!invocation.options.enabled(ShellOption::Bash));
        assert!(matches!(invocation.input, Input::Command(_)));
    }

    #[test]
    fn command_name_and_parameters_are_separate() {
        let (invocation, _) = parse(&[b"nsh", b"-c", b":", b"name", b"one", b"two"]);
        assert_eq!(invocation.arg0, b"name"[..]);
        assert_eq!(invocation.parameters, [b"one".as_slice(), b"two"]);
    }

    #[test]
    fn option_report_uses_scan_point_state() {
        let (_, report) = parse(&[b"nsh", b"-eo"]);
        let report = String::from_utf8(report).unwrap();
        assert!(report.contains("errexit         on\n"), "{report}");
    }

    #[test]
    fn terminal_facts_set_stdin_defaults() {
        let argv = vec![b"nsh".to_vec()];
        let invocation = Invocation::parse(&argv, true, true, |_| Ok(())).unwrap();
        assert!(invocation.options.enabled(ShellOption::Stdin));
        assert!(invocation.options.enabled(ShellOption::Interactive));
        assert!(invocation.options.enabled(ShellOption::Monitor));

        let invocation = Invocation::parse(&argv, false, false, |_| Ok(())).unwrap();
        assert!(!invocation.options.enabled(ShellOption::Interactive));
        assert!(!invocation.options.enabled(ShellOption::Monitor));
    }

    #[test]
    fn missing_command_is_rejected() {
        let argv = vec![b"nsh".to_vec(), b"-c".to_vec()];
        let error = Invocation::parse(&argv, false, false, |_| Ok(())).unwrap_err();
        assert!(matches!(
            error,
            ParseError::Invocation(message) if message == b"-c requires an argument"
        ));
    }

    #[test]
    fn unknown_option_is_rejected() {
        let argv = vec![b"nsh".to_vec(), b"-Q".to_vec()];
        let error = Invocation::parse(&argv, false, false, |_| Ok(())).unwrap_err();
        assert!(matches!(
            error,
            ParseError::Invocation(message) if message == b"Illegal option -Q"
        ));
    }
}
