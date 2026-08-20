//! Literal port of `src/redir.c` / `src/redir.h`.
//! Rules: `docs/spec/port/src/redir.md`.

use crate::error::Error;
use bstr::{BStr, BString};
use core::ffi::c_int;
use nsh_platform::{Descriptor, ShellBytesExt as _};
use std::io::Write;

use crate::context::Shell;
use crate::error::{INTOFF, INTON};
use crate::nodes::{FileRedirectionOperator, HereDocument, Node};

/* flags passed to redirect (redir.h) */
pub const REDIR_PUSH: c_int = 0o1; /* save previous values of file descriptors */
/* #ifdef notyet #define REDIR_BACKQ 02 */
pub const REDIR_SAVEFD2: c_int = 0o3; /* set preverrout */

/*
 * config.h knobs used by this file.  The reference build has
 * `HAVE_MEMFD_CREATE 1` / `USE_MEMFD_CREATE 1`, so `memfd_create` comes from
 * `<sys/mman.h>` (glibc) and system.h's `#ifndef HAVE_MEMFD_CREATE` stub is not
 * compiled; `F_DUPFD_CLOEXEC` is available on the targets `libc` exposes it for.
 */
pub const USE_MEMFD_CREATE: c_int = 1;
pub const HAVE_F_DUPFD_CLOEXEC: c_int = 1;

/// `PIPE_BUF` where available, 4096 otherwise.  4096 on Linux.
const PIPESIZE: usize = 4096;

/// Both owned ends of a pipe. Dropping either field closes that endpoint.
#[derive(Debug)]
pub struct Pipe {
    pub read: Descriptor,
    pub write: Descriptor,
}

enum RedirectSource {
    Noop,
    Close,
    Shared(crate::fd::SharedFd),
    Owned(Descriptor),
}

/// Evaluation-local redirection state. Parsed syntax stays immutable; file
/// names and descriptor words are expanded into this value for one command.
// [spec:nsh:req:idiom.immutable-ast]
pub(crate) enum ExpandedRedirection<'a> {
    File {
        operator: FileRedirectionOperator,
        descriptor: c_int,
        target: BString,
    },
    Descriptor {
        descriptor: c_int,
        source: Option<c_int>,
    },
    HereDocument(&'a HereDocument),
}

impl ExpandedRedirection<'_> {
    fn descriptor(&self) -> c_int {
        match self {
            Self::File { descriptor, .. } | Self::Descriptor { descriptor, .. } => *descriptor,
            Self::HereDocument(document) => document.descriptor,
        }
    }
}

// [spec:dash:def:redir.redirtab]
/// `MKINIT struct redirtab { … }` — absent from the port manifest because the
/// `MKINIT` marker defeated the extractor.
///
/// The C's `next` is gone with the intrusive stack. Saved logical values are
/// shared owners: ordinary unwind restores them, while fork reset drops
/// obsolete backups without changing the active table.
pub struct redirtab {
    renamed: [SavedDescriptor; 10],
}

enum SavedDescriptor {
    Empty,
    Saved(Option<crate::fd::SharedFd>),
}

/// The stack of saved logical-descriptor states.
///
/// The fields are private to `redir.rs`, so `Shell` owns the value and
/// this module owns its shape — nothing outside can reach past the
/// functions below, which is the property the two `static mut`s it
/// replaces never had.
///
/// The live map is [`crate::fd::FdTable`]; this stack records only the
/// values needed to restore command-scoped redirections.
pub struct RedirStack {
    /// One frame per redirection scope, innermost last. A frame's *index*
    /// is what outlives a call here, never a borrow: `openredirect` can
    /// reach command substitution, which pushes and pops frames of its
    /// own and can move the vector out from under a reference.
    list: Vec<redirtab>,
}

impl RedirStack {
    /// `redirlist = NULL` and `closed_redirs = 0`, which is what the two
    /// statics started at.
    pub(crate) const fn new() -> Self {
        RedirStack { list: Vec::new() }
    }
}

/*
 * Process a list of redirection commands.  If the REDIR_PUSH flag is set,
 * old file descriptors are stashed away so that the redirection can be
 * undone by calling popredir.  If the REDIR_BACKQ flag is set, then the
 * standard output, and the standard error if it becomes a duplicate of
 * stdout, is saved in memory.
 */

