//! Command lookup, hashing and external-program execution.
//! Rules: `docs/spec/port/src/exec.md`.
//!
//! `cmdtable` is a `BTreeMap` keyed by command name, not the C's 31
//! chained hash buckets, so `hash` with no operands prints in name order
//! rather than in the order `hashval` happens to chain. Registered in
//! `docs/divergences.md`. Commands are represented by a Rust enum; no
//! tagged union or borrowed C argument vector crosses this module.

use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;
use std::collections::BTreeMap;
use std::ffi::{CStr, CString};
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::PermissionsExt;

use crate::builtins::{BUILTIN_REGULAR, builtincmd};
use crate::error::{E_EXEC, Error, INTOFF, INTON};
use crate::nodes::{Node, funcnode};

// ---------------------------------------------------------------------
// src/exec.h constants
// ---------------------------------------------------------------------

/* values of cmdtype */
pub const CMDUNKNOWN: c_int = -1; /* no entry in table for command */
pub const CMDNORMAL: c_int = 0; /* command is an executable program */
pub const CMDFUNCTION: c_int = 1; /* command is a shell function */
pub const CMDBUILTIN: c_int = 2; /* command is a shell builtin */

/* action to find_command() */
pub const DO_ERR: c_int = 0x01; /* prints errors */
pub const DO_ABS: c_int = 0x02; /* checks absolute paths */
pub const DO_NOFUNC: c_int = 0x04; /* don't return shell functions, for command */
pub const DO_ALTPATH: c_int = 0x08; /* using alternate path */
pub const DO_REGBLTIN: c_int = 0x10; /* regular built-ins and functions only */

const _PATH_BSHELL: &[u8] = b"/bin/sh\0";

// ---------------------------------------------------------------------
// src/exec.h types
// ---------------------------------------------------------------------

// [spec:dash:def:exec.cmdentry.param]
// The C union is represented by `Command`; its tag and payload cannot
// disagree in Rust.

// [spec:dash:def:exec.cmdentry]
#[derive(Clone)]
pub struct cmdentry {
    command: Command,
}

#[derive(Clone)]
enum Command {
    Unknown,
    Normal(c_int),
    Function(funcnode),
    Builtin(&'static builtincmd),
}

impl cmdentry {
    pub(crate) fn unknown() -> Self {
        Self { command: Command::Unknown }
    }

    pub(crate) fn builtin_command(command: &'static builtincmd) -> Self {
        Self { command: Command::Builtin(command) }
    }

    pub(crate) fn cmdtype(&self) -> c_int {
        match self.command {
            Command::Unknown => CMDUNKNOWN,
            Command::Normal(_) => CMDNORMAL,
            Command::Function(_) => CMDFUNCTION,
            Command::Builtin(_) => CMDBUILTIN,
        }
    }

    pub(crate) fn path_index(&self) -> c_int {
        match self.command {
            Command::Normal(index) => index,
            _ => unreachable!("only external commands have PATH indices"),
        }
    }

    pub(crate) fn builtin(&self) -> &'static builtincmd {
        match self.command {
            Command::Builtin(command) => command,
            _ => unreachable!("only builtin commands have builtin entries"),
        }
    }

    pub(crate) fn function(&self) -> funcnode {
        match &self.command {
            Command::Function(function) => function.clone(),
            _ => unreachable!("only shell functions have function bodies"),
        }
    }
}

// [spec:dash:def:exec.tblentry]
/// One cached command resolution.
///
/// The name is the `BTreeMap` key and the command kind is an enum, so the
/// intrusive `next` pointer, flexible array tail and untagged internal union
/// all disappear. Values are boxed because `find_command` keeps their address
/// across operations that can insert another command and rebalance the map.
pub struct tblentry {
    command: Command,
    pub(crate) rehash: bool,
}

