//! Command lookup, hashing and external-program execution.
//! Rules: `docs/spec/port/src/exec.md`.
//!
//! `cmdtable` is a `BTreeMap` keyed by command name, not the C's 31
//! chained hash buckets, so `hash` with no operands prints in name order
//! rather than in the order `hashval` happens to chain. Registered in
//! `docs/divergences.md`. Commands are represented by a Rust enum; no
//! tagged union or borrowed C argument vector crosses this module.

use bstr::{BStr, BString, ByteSlice};
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};
use std::collections::BTreeMap;
use std::ffi::{OsStr, OsString};
use std::path::Path;

use crate::builtins::BuiltinSpec;
use crate::error::{Error, Operation};
use crate::nodes::FunctionDefinition;

mod dialect_dispatch;
pub(crate) use dialect_dispatch::dispatch_changed;

#[cfg(test)]
mod bash_dispatch_tests;

/// Semantic controls for command lookup.
///
/// These used to be the `DO_*` integer bitmask copied from dash.  Keeping the
/// questions as named booleans makes invalid or accidental flag combinations
/// impossible to manufacture with arithmetic.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct CommandSearch {
    report_errors: bool,
    check_absolute: bool,
    skip_functions: bool,
    alternate_path: bool,
    regular_builtins_only: bool,
}

impl CommandSearch {
    pub const DEFAULT: Self = Self {
        report_errors: false,
        check_absolute: false,
        skip_functions: false,
        alternate_path: false,
        regular_builtins_only: false,
    };

    pub const fn reporting_errors(mut self) -> Self {
        self.report_errors = true;
        self
    }

    pub const fn checking_absolute(mut self) -> Self {
        self.check_absolute = true;
        self
    }

    pub const fn skipping_functions(mut self) -> Self {
        self.skip_functions = true;
        self
    }

    pub const fn using_alternate_path(mut self) -> Self {
        self.alternate_path = true;
        self
    }

    pub const fn regular_builtins_only(mut self) -> Self {
        self.regular_builtins_only = true;
        self
    }
}

// ---------------------------------------------------------------------
// src/exec.h types
// ---------------------------------------------------------------------

// The C union is represented by `Command`; its tag and payload cannot
// disagree in Rust.

// [spec:nsh:req:idiom.command-dispatch]
#[derive(Clone)]
pub(crate) enum Command {
    Unknown,
    External { path_index: Option<usize> },
    // [spec:nsh:req:idiom.structural-ast]
    Function(FunctionDefinition),
    Builtin(&'static BuiltinSpec),
}

/// One cached command resolution.
///
/// The name is the `BTreeMap` key and the command kind is an enum, so the
/// intrusive `next` pointer, flexible array tail and untagged internal union
/// all disappear. Values are boxed because `find_command` keeps their address
/// across operations that can insert another command and rebalance the map.
pub struct CommandEntry {
    pub(crate) command: Command,
    pub(crate) rehash: bool,
}

impl CommandEntry {
    /// `builtin_location` arrives as a value rather than being read here, and
    /// that is not a style choice: the only caller is `clearcmdentry`'s
    /// `retain`, whose closure already holds the table borrowed. Reading
    /// the sibling field from inside it would be a second borrow of the
    /// same `Shell`. Copying the option out first is the whole fix, and
    /// it is exact -- nothing in the closure can change it.
    pub(crate) fn path_dependent(&self, builtin_location: Option<usize>) -> bool {
        match self.command {
            Command::External { .. } => true,
            Command::Builtin(cmd) => {
                !cmd.attributes().is_regular() && builtin_location.is_some_and(|index| index > 0)
            }
            _ => false,
        }
    }
}

// ---------------------------------------------------------------------
// module globals
// ---------------------------------------------------------------------

/// The command hash, and where `%builtin` sits in `PATH`.
///
/// The two are one field because they are one question -- "what does
/// this name run" -- and because `clearcmdentry` reads the second while
/// rebuilding the first. `docs/api-design.md` 5 groups them, and
/// function definitions live here too because dash stores them in the
/// same hash.
pub struct CommandTable {
    /// Command names are shell bytes, without an artificial C terminator.
    map: BTreeMap<BString, CommandEntry>,
    /// Index in `PATH` of `%builtin`, when present.
    builtin_location: Option<usize>,
    /// Dialect under which cached built-in entries were classified.
    dispatch_dialect: crate::options::Dialect,
}

impl CommandTable {
    /// An empty hash with no `%builtin` component.
    pub(crate) const fn new() -> Self {
        CommandTable {
            map: BTreeMap::new(),
            builtin_location: None,
            dispatch_dialect: crate::options::Dialect::Posix,
        }
    }

