//! Building a [`Shell`].
//!
//! `docs/api-design.md` §2. The C had no constructor: the shell *was* the
//! process, so its initial state came from `environ`, from `argv`, and
//! from whatever the kernel had already put in place. Every one of those
//! is a thing [dec:nsh:host-owns-the-process] and
//! [dec:nsh:host-owns-streams] say a library takes only when asked, so
//! each becomes a setting here and the defaults are the inert ones.

use bstr::{BStr, BString};
use nsh_platform::NativeStrExt as _;

use crate::context::Shell;
use crate::error::Error;
use crate::options::ShellOption;
use crate::streams::Streams;

enum RequestedOption {
    Named(BString, bool),
    Typed(ShellOption, bool),
}

/// Builds a [`Shell`].
///
/// The default is a shell that is inert with respect to the process it is
/// built in: no variables inherited, `$0` unchanged, no positional
/// parameters, every option off, the working directory left alone, and
/// [`Streams::INHERIT`] for its descriptor namespace.
///
/// `Streams::INHERIT` is the one default that is *not* inert, and it is
/// the exception on purpose: a shell whose output went nowhere by default
/// would be a shell every embedder has to configure before it can be
/// tested at all. §7 is the argument.
///
/// ```ignore
/// let mut sh = Shell::builder()
///     .arg0(BStr::new(b"myapp"))
///     .inherit_env()
///     .option(BStr::new(b"errexit"), true)
///     .build()?;
/// ```
pub struct Builder {
    streams: Streams,
    invocation_name: Option<BString>,
    argument_zero: Option<BString>,
    args: Vec<BString>,
    env: Vec<(BString, BString)>,
    inherit_env: bool,
    options: Vec<RequestedOption>,
    working_directory: Option<std::path::PathBuf>,
    host: Option<Box<dyn crate::host::Host>>,
}

impl Default for Builder {
    fn default() -> Self {
        Builder::new()
    }
}

impl Builder {
    /// A builder with every setting at its default.
    pub fn new() -> Builder {
        Builder {
            streams: Streams::INHERIT,
            invocation_name: None,
            argument_zero: None,
            args: Vec::new(),
            env: Vec::new(),
            inherit_env: false,
            options: Vec::new(),
            working_directory: None,
            host: None,
        }
    }

    /// `$0`, and the name every diagnostic is prefixed with.
    pub fn argument_zero(mut self, argument_zero: &BStr) -> Self {
        self.argument_zero = Some(argument_zero.to_owned());
        self
    }

    /// The process invocation name used by diagnostics that identify the
    /// interpreter rather than the current script or command name.
    ///
    /// A command-file operand or the name after `-c` becomes `$0`, but it
    /// must not replace the interpreter identity captured from raw `argv[0]`.
    /// Callers that do not set this explicitly get the value passed to
    /// [`Builder::arg0`].
    pub fn invocation_name(mut self, name: &BStr) -> Self {
        self.invocation_name = Some(name.to_owned());
        self
    }

    /// The positional parameters `$1`, `$2`, ….
    pub fn args(mut self, args: &[&BStr]) -> Self {
        self.args = args.iter().map(|argument| (*argument).to_owned()).collect();
        self
    }