impl tblentry {
    pub(crate) fn cmdtype(&self) -> c_int {
        match self.command {
            Command::Unknown => CMDUNKNOWN,
            Command::Normal(_) => CMDNORMAL,
            Command::Function(_) => CMDFUNCTION,
            Command::Builtin(_) => CMDBUILTIN,
        }
    }

    pub(crate) fn path_index(&self) -> c_int {
        match self.command {
            Command::Normal(index) => index,
            _ => unreachable!("only external commands have PATH indices"),
        }
    }

    pub(crate) fn builtin(&self) -> &'static builtincmd {
        match self.command {
            Command::Builtin(cmd) => cmd,
            _ => unreachable!("only builtin entries have builtin pointers"),
        }
    }

    /// `builtinloc` arrives as a value rather than being read here, and
    /// that is not a style choice: the only caller is `clearcmdentry`'s
    /// `retain`, whose closure already holds the table borrowed. Reading
    /// the sibling field from inside it would be a second borrow of the
    /// same `Shell`. Copying the `c_int` out first is the whole fix, and
    /// it is exact -- nothing in the closure can change it.
    pub(crate) fn path_dependent(&self, builtinloc: c_int) -> bool {
        match self.command {
            Command::Normal(_) => true,
            Command::Builtin(cmd) => (cmd.flags & BUILTIN_REGULAR) == 0 && builtinloc > 0,
            _ => false,
        }
    }

    pub(crate) fn resolved(&self) -> cmdentry {
        cmdentry {
            command: self.command.clone(),
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
pub struct CmdTable {
    /// Command names are shell bytes, without an artificial C terminator.
    map: BTreeMap<BString, tblentry>,
    /// index in path of %builtin, or -1
    builtinloc: c_int,
}

impl CmdTable {
    /// An empty hash and `builtinloc = -1`, which is what the two
    /// statics were declared with.
    pub(crate) const fn new() -> Self {
        CmdTable {
            map: BTreeMap::new(),
            builtinloc: -1,
        }
    }

    /// Whether an entry would be invalidated by a `PATH` change.
    ///
    /// Reads `builtinloc` on the caller's behalf, so a caller holding an
    /// entry does not have to reach for the sibling field itself. The
    /// two in-module walks cannot use this — they hold the map borrowed
    /// and take the `c_int` by value instead.
    pub(crate) fn path_dependent(&self, cmdp: &tblentry) -> bool {
        cmdp.path_dependent(self.builtinloc)
    }

    /// Every entry, in name order — what `hash` with no operand prints.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&BString, &tblentry)> {
        self.map.iter()
    }

    pub(crate) fn get(&self, name: &BStr) -> Option<&tblentry> {
        self.map.get(name)
    }

    pub(crate) fn resolved(&self, name: &BStr) -> Option<cmdentry> {
        self.get(name).map(tblentry::resolved)
    }
}

// ---------------------------------------------------------------------

/*
 * Exec a program.  Never returns.  If you change this routine, you may
 * have to change the find_command routine as well.
 */

// [spec:dash:def:exec.shellexec-fn]
// [spec:dash:sem:exec.shellexec-fn]
pub fn shellexec(
    sh: &mut crate::context::Shell,
    argv: &[&BStr],
    path: &BStr,
    mut idx: c_int,
) -> Result<crate::eval::Flow, crate::error::Error> {
    let e: c_int;
    let command = argv.first().expect("shellexec needs a command name");

    /* A library shell may fork children, but it may not replace the host
     * process itself without the host's explicit grant. A forked shell owns
     * the child terminus regardless of which host policy its parent used. */
    if sh.shell_level == 0 && !sh.host.may_replace_process() {
        return exec_failure(sh, command, nsh_platform::permission_denied_error_code());
    }

    /* The C's `environment()` leaves its array in the stack allocator; ours
     * owns it, so the `Vec` has to outlive every `execve` below. */
    let envv = crate::var::environment(sh);
    let words: Vec<CString> = argv.iter().map(|word| crate::shell::cstring(word)).collect();
    let arguments: Vec<&CStr> = words.iter().map(|word| word.as_c_str()).collect();
    if let Err(error) = sh.fds.materialize() {
        return exec_failure(
            sh,
            command,
            error
                .raw_os_error()
                .unwrap_or_else(nsh_platform::permission_denied_error_code),
        );
    }
    if command.contains(&b'/') {
        e = tryexec(arguments[0], &arguments, &envv);
    } else {
        let mut se: c_int = nsh_platform::not_found_error_code();
        let mut cursor = PathCursor::new(path);
        while let Some(candidate) = padvance(&mut cursor, command) {
            idx -= 1;
            if idx < 0 && candidate.option.is_none() {
                let candidate = CStr::from_bytes_with_nul(&candidate.path)
                    .expect("PATH candidates are terminated");
                let candidate_error = tryexec(candidate, &arguments, &envv);
                if !nsh_platform::is_path_not_found_error(candidate_error) {
                    se = candidate_error;
                }
            }
        }
        e = se;
    }

    exec_failure(sh, command, e)
}

fn exec_failure(
    sh: &mut crate::context::Shell,
    command: &BStr,
    error: c_int,
) -> Result<crate::eval::Flow, crate::error::Error> {
    /* Map to POSIX errors */
    let exerrno = nsh_platform::command_exec_failure_status(error);
    sh.status = exerrno;
    /* TRACE(("shellexec failed for %s, errno %d, suppressint %d\n", ...)); */
    let mut message = Vec::new();
    message.extend_from_slice(command);
    message.extend_from_slice(b": ");
    message.extend_from_slice(&crate::error::errmsg(&sh.locale, error, E_EXEC));
    /* `exerror(EXEND, msg)`: text *and* control flow, which is why the
     * bridge took the code as a parameter rather than reading it off the
     * value. The text is written here, where dash writes it, and the value
     * it rendered from is dropped -- an `exec` that cannot happen ends the
     * shell, and `docs/api-design.md` 3.3 is explicit that what ends the
     * run is `Flow`, not `Err`. */
    /* Built before the call rather than inside its argument list: the
     * receiver is borrowed for the whole call, so reading the line out of
     * the same shell in an argument is a conflict. */
    let e = crate::error::Error::other(sh.eval.errlinno, exerrno, &message);
    drop(sh.report(e));

    Ok(crate::eval::Flow::END)
}

// [spec:dash:def:exec.tryexec-fn]
// [spec:dash:sem:exec.tryexec-fn]
fn tryexec(command: &CStr, arguments: &[&CStr], env: &[CString]) -> c_int {
    let error = nsh_platform::execute_program(command, &arguments, env);
    if nsh_platform::is_exec_format_error(&error) && command.to_bytes_with_nul() != _PATH_BSHELL {
        let shell = CStr::from_bytes_with_nul(_PATH_BSHELL)
            .expect("the fallback shell path is terminated");
        let mut shell_arguments = Vec::with_capacity(arguments.len() + 1);
        shell_arguments.push(shell);
        shell_arguments.push(command);
        shell_arguments.extend(arguments.iter().skip(1).copied());
        return nsh_platform::execute_program(shell, &shell_arguments, env)
            .raw_os_error()
            .unwrap_or(0);
    }
    error.raw_os_error().unwrap_or(0)
}

// [spec:dash:def:exec.legal-pathopt-fn]
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

    // [spec:dash:def:exec.padvance-magic-fn]
    // [spec:dash:sem:exec.padvance-magic-fn]
    pub fn advance(&mut self, name: &BStr) -> Option<PathAdvance> {
        let rest = self.remaining.take()?;
        let (component, remaining) = match rest.find_byte(b':') {
            Some(colon) => (&rest[..colon], Some(&rest[colon + 1..])),
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

        /* "2" is the possible slash and terminator. An empty component
         * keeps the spare slash byte in its allocation, as dash does. */
        let allocation_len = directory.len() + name.len() + 2;
        let mut path = BString::new(Vec::with_capacity(allocation_len));
        if !directory.is_empty() {
            path.extend_from_slice(directory);
            path.push(b'/');
        }
        path.extend_from_slice(name);
        path.push(0);

        Some(PathAdvance {
            path,
            option,
            allocation_len,
        })
    }
}

/// One independently owned result from a PATH walk.
pub struct PathAdvance {
    /// Candidate path including its trailing NUL.
    pub path: BString,
    /// `%option`, if the PATH element carried one.
    pub option: Option<BString>,
    /// The allocation size dash returned, including its spare byte for an
    /// empty PATH component.
    pub allocation_len: usize,
}

// [spec:dash:def:exec.padvance-fn]
// [spec:dash:sem:exec.padvance-fn]
#[inline]
pub fn padvance(cursor: &mut PathCursor<'_>, name: &BStr) -> Option<PathAdvance> {
    cursor.advance(name)
}

/*** Command hashing code ***/

// [spec:dash:def:exec.test-exec-fn]
// [spec:dash:sem:exec.test-exec-fn]
fn test_exec(fullname: &std::ffi::OsStr, metadata: &std::fs::Metadata) -> bool {
    if !metadata.is_file() {
        return false;
    }

    if (metadata.permissions().mode() & 0o111) != 0o111 &&
        /* HAVE_FACCESSAT; the non-faccessat build uses test_access(statb, X_OK) */
        !crate::builtins::test::test_file_access(
            fullname.as_bytes().as_bstr(),
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

// [spec:dash:def:exec.find-command-fn]
// [spec:dash:sem:exec.find-command-fn]
pub fn find_command(
    sh: &mut crate::context::Shell,
    name: &BStr,
    entry: &mut cmdentry,
    mut act: c_int,
    path: &BStr,
) -> Result<crate::eval::Flow, Error> {
    /* If name contains a slash, don't use PATH or hash table */
    if name.contains(&b'/') {
        if (act & DO_ABS) != 0 {
            let fullname = std::ffi::OsStr::from_bytes(name);
            let executable = std::fs::metadata(fullname)
                .is_ok_and(|metadata| test_exec(fullname, &metadata));
            if !executable {
                *entry = cmdentry::unknown();
                return Ok(crate::eval::Flow::Done(0));
            }
        }
        entry.command = Command::Normal(-1);
        return Ok(crate::eval::Flow::Done(0));
    }

    let configured_path = crate::var::pathval(sh);
    let mut update_table = path.as_bytes() == configured_path.as_slice();
    if !update_table {
        act |= DO_ALTPATH;
    }

    let mut cached = sh
        .commands
        .map
        .get(name)
        .map(|stored| (stored.command.clone(), stored.rehash));

    if let Some((command, rehash)) = &cached {
        let bit = match command {
            Command::Function(_) => DO_NOFUNC,
            Command::Builtin(command) => {
                if (command.flags & BUILTIN_REGULAR) != 0 { 0 } else { DO_REGBLTIN }
            }
            _ => DO_ALTPATH | DO_REGBLTIN,
        };
        if (act & bit) != 0 {
            if (act & bit & DO_REGBLTIN) != 0 {
                *entry = cmdentry::unknown();
                return Ok(crate::eval::Flow::Done(0));
            }
            update_table = false;
            cached = None;
        } else if !rehash {
            entry.command = command.clone();
            return Ok(crate::eval::Flow::Done(0));
        }
    }

    let builtin_command = builtin(name);
    if let Some(command) = builtin_command
        && ((command.flags & BUILTIN_REGULAR) != 0
            || (act & DO_ALTPATH) != 0
            || sh.commands.builtinloc <= 0)
    {
        if update_table {
            addcmdentry(sh, name, Command::Builtin(command));
        }
        entry.command = Command::Builtin(command);
        return Ok(crate::eval::Flow::Done(0));
    }

    if (act & DO_REGBLTIN) != 0 {
        *entry = cmdentry::unknown();
        return Ok(crate::eval::Flow::Done(0));
    }

    let previous = cached.as_ref().filter(|(_, rehash)| *rehash).map_or(-1, |(command, _)| {
        match command {
            Command::Builtin(_) => sh.commands.builtinloc,
            Command::Normal(index) => *index,
            _ => -1,
        }
    });
    let mut error = nsh_platform::not_found_error_code();
    let mut index = -1;
    let mut cursor = PathCursor::new(path);
    while let Some(candidate) = padvance(&mut cursor, name) {
        index += 1;
        if let Some(option) = &candidate.option {
            if option.first() == Some(&b'b') {
                if let Some(command) = builtin_command {
                    if update_table {
                        addcmdentry(sh, name, Command::Builtin(command));
                    }
                    entry.command = Command::Builtin(command);
                    return Ok(crate::eval::Flow::Done(0));
                }
                continue;
            }
            if (act & DO_NOFUNC) != 0 {
                continue;
            }
        }

        let fullname = crate::mystring::cstr_prefix(&candidate.path).to_owned();
        if fullname.first() == Some(&b'/') && index <= previous {
            if index < previous {
                continue;
            }
            if let Some((command, _)) = cached {
                if let Some(stored) = sh.commands.map.get_mut(name) {
                    stored.rehash = false;
                }
                entry.command = command;
                return Ok(crate::eval::Flow::Done(0));
            }
        }

        let fullname_os = std::ffi::OsStr::from_bytes(&fullname);
        let metadata = match std::fs::metadata(fullname_os) {
            Ok(metadata) => metadata,
            Err(io_error) => {
                if let Some(code) = io_error.raw_os_error()
                    && !nsh_platform::is_path_not_found_error(code)
                {
                    error = code;
                }
                continue;
            }
        };

        if candidate.option.is_some() {
            let flow = crate::shellmain::readcmdfile(sh, BStr::new(fullname.as_slice()))?;
            if let exit @ crate::eval::Flow::Exit { .. } = flow {
                return Ok(exit);
            }
            let Some(stored) = sh.commands.map.get_mut(name) else {
                let mut message = name.to_vec();
                message.extend_from_slice(b" not defined in ");
                message.extend_from_slice(&fullname);
                return Err(sh.sh_error_value(&message));
            };
            if stored.cmdtype() != CMDFUNCTION {
                let mut message = name.to_vec();
                message.extend_from_slice(b" not defined in ");
                message.extend_from_slice(&fullname);
                return Err(sh.sh_error_value(&message));
            }
            stored.rehash = false;
            *entry = stored.resolved();
            return Ok(crate::eval::Flow::Done(0));
        }

        error = nsh_platform::permission_denied_error_code();
        if !test_exec(fullname_os, &metadata) {
            continue;
        }
        if update_table {
            addcmdentry(sh, name, Command::Normal(index));
        }
        entry.command = Command::Normal(index);
        return Ok(crate::eval::Flow::Done(0));
    }

    if cached.is_some() && update_table {
        delete_cmd_entry(sh, name);
    }
    if (act & DO_ERR) != 0 {
        let mut message = name.to_vec();
        message.extend_from_slice(b": ");
        message.extend_from_slice(&crate::error::errmsg(&sh.locale, error, E_EXEC));
        sh.sh_warnx(&message);
    }
    *entry = cmdentry::unknown();
    Ok(crate::eval::Flow::Done(0))
}

/*
 * Search the table of builtin commands.
 */

// [spec:dash:def:exec.find-builtin-fn]
// [spec:dash:sem:exec.find-builtin-fn]
pub fn builtin(name: &BStr) -> Option<&'static builtincmd> {
    crate::builtins::builtincmd
        .binary_search_by(|cmd| BStr::new(cmd.name.to_bytes()).cmp(name))
        .ok()
        .map(|index| &crate::builtins::builtincmd[index])
}

/*
 * Called when a cd is done.  Marks all commands so the next time they
 * are executed they will be rehashed.
 */

// [spec:dash:def:exec.hashcd-fn]
// [spec:dash:sem:exec.hashcd-fn]
pub fn hashcd(sh: &mut crate::context::Shell) {
    /* Copied out for the same reason `clearcmdentry` copies it: the
     * walk below holds the table borrowed. */
    let builtinloc = sh.commands.builtinloc;
    for cmdp in sh.commands.map.values_mut() {
        if cmdp.path_dependent(builtinloc) {
            cmdp.rehash = true;
        }
    }
}

/*
 * Fix command hash table when PATH changed.
 * Called before PATH is changed.  The argument is the new value of PATH;
 * pathval() still returns the old value at this point.
 * Called with interrupts off.
 */

// [spec:dash:def:exec.changepath-fn]
// [spec:dash:sem:exec.changepath-fn]
pub fn changepath(sh: &mut crate::context::Shell, newval: &BStr) {
    let bltin = newval
        .split(|&byte| byte == b':')
        .position(|component| component.starts_with(b"%builtin"))
        .map_or(-1, |index| index as c_int);
    sh.commands.builtinloc = bltin;
    clearcmdentry(sh);
}

/*
 * Clear out command entries.  The argument specifies the first entry in
 * PATH which has changed.
 */

// [spec:dash:def:exec.clearcmdentry-fn]
// [spec:dash:sem:exec.clearcmdentry-fn]
pub(crate) fn clearcmdentry(sh: &mut crate::context::Shell) {
    INTOFF(sh);
    let builtinloc = sh.commands.builtinloc;
    sh.commands.map.retain(|_, cmdp| !cmdp.path_dependent(builtinloc));
    INTON(sh);
}

/*
 * Locate a command in the command hash table.  If "add" is nonzero,
 * add the command to the table if it is not already present.  The
 * Interrupts must be off if called with add != 0.
 */

// [spec:dash:def:exec.cmdlookup-fn]
// [spec:dash:sem:exec.cmdlookup-fn]
pub(crate) fn cmdlookup<'a>(
    sh: &'a mut crate::context::Shell,
    name: &BStr,
    add: bool,
) -> Option<&'a mut tblentry> {
    if add {
        Some(sh.commands.map.entry(name.to_owned()).or_insert_with(|| {
            tblentry {
                command: Command::Unknown,
                rehash: false,
            }
        }))
    } else {
        sh.commands.map.get_mut(name)
    }
}