    /// Whether an entry would be invalidated by a `PATH` change.
    ///
    /// Reads `builtin_location` on the caller's behalf, so a caller holding an
    /// entry does not have to reach for the sibling field itself. The
    /// two in-module walks cannot use this — they hold the map borrowed
    /// and take the option by value instead.
    pub(crate) fn path_dependent(&self, command_entry: &CommandEntry) -> bool {
        command_entry.path_dependent(self.builtin_location)
    }

    /// Every entry, in name order — what `hash` with no operand prints.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&BString, &CommandEntry)> {
        self.map.iter()
    }

    pub(crate) fn get(&self, name: &BStr) -> Option<&CommandEntry> {
        self.map.get(name)
    }

    pub(crate) fn resolved(&self, name: &BStr) -> Option<Command> {
        self.get(name).map(|entry| entry.command.clone())
    }
}

// ---------------------------------------------------------------------

/*
 * Exec a program.  Never returns.  If you change this routine, you may
 * have to change the find_command routine as well.
 */

// [spec:dash:sem:exec.shellexec-fn]
// [spec:posix:req:xcu.env.utility-selection-path-search]
// [spec:posix:req:shenv.utility-environment]
// [spec:posix:req:xcurel.file-removal-active-directory]
// [spec:posix:req:xcurel.file-removal-effects]
// [spec:posix:req:xcurel.file-time-values]
// [spec:posix:req:xcurel.mathematical-functions]
// [spec:posix:req:cmd.nonbuiltin-exec-replaces-environment]
// [spec:posix:req:cmd.nonbuiltin-path-search-execl]
// [spec:posix:req:cmd.nonbuiltin-invalid-name-env-unspecified]
// [spec:posix:req:cmd.nonbuiltin-path-search-unsuccessful]
// [spec:posix:req:cmd.nonbuiltin-slash-execl]
// [spec:posix:req:cmd.nonbuiltin-slash-not-found]
// [spec:posix:req:cmd.std-fd-closed]
// [spec:posix:req:cmd.std-fd-nonconforming-environment]
// [spec:nsh:req:idiom.platform-errors]
pub fn execute_external_command(
    shell: &mut crate::context::Shell,
    arguments: &[&BStr],
    path: &BStr,
    path_index: Option<usize>,
) -> Result<crate::evaluation::Flow, crate::error::Error> {
    let command = arguments.first().expect("shellexec needs a command name");

    /* A library shell may fork children, but it may not replace the host
     * process itself without the host's explicit grant. A forked shell owns
     * the child terminus regardless of which host policy its parent used. */
    if shell.shell_level == 0 && !shell.host.may_replace_process() {
        return exec_failure(
            shell,
            command,
            nsh_platform::platform_error(nsh_platform::PlatformErrorKind::PermissionDenied),
        );
    }

    /* The C's `environment()` leaves its array in the stack allocator; ours
     * owns native strings, so the `Vec` has to outlive every execution
     * attempt below. */
    let envv = match crate::variables::environment(shell) {
        Ok(environment) => environment,
        Err(error) => return native_exec_failure(shell, command, &error),
    };
    let arguments: Vec<OsString> = match arguments
        .iter()
        .map(|word| word.try_to_os_string())
        .collect::<std::io::Result<_>>()
    {
        Ok(arguments) => arguments,
        Err(error) => return native_exec_failure(shell, command, &error),
    };
    /* The last fork this process will ever make is behind us, so a
     * `<(list)` name may now stop being close-on-exec. Doing it here rather
     * than when the name was built is what keeps the pipe out of every
     * unrelated child the shell forked in between. */
    // [spec:nsh:req:compat.bash.process-substitution]
    if let Err(error) = crate::evaluation::bash_process_substitution::publish_before_exec(shell) {
        return exec_failure(shell, command, error);
    }
    if let Err(error) = shell.descriptors.materialize() {
        return exec_failure(shell, command, error);
    }
    let error = if nsh_platform::shell_path_has_separator(command) {
        let resolved = nsh_platform::resolve_command_path(Path::new(&arguments[0]), &envv);
        try_external_candidate(resolved.as_os_str(), &arguments, &envv)
    } else {
        let mut search_error =
            nsh_platform::platform_error(nsh_platform::PlatformErrorKind::NotFound);
        let mut cursor = PathCursor::new(path);
        let mut candidate_index = 0usize;
        while let Some(candidate) = cursor.advance(command) {
            if candidate_index >= path_index.unwrap_or(0) && candidate.option.is_none() {
                let candidate = match candidate.path.try_to_path_buf() {
                    Ok(candidate) => candidate,
                    Err(error) => return native_exec_failure(shell, command, &error),
                };
                let candidate = nsh_platform::resolve_command_path(&candidate, &envv);
                let candidate_error =
                    try_external_candidate(candidate.as_os_str(), &arguments, &envv);
                if !nsh_platform::is_path_error(
                    &candidate_error,
                    nsh_platform::PathErrorKind::NotFound,
                ) {
                    search_error = candidate_error;
                }
            }
            candidate_index += 1;
        }
        search_error
    };

    exec_failure(shell, command, error)
}

