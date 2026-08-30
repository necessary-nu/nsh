//! Literal port of `src/redir.c` / `src/redir.h`.
//! Rules: `docs/spec/port/src/redir.md`.

use crate::error::Error;
use bstr::{BStr, BString};
use nsh_platform::{Descriptor, ShellBytesExt as _};
use std::collections::BTreeMap;
use std::io::Write;

use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::nodes::{FileRedirectionOperator, HereDocument, Node, RedirectionDescriptor};
use crate::options::ShellOption;
// [spec:nsh:def:idiom.shell-options]

/// Whether applying redirections is permanent or records a restorable frame.
///
/// A pushed redirection also redirects the evaluator's saved stderr view to
/// the descriptor frame it just captured. Those operations were previously
/// encoded by overlapping `01` and `03` masks, so no third valid combination
/// existed even though the integer API appeared to permit one.
// [spec:nsh:req:idiom.operation-modes]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum RedirectionMode {
    Apply,
    Push,
}

/// `PIPE_BUF` where available, 4096 otherwise.  4096 on Linux.
const PIPE_BUFFER_SIZE: usize = 4096;

/// Both owned ends of a pipe. Dropping either field closes that endpoint.
#[derive(Debug)]
pub struct Pipe {
    pub read: Descriptor,
    pub write: Descriptor,
}

enum RedirectSource {
    Noop,
    Close,
    Shared(crate::descriptors::SharedDescriptor),
    Owned(Descriptor),
}

/// Evaluation-local redirection state. Parsed syntax stays immutable; file
/// names and descriptor words are expanded into this value for one command.
// [spec:nsh:req:idiom.immutable-ast]
// [spec:nsh:def:idiom.logical-descriptors]
pub(crate) enum ExpandedRedirection<'a> {
    File {
        operator: FileRedirectionOperator,
        descriptor: RedirectionDescriptor,
        target: BString,
    },
    Descriptor {
        descriptor: RedirectionDescriptor,
        source: Option<LogicalDescriptor>,
    },
    HereDocument(&'a HereDocument),
    /// Bash's `<<< word`, already expanded to the bytes the descriptor
    /// reads. The trailing newline is part of the content.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    HereString {
        descriptor: RedirectionDescriptor,
        content: BString,
    },
    /// A redirection whose operand did not expand to exactly one
    /// pathname. Bash refuses it, reports the word, and gives the
    /// command status 1.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    Ambiguous {
        descriptor: RedirectionDescriptor,
        word: BString,
    },
}

impl ExpandedRedirection<'_> {
    fn descriptor(&self) -> &RedirectionDescriptor {
        match self {
            Self::File { descriptor, .. } | Self::Descriptor { descriptor, .. } => descriptor,
            Self::HereDocument(document) => &document.descriptor,
            Self::HereString { descriptor, .. } | Self::Ambiguous { descriptor, .. } => descriptor,
        }
    }

    /// Whether this redirection closes the slot rather than opening it.
    ///
    /// `{name}<&-` is the one form that reads `name` instead of assigning
    /// it: the script is closing a descriptor it already has, so the
    /// number has to come from where the script put it.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn closes(&self) -> bool {
        matches!(self, Self::Descriptor { source: None, .. })
    }
}

/// The lowest slot no descriptor occupies, at or above the inherited range.
///
/// Bash allocates from ten upward -- above every number a script can write
/// as an ordinary IO_NUMBER without counting -- and reuses a slot once it
/// is closed, which is why this searches rather than counting up from the
/// last one handed out. `RLIMIT_NOFILE` bounds it, as it bounds a number
/// the script writes itself.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn allocate_descriptor(shell: &mut Shell) -> Result<LogicalDescriptor, Error> {
    let limit = nsh_platform::descriptor_limit() as usize;
    let free = (LogicalDescriptor::INHERITED..limit)
        .filter_map(LogicalDescriptor::from_index)
        .find(|candidate| shell.descriptors.get(*candidate).is_none());
    free.ok_or_else(|| {
        let line = shell.evaluation.diagnostic_line;
        shell
            .diagnostics()
            .report(Error::other(line, 1, b"no free file descriptor"))
    })
}

/// The slot `{name}<&-` closes, read out of `name`.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn named_descriptor(shell: &mut Shell, name: &BStr) -> Result<LogicalDescriptor, Error> {
    let held = crate::variables::lookup_bytes(shell, name);
    if let Some(held) = &held
        && let Some(descriptor) = LogicalDescriptor::from_digits(held)
    {
        return Ok(descriptor);
    }
    let mut message = name.to_vec();
    message.extend_from_slice(b": ambiguous redirect");
    let line = shell.evaluation.diagnostic_line;
    Err(shell.diagnostics().report(Error::other(line, 1, &message)))
}

