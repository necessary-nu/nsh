//! Shell-option state and the `set` builtin's option scan.
//! Rules: `docs/spec/port/src/options.md`.
//!
//! Each option has a [`ShellOption`] identity and one [`model::OptionSpec`]
//! containing its long name and optional invocation letter. Runtime state is
//! an [`OptionSet`], never a byte array indexed by translated C macros.

use crate::context::Shell;
use crate::error::Error;
use crate::output::OutputDestination;
use bstr::{BStr, BString};

mod dialect;
pub(crate) use dialect::Dialect;

mod model;
pub use model::ShellOption;
pub(crate) use model::{OPTION_SPECS, OptionSet};

mod bash_options;
pub(crate) use bash_options::BashShopt;
pub(crate) use bash_options::NAMES as BASH_OPTION_NAMES;

#[cfg(test)]
mod bash_mode_tests;

/// The shell's positional parameters.
///
/// The C distinguished owned strings from a borrowed `char **` installed
/// while evaluating a function. Both cases have value semantics here: a
/// function gets a copy of its argument words, then the caller's list is
/// moved back when the function returns. This is the same observable
/// behaviour (including `shift`) without a pointer-lifetime mode.
pub struct PositionalParameters {
    pub parameter_count: usize, /* # of positional parameters (without $0) */
    pub option_index: usize,    /* next parameter to be processed by getopts */
    pub option_offset: Option<usize>, /* offset in getopts' current word */
    words: Vec<BString>,
}

impl PositionalParameters {
    pub const fn new() -> PositionalParameters {
        PositionalParameters {
            parameter_count: 0,
            option_index: 0,
            option_offset: None,
            words: Vec::new(),
        }
    }

    /// Drop the first `n` parameters: what `shift` does, in the module
    /// that knows how they are stored.
    ///
    /// A function's parameter list is a call-scoped owned copy, so shifting
    /// it mutates exactly the list that is restored away on return.
    pub(crate) fn drop_first(&mut self, n: usize) {
        self.parameter_count -= n;
        self.words.drain(..n);
        self.option_index = 1;
        self.option_offset = None;
    }

    /// Snapshot positional parameters for expansion and `getopts`.
    pub(crate) fn words(&self) -> Vec<BString> {
        self.words.clone()
    }

    fn replace(&mut self, words: Vec<BString>) {
        self.parameter_count = words.len();
        self.words = words;
        self.option_index = 1;
        self.option_offset = None;
    }
}

/// The shell's option flags — `set -e`, `set -x`, `-i` and the rest.
///
/// `docs/api-design.md` 5 calls the field `options`; the type cannot be
/// `Options` because that name is already this module's builtin option
/// *parser*, which is a different thing and stays call-scoped per 5.2.
///
/// Runtime state is addressed only by [`ShellOption`]. The metadata table is
/// declarative and cannot drift away from that typed identity.
// [spec:nsh:def:idiom.shell-options]
// [spec:nsh:req:compat.bash.state-isolation]
pub struct ShellOptions {
    pub(crate) state: OptionSet,
    /// Bash's `shopt` namespace. An explicit value is distinct from the
    /// interactive default, so `shopt -u expand_aliases` remains off in an
    /// interactive shell.
    bash_options: bash_options::BashOptions,
    /// `shellparam` — the positional parameters, `$1` onwards, and
    /// `getopts`' place in them.
    ///
    /// `pub(crate)` rather than private: `getopts.rs` and `shift.rs`
    /// read and write its members directly and there is no invariant
    /// across them the flags array has. It is here because
    /// `docs/api-design.md` §5 puts it here — one row for everything
    /// `set` and the option scan own.
    pub(crate) positional_parameters: PositionalParameters,
    /// Whether the parsed startup request contains a `-c` command.
    ///
    /// SIGINT policy distinguishes `-s -c command` from plain `-s`; the
    /// command bytes themselves live in [`crate::Startup`], not shell option
    /// state.
    pub(crate) command_source: bool,
    /// `$0`, as owned shell bytes.
    argument_zero: Option<BString>,
    /// The process invocation name before a command-file operand replaces
    /// `$0`. Output failures in the Smoosh profile identify the interpreter,
    /// not the script it is reading.
    pub(crate) invocation_name: Option<BString>,
}