fn native_exec_failure(
    shell: &mut crate::context::Shell,
    command: &BStr,
    error: &std::io::Error,
) -> Result<crate::evaluation::Flow, crate::error::Error> {
    let status = crate::status::ExitStatus::NOT_EXECUTABLE;
    shell.status = status;
    let mut message = command.to_vec();
    message.extend_from_slice(b": ");
    message.extend_from_slice(shell.locale.error_message(error).as_bytes());
    let diagnostic = crate::error::Error::other(shell.evaluation.diagnostic_line, status, &message);
    drop(shell.diagnostics().report(diagnostic));
    Ok(crate::evaluation::Flow::END)
}

// [spec:posix:req:exit.status-command-not-found]
// [spec:posix:req:exit.status-not-executable]
fn exec_failure(
    shell: &mut crate::context::Shell,
    command: &BStr,
    error: std::io::Error,
) -> Result<crate::evaluation::Flow, crate::error::Error> {
    /* Map to POSIX errors */
    let execution_status = nsh_platform::command_exec_failure_status(&error);
    shell.status = crate::status::ExitStatus::from_code(execution_status);
    let mut message = Vec::new();
    message.extend_from_slice(command);
    message.extend_from_slice(b": ");
    message.extend_from_slice(&crate::error::error_message(
        &shell.locale,
        &error,
        Operation::Execute,
    ));
    /* `exerror(EXEND, msg)`: text *and* control flow, which is why the
     * bridge took the code as a parameter rather than reading it off the
     * value. The text is written here, where dash writes it, and the value
     * it rendered from is dropped -- an `exec` that cannot happen ends the
     * shell, and `docs/api-design.md` 3.3 is explicit that what ends the
     * run is `Flow`, not `Err`. */
    /* Built before the call rather than inside its argument list: the
     * receiver is borrowed for the whole call, so reading the line out of
     * the same shell in an argument is a conflict. */
    let error =
        crate::error::Error::other(shell.evaluation.diagnostic_line, execution_status, &message);
    drop(shell.diagnostics().report(error));

    Ok(crate::evaluation::Flow::END)
}

// [spec:dash:sem:exec.tryexec-fn]
// [spec:posix:req:cmd.nonbuiltin-enoexec-script]
// [spec:posix:req:cmd.nonbuiltin-slash-enoexec-script]
fn try_external_candidate(
    command: &OsStr,
    arguments: &[OsString],
    env: &[(OsString, OsString)],
) -> std::io::Error {
    let program = nsh_platform::ProgramImage::new(command.into(), arguments.to_vec(), env.to_vec());
    let error = nsh_platform::execute_program(program);
    let shell = nsh_platform::fallback_shell();
    if nsh_platform::is_exec_format_error(&error) && command != shell {
        let mut shell_arguments = Vec::with_capacity(arguments.len() + 1);
        shell_arguments.push(shell.to_os_string());
        shell_arguments.push(command.to_os_string());
        shell_arguments.extend(arguments.iter().skip(1).cloned());
        let program = nsh_platform::ProgramImage::new(shell.into(), shell_arguments, env.to_vec());
        return nsh_platform::execute_program(program);
    }
    error
}