/// The slot a redirection acts on, allocating one where the source wrote
/// `{name}` rather than a number.
///
/// The assignment back into `name` is not done here: Bash leaves the name
/// untouched when the open fails, so the number is only published once
/// there is a descriptor behind it.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn resolve_descriptor(
    shell: &mut Shell,
    redirection: &ExpandedRedirection<'_>,
) -> Result<LogicalDescriptor, Error> {
    match redirection.descriptor() {
        RedirectionDescriptor::Fixed(descriptor) => Ok(*descriptor),
        RedirectionDescriptor::Allocated(name) if redirection.closes() => {
            let name = BString::from(name.as_bstr());
            named_descriptor(shell, BStr::new(&name))
        }
        RedirectionDescriptor::Allocated(_) => allocate_descriptor(shell),
    }
}

/// The C's `next` is gone with the intrusive stack. Saved logical values are
/// shared owners: ordinary unwind restores them, while fork reset drops
/// obsolete backups without changing the active table.
pub struct RedirectionFrame {
    /// What each slot this frame redirected held before it did.
    ///
    /// A map rather than one entry per nameable slot: the set of slots a
    /// script can name is every decimal number, and a frame touches a
    /// handful. Absent means "this frame did not redirect that slot", so
    /// the enum that used to say so is the key's presence now. `Some(None)`
    /// is a slot that was genuinely closed, which is not the same thing.
    saved_descriptors: BTreeMap<LogicalDescriptor, Option<crate::descriptors::SharedDescriptor>>,
}

/// The stack of saved logical-descriptor states.
///
/// The fields are private to `redir.rs`, so `Shell` owns the value and
/// this module owns its shape — nothing outside can reach past the
/// functions below, which is the property the two `static mut`s it
/// replaces never had.
///
/// The live map is [`crate::descriptors::FdTable`]; this stack records only the
/// values needed to restore command-scoped redirections.
pub struct RedirectionStack {
    /// One frame per redirection scope, innermost last. A frame's *index*
    /// is what outlives a call here, never a borrow: `openredirect` can
    /// reach command substitution, which pushes and pops frames of its
    /// own and can move the vector out from under a reference.
    frames: Vec<RedirectionFrame>,
}

impl RedirectionStack {
    /// `redirlist = NULL` and `closed_redirs = 0`, which is what the two
    /// statics started at.
    pub(crate) const fn new() -> Self {
        RedirectionStack { frames: Vec::new() }
    }
}

/*
 * Process a list of redirection commands.  If the REDIR_PUSH flag is set,
 * old file descriptors are stashed away so that the redirection can be
 * undone by calling popredir.  If the REDIR_BACKQ flag is set, then the
 * standard output, and the standard error if it becomes a duplicate of
 * stdout, is saved in memory.
 */