// [spec:dash:def:redir.redirect-fn]
// [spec:dash:sem:redir.redirect-fn]
// [spec:posix:sem:shell.redirection-processing]
// [spec:posix:def:redir.purpose]
// [spec:posix:sem:redir.evaluation-order]
pub(crate) fn redirect(
    sh: &mut Shell,
    redir: &[ExpandedRedirection<'_>],
    flags: c_int,
) -> Result<(), Error> {
    let sv: Option<usize>;

    /* #if notyet — the `memory[10]` in-memory sink is not compiled. */
    if redir.is_empty() {
        return Ok(());
    }
    INTOFF(sh);
    /* `sv = redirlist` — the frame `pushredir` just pushed, and NULL when
     * there is none, which is what `checked_sub` says. */
    sv = if (flags & REDIR_PUSH) != 0 {
        sh.redirs.list.len().checked_sub(1)
    } else {
        None
    };
    /* The C walks the list through `n->nfile.next`, which is the same offset
     * in every redirection arm; the list is a `Vec` now. */
    for n in redir {
        let fd = n.descriptor();
        let source = openredirect(sh, n)?;
        if !matches!(source, RedirectSource::Noop) {
            /* The C's `fd == 0` is "this redirection replaced the shell's
             * own input", which is what makes the buffered parse state
             * stale -- not descriptor 0 for its own sake. */
            if fd == 0 {
                crate::input::reset_input(sh);
            }

            if let Some(svi) = sv {
                let p_slot = fd as usize;
                if matches!(sh.redirs.list[svi].renamed[p_slot], SavedDescriptor::Empty) {
                    let saved = sh
                        .fds
                        .get(fd)
                        .map_err(|error| descriptor_error(sh, fd, error))?;
                    sh.redirs.list[svi].renamed[p_slot] = SavedDescriptor::Saved(saved);
                }
            }

            /* The `?` returns between the INTOFF above and the INTON below,
             * leaking the counter exactly as the old `sh_dup2` error path
             * did; see docs/errors-are-values.md 2.4. */
            install_redirect(sh, fd, source)?;
        }
    }
    INTON(sh);
    /* NB: REDIR_SAVEFD2 is 03, so this test also fires for a plain
     * REDIR_PUSH (01); reproduced verbatim (src/redir.c:184).
     *
     * The C indexes slot 2 because that is where the shell's stderr is.
     * The slot follows the frontend's stderr instead -- and if that was
     * put past the end of `renamed`, which covers the ten descriptors
     * redirection can name, there is nothing saved to point the trace
     * stream at and it stays where it was. */
    if (flags & REDIR_SAVEFD2) != 0 {
        /* The C dereferences `sv` here without testing it, and gets away
         * with it because REDIR_SAVEFD2 is 03: every caller that reaches
         * this line passed REDIR_PUSH and so has a frame. */
        if let Some(svi) = sv {
            let renamed = &sh.redirs.list[svi].renamed;
            if let Some(SavedDescriptor::Saved(Some(saved))) = renamed.get(2) {
                let destination = crate::fd::FdRef::default();
                destination.replace(Some(saved.clone()));
                sh.io.previous_stderr().set_destination(destination);
            }
        }
    }
    Ok(())
}

// [spec:dash:def:redir.sh-open-fail-fn]
// [spec:dash:sem:redir.sh-open-fail-fn]
// [spec:nsh:req:idiom.platform-errors]
fn sh_open_fail(
    sh: &mut crate::context::Shell,
    pathname: &BStr,
    mode: nsh_platform::OpenMode,
    error: &std::io::Error,
) -> Error {
    sh_open_fail_with_context(sh, pathname, mode, error, OpenFailureContext::Ordinary)
}

fn sh_open_fail_with_context(
    sh: &mut crate::context::Shell,
    pathname: &BStr,
    mode: nsh_platform::OpenMode,
    error: &std::io::Error,
    context: OpenFailureContext,
) -> Error {
    let (word, action): (&[u8], c_int) = if mode.creates() {
        (b"create", crate::error::E_CREAT)
    } else {
        (b"open", crate::error::E_OPEN)
    };
    let mut message = b"cannot ".to_vec();
    message.extend_from_slice(word);
    message.push(b' ');
    message.extend_from_slice(pathname);
    message.extend_from_slice(b": ");
    message.extend_from_slice(&crate::error::errmsg(&sh.locale, error, action));
    let status = context.status(error);
    sh.report(Error::other(
        sh.eval.errlinno,
        c_int::from(status.code()),
        &message,
    ))
}

#[derive(Copy, Clone)]
enum OpenFailureContext {
    Ordinary,
    CommandFile,
}

impl OpenFailureContext {
    // [spec:posix:req:sh.exit-status-values]
    // [spec:posix:req:xcu.exit-status.listed-values-binding]
    fn status(self, error: &std::io::Error) -> crate::status::ExitStatus {
        if matches!(self, OpenFailureContext::CommandFile)
            && nsh_platform::is_path_error(error, nsh_platform::PathErrorKind::NotFound)
        {
            crate::status::ExitStatus::NOT_FOUND
        } else {
            crate::status::ExitStatus::from_raw(2)
        }
    }
}

// [spec:dash:def:redir.sh-open-fn]
// [spec:dash:sem:redir.sh-open-fn]
// [spec:posix:req:xcurel.file-access-permissions]
// [spec:posix:req:xcurel.file-open-access-mode]
// [spec:posix:req:xcurel.pathname-resolution]
pub fn sh_open(
    sh: &mut Shell,
    pathname: &BStr,
    mode: nsh_platform::OpenMode,
    mayfail: c_int,
) -> Result<Option<Descriptor>, Error> {
    sh_open_with_context(sh, pathname, mode, mayfail, OpenFailureContext::Ordinary)
}

fn sh_open_with_context(
    sh: &mut Shell,
    pathname: &BStr,
    mode: nsh_platform::OpenMode,
    mayfail: c_int,
    context: OpenFailureContext,
) -> Result<Option<Descriptor>, Error> {
    loop {
        let result = pathname
            .try_to_path_buf()
            .and_then(|path| nsh_platform::open_path(&path, mode));
        match result {
            Ok(fd) => return Ok(Some(fd)),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                /* An EINTR return is a place the shell is looking, so take
                 * delivery here if an interrupt is due. `sa_flags = 0` is why
                 * this return exists at all -- dash never restarts a syscall.
                 * Otherwise retry, which is the C, whose extra
                 * `pending_sig == 0` test this replaces: a signal that is
                 * pending but not *due* (suppressed, or trapped and handled
                 * elsewhere) is no reason to abandon the open. */
                if let Some(err) = crate::error::poll_interrupt(sh) {
                    return Err(err);
                }
                if crate::siginbox::signals().pending_signal() == 0 {
                    continue;
                }
                if mayfail != 0 {
                    return Ok(None);
                }
                return Err(sh_open_fail_with_context(
                    sh, pathname, mode, &error, context,
                ));
            }
            Err(error) if mayfail != 0 => return Ok(None),
            Err(error) => {
                return Err(sh_open_fail_with_context(
                    sh, pathname, mode, &error, context,
                ));
            }
        }
    }
}