// [spec:dash:sem:exec.legal-pathopt-fn]
fn legal_path_option(option: &[u8]) -> bool {
    option.starts_with(b"builtin") || option.starts_with(b"func")
}

/*
 * Do a path search.  The variable path (passed by reference) should be
 * set to the start of the path before the first call; padvance will update
 * this value as it proceeds.  Successive calls to padvance will return
 * the possible path expansions in sequence. An option (indicated by a
 * percent sign) is returned with the candidate instead of being published
 * through process-global scratch state.
 *
 * If magic is 0 then pathopt recognition will be disabled.  If magic is
 * 1 we shall recognise %builtin/%func.  Otherwise we shall accept any
 * pathopt.
 */

/// Stateful walk over the components of one `PATH` value.
pub struct PathCursor<'a> {
    remaining: Option<&'a [u8]>,
    magic: bool,
}

impl<'a> PathCursor<'a> {
    pub fn new(path: &'a BStr) -> Self {
        Self {
            remaining: Some(path.as_bytes()),
            magic: true,
        }
    }

    pub fn literal(path: &'a BStr) -> Self {
        Self {
            remaining: Some(path.as_bytes()),
            magic: false,
        }
    }

    // [spec:dash:sem:exec.padvance-magic-fn]
    // [spec:dash:sem:exec.padvance-fn]
    pub fn advance(&mut self, name: &BStr) -> Option<PathAdvance> {
        let rest = self.remaining.take()?;
        let separator = nsh_platform::search_path_separator();
        let (component, remaining) = match rest.find_byte(separator) {
            Some(at) => (&rest[..at], Some(&rest[at + 1..])),
            None => (rest, None),
        };
        self.remaining = remaining;

        let mut directory = component;
        let mut option = None;
        if self.magic {
            if let Some(stripped) = component.strip_prefix(b"%") {
                if legal_path_option(stripped) {
                    option = Some(BString::from(stripped));
                    directory = if stripped.starts_with(b"builtin") {
                        &stripped[b"builtin".len()..]
                    } else {
                        &stripped[b"func".len()..]
                    };
                }
            } else if let Some(percent) = component.find_byte(b'%') {
                let candidate = &component[percent + 1..];
                if legal_path_option(candidate) {
                    option = Some(BString::from(candidate));
                    directory = &component[..percent];
                }
            }
        }

        let capacity = directory.len() + name.len() + usize::from(!directory.is_empty());
        let mut path = BString::new(Vec::with_capacity(capacity));
        if !directory.is_empty() {
            path.extend_from_slice(directory);
            path.push(nsh_platform::shell_directory_separator());
        }
        path.extend_from_slice(name);

        Some(PathAdvance { path, option })
    }
}

/// One independently owned result from a PATH walk.
pub struct PathAdvance {
    /// Candidate path bytes.
    pub path: BString,
    /// `%option`, if the PATH element carried one.
    pub option: Option<BString>,
}

/*** Command hashing code ***/

// [spec:dash:sem:exec.test-exec-fn]
fn is_executable_candidate(full_path: &[u8], metadata: &nsh_platform::FileMetadata) -> bool {
    if metadata.kind != nsh_platform::FileKind::Regular {
        return false;
    }

    if (metadata.mode & 0o111) != 0o111
        && !crate::builtins::test::test_file_access(
            full_path.as_bstr(),
            nsh_platform::AccessMode::EXEC_OK,
        )
    {
        return false;
    }

    true
}

/*
 * Resolve a command name.  If you change this routine, you may have to
 * change the shellexec routine as well.
 */