    /// Variables in the initial environment, exported, as `execve` would
    /// have delivered them.
    ///
    /// Additive: two calls are the union, and a repeated name is resolved
    /// the way a repeated `environ` entry is -- the last one wins, because
    /// both go through the same `setvareq`.
    pub fn env<K, V>(mut self, variables: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<BString>,
        V: Into<BString>,
    {
        self.env.extend(
            variables
                .into_iter()
                .map(|(name, value)| (name.into(), value.into())),
        );
        self
    }

    /// Take the calling process's environment.
    ///
    /// Separate from [`Builder::env`] because the process environment is
    /// read in its native `OsString` representation and crosses into the
    /// byte-oriented shell variable table only while the shell is built.
    ///
    /// The environment is snapshotted into storage owned by the shell;
    /// no pointer into the process-global environment survives the build.
    /// Pairs from [`Builder::env`] are applied on top.
    pub fn inherit_env(mut self) -> Self {
        self.inherit_env = true;
        self
    }

    /// Where the shell's own three streams come from.
    pub fn streams(mut self, streams: Streams) -> Self {
        self.streams = streams;
        self
    }

    /// Set one shell option by its `set -o` long name or its letter:
    /// `option(b"errexit", true)` and `option(b"e", true)` are the same.
    ///
    /// Applied in call order, after the environment and the parameters,
    /// so an option whose effect depends on them sees them.
    pub fn option(mut self, name: &BStr, on: bool) -> Self {
        self.options
            .push(RequestedOption::Named(name.to_owned(), on));
        self
    }

    /// Set one shell option by typed identity.
    ///
    /// This is the frontend-friendly form: command-line parsing can resolve
    /// spelling once and hand the builder a value rather than asking the core
    /// to parse an invocation.
    pub fn shell_option(mut self, option: ShellOption, on: bool) -> Self {
        self.options.push(RequestedOption::Typed(option, on));
        self
    }

    /// What the library may do to the process, and who does it.
    ///
    /// Without this the shell gets [`crate::host::NoHost`]: it installs no
    /// signal handler and refuses `exec`, which is the correct default for
    /// a library and is not what a shell frontend wants.
    pub fn host(mut self, host: impl crate::host::Host + 'static) -> Self {
        self.host = Some(Box::new(host));
        self
    }

    /// The shell's working directory.
    ///
    /// Per-instance in the sense that `$PWD` is, and process-wide in the
    /// sense that `chdir` is -- this calls `chdir`, so it moves every
    /// shell in the process and the process itself. `docs/api-design.md`
    /// §6 is why that is the honest thing to do rather than the
    /// alternative of a shell whose idea of "." differs from the kernel's.
    pub fn working_directory(mut self, dir: impl AsRef<std::path::Path>) -> Self {
        self.working_directory = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Build the shell.
    ///
    /// The order below is the constraint, not a preference. Initialization
    /// has to come first because everything after it writes into tables it
    /// creates; the options come after the parameters because
    /// `optschanged` acts on the finished set; and the working directory
    /// comes last because `setpwd` writes `PWD` through the variable
    /// table and wants the kernel already moved.
    pub fn build(self) -> Result<Shell, Error> {
        let mut shell = Shell::try_new(self.streams).map_err(|error| {
            Error::other(
                0,
                2,
                format!("cannot snapshot shell streams: {error}").as_bytes(),
            )
        })?;
        /* Before anything else the host might be asked about. `attach` is
         * specified as happening exactly once, and this is it: the sink is
         * the only part of the shell a signal handler may touch, so the
         * host has to be holding it before a handler could exist. */
        if let Some(host) = self.host {
            shell.host = host;
        }
        shell.host.attach(crate::signal_inbox::signals());
        let mut environment = if self.inherit_env {
            nsh_platform::process_environment()
                .into_iter()
                .map(|(name, value)| {
                    (
                        BString::from(name.to_shell_bytes()),
                        BString::from(value.to_shell_bytes()),
                    )
                })
                .collect()
        } else {
            Vec::new()
        };
        environment.extend(self.env);
        shell.initialize_from(&environment)?;

        if let Some(invocation_name) = &self.invocation_name {
            shell
                .options
                .set_invocation_name(BStr::new(&invocation_name[..]));
        }
        if let Some(argument_zero) = &self.argument_zero {
            shell.options.set_arg0(BStr::new(&argument_zero[..]));
        }

        if !self.args.is_empty() {
            let refs: Vec<&BStr> = self
                .args
                .iter()
                .map(|argument| BStr::new(&argument[..]))
                .collect();
            crate::options::set_positional_parameters(&mut shell, &refs);
        }

        for option in &self.options {
            match option {
                RequestedOption::Named(name, on) => {
                    crate::options::set_option_by_name(&mut shell, BStr::new(&name[..]), *on)?;
                }
                RequestedOption::Typed(option, on) => {
                    crate::options::set_typed_option(&mut shell, *option, *on);
                }
            }
        }
        crate::options::apply_option_changes(&mut shell)?;

        if let Some(dir) = &self.working_directory {
            /* `Error::Other` because the taxonomy's `Io` variant is
             * not promoted yet -- §3.4's "start with `Other`, promote
             * the interesting ones after". Status 2 is what dash's
             * `sh_error` takes, and what a failed `cd` leaves. */
            nsh_platform::set_current_directory(dir).map_err(|error| {
                Error::other(
                    0,
                    2,
                    format!(
                        "can't cd to {}: {}",
                        dir.display(),
                        shell.locale.error_message(&error)
                    )
                    .as_bytes(),
                )
            })?;
            crate::working_directory::update_current_directory(
                &mut shell,
                crate::working_directory::DirectoryUpdate::Unknown,
                false,
            )?;
        }
        Ok(shell)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `lookupvar` as a byte string, or `None` when the shell has no such
    /// variable.
    fn var(shell: &mut Shell, name: &BStr) -> Option<Vec<u8>> {
        crate::variables::lookup_bytes(shell, name).map(Vec::from)
    }

    fn process_probe() -> (BString, BString) {
        let mut empty = Shell::builder().build().unwrap();
        nsh_platform::process_environment()
            .into_iter()
            .map(|(name, value)| {
                (
                    BString::from(name.to_shell_bytes()),
                    BString::from(value.to_shell_bytes()),
                )
            })
            .find(|(name, _)| {
                let mut bytes = name.iter().copied();
                matches!(bytes.next(), Some(b'a'..=b'z' | b'A'..=b'Z' | b'_'))
                    && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
                    && var(&mut empty, BStr::new(name.as_slice())).is_none()
            })
            .expect("the test process has a non-builtin environment entry")
    }

    #[test]
    fn a_builder_with_no_env_setting_inherits_nothing_from_the_process() {
        let (name, _) = process_probe();
        let mut shell = Shell::builder().build().unwrap();
        assert_eq!(var(&mut shell, BStr::new(name.as_slice())), None);
    }

    #[test]
    fn inherit_env_takes_the_processs_environment() {
        let (name, value) = process_probe();
        let mut shell = Shell::builder().inherit_env().build().unwrap();
        assert_eq!(
            var(&mut shell, BStr::new(name.as_slice())).as_deref(),
            Some(value.as_slice())
        );
    }

    #[test]
    fn explicit_pairs_are_set_and_exported() {
        let mut shell = Shell::builder()
            .env([("NSH_EXPLICIT", "a value with spaces")])
            .build()
            .unwrap();
        assert_eq!(
            var(&mut shell, BStr::new(b"NSH_EXPLICIT")).as_deref(),
            Some(&b"a value with spaces"[..])
        );
    }

    // [spec:nsh:def:idiom.shell-options]
    /// The two spellings the sketch promised are the same option reach the
    /// same flag. This is the whole of `set_option_by_name`'s contract.
    #[test]
    fn an_option_is_the_same_set_by_long_name_or_by_letter() {
        let by_name = Shell::builder()
            .option(BStr::new(b"errexit"), true)
            .build()
            .unwrap();
        let by_letter = Shell::builder()
            .option(BStr::new(b"e"), true)
            .build()
            .unwrap();
        assert!(
            by_name
                .options
                .enabled(crate::options::ShellOption::Errexit)
        );
        assert_eq!(
            by_name
                .options
                .enabled(crate::options::ShellOption::Errexit),
            by_letter
                .options
                .enabled(crate::options::ShellOption::Errexit)
        );
    }

    // [spec:nsh:req:compat.bash.selection/test]
    #[test]
    fn arg0_does_not_select_bash_mode() {
        let shell = Shell::builder()
            .argument_zero(BStr::new(b"bash"))
            .build()
            .unwrap();
        assert_eq!(shell.options.dialect(), crate::options::Dialect::Posix);

        let selected = Shell::builder()
            .argument_zero(BStr::new(b"nsh"))
            .option(BStr::new(b"bash"), true)
            .build()
            .unwrap();
        assert_eq!(selected.options.dialect(), crate::options::Dialect::Bash);
    }

    #[test]
    fn invocation_name_survives_arg_zero() {
        let shell = Shell::builder()
            .invocation_name(BStr::new(b"nsh"))
            .argument_zero(BStr::new(b"script.sh"))
            .build()
            .unwrap();

        assert_eq!(shell.options.argument_zero(), Some(BStr::new(b"script.sh")));
        assert_eq!(
            shell
                .options
                .invocation_name
                .as_ref()
                .map(|name| name.as_slice()),
            Some(b"nsh".as_slice())
        );
    }

    // [spec:nsh:req:compat.bash.state-isolation/test]
    #[test]
    fn shell_values_hold_isolated_dialects() {
        let mut selected = Shell::builder()
            .option(BStr::new(b"bash"), true)
            .build()
            .unwrap();
        let ordinary = Shell::builder().build().unwrap();

        assert_eq!(selected.options.dialect(), crate::options::Dialect::Bash);
        assert_eq!(ordinary.options.dialect(), crate::options::Dialect::Posix);

        crate::options::set_option_by_name(&mut selected, BStr::new(b"bash"), false).unwrap();
        assert_eq!(selected.options.dialect(), crate::options::Dialect::Posix);
        assert_eq!(ordinary.options.dialect(), crate::options::Dialect::Posix);
    }

    // [spec:nsh:req:compat.bash.default-isolation/test]
    #[test]
    fn disabling_bash_restores_default_dialect() {
        let mut shell = Shell::builder()
            .option(BStr::new(b"bash"), true)
            .build()
            .unwrap();
        assert_eq!(shell.options.dialect(), crate::options::Dialect::Bash);

        crate::options::set_option_by_name(&mut shell, BStr::new(b"bash"), false).unwrap();

        assert_eq!(shell.options.dialect(), crate::options::Dialect::Posix);
        for spec in crate::options::OPTION_SPECS {
            assert!(
                !shell.options.enabled(spec.option),
                "option {:?} was not restored",
                spec.option
            );
        }
    }

    /// The default is inert: every option off, which is what makes a
    /// library-built shell non-interactive with no job control.
    #[test]
    fn the_default_shell_has_every_option_off() {
        let shell = Shell::builder().build().unwrap();
        for spec in crate::options::OPTION_SPECS {
            assert!(
                !shell.options.enabled(spec.option),
                "option {:?} was not off",
                spec.option
            );
        }
    }

    /// `inherit_env` and `env` compose rather than conflict: the explicit
    /// pair is applied on top of the borrowed import.
    #[test]
    fn explicit_pairs_override_the_inherited_environment() {
        let (name, _) = process_probe();
        let mut shell = Shell::builder()
            .inherit_env()
            .env([(name.clone(), BString::from("explicit"))])
            .build()
            .unwrap();
        assert_eq!(
            var(&mut shell, BStr::new(name.as_slice())).as_deref(),
            Some(&b"explicit"[..])
        );
    }

    // [spec:nsh:req:embedding-safety.process-environment-is-read-only/test]
    #[test]
    fn build_preserves_process_environment() {
        let before = nsh_platform::process_environment();
        let _shell = Shell::builder()
            .inherit_env()
            .env([("LC_ALL", "C"), ("LANG", "nsh-invalid-locale")])
            .build()
            .unwrap();
        let after = nsh_platform::process_environment();
        assert_eq!(after, before);
    }
}