// [spec:dash:sem:redir.redirect-fn]
// [spec:dash:sem:redir.update-closed-redirs-fn]
// [spec:posix:sem:shell.redirection-processing]
// [spec:posix:def:redir.purpose]
// [spec:posix:sem:redir.evaluation-order]
pub(crate) fn redirect(
    shell: &mut Shell,
    redirections: &[ExpandedRedirection<'_>],
    mode: RedirectionMode,
) -> Result<(), Error> {
    if redirections.is_empty() {
        return Ok(());
    }
    let saved_frame = crate::error::with_interrupts_deferred(shell, |shell| {
        /* `sv = redirlist` — the frame `pushredir` just pushed, and NULL when
         * there is none, which is what `checked_sub` says. */
        let saved_frame = if mode == RedirectionMode::Push {
            shell.redirections.frames.len().checked_sub(1)
        } else {
            None
        };
        /* The C walks the list through `n->nfile.next`, which is the same offset
         * in every redirection arm; the list is a `Vec` now. */
        for redirection in redirections {
            /* A slot the shell chose is the script's from here on. Bash
             * does not undo one when the command finishes -- `{ echo; }
             * {fd}<<< walrus` leaves `$fd` readable afterwards, where the
             * same group with `7<` gets 7 back -- so it is never saved
             * into the frame that would restore it. */
            // [spec:nsh:req:compat.bash.expansion-globbing]
            let allocates = matches!(
                redirection.descriptor(),
                RedirectionDescriptor::Allocated(_)
            ) && !redirection.closes();
            let descriptor = resolve_descriptor(shell, redirection)?;
            reject_unusable_descriptor(shell, descriptor)?;
            let source = open_redirection(shell, redirection, descriptor)?;
            if !matches!(source, RedirectSource::Noop) {
                /* The C's `fd == 0` is "this redirection replaced the shell's
                 * own input", which is what makes the buffered parse state
                 * stale -- not descriptor 0 for its own sake. */
                if descriptor == LogicalDescriptor::STDIN {
                    crate::input::reset_input(shell);
                }

                if let Some(frame_index) = saved_frame
                    && !allocates
                    && !shell.redirections.frames[frame_index]
                        .saved_descriptors
                        .contains_key(&descriptor)
                {
                    let saved = shell.descriptors.get(descriptor);
                    shell.redirections.frames[frame_index]
                        .saved_descriptors
                        .insert(descriptor, saved);
                }

                install_redirection(shell, descriptor, source)?;
                /* `{name}>file` publishes the slot it was given, and only
                 * now: an open that failed leaves the name as it was. */
                // [spec:nsh:req:compat.bash.expansion-globbing]
                if let RedirectionDescriptor::Allocated(name) = redirection.descriptor()
                    && allocates
                {
                    let name = BString::from(name.as_bstr());
                    let digits = descriptor.digits();
                    crate::variables::arrays::assign_text_target(
                        shell,
                        BStr::new(&name),
                        BStr::new(&digits),
                        false,
                    )?;
                }
            }
        }
        Ok(saved_frame)
    })?;
    /* The C indexes slot 2 because that is where the shell's stderr is.
     * The slot follows the frontend's stderr instead -- and if that was
     * put past the end of `renamed`, which covers the ten descriptors
     * redirection can name, there is nothing saved to point the trace
     * stream at and it stays where it was. */
    if mode == RedirectionMode::Push {
        if let Some(frame_index) = saved_frame {
            let saved_descriptors = &shell.redirections.frames[frame_index].saved_descriptors;
            if let Some(Some(saved)) = saved_descriptors.get(&LogicalDescriptor::STDERR) {
                let destination = crate::descriptors::DescriptorSlot::default();
                destination.replace(Some(saved.clone()));
                shell.io.previous_stderr().set_destination(destination);
            }
        }
    }
    Ok(())
}

// [spec:dash:sem:redir.sh-open-fail-fn]
// [spec:nsh:req:idiom.platform-errors]
fn open_error(
    shell: &mut crate::context::Shell,
    pathname: &BStr,
    mode: nsh_platform::OpenMode,
    error: &std::io::Error,
) -> Error {
    open_error_with_context(shell, pathname, mode, error, OpenFailureContext::Ordinary)
}

fn open_error_with_context(
    shell: &mut crate::context::Shell,
    pathname: &BStr,
    mode: nsh_platform::OpenMode,
    error: &std::io::Error,
    context: OpenFailureContext,
) -> Error {
    let (word, operation): (&[u8], crate::error::Operation) = if mode.creates() {
        (b"create", crate::error::Operation::Create)
    } else {
        (b"open", crate::error::Operation::Open)
    };
    let mut message = b"cannot ".to_vec();
    message.extend_from_slice(word);
    message.push(b' ');
    message.extend_from_slice(pathname);
    message.extend_from_slice(b": ");
    message.extend_from_slice(&crate::error::error_message(
        &shell.locale,
        error,
        operation,
    ));
    let status = context.status(shell.options.dialect(), error);
    let line = shell.evaluation.diagnostic_line;
    shell
        .diagnostics()
        .report(Error::other(line, i32::from(status.code()), &message))
}

#[derive(Copy, Clone)]
enum OpenFailureContext {
    Ordinary,
    CommandFile,
}

impl OpenFailureContext {
    // [spec:posix:req:sh.exit-status-values]
    // [spec:posix:req:xcu.exit-status.listed-values-binding]
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn status(
        self,
        dialect: crate::options::Dialect,
        error: &std::io::Error,
    ) -> crate::status::ExitStatus {
        if matches!(self, OpenFailureContext::CommandFile)
            && nsh_platform::is_path_error(error, nsh_platform::PathErrorKind::NotFound)
        {
            crate::status::ExitStatus::NOT_FOUND
        } else if dialect == crate::options::Dialect::Bash {
            /* Bash takes 1 from a redirection it could not open, where
             * dash takes 2. Both report and carry on. */
            crate::status::ExitStatus::FAILURE
        } else {
            crate::status::ExitStatus::from_code(2)
        }
    }
}

// [spec:dash:sem:redir.sh-open-fn]
// [spec:posix:req:xcurel.file-access-permissions]
// [spec:posix:req:xcurel.file-open-access-mode]
// [spec:posix:req:xcurel.pathname-resolution]
pub fn open_file(
    shell: &mut Shell,
    pathname: &BStr,
    mode: nsh_platform::OpenMode,
    may_fail: bool,
) -> Result<Option<Descriptor>, Error> {
    open_file_with_context(
        shell,
        pathname,
        mode,
        may_fail,
        OpenFailureContext::Ordinary,
    )
}

fn open_file_with_context(
    shell: &mut Shell,
    pathname: &BStr,
    mode: nsh_platform::OpenMode,
    may_fail: bool,
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
                if let Some(err) = crate::error::poll_interrupt(shell.interrupt_context()) {
                    return Err(err);
                }
                if crate::signal_inbox::signals().pending_signal().is_none() {
                    continue;
                }
                if may_fail {
                    return Ok(None);
                }
                return Err(open_error_with_context(
                    shell, pathname, mode, &error, context,
                ));
            }
            Err(_) if may_fail => return Ok(None),
            Err(error) => {
                return Err(open_error_with_context(
                    shell, pathname, mode, &error, context,
                ));
            }
        }
    }
}