impl ShellOptions {
    /// Every shell option defaults off.
    pub(crate) const fn new() -> Self {
        ShellOptions {
            state: OptionSet::EMPTY,
            bash_options: bash_options::BashOptions::new(),
            positional_parameters: PositionalParameters::new(),
            command_source: false,
            argument_zero: None,
            invocation_name: None,
        }
    }

    #[inline]
    pub(crate) const fn enabled(&self, option: ShellOption) -> bool {
        self.state.0 & option.mask() != 0
    }

    pub(crate) fn set(&mut self, option: ShellOption, enabled: bool) {
        if enabled {
            self.state.0 |= option.mask();
        } else {
            self.state.0 &= !option.mask();
        }
    }

    pub(crate) fn set_arg0(&mut self, value: &BStr) {
        let owned = value.to_owned();
        if self.invocation_name.is_none() {
            self.invocation_name = Some(owned.clone());
        }
        self.argument_zero = Some(owned);
    }

    pub(crate) fn set_invocation_name(&mut self, value: &BStr) {
        self.invocation_name = Some(value.to_owned());
    }

    pub(crate) fn argument_zero(&self) -> Option<&BStr> {
        self.argument_zero.as_deref().map(BStr::new)
    }
}

// [spec:dash:sem:options.optschanged-fn]
/// Returns rather than raising, because `setjobctl` can fail and one of
/// this function's callers is teardown. See `jobs::setjobctl`.
pub fn apply_option_changes(shell: &mut crate::context::Shell) -> Result<(), crate::error::Error> {
    crate::execution::dispatch_changed(shell);
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    crate::variables::special::dialect_changed(shell);
    crate::variables::special::options_changed(shell);
    crate::trap::set_interactive_signal_policy(
        shell,
        shell.options.enabled(ShellOption::Interactive),
    );
    crate::editor::refresh_editor_configuration(shell);
    crate::jobs::set_job_control(shell, shell.options.enabled(ShellOption::Monitor))
}

/// Apply the side effects of a changed option set.
pub(crate) fn options_changed(shell: &mut Shell) -> Result<(), Error> {
    apply_option_changes(shell)
}

/// What a `set` option scan found.
#[derive(Debug)]
pub(crate) struct Scan {
    /// The first argument the scan did not consume.
    pub(crate) next: usize,
}

// [spec:dash:sem:options.options-fn]
// [spec:posix:req:builtin.set.options-both-forms]
// [spec:posix:req:builtin.set.utility-syntax-guidelines]
// [spec:posix:req:builtin.set.opt-a-allexport]
// [spec:posix:sem:builtin.set.opt-a-separate-environments]
// [spec:posix:req:builtin.set.opt-c-noclobber]
// [spec:posix:req:builtin.set.opt-e-errexit]
// [spec:posix:req:builtin.set.opt-e-per-environment]
// [spec:posix:req:builtin.set.opt-f-noglob]
// [spec:posix:req:builtin.set.opt-h]
// [spec:posix:req:builtin.set.opt-m-monitor]
// [spec:posix:req:builtin.set.opt-n-noexec]
// [spec:posix:req:builtin.set.opt-v-verbose]
// [spec:posix:req:builtin.set.opt-x-xtrace]
// [spec:posix:req:builtin.set.options-default-off]
// [spec:posix:req:builtin.set.first-argument-hyphen]
// [spec:posix:req:builtin.set.double-hyphen]
pub(crate) fn options(
    shell: &mut crate::context::Shell,
    args: &[&BStr],
    start: usize,
) -> Result<Scan, Error> {
    let mut scan = Scan { next: start };

    while let Some(word) = args.get(scan.next) {
        scan.next += 1;
        /* `c = *p++`: the first byte decides, and the cluster starts at
         * the second. An empty word takes the `else` and is put back. */
        let prefix = word.first().copied().unwrap_or(0);
        let enabled = if prefix == b'-' {
            if word.len() == 1 || &word[..] == b"--" {
                /* "-" means turn off -x and -v */
                if word.len() == 1 {
                    shell.options.set(ShellOption::Verbose, false);
                    shell.options.set(ShellOption::Xtrace, false);
                }
                /* "--" means reset params */
                else if scan.next >= args.len() {
                    set_positional_parameters(shell, &args[scan.next..]);
                }
                break; /* "-" or "--" terminates options */
            }
            true
        } else if prefix == b'+' {
            false
        } else {
            scan.next -= 1;
            break;
        };
        let mut cluster_index = 1usize;
        while let Some(&option) = word.get(cluster_index) {
            cluster_index += 1;
            if option == b'o' {
                minus_o(shell, args.get(scan.next).copied(), enabled)?;
                if scan.next < args.len() {
                    scan.next += 1;
                }
            } else {
                set_option(shell, option, enabled)?;
            }
        }
    }

    Ok(scan)
}