/*
 * Delete a command table entry by name.
 */

// [spec:dash:def:exec.delete-cmd-entry-fn]
// [spec:dash:sem:exec.delete-cmd-entry-fn]
pub(crate) fn delete_cmd_entry(sh: &mut crate::context::Shell, name: &BStr) {
    INTOFF(sh);
    sh.commands.map.remove(name);
    INTON(sh);
}

// [spec:dash:def:exec.getcmdentry-fn]
// [spec:dash:sem:exec.getcmdentry-fn]
//
// The whole function lives inside `#ifdef notdef` in `src/exec.c`
// (lines 698-712) and is not compiled into the shell. It is carried
// here as an annotated, never-compiled stub so the manifest symbol has
// a target site; `#[cfg(any())]` is the Rust equivalent of the
// unsatisfiable `#ifdef notdef` guard, and the body is the literal
// translation of the dead C.
#[cfg(any())]
pub fn getcmdentry(sh: &crate::context::Shell, name: &BStr) -> cmdentry {
    sh.commands.resolved(name).unwrap_or_else(cmdentry::unknown)
}

/*
 * Add a new command entry, replacing any existing command entry for
 * the same name - except special builtins.
 */

// [spec:dash:def:exec.addcmdentry-fn]
// [spec:dash:sem:exec.addcmdentry-fn]
fn addcmdentry(sh: &mut crate::context::Shell, name: &BStr, command: Command) {
    let cmdp = cmdlookup(sh, name, true).expect("adding returns an entry");
    cmdp.command = command;
    cmdp.rehash = false;
}