/// Open a path for input without exposing the platform's numeric open flags
/// to callers outside the redirection subsystem.
pub fn sh_open_read(
    sh: &mut Shell,
    pathname: &BStr,
    mayfail: c_int,
) -> Result<Option<Descriptor>, Error> {
    sh_open(sh, pathname, nsh_platform::OpenMode::ReadOnly, mayfail)
}

/// Open `sh`'s command-file operand, preserving its POSIX status class.
pub fn sh_open_command_file(sh: &mut Shell, pathname: &BStr) -> Result<Descriptor, Error> {
    sh_open_with_context(
        sh,
        pathname,
        nsh_platform::OpenMode::ReadOnly,
        0,
        OpenFailureContext::CommandFile,
    )
    .map(|fd| fd.expect("a mandatory command-file open returns a descriptor"))
}

// [spec:dash:def:redir.openredirect-fn]
// [spec:dash:sem:redir.openredirect-fn]
// [spec:posix:req:redir.open-failure]
// [spec:posix:req:redir.input]
// [spec:posix:req:redir.output-noclobber]
// [spec:posix:req:redir.output-noclobber-atomicity]
// [spec:posix:req:redir.output-truncate]
// [spec:posix:req:redir.append]
// [spec:posix:req:redir.dup-input]
// [spec:posix:req:redir.dup-input-close]
// [spec:posix:req:redir.dup-output]
// [spec:posix:req:redir.dup-output-close]
// [spec:posix:req:redir.open-read-write]
// [spec:posix:req:xcurel.file-create-if-absent]
// [spec:posix:req:xcurel.file-creation-attributes]
// [spec:posix:req:xcurel.file-create-existing-actions]
// [spec:posix:def:xcurel.file-create-existing-codes]
// [spec:posix:req:xcurel.file-append-mode]
fn openredirect(sh: &mut Shell, redir: &ExpandedRedirection<'_>) -> Result<RedirectSource, Error> {
    let source = match redir {
        ExpandedRedirection::File {
            operator, target, ..
        } => open_file_redirection(sh, *operator, BStr::new(target.as_slice()))?,
        ExpandedRedirection::Descriptor { descriptor, source } => {
            open_descriptor_redirection(sh, *descriptor, *source)?
        }
        ExpandedRedirection::HereDocument(document) => {
            RedirectSource::Owned(openhere(sh, document)?)
        }
    };

    Ok(source)
}