// [spec:dash:sem:options.minus-o-fn]
// [spec:posix:sem:builtin.set.opt-o-report]
// [spec:posix:sem:builtin.set.plus-o-report]
// [spec:posix:req:builtin.set.opt-o-option]
// [spec:posix:def:builtin.set.opt-o-allexport]
// [spec:posix:def:builtin.set.opt-o-errexit]
// [spec:posix:req:builtin.set.opt-o-monitor]
// [spec:posix:def:builtin.set.opt-o-noglob]
// [spec:posix:def:builtin.set.opt-o-noexec]
// [spec:posix:def:builtin.set.opt-o-noclobber]
// [spec:posix:req:builtin.set.opt-o-nolog]
// [spec:posix:def:builtin.set.opt-o-notify]
// [spec:posix:def:builtin.set.opt-o-nounset]
// [spec:posix:sem:builtin.set.opt-o-pipefail]
// [spec:posix:def:builtin.set.opt-o-verbose]
// [spec:posix:req:builtin.set.opt-o-vi]

/// How one option is named and valued in a listing, which depends on the
/// dialect doing the listing.
///
/// The dialect switch is one boolean with two names. A script running in
/// Bash mode asks `set -o` and must see Bash's vocabulary: `posix`, and
/// off, because Bash has no option called `bash` and a listing that
/// invented one would be answering a question nobody asked. Outside the
/// dialect the same boolean is `nsh`'s own `bash` option, and off.
// [spec:nsh:req:compat.bash.posix-option]
pub(crate) fn presented_option(
    spec: model::OptionSpec,
    dialect: Dialect,
    options: &ShellOptions,
) -> (&'static [u8], bool) {
    let on = options.enabled(spec.option);
    if spec.option == ShellOption::Bash && dialect == Dialect::Bash {
        (b"posix", !on)
    } else {
        (spec.name, on)
    }
}