/*
 * Define a shell function.
 */

// [spec:dash:def:exec.defun-fn]
// [spec:dash:sem:exec.defun-fn]
pub fn defun(sh: &mut crate::context::Shell, func: &Node) {
    INTOFF(sh);
    addcmdentry(
        sh,
        func.ndefun().text.as_bstr(),
        Command::Function(func.clone()),
    );
    INTON(sh);
}

/*
 * Delete a function if it exists.
 */

// [spec:dash:def:exec.unsetfunc-fn]
// [spec:dash:sem:exec.unsetfunc-fn]
pub fn unsetfunc(sh: &mut crate::context::Shell, name: &BStr) {
    if cmdlookup(sh, name, false).is_some_and(|cmdp| cmdp.cmdtype() == CMDFUNCTION) {
        delete_cmd_entry(sh, name);
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
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;

        changepath(sh, BStr::new(b"/bin:%builtin:/usr/bin"));
        assert_eq!(sh.commands.builtinloc, 1);

        changepath(sh, BStr::new(b"%builtin:/bin"));
        assert_eq!(sh.commands.builtinloc, 0);

        changepath(sh, BStr::new(b"/bin:/usr/bin"));
        assert_eq!(sh.commands.builtinloc, -1, "no %builtin is -1, not 0");
    }

    /// What `clearcmdentry` keeps, which is the predicate the walk runs
    /// while it holds the table borrowed. An external command is always
    /// invalidated by a `PATH` change; an entry that names nothing is
    /// not. Pinned because the `builtinloc` the predicate reads is now
    /// copied out before the walk rather than read inside it, and a
    /// wrong copy would show up here as the wrong survivor.
    // [spec:dash:sem:exec.clearcmdentry-fn/test]
    #[test]
    fn clearing_drops_only_path_dependent_entries() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;

        let external = BStr::new(b"Texternal");
        let unknown = BStr::new(b"Tunknown");
        addcmdentry(sh, external, Command::Normal(0));
        cmdlookup(sh, unknown, true);

        let e = sh.commands.get(external).expect("external entry");
        assert!(sh.commands.path_dependent(e));
        let u = sh.commands.get(unknown).expect("unknown entry");
        assert!(!sh.commands.path_dependent(u));

        clearcmdentry(sh);

        assert!(sh.commands.get(external).is_none(), "an external command does not survive a PATH change");
        assert!(sh.commands.get(unknown).is_some(), "an entry naming nothing has nothing to invalidate");
    }

    // [spec:dash:sem:exec.find-builtin-fn/test]
    #[test]
    fn generated_builtin_lookup_round_trips() {
        for expected in &crate::builtins::builtincmd {
            assert!(core::ptr::eq(
                builtin(BStr::new(expected.name.to_bytes())).expect("generated builtin"),
                expected,
            ));
        }

        for absent in [b"" as &[u8], b"/", b"alia", b"aliasx", b"waitx", b"zz"] {
            assert!(builtin(BStr::new(absent)).is_none());
        }
    }

    /// `printf` is a builtin, and finding it here is what keeps a script
    /// off the PATH search: with `PATH` empty or `printf` missing from
    /// it, the utility is still there. See
    /// `[dec:nsh:printf-is-parsed-not-interpreted]`.
    #[test]
    fn printf_is_a_builtin() {
        let found = builtin(BStr::new(b"printf")).expect("printf builtin");
        assert!(core::ptr::eq(found, crate::builtins::PRINTFCMD));
        /* `echo` shares printf.c with it and is the neighbouring row. */
        assert!(builtin(BStr::new(b"echo")).is_some());
    }

    /// The table is binary-searched, so its order is load-bearing —
    /// adding a row must not have disturbed it.
    #[test]
    fn the_builtin_table_stays_sorted() {
        let names: Vec<&[u8]> = crate::builtins::builtincmd
            .iter()
            .map(|cmd| cmd.name.to_bytes())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert_eq!(names.len(), crate::builtins::NUMBUILTINS);
    }
}