/// Expand a file redirection's operand into the pathname it names.
///
/// Bash expands it the way it expands any other word -- brace expansion,
/// field splitting and pathname expansion included -- and refuses the
/// redirect unless exactly one field survives. POSIX expands the word
/// without splitting and uses whatever comes out, so the two dialects
/// differ in the *number* of results they will accept, not in the
/// expansions they run.
// [spec:nsh:req:compat.bash.expansion-globbing]
// [spec:posix:req:redir.word-expansion]
// [spec:posix:req:redir.word-pathname-expansion]
pub(crate) fn expand_file_target<'a>(
    shell: &mut Shell,
    redirection: &crate::nodes::FileRedirection,
) -> Result<ExpandedRedirection<'a>, Error> {
    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let mut fields = crate::expand::ExpandedFields::new();
    let target = Node::Word(redirection.target.clone());
    let mut mode = crate::expand::ExpansionMode::TILDE | crate::expand::ExpansionMode::REDIRECTION;
    if bash {
        mode = mode | crate::expand::ExpansionMode::SPLIT;
    }
    crate::expand::expand_argument(shell, &target, Some(&mut fields), mode)?;
    if !bash {
        /* `fn.list->text` with no null check: no EXP_FULL means
         * `expandarg` took its single-field arm. */
        debug_assert_eq!(fields.fields.len(), 1, "an unsplit expansion is one field");
    }
    if bash && fields.fields.len() != 1 {
        let mut word = BString::default();
        redirection.target.word.render(&mut word);
        return Ok(ExpandedRedirection::Ambiguous {
            descriptor: redirection.descriptor.clone(),
            word,
        });
    }
    Ok(ExpandedRedirection::File {
        operator: redirection.operator,
        descriptor: redirection.descriptor.clone(),
        target: fields.fields.remove(0).text,
    })
}

/// Open a path for input without exposing the platform's numeric open flags
/// to callers outside the redirection subsystem.
pub fn open_file_for_reading(
    shell: &mut Shell,
    pathname: &BStr,
    may_fail: bool,
) -> Result<Option<Descriptor>, Error> {
    open_file(shell, pathname, nsh_platform::OpenMode::ReadOnly, may_fail)
}

/// Open `sh`'s command-file operand, preserving its POSIX status class.
pub fn open_command_file(shell: &mut Shell, pathname: &BStr) -> Result<Descriptor, Error> {
    open_file_with_context(
        shell,
        pathname,
        nsh_platform::OpenMode::ReadOnly,
        false,
        OpenFailureContext::CommandFile,
    )
    .map(|descriptor| descriptor.expect("a mandatory command-file open returns a descriptor"))
}

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
/// Refuse a slot the host could never hold.
///
/// POSIX leaves the highest nameable descriptor implementation-defined,
/// and the table here is sparse, so a slot is only a map key until a child
/// is exec'd -- `exec 1000000>file` would otherwise succeed now and fail
/// much later, somewhere the script cannot see. The line is the host's
/// `RLIMIT_NOFILE`, which is the same line Bash reports at.
// [spec:posix:syn:redir.format]
fn reject_unusable_descriptor(
    shell: &mut Shell,
    descriptor: LogicalDescriptor,
) -> Result<(), Error> {
    if descriptor.index() < nsh_platform::descriptor_limit() as usize {
        return Ok(());
    }
    let mut message = descriptor.digits();
    message.extend_from_slice(b": Bad file descriptor");
    let line = shell.evaluation.diagnostic_line;
    Err(shell.diagnostics().report(Error::other(line, 1, &message)))
}