// [spec:dash:sem:exec.find-command-fn]
// [spec:posix:req:cmd.search-applies]
// [spec:posix:req:cmd.search-special-builtin]
// [spec:posix:req:cmd.search-unspecified-utility-names]
// [spec:posix:req:cmd.search-function]
// [spec:posix:req:cmd.search-intrinsic-utility]
// [spec:posix:req:cmd.search-path-associated-builtin]
// [spec:posix:req:cmd.search-path-non-builtin]
// [spec:posix:req:cmd.search-remembered-location]
// [spec:posix:req:cmd.search-path-unsuccessful]
// [spec:posix:req:cmd.search-name-with-slash]
pub fn find_command(
    shell: &mut crate::context::Shell,
    name: &BStr,
    entry: &mut Command,
    mut search: CommandSearch,
    path: &BStr,
) -> Result<crate::evaluation::Flow, Error> {
    let dialect = shell.options.dialect();
    shell.commands.ensure_dispatch(dialect);

    /* If name contains a slash, don't use PATH or hash table */
    if nsh_platform::shell_path_has_separator(name) {
        if search.check_absolute {
            let environment = crate::variables::environment(shell)
                .map_err(|error| native_string_error(shell, name, &error))?;
            let native = name
                .try_to_path_buf()
                .map_err(|error| native_string_error(shell, name, &error))?;
            let resolved = nsh_platform::resolve_command_path(&native, &environment);
            let resolved_bytes = resolved.to_shell_bytes();
            let executable = nsh_platform::path_metadata(&resolved, true)
                .is_ok_and(|metadata| is_executable_candidate(&resolved_bytes, &metadata));
            if !executable {
                *entry = Command::Unknown;
                return Ok(crate::evaluation::Flow::Done((0).into()));
            }
        }
        *entry = Command::External { path_index: None };
        return Ok(crate::evaluation::Flow::Done((0).into()));
    }

    let configured_path = crate::variables::path_value(shell);
    let mut update_table = path.as_bytes() == configured_path.as_slice();
    if !update_table {
        search = search.using_alternate_path();
    }

    let mut cached = shell
        .commands
        .map
        .get(name)
        .map(|stored| (stored.command.clone(), stored.rehash));

    if let Some((command, rehash)) = &cached {
        let conflicts_with_search = match command {
            Command::Function(_) => search.skip_functions,
            Command::Builtin(command) => {
                !command.attributes().is_regular() && search.regular_builtins_only
            }
            Command::External { .. } | Command::Unknown => {
                search.alternate_path || search.regular_builtins_only
            }
        };
        if conflicts_with_search {
            if search.regular_builtins_only && !matches!(command, Command::Function(_)) {
                *entry = Command::Unknown;
                return Ok(crate::evaluation::Flow::Done((0).into()));
            }
            update_table = false;
            cached = None;
        } else if !rehash {
            *entry = command.clone();
            return Ok(crate::evaluation::Flow::Done((0).into()));
        }
    }

    let builtin_command = builtin(shell, name);
    if let Some(command) = builtin_command
        && (command.attributes().is_regular()
            || search.alternate_path
            || !shell
                .commands
                .builtin_location
                .is_some_and(|index| index > 0))
    {
        if update_table {
            cache_command(&mut shell.commands, name, Command::Builtin(command));
        }
        *entry = Command::Builtin(command);
        return Ok(crate::evaluation::Flow::Done((0).into()));
    }

    if search.regular_builtins_only {
        *entry = Command::Unknown;
        return Ok(crate::evaluation::Flow::Done((0).into()));
    }

    let environment = crate::variables::environment(shell)
        .map_err(|error| native_string_error(shell, name, &error))?;
    let previous: Option<usize> =
        cached
            .as_ref()
            .filter(|(_, rehash)| *rehash)
            .and_then(|(command, _)| match command {
                Command::Builtin(_) => shell.commands.builtin_location,
                Command::External { path_index } => *path_index,
                _ => None,
            });
    let mut error = nsh_platform::platform_error(nsh_platform::PlatformErrorKind::NotFound);
    let mut cursor = PathCursor::new(path);
    let mut index = 0usize;
    while let Some(candidate) = cursor.advance(name) {
        let candidate_index = index;
        index += 1;
        if let Some(option) = &candidate.option {
            if option.first() == Some(&b'b') {
                if let Some(command) = builtin_command {
                    if update_table {
                        cache_command(&mut shell.commands, name, Command::Builtin(command));
                    }
                    *entry = Command::Builtin(command);
                    return Ok(crate::evaluation::Flow::Done((0).into()));
                }
                continue;
            }
            if search.skip_functions {
                continue;
            }
        }

        let full_path = candidate.path;
        if nsh_platform::shell_path_is_absolute(&full_path)
            && previous.is_some_and(|previous| candidate_index <= previous)
        {
            if previous.is_some_and(|previous| candidate_index < previous) {
                continue;
            }
            if let Some((command, _)) = cached {
                if let Some(stored) = shell.commands.map.get_mut(name) {
                    stored.rehash = false;
                }
                *entry = command;
                return Ok(crate::evaluation::Flow::Done((0).into()));
            }
        }

        let native = full_path.try_to_path_buf().map_err(|io_error| {
            native_string_error(shell, BStr::new(full_path.as_slice()), &io_error)
        })?;
        let resolved = nsh_platform::resolve_command_path(&native, &environment);
        let resolved_bytes = resolved.to_shell_bytes();
        let metadata = match nsh_platform::path_metadata(&resolved, true) {
            Ok(metadata) => metadata,
            Err(io_error) => {
                if !nsh_platform::is_path_error(&io_error, nsh_platform::PathErrorKind::NotFound) {
                    error = io_error;
                }
                continue;
            }
        };

        if candidate.option.is_some() {
            let flow = crate::runtime::read_command_file(shell, BStr::new(full_path.as_slice()))?;
            if let exit @ crate::evaluation::Flow::Exit { .. } = flow {
                return Ok(exit);
            }
            let Some(stored) = shell.commands.map.get_mut(name) else {
                let mut message = name.to_vec();
                message.extend_from_slice(b" not defined in ");
                message.extend_from_slice(&full_path);
                return Err(shell.diagnostics().shell_error(&message));
            };
            if !matches!(stored.command, Command::Function(_)) {
                let mut message = name.to_vec();
                message.extend_from_slice(b" not defined in ");
                message.extend_from_slice(&full_path);
                return Err(shell.diagnostics().shell_error(&message));
            }
            stored.rehash = false;
            *entry = stored.command.clone();
            return Ok(crate::evaluation::Flow::Done((0).into()));
        }

        error = nsh_platform::platform_error(nsh_platform::PlatformErrorKind::PermissionDenied);
        if !is_executable_candidate(&resolved_bytes, &metadata) {
            continue;
        }
        if update_table {
            cache_command(
                &mut shell.commands,
                name,
                Command::External {
                    path_index: Some(candidate_index),
                },
            );
        }
        *entry = Command::External {
            path_index: Some(candidate_index),
        };
        return Ok(crate::evaluation::Flow::Done((0).into()));
    }

    if cached.is_some() && update_table {
        remove_command_entry(&mut shell.interrupt_deferral, &mut shell.commands, name);
    }
    if search.report_errors {
        let mut message = name.to_vec();
        message.extend_from_slice(b": ");
        message.extend_from_slice(&crate::error::error_message(
            &shell.locale,
            &error,
            Operation::Execute,
        ));
        shell.diagnostics().shell_warning(&message);
    }
    *entry = Command::Unknown;
    Ok(crate::evaluation::Flow::Done((0).into()))
}

