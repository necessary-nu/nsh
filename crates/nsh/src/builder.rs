//! Building a [`Shell`].
//!
//! `docs/api-design.md` §2. The C had no constructor: the shell *was* the
//! process, so its initial state came from `environ`, from `argv`, and
//! from whatever the kernel had already put in place. Every one of those
//! is a thing [dec:nsh:host-owns-the-process] and
//! [dec:nsh:host-owns-streams] say a library takes only when asked, so
//! each becomes a setting here and the defaults are the inert ones.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::streams::Streams;
use crate::var::EnvSource;

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
    arg0: Option<BString>,
    args: Vec<BString>,
    env: Vec<(BString, BString)>,
    inherit_env: bool,
    options: Vec<(BString, bool)>,
    cwd: Option<std::path::PathBuf>,
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
            arg0: None,
            args: Vec::new(),
            env: Vec::new(),
            inherit_env: false,
            options: Vec::new(),
            cwd: None,
            host: None,
        }
    }

    /// `$0`, and the name every diagnostic is prefixed with.
    pub fn arg0(mut self, arg0: &BStr) -> Self {
        self.arg0 = Some(arg0.to_owned());
        self
    }

    /// The positional parameters `$1`, `$2`, ….
    pub fn args(mut self, args: &[&BStr]) -> Self {
        self.args = args.iter().map(|a| (*a).to_owned()).collect();
        self
    }

    /// Variables in the initial environment, exported, as `execve` would
    /// have delivered them.
    ///
    /// Additive: two calls are the union, and a repeated name is resolved
    /// the way a repeated `environ` entry is -- the last one wins, because
    /// both go through the same `setvareq`.
    pub fn env<K, V>(mut self, vars: impl IntoIterator<Item = (K, V)>) -> Self
    where
        K: Into<BString>,
        V: Into<BString>,
    {
        self.env
            .extend(vars.into_iter().map(|(k, v)| (k.into(), v.into())));
        self
    }

    /// Take the calling process's environment.
    ///
    /// Separate from [`Builder::env`] because `std::env::vars_os` yields
    /// `OsString`, which is bytes on Unix but is not `Into<BString>`.
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
        self.options.push((name.to_owned(), on));
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
    pub fn cwd(mut self, dir: impl AsRef<std::path::Path>) -> Self {
        self.cwd = Some(dir.as_ref().to_path_buf());
        self
    }

    /// Build the shell.
    ///
    /// The order below is the constraint, not a preference. `init_from`
    /// has to come first because everything after it writes into tables it
    /// creates; the options come after the parameters because
    /// `optschanged` acts on the finished set; and the working directory
    /// comes last because `setpwd` writes `PWD` through the variable
    /// table and wants the kernel already moved.
    pub fn build(self) -> Result<Shell, Error> {
        let mut sh = Shell::try_new(self.streams).map_err(|error| {
            Error::other(0, 2, format!("cannot snapshot shell streams: {error}").as_bytes())
        })?;
        /* Before anything else the host might be asked about. `attach` is
         * specified as happening exactly once, and this is it: the sink is
         * the only part of the shell a signal handler may touch, so the
         * host has to be holding it before a handler could exist. */
        if let Some(host) = self.host {
            sh.host = host;
        }
        sh.host.attach(crate::siginbox::signals());
        let source = if self.inherit_env {
            EnvSource::Process
        } else {
            EnvSource::Explicit(&self.env)
        };
        crate::init::init_from(&mut sh, source)?;

            /* `inherit_env` picked the process snapshot, so explicit pairs
             * have not been applied yet. They go on top, which is what
             * makes the two settings compose rather than conflict. */
            if self.inherit_env && !self.env.is_empty() {
                crate::var::mkinit_env_pairs(&mut sh, &self.env)?;
            }

            if let Some(arg0) = &self.arg0 {
                sh.options.set_arg0(BStr::new(&arg0[..]));
            }

            if !self.args.is_empty() {
                let refs: Vec<&BStr> =
                    self.args.iter().map(|a| BStr::new(&a[..])).collect();
                crate::options::setparam(&mut sh, &refs);
            }

            for (name, on) in &self.options {
                crate::options::set_option_by_name(&mut sh, BStr::new(&name[..]), *on)?;
            }
            crate::options::optschanged(&mut sh)?;

        if let Some(dir) = &self.cwd {
                /* `Error::Other` because the taxonomy's `Io` variant is
                 * not promoted yet -- §3.4's "start with `Other`, promote
                 * the interesting ones after". Status 2 is what dash's
                 * `sh_error` takes, and what a failed `cd` leaves. */
                std::env::set_current_dir(dir).map_err(|e| {
                    Error::other(
                        0,
                        2,
                        format!("can't cd to {}: {}", dir.display(), e).as_bytes(),
                    )
                })?;
            crate::cd::setpwd_inner(&mut sh, crate::cd::Pwd::Unknown, 0)?;
        }
        Ok(sh)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `lookupvar` as a byte string, or `None` when the shell has no such
    /// variable.
    fn var(sh: &mut Shell, name: &str) -> Option<Vec<u8>> {
        crate::var::lookup_bytes(sh, bstr::BStr::new(name)).map(Vec::from)
    }

    #[test]
    fn a_builder_with_no_env_setting_inherits_nothing_from_the_process() {
        nsh_platform::set_process_environment(
            std::ffi::OsStr::new("NSH_BUILDER_PROBE"),
            std::ffi::OsStr::new("from-the-process"),
        );
        let mut sh = Shell::builder().build().unwrap();
        assert_eq!(var(&mut sh, "NSH_BUILDER_PROBE"), None);
    }

    #[test]
    fn inherit_env_takes_the_processs_environment() {
        {
            nsh_platform::set_process_environment(
                std::ffi::OsStr::new("NSH_BUILDER_PROBE2"),
                std::ffi::OsStr::new("from-the-process"),
            );
            let mut sh = Shell::builder().inherit_env().build().unwrap();
            assert_eq!(
                var(&mut sh, "NSH_BUILDER_PROBE2").as_deref(),
                Some(&b"from-the-process"[..])
            );
        }
    }

    #[test]
    fn explicit_pairs_are_set_and_exported() {
        let mut sh = Shell::builder()
            .env([("NSH_EXPLICIT", "a value with spaces")])
            .build()
            .unwrap();
        assert_eq!(
            var(&mut sh, "NSH_EXPLICIT").as_deref(),
            Some(&b"a value with spaces"[..])
        );
    }

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
        assert_eq!(by_name.options.flag(crate::options::eflag), 1);
        assert_eq!(
            by_name.options.flag(crate::options::eflag),
            by_letter.options.flag(crate::options::eflag)
        );
    }

    /// The default is inert: every option off, which is what makes a
    /// library-built shell non-interactive with no job control.
    #[test]
    fn the_default_shell_has_every_option_off() {
        let sh = Shell::builder().build().unwrap();
        for i in 0..crate::options::NOPTS {
            assert_eq!(sh.options.flag(i), 0, "option {i} was not off");
        }
    }

    /// `inherit_env` and `env` compose rather than conflict: the explicit
    /// pair is applied on top of the borrowed import.
    #[test]
    fn explicit_pairs_override_the_inherited_environment() {
        {
            nsh_platform::set_process_environment(
                std::ffi::OsStr::new("NSH_BUILDER_PROBE3"),
                std::ffi::OsStr::new("inherited"),
            );
            let mut sh = Shell::builder()
                .inherit_env()
                .env([("NSH_BUILDER_PROBE3", "explicit")])
                .build()
                .unwrap();
            assert_eq!(
                var(&mut sh, "NSH_BUILDER_PROBE3").as_deref(),
                Some(&b"explicit"[..])
            );
        }
    }
}