fn open_file_redirection(
    sh: &mut Shell,
    operator: FileRedirectionOperator,
    target: &BStr,
) -> Result<RedirectSource, Error> {
    let source = match operator {
        FileRedirectionOperator::Read => RedirectSource::Owned(
            sh_open(sh, target, nsh_platform::OpenMode::ReadOnly, 0)?
                .expect("a mandatory open returns a descriptor"),
        ),
        FileRedirectionOperator::ReadWrite => RedirectSource::Owned(
            sh_open(sh, target, nsh_platform::OpenMode::ReadWriteCreate, 0)?
                .expect("a mandatory open returns a descriptor"),
        ),
        FileRedirectionOperator::Write | FileRedirectionOperator::Clobber => {
            let mut fell_through = true;
            let mut opened = None;
            if operator == FileRedirectionOperator::Write {
                /* Take care of noclobber mode. */
                if sh.options.flag(crate::options::Cflag) != 0 {
                    if !target
                        .try_to_path_buf()
                        .is_ok_and(|path| nsh_platform::path_exists(&path))
                    {
                        /* goto do_open */
                        return Ok(RedirectSource::Owned(
                            sh_open(sh, target, nsh_platform::OpenMode::WriteCreateExclusive, 0)?
                                .expect("a mandatory open returns a descriptor"),
                        ));
                    }

                    if target
                        .try_to_path_buf()
                        .is_ok_and(|path| nsh_platform::path_is_file(&path))
                    {
                        /* goto ecreate */
                        let error = nsh_platform::platform_error(
                            nsh_platform::PlatformErrorKind::AlreadyExists,
                        );
                        return Err(sh_open_fail(
                            sh,
                            target,
                            nsh_platform::OpenMode::WriteCreateTruncate,
                            &error,
                        ));
                    }

                    let fv = sh_open(sh, target, nsh_platform::OpenMode::WriteOnly, 0)?
                        .expect("a mandatory open returns a descriptor");
                    if nsh_platform::fd_is_regular_file(&fv).unwrap_or(false) {
                        drop(fv);
                        /* goto ecreate */
                        let error = nsh_platform::platform_error(
                            nsh_platform::PlatformErrorKind::AlreadyExists,
                        );
                        return Err(sh_open_fail(
                            sh,
                            target,
                            nsh_platform::OpenMode::WriteCreateTruncate,
                            &error,
                        ));
                    }
                    opened = Some(fv);
                    fell_through = false;
                }
                /* FALLTHROUGH */
            }
            if fell_through {
                RedirectSource::Owned(
                    sh_open(sh, target, nsh_platform::OpenMode::WriteCreateTruncate, 0)?
                        .expect("a mandatory open returns a descriptor"),
                )
            } else {
                RedirectSource::Owned(opened.expect("the noclobber path opened a descriptor"))
            }
        }
        FileRedirectionOperator::Append => RedirectSource::Owned(
            sh_open(sh, target, nsh_platform::OpenMode::WriteCreateAppend, 0)?
                .expect("a mandatory open returns a descriptor"),
        ),
    };

    Ok(source)
}