fn open_redirection(
    shell: &mut Shell,
    redirection: &ExpandedRedirection<'_>,
    descriptor: LogicalDescriptor,
) -> Result<RedirectSource, Error> {
    let source = match redirection {
        ExpandedRedirection::File {
            operator, target, ..
        } => open_file_redirection(shell, *operator, BStr::new(target.as_slice()))?,
        ExpandedRedirection::Descriptor { source, .. } => {
            open_descriptor_redirection(shell, descriptor, *source)?
        }
        ExpandedRedirection::HereDocument(document) => {
            RedirectSource::Owned(open_here_document(shell, document)?)
        }
        ExpandedRedirection::HereString { content, .. } => {
            let content = content.clone();
            RedirectSource::Owned(open_here_content(shell, &content)?)
        }
        ExpandedRedirection::Ambiguous { word, .. } => {
            let mut message = word.to_vec();
            message.extend_from_slice(b": ambiguous redirect");
            let line = shell.evaluation.diagnostic_line;
            return Err(shell.diagnostics().report(Error::other(line, 1, &message)));
        }
    };

    Ok(source)
}

fn open_file_redirection(
    shell: &mut Shell,
    operator: FileRedirectionOperator,
    target: &BStr,
) -> Result<RedirectSource, Error> {
    let source = match operator {
        FileRedirectionOperator::Read => RedirectSource::Owned(
            open_file(shell, target, nsh_platform::OpenMode::ReadOnly, false)?
                .expect("a mandatory open returns a descriptor"),
        ),
        FileRedirectionOperator::ReadWrite => RedirectSource::Owned(
            open_file(
                shell,
                target,
                nsh_platform::OpenMode::ReadWriteCreate,
                false,
            )?
            .expect("a mandatory open returns a descriptor"),
        ),
        FileRedirectionOperator::Write | FileRedirectionOperator::Clobber => {
            let mut fell_through = true;
            let mut opened = None;
            if operator == FileRedirectionOperator::Write {
                /* Take care of noclobber mode. */
                if shell.options.enabled(ShellOption::NoClobber) {
                    if !target
                        .try_to_path_buf()
                        .is_ok_and(|path| nsh_platform::path_exists(&path))
                    {
                        /* goto do_open */
                        return Ok(RedirectSource::Owned(
                            open_file(
                                shell,
                                target,
                                nsh_platform::OpenMode::WriteCreateExclusive,
                                false,
                            )?
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
                        return Err(open_error(
                            shell,
                            target,
                            nsh_platform::OpenMode::WriteCreateTruncate,
                            &error,
                        ));
                    }

                    let fv = open_file(shell, target, nsh_platform::OpenMode::WriteOnly, false)?
                        .expect("a mandatory open returns a descriptor");
                    match nsh_platform::fd_is_regular_file(&fv) {
                        Ok(true) => {
                            drop(fv);
                            /* goto ecreate */
                            let error = nsh_platform::platform_error(
                                nsh_platform::PlatformErrorKind::AlreadyExists,
                            );
                            return Err(open_error(
                                shell,
                                target,
                                nsh_platform::OpenMode::WriteCreateTruncate,
                                &error,
                            ));
                        }
                        Ok(false) => {}
                        Err(error) => {
                            return Err(open_error(
                                shell,
                                target,
                                nsh_platform::OpenMode::WriteOnly,
                                &error,
                            ));
                        }
                    }
                    opened = Some(fv);
                    fell_through = false;
                }
                /* FALLTHROUGH */
            }
            if fell_through {
                RedirectSource::Owned(
                    open_file(
                        shell,
                        target,
                        nsh_platform::OpenMode::WriteCreateTruncate,
                        false,
                    )?
                    .expect("a mandatory open returns a descriptor"),
                )
            } else {
                RedirectSource::Owned(opened.expect("the noclobber path opened a descriptor"))
            }
        }
        FileRedirectionOperator::Append => RedirectSource::Owned(
            open_file(
                shell,
                target,
                nsh_platform::OpenMode::WriteCreateAppend,
                false,
            )?
            .expect("a mandatory open returns a descriptor"),
        ),
    };

    Ok(source)
}

fn open_descriptor_redirection(
    shell: &mut Shell,
    descriptor: LogicalDescriptor,
    source: Option<LogicalDescriptor>,
) -> Result<RedirectSource, Error> {
    let Some(source) = source else {
        return Ok(RedirectSource::Close);
    };
    if source == descriptor {
        Ok(RedirectSource::Noop)
    } else {
        let source_fd = shell.descriptors.get(source).ok_or_else(|| {
            descriptor_error(
                shell,
                source,
                nsh_platform::platform_error(nsh_platform::PlatformErrorKind::BadDescriptor),
            )
        })?;
        Ok(RedirectSource::Shared(source_fd))
    }
}

/// The status is the dialect's. Every other refusal in this module
/// answers 1, which is what Bash's `redirection_error` takes for all of
/// them; this one took dash's `sh_error` 2 instead, and while a failed
/// redirection on a special built-in ended the shell the difference was
/// not visible from a script. It is now, because Bash mode carries on
/// past that failure: `echo foo >&10` answers 1 in Bash and 2 in dash,
/// and `is_fd_open() { : >&$1; }` -- the survey's way of finding the
/// first free descriptor -- reads the number back.
// [spec:nsh:req:compat.bash.error-boundary]
pub(crate) fn descriptor_error(
    shell: &mut Shell,
    source: LogicalDescriptor,
    error: std::io::Error,
) -> Error {
    let mut message = Vec::new();
    write!(&mut message, "{}", source).expect("writing to a Vec cannot fail");
    message.extend_from_slice(b": ");
    message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
    if shell.options.dialect() != crate::options::Dialect::Bash {
        return shell.diagnostics().shell_error(&message);
    }
    let line = shell.evaluation.diagnostic_line;
    shell.diagnostics().report(Error::other(line, 1, &message))
}

// [spec:dash:sem:redir.dupredirect-fn]
// [spec:dash:sem:redir.sh-dup2-fn]
// [spec:nsh:req:idiom.no-raw-fd-core]
fn install_redirection(
    shell: &mut Shell,
    target: LogicalDescriptor,
    source: RedirectSource,
) -> Result<(), Error> {
    match source {
        RedirectSource::Noop => Ok(()),
        RedirectSource::Close => {
            shell.descriptors.replace(target, None);
            Ok(())
        }
        RedirectSource::Shared(source) => {
            shell.descriptors.replace(target, Some(source));
            Ok(())
        }
        RedirectSource::Owned(source) => shell
            .descriptors
            .install_owned(target, source)
            .map(|_| ())
            .map_err(|error| descriptor_error(shell, target, error)),
    }
}

// [spec:dash:sem:redir.sh-pipe-fn]
// [spec:nsh:req:idiom.filesystem-account-bytes]
pub fn create_pipe(shell: &mut crate::context::Shell, memfd: bool) -> Result<(Pipe, bool), Error> {
    if memfd {
        if let Ok(read_fd) = nsh_platform::anonymous_file("dash") {
            let write_fd = nsh_platform::duplicate_fd(&read_fd)
                .map_err(|_| shell.diagnostics().shell_error(b"Pipe call failed"))?;
            let read = nsh_platform::move_fd_cloexec(read_fd, LogicalDescriptor::INHERITED as i32)
                .map_err(|_| shell.diagnostics().shell_error(b"Pipe call failed"))?;
            let write =
                nsh_platform::move_fd_cloexec(write_fd, LogicalDescriptor::INHERITED as i32)
                    .map_err(|_| shell.diagnostics().shell_error(b"Pipe call failed"))?;
            return Ok((Pipe { read, write }, true));
        }
    }

    let (read, write) =
        nsh_platform::pipe().map_err(|_| shell.diagnostics().shell_error(b"Pipe call failed"))?;
    let read = nsh_platform::move_fd_cloexec(read, LogicalDescriptor::INHERITED as i32)
        .map_err(|_| shell.diagnostics().shell_error(b"Pipe call failed"))?;
    let write = nsh_platform::move_fd_cloexec(write, LogicalDescriptor::INHERITED as i32)
        .map_err(|_| shell.diagnostics().shell_error(b"Pipe call failed"))?;
    Ok((Pipe { read, write }, false))
}

/*
 * Handle here documents.  Normally we fork off a process to write the
 * data to a pipe.  If the document is short, we can stuff the data in
 * the pipe without forking.
 */

// [spec:dash:sem:redir.openhere-fn]
// [spec:posix:sem:redir.here-doc-fd-type]
// [spec:posix:req:redir.here-doc-expansion]
fn open_here_document(shell: &mut Shell, document: &HereDocument) -> Result<Descriptor, Error> {
    let expanded_content;

    let content: &[u8] = if document.expand {
        let word_node = Node::Word(document.body.clone());
        crate::expand::expand_argument(
            shell,
            &word_node,
            None,
            crate::expand::ExpansionMode::QUOTED,
        )?;
        /* The C reads the expansion back out of the region as
         * `stackblock()`.  The expansion buffer is owned now, so the read is
         * named.  Two consequences, both in the port's favour: the bytes
         * cannot be moved by the `sh_pipe`/`forkshell` allocations below —
         * the C's were only safe because neither happens to `stalloc` — and
         * the result carries its own byte length. */
        expanded_content = bstr::BString::from(crate::expand::expansion_result(shell));
        expanded_content.as_slice()
    } else {
        document.body.word.as_bstr()
    };

    open_here_content(shell, content)
}

/// Publish a here-document or here-string body on a readable descriptor.
///
/// A short body is stuffed into the pipe or a memory file; a long one is
/// written by a forked writer, because the pipe would otherwise fill.
fn open_here_content(shell: &mut Shell, content: &[u8]) -> Result<Descriptor, Error> {
    let content_length = content.len();
    let (pipe, memory_backed) = create_pipe(shell, content_length > PIPE_BUFFER_SIZE)?;

    if memory_backed || content_length <= PIPE_BUFFER_SIZE {
        nsh_platform::write_all(&pipe.write, content)
            .map_err(|error| here_document_write_error(shell, error))?;
        if memory_backed {
            nsh_platform::seek_start(&pipe.write)
                .map_err(|error| here_document_write_error(shell, error))?;
        }
        /* goto out */
        drop(pipe.write);
        return Ok(pipe.read);
    }

    if matches!(
        crate::jobs::fork_shell(shell, None, None, crate::jobs::ForkMode::WithoutJob)?,
        nsh_platform::ForkResult::Child
    ) {
        drop(pipe.read);
        nsh_platform::configure_here_document_writer_signals();
        if let Err(error) = nsh_platform::write_all(&pipe.write, content) {
            drop(here_document_write_error(shell, error));
            nsh_platform::flush_coverage_profile();
            nsh_platform::exit_immediately(1);
        }
        nsh_platform::flush_coverage_profile();
        nsh_platform::exit_immediately(0);
    }
    /* out: */
    drop(pipe.write);
    Ok(pipe.read)
}

fn here_document_write_error(shell: &mut Shell, error: std::io::Error) -> Error {
    let mut message = b"here document write error: ".to_vec();
    message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
    shell.diagnostics().shell_error(&message)
}

/*
 * Undo the effects of the last redirection.
 */

// [spec:dash:sem:redir.popredir-fn]
pub fn pop_redirection(shell: &mut Shell, discard: bool) {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let frame_index = shell.redirections.frames.len() - 1;
        /* Ascending, as the fixed table's index walk was: a frame that
         * saved both 0 and 2 has to put them back in the same order it
         * used to, because restoring slot 0 resets the parser's input. */
        let saved = std::mem::take(&mut shell.redirections.frames[frame_index].saved_descriptors);
        for (descriptor, held) in saved {
            if discard {
                continue;
            }
            if descriptor == LogicalDescriptor::STDIN {
                crate::input::reset_input(shell);
            }
            shell.descriptors.replace(descriptor, held);
        }
        /* `redirlist = rp->next` — which also drops anything pushed above `rp`
         * and never popped, as the C's assignment did. */
        shell.redirections.frames.truncate(frame_index);
    });
}