// [spec:posix:def:builtin.set.opt-o-xtrace]
fn minus_o(
    shell: &mut crate::context::Shell,
    name: Option<&BStr>,
    enabled: bool,
) -> Result<Option<ShellOption>, Error> {
    match name {
        None => {
            if enabled {
                let heading = b"Current option settings\n";
                shell.write_output(OutputDestination::Stdout, heading)?;
                let dialect = shell.options.dialect();
                for spec in OPTION_SPECS {
                    let (name, on) = presented_option(spec, dialect, &shell.options);
                    let mut line = name.to_vec();
                    if line.len() < 16 {
                        line.resize(16, b' ');
                    }
                    line.extend_from_slice(if on { b"on\n" } else { b"off\n" });
                    shell.write_output(OutputDestination::Stdout, &line)?;
                }
            } else {
                let dialect = shell.options.dialect();
                for spec in OPTION_SPECS {
                    let (name, on) = presented_option(spec, dialect, &shell.options);
                    let mut line = b"set ".to_vec();
                    line.extend_from_slice(if on { b"-o " } else { b"+o " });
                    line.extend_from_slice(name);
                    line.push(b'\n');
                    shell.write_output(OutputDestination::Stdout, &line)?;
                }
            }
        }
        Some(name) => {
            // [spec:nsh:req:compat.bash.posix-option]
            if name == b"posix".as_slice() {
                /* `posix` is the `bash` dialect option read the other way
                 * round, and it is deliberately not Bash's own reading.
                 * Bash's POSIX mode corrects about fifty behaviours where
                 * its default contradicts the standard while keeping its
                 * extensions, because POSIX permits extensions. This shell
                 * starts from the standard instead: the dialect *is* the
                 * departure, so asking for POSIX means ending it. A script
                 * that reaches for `set -o posix` after finding
                 * `$BASH_VERSION` gets a shell that conforms, not one that
                 * conforms except where it felt like not to.
                 * [dec:nsh:we-own-the-defects] */
                set_typed_option(shell, ShellOption::Bash, !enabled);
                return Ok(Some(ShellOption::Bash));
            }
            for spec in OPTION_SPECS {
                if name == spec.name {
                    set_typed_option(shell, spec.option, enabled);
                    return Ok(Some(spec.option));
                }
            }
            let mut message = b"Illegal option -o ".to_vec();
            message.extend_from_slice(name);
            return Err(shell.diagnostics().shell_error(&message));
        }
    }
    Ok(None)
}

// [spec:dash:sem:options.setoption-fn]
/// Set one option by its `set -o` long name or its single letter.
///
/// `set_option_by_name(sh, b"errexit", true)` and
/// `set_option_by_name(sh, b"e", true)` are the same option, which is what
/// [`crate::builder::Builder::option`] promises.
///
/// This is a third entry point beside `minus_o` and `setoption` rather
/// than a replacement for either, because those two are shaped by the
/// command line they parse: `minus_o` doubles as `set -o`'s *listing* when
/// it is given no name, and `setoption` carries the ksh `-V`/`-E` mutual
/// exclusion. A builder wants neither, and wants the name and the letter
/// to be one call.
///
/// The caller is responsible for `optschanged` afterwards. It is not done
/// here because a builder sets several options and the teardown that
/// `optschanged` triggers -- `setinteractive`, `histedit`, `setjobctl` --
/// should run once against the finished set, not once per option.
pub(crate) fn set_option_by_name(
    shell: &mut crate::context::Shell,
    name: &BStr,
    on: bool,
) -> Result<(), Error> {
    if name.len() == 1 {
        set_option(shell, name[0], on).map(|_| ())
    } else {
        minus_o(shell, Some(name), on).map(|_| ())
    }
}

/// The Bash dialect's own reading of an option letter.
///
/// Bash spells `errtrace` `-E` and `functrace` `-T`, and `-E` is already
/// `emacs` in the POSIX dialect. The letter therefore cannot live in
/// [`OPTION_SPECS`], which both dialects read; it is resolved here, ahead
/// of the table, and only while Bash Compatibility Mode is on.
// [spec:nsh:req:compat.bash.traps-introspection]
fn bash_option_letter(dialect: Dialect, flag: u8) -> Option<ShellOption> {
    if dialect != Dialect::Bash {
        return None;
    }
    match flag {
        b'E' => Some(ShellOption::Errtrace),
        b'T' => Some(ShellOption::Functrace),
        _ => None,
    }
}

fn set_option(
    shell: &mut crate::context::Shell,
    flag: u8,
    enabled: bool,
) -> Result<ShellOption, Error> {
    if let Some(option) = bash_option_letter(shell.options.dialect(), flag) {
        set_typed_option(shell, option, enabled);
        return Ok(option);
    }
    for spec in OPTION_SPECS {
        if spec.letter == Some(flag) {
            set_typed_option(shell, spec.option, enabled);
            return Ok(spec.option);
        }
    }
    let mut message = b"Illegal option -".to_vec();
    message.push(flag);
    Err(shell.diagnostics().shell_error(&message))
}