fn open_descriptor_redirection(
    sh: &mut Shell,
    descriptor: c_int,
    source: Option<c_int>,
) -> Result<RedirectSource, Error> {
    let Some(source) = source else {
        return Ok(RedirectSource::Close);
    };
    if source == descriptor {
        Ok(RedirectSource::Noop)
    } else {
        let source_fd = sh
            .fds
            .get(source)
            .map_err(|error| descriptor_error(sh, source, error))?
            .ok_or_else(|| {
                descriptor_error(
                    sh,
                    source,
                    nsh_platform::platform_error(nsh_platform::PlatformErrorKind::BadDescriptor),
                )
            })?;
        Ok(RedirectSource::Shared(source_fd))
    }
}

pub(crate) fn descriptor_error(sh: &mut Shell, source: c_int, error: std::io::Error) -> Error {
    let mut message = Vec::new();
    write!(&mut message, "{}", source).expect("writing to a Vec cannot fail");
    message.extend_from_slice(b": ");
    message.extend_from_slice(sh.locale.error_message(&error).as_bytes());
    sh.sh_error_value(&message)
}

// [spec:dash:def:redir.dupredirect-fn]
// [spec:dash:sem:redir.dupredirect-fn]
// [spec:dash:def:redir.sh-dup2-fn]
// [spec:dash:sem:redir.sh-dup2-fn]
fn install_redirect(sh: &mut Shell, target: c_int, source: RedirectSource) -> Result<(), Error> {
    match source {
        RedirectSource::Noop => Ok(()),
        RedirectSource::Close => sh
            .fds
            .replace(target, None)
            .map(|_| ())
            .map_err(|error| descriptor_error(sh, target, error)),
        RedirectSource::Shared(source) => sh
            .fds
            .replace(target, Some(source))
            .map(|_| ())
            .map_err(|error| descriptor_error(sh, target, error)),
        RedirectSource::Owned(source) => {
            let number = source.number();
            sh.fds
                .install_owned(target, source)
                .map(|_| ())
                .map_err(|error| descriptor_error(sh, number, error))
        }
    }
}

// [spec:dash:def:redir.sh-pipe-fn]
// [spec:dash:sem:redir.sh-pipe-fn]
// [spec:nsh:req:idiom.filesystem-account-bytes]
pub fn sh_pipe(sh: &mut crate::context::Shell, memfd: bool) -> Result<(Pipe, bool), Error> {
    if memfd && USE_MEMFD_CREATE != 0 {
        if let Ok(read_fd) = nsh_platform::anonymous_file("dash") {
            let source = read_fd.number();
            let write_fd = nsh_platform::duplicate_fd(&read_fd)
                .map_err(|error| descriptor_error(sh, source, error))?;
            let read = nsh_platform::move_fd_cloexec(read_fd, crate::fd::SLOT_COUNT as i32)
                .map_err(|error| descriptor_error(sh, source, error))?;
            let source = write_fd.number();
            let write = nsh_platform::move_fd_cloexec(write_fd, crate::fd::SLOT_COUNT as i32)
                .map_err(|error| descriptor_error(sh, source, error))?;
            return Ok((Pipe { read, write }, true));
        }
    }

    let (read, write) = nsh_platform::pipe().map_err(|_| sh.sh_error_value(b"Pipe call failed"))?;
    let read = nsh_platform::move_fd_cloexec(read, crate::fd::SLOT_COUNT as i32)
        .map_err(|_| sh.sh_error_value(b"Pipe call failed"))?;
    let write = nsh_platform::move_fd_cloexec(write, crate::fd::SLOT_COUNT as i32)
        .map_err(|_| sh.sh_error_value(b"Pipe call failed"))?;
    Ok((Pipe { read, write }, false))
}

/*
 * Handle here documents.  Normally we fork off a process to write the
 * data to a pipe.  If the document is short, we can stuff the data in
 * the pipe without forking.
 */