fn native_string_error(
    shell: &mut crate::context::Shell,
    subject: &BStr,
    error: &std::io::Error,
) -> Error {
    let mut message = subject.to_vec();
    message.extend_from_slice(b": ");
    message.extend_from_slice(shell.locale.error_message(error).as_bytes());
    shell.diagnostics().shell_error(&message)
}

/*
 * Search the table of builtin commands.
 */

// [spec:dash:sem:exec.find-builtin-fn]
pub fn builtin(shell: &crate::context::Shell, name: &BStr) -> Option<&'static BuiltinSpec> {
    if shell.options.dialect() == crate::options::Dialect::Bash
        && let Ok(index) =
            crate::builtins::BASH_BUILTINS.binary_search_by(|cmd| cmd.name().cmp(name))
    {
        return Some(&crate::builtins::BASH_BUILTINS[index]);
    }
    crate::builtins::BUILTINS
        .binary_search_by(|cmd| cmd.name().cmp(name))
        .ok()
        .map(|index| &crate::builtins::BUILTINS[index])
}

/*
 * Called when a cd is done.  Marks all commands so the next time they
 * are executed they will be rehashed.
 */

// [spec:dash:sem:exec.hashcd-fn]
pub fn invalidate_cache_after_directory_change(shell: &mut crate::context::Shell) {
    /* Copied out for the same reason `clearcmdentry` copies it: the
     * walk below holds the table borrowed. */
    let builtin_location = shell.commands.builtin_location;
    for command_entry in shell.commands.map.values_mut() {
        if command_entry.path_dependent(builtin_location) {
            command_entry.rehash = true;
        }
    }
}