pub(crate) fn set_typed_option(
    shell: &mut crate::context::Shell,
    option: ShellOption,
    enabled: bool,
) {
    shell.options.set(option, enabled);
    if enabled {
        /* #%$ hack for ksh semantics */
        if option == ShellOption::Vi {
            shell.options.set(ShellOption::Emacs, false);
        } else if option == ShellOption::Emacs {
            shell.options.set(ShellOption::Vi, false);
        }
    }
}

/*
 * Set the shell parameters.
 */

// [spec:dash:sem:options.setparam-fn]
// [spec:posix:sem:param.positional-assignment]
pub fn set_positional_parameters(shell: &mut Shell, argv: &[&BStr]) {
    /* Copied out in full before the old list goes, as the C's
     * `savestr` loop is: `freeparam` comes after the copy there too. */
    let words: Vec<BString> = argv.iter().map(|word| BString::from(*word)).collect();
    shell.options.positional_parameters.replace(words);
}

/// `saveparam = shellparam`, which is a copy in the C only because
/// `shellparam.malloc = 0` on the next line disarms the `freeparam` that
/// would otherwise free what the copy still points at. One move says both.
pub fn take_positional_parameters(shell: &mut Shell) -> PositionalParameters {
    core::mem::replace(
        &mut shell.options.positional_parameters,
        PositionalParameters::new(),
    )
}

/// Drop the function's parameters and restore the caller's saved value.
pub fn restore_positional_parameters(shell: &mut Shell, saved: PositionalParameters) {
    shell.options.positional_parameters = saved;
}

/*
 * The shift builtin command.
 */

/*
 * The set command builtin.
 */

// [spec:dash:sem:options.getoptsreset-fn]
// [spec:posix:req:builtin.getopts.env-optind]
// [spec:posix:sem:builtin.getopts.reset]
pub fn reset_getopts(shell: &mut crate::context::Shell, _value: &BStr) {
    shell.options.positional_parameters.option_index = 1;
    shell.options.positional_parameters.option_offset = None;
}

/*
 * The getopts builtin.  Shellparam.optnext points to the next argument
 * to be processed.  Shellparam.optptr points to the next character to
 * be processed in the current argument.  If shellparam.optnext is NULL,
 * then it's the first time getopts has been called.
 */

/// The option scan a builtin runs over its own arguments.
///
/// This is `nextopt` with its state made local. dash keeps that state in
/// three globals -- `argptr`, `optptr` and `optionarg` -- which
/// `evalbltin` reinitialises before every builtin, and which the builtin
/// reads back after the scan to find its operands. The reinitialisation
/// is the tell: the state was never shared, only ambient, so it belongs to
/// the builtin that is scanning.
///
/// The C's comment above `nextopt` asks for it to be replaced by
/// `getopt(3)`, and says why it cannot be: the library's version keeps
/// *its* state in a process global, which a shell cannot reset portably.
/// Neither can this one, which is why it is a value.
///
/// [`Options::operands`] is the `argptr` a builtin reads afterwards.
// [spec:posix:req:xcu.options.unrecognized-diagnostic]
// [spec:posix:req:xcu.options.eight-bit-transparency]
// [spec:posix:req:xcu.operands.hyphen-means-stdin]
// [spec:posix:req:xcu.operands.processing-order]
pub struct Options<'a> {
    words: &'a [&'a BStr],
    /// The next word to look at: dash's `argptr`.
    next: usize,
    /// How far a run of clustered options has got through a word already
    /// consumed: dash's `optptr`. `None` is its NULL.
    cluster: Option<(usize, usize)>,
    /// dash's `optionarg`.
    option_argument: Option<&'a BStr>,
}