// [spec:dash:def:redir.openhere-fn]
// [spec:dash:sem:redir.openhere-fn]
// [spec:posix:sem:redir.here-doc-fd-type]
// [spec:posix:req:redir.here-doc-expansion]
fn openhere(sh: &mut Shell, document: &HereDocument) -> Result<Descriptor, Error> {
    let len: usize;
    let expanded;

    let p: &[u8] = if document.expand {
        let doc = Node::Word(document.body.clone());
        crate::expand::expandarg(sh, &doc, None, crate::expand::EXP_QUOTED)?;
        /* The C reads the expansion back out of the region as
         * `stackblock()`.  The expansion buffer is owned now, so the read is
         * named.  Two consequences, both in the port's favour: the bytes
         * cannot be moved by the `sh_pipe`/`forkshell` allocations below —
         * the C's were only safe because neither happens to `stalloc` — and
         * they are still NUL-terminated by `argstr`.
         *
         * The `strlen` the C applied here has moved *into*
         * `expansion_result`, which hands back the bytes it would have
         * counted rather than the base of them. */
        expanded = bstr::BString::from(crate::expand::expansion_result(sh));
        expanded.as_slice()
    } else {
        /* The unexpanded document is the node's own text. `as_bstr` drops
         * the counted terminator and `cstr_prefix` stops at the first NUL
         * within what is left, which together are the C's `strlen` — the
         * second half matters because a here-document body can carry an
         * embedded NUL and the terminator is then not the one `strlen`
         * would have found. */
        crate::mystring::cstr_prefix(document.body.word.as_bstr())
    };

    len = p.len();
    let (pip, memfd) = sh_pipe(sh, len > PIPESIZE)?;

    if memfd || len <= PIPESIZE {
        /* The return is discarded, as the C discards it, and the 8.3
         * audit flagged both this and the sibling in the forked child
         * below as places an interrupt could be dropped. They are not:
         * `output::write_fd` retries EINTR internally and the output path
         * is deliberately *not* a poll site -- dash collects output
         * errors in `outerr` and checks them separately rather than
         * raising, and making it fallible is the shape 4.3 argues
         * against. They become live the day someone changes that, and
         * that is a different node's decision. */
        crate::output::xwrite(&pip.write, p);
        let _ = nsh_platform::seek_start(&pip.write);
        /* goto out */
        drop(pip.write);
        return Ok(pip.read);
    }

    if matches!(
        crate::jobs::forkshell(sh, None, None, crate::jobs::FORK_NOJOB)?,
        nsh_platform::ForkResult::Child
    ) {
        drop(pip.read);
        nsh_platform::configure_here_document_writer_signals();
        crate::output::xwrite(&pip.write, p);
        crate::shell::flush_coverage();
        nsh_platform::exit_immediately(0);
    }
    /* out: */
    drop(pip.write);
    Ok(pip.read)
}

/*
 * Undo the effects of the last redirection.
 */

// [spec:dash:def:redir.popredir-fn]
// [spec:dash:sem:redir.popredir-fn]
pub fn popredir(sh: &mut Shell, drop: c_int) {
    let rp: usize;
    let mut i: c_int;

    INTOFF(sh);
    rp = sh.redirs.list.len() - 1;
    i = 0;
    while i < 10 {
        let renamed = std::mem::replace(
            &mut sh.redirs.list[rp].renamed[i as usize],
            SavedDescriptor::Empty,
        );

        if matches!(renamed, SavedDescriptor::Empty) {
            i += 1;
            continue;
        }

        match renamed {
            SavedDescriptor::Saved(saved) => {
                if drop == 0 {
                    if i == 0 {
                        crate::input::reset_input(sh);
                    }
                    let _ = sh.fds.replace(i, saved);
                }
            }
            SavedDescriptor::Empty => unreachable!(),
        }
        i += 1;
    }
    /* `redirlist = rp->next` — which also drops anything pushed above `rp`
     * and never popped, as the C's assignment did. */
    sh.redirs.list.truncate(rp);
    INTON(sh);
}

/*
 * Undo all redirections.  Called on error or interrupt.
 */

/* mkinit EXITRESET fragment from src/redir.c:443-448. */
pub fn mkinit_exitreset(sh: &mut Shell) {
    /*
     * Discard all saved file descriptors.
     */
    unwindredir(sh, 0);
}