/*
 * Fix command hash table when PATH changed.
 * Called before PATH is changed.  The argument is the new value of PATH;
 * pathval() still returns the old value at this point.
 * Called with interrupts off.
 */

// [spec:dash:sem:exec.changepath-fn]
pub fn update_search_path(
    interrupts: &mut crate::error::InterruptDeferral,
    commands: &mut CommandTable,
    newval: &BStr,
) {
    let builtin_location = newval
        .split(|&byte| byte == nsh_platform::search_path_separator())
        .position(|component| component.starts_with(b"%builtin"));
    commands.builtin_location = builtin_location;
    clear_command_cache(interrupts, commands);
}

/*
 * Clear out command entries.  The argument specifies the first entry in
 * PATH which has changed.
 */

// [spec:dash:sem:exec.clearcmdentry-fn]
pub(crate) fn clear_command_cache(
    interrupts: &mut crate::error::InterruptDeferral,
    commands: &mut CommandTable,
) {
    interrupts.run_with(commands, |commands| {
        let builtin_location = commands.builtin_location;
        commands
            .map
            .retain(|_, command_entry| !command_entry.path_dependent(builtin_location));
    });
}

/*
 * Locate a command in the command hash table.  If "add" is nonzero,
 * add the command to the table if it is not already present.  The
 * Interrupts must be off if called with add != 0.
 */

// [spec:dash:sem:exec.cmdlookup-fn]
pub(crate) fn lookup_cached_command<'a>(
    commands: &'a mut CommandTable,
    name: &BStr,
    add: bool,
) -> Option<&'a mut CommandEntry> {
    if add {
        Some(
            commands
                .map
                .entry(name.to_owned())
                .or_insert_with(|| CommandEntry {
                    command: Command::Unknown,
                    rehash: false,
                }),
        )
    } else {
        commands.map.get_mut(name)
    }
}

/*
 * Delete a command table entry by name.
 */

// [spec:dash:sem:exec.delete-cmd-entry-fn]
pub(crate) fn remove_command_entry(
    interrupts: &mut crate::error::InterruptDeferral,
    commands: &mut CommandTable,
    name: &BStr,
) {
    interrupts.run_with(commands, |commands| {
        commands.map.remove(name);
    });
}

/*
 * Add a new command entry, replacing any existing command entry for
 * the same name - except special builtins.
 */

// [spec:dash:sem:exec.addcmdentry-fn]
fn cache_command(commands: &mut CommandTable, name: &BStr, command: Command) {
    let command_entry =
        lookup_cached_command(commands, name, true).expect("adding returns an entry");
    command_entry.command = command;
    command_entry.rehash = false;
}

/*
 * Define a shell function.
 */

// [spec:dash:sem:exec.defun-fn]
pub fn define_function(
    interrupts: &mut crate::error::InterruptDeferral,
    commands: &mut CommandTable,
    definition: &FunctionDefinition,
) {
    interrupts.run_with(commands, |commands| {
        cache_command(
            commands,
            definition.name.as_bstr(),
            Command::Function(definition.clone()),
        );
    });
}

/*
 * Delete a function if it exists.
 */

// [spec:dash:sem:exec.unsetfunc-fn]
pub fn unset_function(
    interrupts: &mut crate::error::InterruptDeferral,
    commands: &mut CommandTable,
    name: &BStr,
) {
    if lookup_cached_command(commands, name, false)
        .is_some_and(|command_entry| matches!(command_entry.command, Command::Function(_)))
    {
        remove_command_entry(interrupts, commands, name);
    }
}