/*
 * Undo all redirections.  Called on error or interrupt.
 */

impl Shell {
    /// Restore every command-scoped redirection before recovery or shutdown.
    pub(crate) fn restore_saved_redirections(&mut self) {
        while !self.redirections.frames.is_empty() {
            pop_redirection(self, false);
        }
    }

    /// Consume inherited restoration frames without changing active slots.
    pub(crate) fn discard_saved_redirections(&mut self) {
        let inherited = core::mem::take(&mut self.redirections.frames);
        drop(inherited);
    }
}

/*
 * Move a file descriptor to > 10.  Invokes sh_error on error unless
 * the original file dscriptor is not open.
 */

// [spec:dash:sem:redir.savefd-fn]
/// Move an owned descriptor above the shell redirection range.
pub fn move_descriptor_above(shell: &mut Shell, fd: Descriptor) -> Result<Descriptor, Error> {
    nsh_platform::move_fd_cloexec(fd, 10).map_err(|error| {
        let message = shell.locale.error_message(&error);
        shell.diagnostics().shell_error(message.as_bytes())
    })
}

/// Duplicate a process-table slot above the shell redirection range.
pub fn copy_slot_above(
    shell: &mut Shell,
    from: LogicalDescriptor,
) -> Result<Option<Descriptor>, Error> {
    let source = shell.descriptors.get(from);
    source
        .map(|source| nsh_platform::duplicate_cloexec(&source, 10))
        .transpose()
        .map_err(|error| descriptor_error(shell, from, error))
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
/// There is no longer a `setjmp`, handler, or saved interrupt counter here.
/// [`redirect`] owns a structured deferral scope and restores its caller's
/// depth on every ordinary return, including an error caught here.
// [spec:dash:sem:redir.redirectsafe-fn]
// [spec:dash:sem:expand.restore-handler-expandarg-fn]
pub(crate) fn redirect_safely(
    shell: &mut Shell,
    redirections: &[ExpandedRedirection<'_>],
    mode: RedirectionMode,
) -> Result<(), Error> {
    redirect(shell, redirections, mode)
}

// [spec:dash:sem:redir.unwindredir-fn]
/// `stop` was the `redirtab *` to unwind back to; a stack in a vector says
/// the same thing with the depth to unwind back to.
pub fn unwind_redirections(shell: &mut Shell, stop: usize) {
    while shell.redirections.frames.len() != stop {
        pop_redirection(shell, false);
    }
}

// [spec:dash:sem:redir.pushredir-fn]
pub(crate) fn push_redirections(
    shell: &mut Shell,
    redirections: &[ExpandedRedirection<'_>],
) -> usize {
    let depth = shell.redirections.frames.len();
    if redirections.is_empty() {
        return depth;
    }

    shell.redirections.frames.push(RedirectionFrame {
        saved_descriptors: BTreeMap::new(),
    });

    depth
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

    use super::{LogicalDescriptor, OpenFailureContext};
    use crate::Shell;

    // [spec:posix:req:sh.exit-status-values/test]
    // [spec:posix:req:xcu.exit-status.listed-values-binding/test]
    #[test]
    fn command_file_open_status() {
        let missing = nsh_platform::platform_error(nsh_platform::PlatformErrorKind::NotFound);
        let denied =
            nsh_platform::platform_error(nsh_platform::PlatformErrorKind::PermissionDenied);

        let posix = crate::options::Dialect::Posix;
        assert_eq!(
            OpenFailureContext::CommandFile.status(posix, &missing),
            crate::status::ExitStatus::NOT_FOUND,
        );
        assert_eq!(
            OpenFailureContext::Ordinary.status(posix, &missing).code(),
            2,
        );
        assert_eq!(
            OpenFailureContext::CommandFile
                .status(posix, &denied)
                .code(),
            2,
        );
        // Bash takes 1 from a redirection it could not open.
        // [spec:nsh:req:compat.bash.expansion-globbing/test]
        let bash = crate::options::Dialect::Bash;
        assert_eq!(
            OpenFailureContext::Ordinary.status(bash, &missing).code(),
            1,
        );
    }

    /// Opening directly into the target means the target was closed before
    /// the redirection. Unwind must close it again, not save and restore the
    /// file that the open itself just placed there.
    // [spec:dash:sem:redir.popredir-fn/test]
    #[test]
    fn open_into_target_restores_closed_slot() {
        let status = nsh_platform::run_in_child(|| {
            let mut shell = Shell::builder().build().unwrap();
            let descriptor = LogicalDescriptor::new(3).unwrap();
            shell.descriptors.replace(descriptor, None);
            if shell.run(b"{ :; } 3>/dev/null").is_err() {
                nsh_platform::exit_immediately(2);
            }
            if shell.descriptors.is_open(descriptor) {
                nsh_platform::exit_immediately(3);
            }
            nsh_platform::exit_immediately(0);
        })
        .unwrap();

        assert_eq!(status, 0, "child failed at step {status}");
    }
}