/* mkinit FORKRESET fragment from src/redir.c:450-452. */
pub fn mkinit_forkreset(sh: &mut Shell) {
    /* `redirlist = NULL`: abandon saved states without restoring them. The
     * current logical table owns what survives in the child. */
    sh.redirs.list.clear();
}

/*
 * Move a file descriptor to > 10.  Invokes sh_error on error unless
 * the original file dscriptor is not open.
 */

// [spec:dash:def:redir.savefd-fn]
// [spec:dash:sem:redir.savefd-fn]
/// Move an owned descriptor above the shell redirection range.
pub fn move_fd_above(sh: &mut Shell, fd: Descriptor) -> Result<Descriptor, Error> {
    let number = fd.number();
    nsh_platform::move_fd_cloexec(fd, 10).map_err(|error| descriptor_error(sh, number, error))
}

/// Duplicate a process-table slot above the shell redirection range.
pub fn copy_slot_above(sh: &mut Shell, from: c_int) -> Result<Option<Descriptor>, Error> {
    let source = sh
        .fds
        .get(from)
        .map_err(|error| descriptor_error(sh, from, error))?;
    source
        .map(|source| nsh_platform::duplicate_cloexec(&source, 10))
        .transpose()
        .map_err(|error| descriptor_error(sh, from, error))
}