impl<'a> Options<'a> {
    /// Scan `args` from the first word after the command name, which is
    /// where `evalbltin`'s `argptr = argv + 1` starts.
    pub fn new(args: &'a [&'a BStr]) -> Self {
        Self::from(args, 1)
    }

    /// Scan from `start` when a caller has already consumed leading words.
    pub fn from(args: &'a [&'a BStr], start: usize) -> Self {
        Options {
            words: args,
            next: start.min(args.len()),
            cluster: None,
            option_argument: None,
        }
    }

    /// The next option, or `None` at the end of the options -- dash's
    /// `'\0'`.
    ///
    /// `optstring` is the C's, minus its terminator: a letter, optionally
    /// followed by `:` to say the option takes an argument.
    ///
    /// The diagnostic capability is a parameter rather than a field because
    /// `Options` borrows the caller's argument words. It exposes exactly the
    /// reporting operation needed for a bad option, not the rest of the shell.
    // [spec:dash:sem:options.nextopt-fn]
    pub fn next(
        &mut self,
        diagnostics: &mut crate::error::Diagnostics<'_>,
        optstring: &[u8],
    ) -> Result<Option<u8>, Error> {
        /* `p = optptr; if (p == NULL || *p == '\0')` -- the run in
         * progress is exhausted, so the next word starts a new one. */
        let (word_index, mut offset) = match self.cluster {
            Some((word_index, offset)) if offset < self.words[word_index].len() => {
                (word_index, offset)
            }
            _ => {
                let word_index = self.next;
                /* `p == NULL || *p != '-' || *++p == '\0'`: the end of
                 * the list, a word that is not an option, or a lone `-`.
                 * None of the three is consumed. The `?` this used to take
                 * on the `Option` is spelled out now that the scan can
                 * fail for a second reason. */
                let Some(&word) = self.words.get(word_index) else {
                    return Ok(None);
                };
                if word.first() != Some(&b'-') || word.len() < 2 {
                    return Ok(None);
                }
                self.next = word_index + 1; /* argptr++ */
                if word == b"--" {
                    /* consumed, and it ends the options */
                    return Ok(None);
                }
                (word_index, 1)
            }
        };

        let option = self.words[word_index][offset];
        offset += 1;

        /* Find `c` in the option string.  A `:` belongs to the option
         * before it, so the scan steps over one; running off the end is
         * the C reading its terminator, and the option is not ours. */
        let mut specification_index = 0usize;
        loop {
            let specification_byte = optstring.get(specification_index).copied().unwrap_or(0);
            if specification_byte == option {
                break;
            }
            if specification_byte == 0 {
                let mut message = b"Illegal option -".to_vec();
                message.push(option);
                /* A stop: the loop would spin on the terminator. */
                return Err(diagnostics.shell_error(&message));
            }
            specification_index += 1;
            if optstring.get(specification_index) == Some(&b':') {
                specification_index += 1;
            }
        }

        specification_index += 1;
        if optstring.get(specification_index) == Some(&b':') {
            /* The option takes an argument: the rest of this word if
             * there is any, otherwise the next word. */
            let bytes = self.words[word_index];
            if offset < bytes.len() {
                self.option_argument = Some(BStr::new(&bytes[offset..]));
            } else {
                match self.words.get(self.next) {
                    Some(argument) => {
                        self.option_argument = Some(argument);
                        self.next += 1;
                    }
                    None => {
                        let mut message = b"No arg for -".to_vec();
                        message.push(option);
                        message.extend_from_slice(b" option");
                        /* A stop: `arg()` would otherwise be asked for an
                         * `optionarg` that was never set. */
                        return Err(diagnostics.shell_error(&message));
                    }
                }
            }
            self.cluster = None; /* p = NULL */
        } else {
            self.cluster = Some((word_index, offset));
        }

        Ok(Some(option))
    }

    /// The argument of the option just returned: dash's `optionarg`.
    ///
    /// Only an option the option string marked with `:` has one, and
    /// [`Options::next`] raises rather than return such an option without
    /// it, so a caller that asks in the right place always gets it.
    pub fn arg(&self) -> &'a BStr {
        self.option_argument
            .expect("an option marked `:` has an argument or does not return")
    }

    /// The words the scan stopped in front of: dash's `argptr`, read back
    /// after `nextopt` has returned `'\0'`.
    pub fn operands(&self) -> &'a [&'a BStr] {
        &self.words[self.next..]
    }
}

#[cfg(test)]
mod tests;