/*
 * Locate and print what a word is...
 */

#[cfg(test)]
mod tests {
    use super::*;

    /// `%builtin`'s position in `PATH` is what makes a cached entry
    /// stale, and `changepath` is the variable hook that finds it. This
    /// is the field the hook could not reach before it carried a
    /// receiver.
    // [spec:dash:sem:exec.changepath-fn/test]
    #[test]
    fn changepath_files_the_builtin_slot() {
        let _g = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;

        let separator = [nsh_platform::search_path_separator()];
        let first = [b"/bin".as_slice(), b"%builtin", b"/usr/bin"].join(separator.as_slice());
        update_search_path(
            &mut shell.interrupt_deferral,
            &mut shell.commands,
            BStr::new(&first),
        );
        assert_eq!(shell.commands.builtin_location, Some(1));

        let second = [b"%builtin".as_slice(), b"/bin"].join(separator.as_slice());
        update_search_path(
            &mut shell.interrupt_deferral,
            &mut shell.commands,
            BStr::new(&second),
        );
        assert_eq!(shell.commands.builtin_location, Some(0));

        let third = [b"/bin".as_slice(), b"/usr/bin"].join(separator.as_slice());
        update_search_path(
            &mut shell.interrupt_deferral,
            &mut shell.commands,
            BStr::new(&third),
        );
        assert_eq!(shell.commands.builtin_location, None);
    }

    /// What `clearcmdentry` keeps, which is the predicate the walk runs
    /// while it holds the table borrowed. An external command is always
    /// invalidated by a `PATH` change; an entry that names nothing is
    /// not. Pinned because the built-in location the predicate reads is now
    /// copied out before the walk rather than read inside it, and a
    /// wrong copy would show up here as the wrong survivor.
    // [spec:dash:sem:exec.clearcmdentry-fn/test]
    #[test]
    fn clearing_drops_only_path_dependent_entries() {
        let _g = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;

        let external = BStr::new(b"Texternal");
        let unknown = BStr::new(b"Tunknown");
        cache_command(
            &mut shell.commands,
            external,
            Command::External {
                path_index: Some(0),
            },
        );
        lookup_cached_command(&mut shell.commands, unknown, true);

        let error = shell.commands.get(external).expect("external entry");
        assert!(shell.commands.path_dependent(error));
        let u = shell.commands.get(unknown).expect("unknown entry");
        assert!(!shell.commands.path_dependent(u));

        clear_command_cache(&mut shell.interrupt_deferral, &mut shell.commands);

        assert!(
            shell.commands.get(external).is_none(),
            "an external command does not survive a PATH change"
        );
        assert!(
            shell.commands.get(unknown).is_some(),
            "an entry naming nothing has nothing to invalidate"
        );
    }

    // [spec:dash:sem:exec.find-builtin-fn/test]
    #[test]
    fn builtin_lookup_round_trips() {
        let shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        for expected in crate::builtins::BUILTINS {
            assert!(core::ptr::eq(
                builtin(&shell, expected.name()).expect("registered builtin"),
                expected,
            ));
        }

        for absent in [b"" as &[u8], b"/", b"alia", b"aliasx", b"waitx", b"zz"] {
            assert!(builtin(&shell, BStr::new(absent)).is_none());
        }
    }

    /// `printf` is a builtin, and finding it here is what keeps a script
    /// off the PATH search: with `PATH` empty or `printf` missing from
    /// it, the utility is still there. See
    /// `[dec:nsh:printf-is-parsed-not-interpreted]`.
    #[test]
    fn printf_is_a_builtin() {
        let shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let found = builtin(&shell, BStr::new(b"printf")).expect("printf builtin");
        assert_eq!(found.id(), crate::builtins::BuiltinId::Printf);
        /* `echo` shares printf.c with it and is the neighbouring row. */
        assert!(builtin(&shell, BStr::new(b"echo")).is_some());
    }

    /// The table is binary-searched, so its order is load-bearing —
    /// adding a row must not have disturbed it.
    #[test]
    fn the_builtin_table_stays_sorted() {
        let names: Vec<&[u8]> = crate::builtins::BUILTINS
            .iter()
            .map(|cmd| cmd.name().as_bytes())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
    }
}