/// `redirect`, with the diagnostic it can produce handed back rather than
/// jumped with.
///
/// The C returns `setjmp(jmploc.loc) * 2` — 0, or the 2 a redirection
/// error takes. It returns the error itself, because `evalcommand`'s
/// `bail:` has to *re-raise* it when the command is a special built-in
/// (POSIX's "an error in a special built-in exits a non-interactive
/// shell") and an int cannot be re-raised.
///
/// There is no longer a `setjmp` here, or a handler, or a
/// `SAVEINT`-shaped reason for one. What is left of the C's frame is the
/// `SAVEINT`/`RESTOREINT` pair itself, and it stays exactly where it was:
/// this is a catch that returns into the middle of `evalcommand` rather
/// than to a top level, so it restores the counter's saved value instead
/// of resetting it (§2.4). `RESTOREINT` is still skipped when the error
/// leaves, because the `?`-shaped return skips it exactly as the longjmp
/// did, and the outermost `FORCEINTON` is what clears the leak.
// [spec:dash:def:redir.redirectsafe-fn]
// [spec:dash:sem:redir.redirectsafe-fn]
pub(crate) fn redirectsafe(
    sh: &mut Shell,
    redir: &[ExpandedRedirection<'_>],
    flags: c_int,
) -> Result<(), Error> {
    let mut saveint: c_int = 0;

    crate::SAVEINT!(sh, saveint);
    let redirect_error = redirect(sh, redir, flags).err();
    let caught = crate::expand::restore_handler_expandarg(sh, redirect_error);
    if let Some(e) = caught {
        /* The C's `longjmp` from `restore_handler_expandarg` left before
         * the `RESTOREINT` below; so does this. */
        return Err(e);
    }
    crate::RESTOREINT!(sh, saveint);

    Ok(())
}

// [spec:dash:def:redir.unwindredir-fn]
// [spec:dash:sem:redir.unwindredir-fn]
/// `stop` was the `redirtab *` to unwind back to; a stack in a vector says
/// the same thing with the depth to unwind back to.
pub fn unwindredir(sh: &mut Shell, stop: usize) {
    while sh.redirs.list.len() != stop {
        popredir(sh, 0);
    }
}

// [spec:dash:def:redir.pushredir-fn]
// [spec:dash:sem:redir.pushredir-fn]
pub(crate) fn pushredir(sh: &mut Shell, redir: &[ExpandedRedirection<'_>]) -> usize {
    let q: usize;

    q = sh.redirs.list.len();
    if redir.is_empty() {
        return q; /* goto out */
    }

    sh.redirs.list.push(redirtab {
        renamed: std::array::from_fn(|_| SavedDescriptor::Empty),
    });

    q
}

#[cfg(test)]
mod tests {
    //! `redirectsafe`'s half of the decision `expand::restore_handler_expandarg`
    //! makes for it and for `parser::expandstr`.
    //!
    //! The helper lives in `expand.rs` because the C put it there; these
    //! are here because `redirectsafe` is the caller whose surrounding
    //! state -- a half-applied redirection and a hand-saved interrupt
    //! counter -- is what makes getting it wrong dangerous.
    //!
    //! What is pinned is the decision, which is now a match on the
    //! value's own type rather than a comparison against a global that
    //! some other frame may have written since. The `ifsfree` half is
    //! pinned where it is observable -- as the field count of the word
    //! after a failure, in `tests/errors_are_values.rs`.

    use super::OpenFailureContext;
    use crate::Shell;
    use crate::error::Error;
    use crate::expand::restore_handler_expandarg;

    fn diagnostic() -> Error {
        Error::Other {
            line: 7,
            status: 2,
            message: bstr::BString::from(&b"Bad substitution"[..]),
        }
    }

    // [spec:posix:req:sh.exit-status-values/test]
    // [spec:posix:req:xcu.exit-status.listed-values-binding/test]
    #[test]
    fn command_file_open_status() {
        let missing = nsh_platform::platform_error(nsh_platform::PlatformErrorKind::NotFound);
        let denied =
            nsh_platform::platform_error(nsh_platform::PlatformErrorKind::PermissionDenied);

        assert_eq!(
            OpenFailureContext::CommandFile.status(&missing),
            crate::status::ExitStatus::NOT_FOUND,
        );
        assert_eq!(OpenFailureContext::Ordinary.status(&missing).code(), 2,);
        assert_eq!(OpenFailureContext::CommandFile.status(&denied).code(), 2,);
    }

    /// Nothing went wrong: nothing comes back.
    // [spec:dash:sem:expand.restore-handler-expandarg-fn/test]
    #[test]
    fn a_clean_frame_returns_nothing() {
        let _guard = crate::testutil::lock();
        let mut sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        assert!(restore_handler_expandarg(&mut sh, None).is_none());
    }

    /// A diagnostic is handed straight back, text, status and line
    /// intact -- the arm that used to be `exception == EXERROR`.
    // [spec:dash:sem:expand.restore-handler-expandarg-fn/test]
    #[test]
    fn caught_diagnostic_comes_back() {
        let _guard = crate::testutil::lock();
        let mut sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let got = restore_handler_expandarg(&mut sh, Some(diagnostic()))
            .expect("the caught diagnostic is the frame's to return");
        assert_eq!(got.message(), "Bad substitution");
        assert_eq!(got.status(), 2);
        assert_eq!(got.line(), 7);
        assert!(!got.is_interrupt());
    }

    /// An interrupt comes back too, and is *not* the same arm: the C
    /// re-raised it from here rather than swallowing it, and the frames
    /// above must be able to tell the two apart. Getting this wrong is a
    /// shell that stops answering `^C`.
    // [spec:dash:sem:expand.restore-handler-expandarg-fn/test]
    #[test]
    fn an_interrupt_comes_back_as_one() {
        let _guard = crate::testutil::lock();
        let mut sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let got = restore_handler_expandarg(
            &mut sh,
            Some(Error::Interrupted {
                signal: crate::status::Signal::from_raw(nsh_platform::interrupt_signal().number()),
            }),
        )
        .expect("an interrupt must not be swallowed by this frame");
        assert!(got.is_interrupt());
        assert_eq!(
            got.status(),
            nsh_platform::interrupt_signal().number() + 128
        );
    }

    /// Opening directly into the target means the target was closed before
    /// the redirection. Unwind must close it again, not save and restore the
    /// file that the open itself just placed there.
    // [spec:dash:sem:redir.popredir-fn/test]
    #[test]
    fn open_into_target_restores_closed_slot() {
        let status = nsh_platform::run_in_child(|| {
            let mut sh = Shell::builder().build().unwrap();
            sh.fds.replace(3, None).unwrap();
            if sh.run(b"{ :; } 3>/dev/null").is_err() {
                nsh_platform::exit_immediately(2);
            }
            if sh.fds.is_open(3) {
                nsh_platform::exit_immediately(3);
            }
            nsh_platform::exit_immediately(0);
        })
        .unwrap();

        assert_eq!(status, 0, "child failed at step {status}");
    }
}
