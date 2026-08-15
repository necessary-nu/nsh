//! Literal port of `src/redir.c` / `src/redir.h`.
//! Rules: `docs/spec/port/src/redir.md`.

use crate::error::Error;
use core::ptr::null_mut;
use libc::{c_char, c_int, c_uint, c_void, size_t};
use std::ffi::CStr;
use std::io::Write;

use crate::context::Shell;
use crate::error::{INTOFF, INTON};
use crate::nodes::{NAPPEND, NCLOBBER, NFROM, NFROMFD, NFROMTO, NTO, NTOFD, NXHERE, Node};

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

const EMPTY: c_int = -2; /* marks an unused slot in redirtab */
const CLOSED: c_int = -1; /* fd opened for redir needs to be closed */

/// `PIPE_BUF` where available, 4096 otherwise.  4096 on Linux.
const PIPESIZE: size_t = 4096;

// [spec:dash:def:redir.redirtab]
/// `MKINIT struct redirtab { … }` — absent from the port manifest because the
/// `MKINIT` marker defeated the extractor.
///
/// The C's `next` is gone with the intrusive stack. The slots stay plain
/// `c_int`s rather than `OwnedFd`s on purpose: `popredir` restores with
/// `dup2` and then closes, it is reached from the unwind path, and giving a
/// slot a destructor would move a descriptor close to a point the C never
/// had one. `docs/std-replacements.md` §4.9.
#[repr(C)]
pub struct redirtab {
    pub renamed: [c_int; 10],
}

/// What this module owns of the shell's descriptor state: the stack of
/// saved-descriptor frames, and the bitmap of descriptors the shell has
/// closed rather than saved.
///
/// The fields are private to `redir.rs`, so `Shell` owns the value and
/// this module owns its shape — nothing outside can reach past the
/// functions below, which is the property the two `static mut`s it
/// replaces never had.
///
/// `docs/api-design.md` §5 puts both inside an `fds: FdTable` together
/// with a logical-to-real descriptor map that does not exist yet. When
/// that map arrives this becomes part of it; until then it is its own
/// field, because inventing the surrounding type to hold two members it
/// does not yet have would be guessing at the shape.
pub struct RedirStack {
    /// One frame per redirection scope, innermost last. A frame's *index*
    /// is what outlives a call here, never a borrow: `openredirect` can
    /// reach command substitution, which pushes and pops frames of its
    /// own and can move the vector out from under a reference.
    list: Vec<redirtab>,
    /// Bit map of currently closed file descriptors.
    closed: c_uint,
}

impl RedirStack {
    /// `redirlist = NULL` and `closed_redirs = 0`, which is what the two
    /// statics started at.
    pub(crate) const fn new() -> Self {
        RedirStack {
            list: Vec::new(),
            closed: 0,
        }
    }
}

use crate::system::errno;

// [spec:dash:def:redir.update-closed-redirs-fn]
// [spec:dash:sem:redir.update-closed-redirs-fn]
unsafe fn update_closed_redirs(sh: &mut Shell, fd: c_int, nfd: c_int) -> c_uint {
    let val: c_uint = sh.redirs.closed;
    let bit: c_uint = 1u32 << fd;

    if nfd >= 0 {
        sh.redirs.closed &= !bit;
    } else {
        sh.redirs.closed |= bit;
    }

    val & bit
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
pub unsafe fn redirect(
    sh: &mut Shell,
    redir: &[Node],
    flags: c_int,
) -> Result<(), Error> {
    let sv: Option<usize>;
    let mut i: c_int;
    let mut fd: c_int;
    let mut newfd: c_int;

    /* #if notyet — the `memory[10]` in-memory sink is not compiled. */
    if redir.is_empty() {
        return Ok(());
    }
    INTOFF();
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
        newfd = openredirect(sh, n)?;
        if newfd >= -1 {
            fd = n.redir_fd();
            /* The C's `fd == 0` is "this redirection replaced the shell's
             * own input", which is what makes the buffered parse state
             * stale -- not descriptor 0 for its own sake. */
            if fd == crate::streams::streams().stdin {
                crate::input::reset_input(sh);
            }

            if let Some(svi) = sv {
                let closed: c_uint;

                /* The C takes `p = &sv->renamed[fd]` before `fd` becomes -1,
                 * so the write below lands in the slot the read came from. */
                let p_slot = fd as usize;
                i = sh.redirs.list[svi].renamed[p_slot];

                closed = update_closed_redirs(sh, fd, newfd);

                if i == EMPTY {
                    i = CLOSED;
                    if fd != newfd && closed == 0 {
                        i = savefd(fd, fd)?;
                        fd = -1;
                    }
                }

                sh.redirs.list[svi].renamed[p_slot] = i;
            }

            if fd != newfd {
                /* The `?` returns between the INTOFF above and the INTON
                 * below, leaking the counter exactly as the longjmp out of
                 * `sh_dup2` did; see docs/errors-are-values.md 2.4. */
                dupredirect(n, newfd)?;
            }
        }
    }
    INTON();
    /* NB: REDIR_SAVEFD2 is 03, so this test also fires for a plain
     * REDIR_PUSH (01); reproduced verbatim (src/redir.c:184).
     *
     * The C indexes slot 2 because that is where the shell's stderr is.
     * The slot follows the frontend's stderr instead -- and if that was
     * put past the end of `renamed`, which covers the ten descriptors
     * redirection can name, there is nothing saved to point the trace
     * stream at and it stays where it was. */
    let serr: c_int = crate::streams::streams().stderr;
    if (flags & REDIR_SAVEFD2) != 0 {
        /* The C dereferences `sv` here without testing it, and gets away
         * with it because REDIR_SAVEFD2 is 03: every caller that reaches
         * this line passed REDIR_PUSH and so has a frame. */
        if let Some(svi) = sv {
            let renamed = sh.redirs.list[svi].renamed;
            if (serr as usize) < renamed.len() && renamed[serr as usize] >= 0 {
                (*crate::output::previous_stderr()).fd = renamed[serr as usize];
            }
        }
    }
    Ok(())
}

// [spec:dash:def:redir.sh-open-fail-fn]
// [spec:dash:sem:redir.sh-open-fail-fn]
unsafe fn sh_open_fail(pathname: *const c_char, flags: c_int, e: c_int) -> Error {
    let mut word: *const c_char;
    let mut action: c_int;

    word = b"open\0".as_ptr() as *const c_char;
    action = crate::error::E_OPEN;
    if (flags & libc::O_CREAT) != 0 {
        word = b"create\0".as_ptr() as *const c_char;
        action = crate::error::E_CREAT;
    }

    let mut message = b"cannot ".to_vec();
    message.extend_from_slice(CStr::from_ptr(word).to_bytes());
    message.push(b' ');
    message.extend_from_slice(CStr::from_ptr(pathname).to_bytes());
    message.extend_from_slice(b": ");
    message.extend_from_slice(CStr::from_ptr(crate::error::errmsg(e, action)).to_bytes());
    crate::error::sh_error_value(&message)
}

// [spec:dash:def:redir.sh-open-fn]
// [spec:dash:sem:redir.sh-open-fn]
pub unsafe fn sh_open(
    sh: &mut Shell,
    pathname: *const c_char,
    flags: c_int,
    mayfail: c_int,
) -> Result<c_int, Error> {
    let mut fd: c_int;
    let mut e: c_int;

    loop {
        fd = libc::open64(pathname, flags, 0o666);
        e = errno();
        if !(fd < 0 && e == libc::EINTR) {
            break;
        }
        /* An EINTR return is a place the shell is looking, so take
         * delivery here if an interrupt is due. `sa_flags = 0` is why
         * this return exists at all -- dash never restarts a syscall.
         * Otherwise retry, which is the C, whose extra `pending_sig == 0`
         * test this replaces: a signal that is pending but not *due*
         * (suppressed, or trapped and handled elsewhere) is no reason to
         * abandon the open. */
        if let Some(err) = crate::error::poll_interrupt(sh) {
            return Err(err);
        }
        if crate::trap::pending_sig != 0 {
            break;
        }
    }

    if mayfail != 0 || fd >= 0 {
        return Ok(fd);
    }

    Err(sh_open_fail(pathname, flags, e))
}

// [spec:dash:def:redir.openredirect-fn]
// [spec:dash:sem:redir.openredirect-fn]
unsafe fn openredirect(sh: &mut Shell, redir: &Node) -> Result<c_int, Error> {
    let mut sb: libc::stat64 = core::mem::zeroed();
    let mut fname: *mut c_char = null_mut();
    let mut flags: c_int;
    let f: c_int;

    match redir.node_type() {
        NFROM => {
            flags = libc::O_RDONLY;
            /* do_open: */
            f = sh_open(sh, redir.nfile().expfname_ptr(), flags, 0)?;
        }
        NFROMTO => {
            flags = libc::O_RDWR | libc::O_CREAT;
            f = sh_open(sh, redir.nfile().expfname_ptr(), flags, 0)?;
        }
        NTO | NCLOBBER => {
            let mut fell_through = true;
            let mut fv: c_int = 0;
            if redir.node_type() == NTO {
                /* Take care of noclobber mode. */
                if sh.options.flag(crate::options::Cflag) != 0 {
                    fname = redir.nfile().expfname_ptr();
                    if libc::stat64(fname, &mut sb) < 0 {
                        flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL;
                        /* goto do_open */
                        return sh_open(sh, fname, flags, 0);
                    }

                    if (sb.st_mode & libc::S_IFMT) == libc::S_IFREG {
                        /* goto ecreate */
                        return Err(sh_open_fail(fname, libc::O_CREAT, libc::EEXIST));
                    }

                    fv = sh_open(sh, fname, libc::O_WRONLY, 0)?;
                    if libc::fstat64(fv, &mut sb) == 0
                        && (sb.st_mode & libc::S_IFMT) == libc::S_IFREG
                    {
                        libc::close(fv);
                        /* goto ecreate */
                        return Err(sh_open_fail(fname, libc::O_CREAT, libc::EEXIST));
                    }
                    fell_through = false;
                }
                /* FALLTHROUGH */
            }
            if fell_through {
                flags = libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC;
                f = sh_open(sh, redir.nfile().expfname_ptr(), flags, 0)?;
            } else {
                f = fv;
            }
        }
        NAPPEND => {
            flags = libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND;
            f = sh_open(sh, redir.nfile().expfname_ptr(), flags, 0)?;
        }
        NTOFD | NFROMFD => {
            let mut fv = redir.ndup().dupfd.get();
            if fv == redir.ndup().fd {
                fv = -2;
            }
            f = fv;
        }
        /*
         * default:
         *   #ifdef DEBUG
         *      abort();
         *   #endif
         *   / * Fall through to eliminate warning. * /
         * case NHERE: case NXHERE:
         */
        _ => {
            if crate::shell::DEBUG {
                std::process::abort();
            }
            f = openhere(sh, redir)?;
        }
    }

    Ok(f)
}

// [spec:dash:def:redir.sh-dup2-fn]
// [spec:dash:sem:redir.sh-dup2-fn]
unsafe fn sh_dup2(ofd: c_int, nfd: c_int, cfd: c_int) -> Result<c_int, Error> {
    let mut nfd = nfd;
    let mut cfd = cfd;

    if nfd < 0 {
        nfd = libc::dup(ofd);
        if nfd >= 0 {
            cfd = -1;
        }
    } else {
        nfd = libc::dup2(ofd, nfd);
    }
    if cfd >= 0 {
        libc::close(cfd);
    }
    if nfd < 0 {
        let mut message = Vec::new();
        write!(&mut message, "{}", ofd).expect("writing to a Vec cannot fail");
        message.extend_from_slice(b": ");
        message.extend_from_slice(CStr::from_ptr(libc::strerror(errno())).to_bytes());
        return Err(crate::error::sh_error_value(&message));
    }

    Ok(nfd)
}

// [spec:dash:def:redir.dupredirect-fn]
// [spec:dash:sem:redir.dupredirect-fn]
/// The extracted `def` signature carries a stray `#endif`; the real signature
/// (outside `#ifdef notyet`) is `static void dupredirect(union node *, int)`.
unsafe fn dupredirect(redir: &Node, f: c_int) -> Result<(), Error> {
    let fd: c_int = redir.redir_fd();

    if redir.node_type() == NTOFD || redir.node_type() == NFROMFD {
        /* if not ">&-" */
        if f >= 0 {
            sh_dup2(f, fd, -1)?;
            return Ok(());
        }
        libc::close(fd);
    } else {
        sh_dup2(f, fd, f)?;
    }
    Ok(())
}

// [spec:dash:def:redir.sh-pipe-fn]
// [spec:dash:sem:redir.sh-pipe-fn]
pub unsafe fn sh_pipe(pip: *mut c_int, memfd: c_int) -> Result<c_int, Error> {
    if memfd != 0 {
        *pip.offset(0) = if USE_MEMFD_CREATE != 0 {
            libc::memfd_create(b"dash\0".as_ptr() as *const c_char, 0)
        } else {
            -1
        };
        if *pip.offset(0) >= 0 {
            *pip.offset(1) = sh_dup2(*pip.offset(0), -1, *pip.offset(0))?;
            return Ok(1);
        }
    }

    if libc::pipe(pip) < 0 {
        return Err(crate::error::sh_error_value(b"Pipe call failed"));
    }

    Ok(0)
}

/*
 * Handle here documents.  Normally we fork off a process to write the
 * data to a pipe.  If the document is short, we can stuff the data in
 * the pipe without forking.
 */

// [spec:dash:def:redir.openhere-fn]
// [spec:dash:sem:redir.openhere-fn]
unsafe fn openhere(sh: &mut Shell, redir: &Node) -> Result<c_int, Error> {
    let len: size_t;
    let mut pip: [c_int; 2] = [0; 2];
    let memfd: c_int;
    let mut p: *mut c_char;

    /* `redir->nhere.doc` is the slot `parseheredoc` filled; the C would have
     * dereferenced a null pointer had it not run. */
    let doc: &Node = redir.nhere().doc.get().unwrap();
    p = doc.narg().text.as_ptr();
    if redir.node_type() == NXHERE {
        crate::expand::expandarg(sh, doc, None, crate::expand::EXP_QUOTED)?;
        /* The C reads the expansion back out of the region as
         * `stackblock()`.  The expansion buffer is owned now, so the read is
         * named.  Two consequences, both in the port's favour: the bytes
         * cannot be moved by the `sh_pipe`/`forkshell` allocations below —
         * the C's were only safe because neither happens to `stalloc` — and
         * they are still NUL-terminated by `argstr`, which is what the
         * `strlen` on the next line needs. */
        p = crate::expand::expansion_result();
    }

    len = CStr::from_ptr(p).count_bytes();
    memfd = sh_pipe(pip.as_mut_ptr(), (len > PIPESIZE) as c_int)?;

    if memfd != 0 || len <= PIPESIZE {
        /* The return is discarded, as the C discards it, and the 8.3
         * audit flagged both this and the sibling in the forked child
         * below as places an interrupt could be dropped. They are not:
         * `output::write_fd` retries EINTR internally and the output path
         * is deliberately *not* a poll site -- dash collects output
         * errors in `outerr` and checks them separately rather than
         * raising, and making it fallible is the shape 4.3 argues
         * against. They become live the day someone changes that, and
         * that is a different node's decision. */
        crate::output::xwrite(pip[1], p as *const c_void, len);
        libc::lseek(pip[1], 0, libc::SEEK_SET);
        /* goto out */
        libc::close(pip[1]);
        return Ok(pip[0]);
    }

    if crate::jobs::forkshell(sh, None, None, crate::jobs::FORK_NOJOB)? == 0 {
        libc::close(pip[0]);
        libc::signal(libc::SIGINT, libc::SIG_IGN);
        libc::signal(libc::SIGQUIT, libc::SIG_IGN);
        libc::signal(libc::SIGHUP, libc::SIG_IGN);
        libc::signal(libc::SIGTSTP, libc::SIG_IGN);
        libc::signal(libc::SIGPIPE, libc::SIG_DFL);
        crate::output::xwrite(pip[1], p as *const c_void, len);
        crate::shell::flush_coverage();
        libc::_exit(0);
    }
    /* out: */
    libc::close(pip[1]);
    Ok(pip[0])
}

/*
 * Undo the effects of the last redirection.
 */

// [spec:dash:def:redir.popredir-fn]
// [spec:dash:sem:redir.popredir-fn]
pub unsafe fn popredir(sh: &mut Shell, drop: c_int) {
    let rp: usize;
    let mut i: c_int;

    INTOFF();
    rp = sh.redirs.list.len() - 1;
    i = 0;
    while i < 10 {
        let closed: c_uint;
        let renamed: c_int = sh.redirs.list[rp].renamed[i as usize];

        if renamed == EMPTY {
            i += 1;
            continue;
        }

        closed = if drop != 0 {
            1
        } else {
            update_closed_redirs(sh, i, renamed)
        };

        match renamed {
            CLOSED => {
                if closed == 0 {
                    libc::close(i);
                }
            }
            _ => {
                if drop == 0 {
                    if i == 0 {
                        crate::input::reset_input(sh);
                    }
                    libc::dup2(renamed, i);
                }
                libc::close(renamed);
            }
        }
        i += 1;
    }
    /* `redirlist = rp->next` — which also drops anything pushed above `rp`
     * and never popped, as the C's assignment did. */
    sh.redirs.list.truncate(rp);
    INTON();
}

/*
 * Undo all redirections.  Called on error or interrupt.
 */

/* mkinit EXITRESET fragment from src/redir.c:443-448. */
pub unsafe fn mkinit_exitreset(sh: &mut Shell) {
    /*
     * Discard all saved file descriptors.
     */
    unwindredir(sh, 0);
}

/* mkinit FORKRESET fragment from src/redir.c:450-452. */
pub unsafe fn mkinit_forkreset(sh: &mut Shell) {
    /* `redirlist = NULL`: the frames are abandoned, not popped, so no
     * descriptor is restored or closed.  The slots are plain integers, so
     * clearing the vector abandons them the same way. */
    sh.redirs.list.clear();
}

/*
 * Move a file descriptor to > 10.  Invokes sh_error on error unless
 * the original file dscriptor is not open.
 */

// [spec:dash:def:redir.savefd-fn]
// [spec:dash:sem:redir.savefd-fn]
pub unsafe fn savefd(from: c_int, ofd: c_int) -> Result<c_int, Error> {
    let newfd: c_int;
    let err: c_int;

    /* #if HAVE_F_DUPFD_CLOEXEC */
    newfd = libc::fcntl(from, libc::F_DUPFD_CLOEXEC, 10);

    err = if newfd < 0 { errno() } else { 0 };
    if err != libc::EBADF {
        libc::close(ofd);
        if err != 0 {
            let mut message = Vec::new();
            write!(&mut message, "{}", from).expect("writing to a Vec cannot fail");
            message.extend_from_slice(b": ");
            message.extend_from_slice(CStr::from_ptr(libc::strerror(err)).to_bytes());
            return Err(crate::error::sh_error_value(&message));
        } else if HAVE_F_DUPFD_CLOEXEC == 0 {
            libc::fcntl(newfd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }

    Ok(newfd)
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
pub unsafe fn redirectsafe(
    sh: &mut Shell,
    redir: &[Node],
    flags: c_int,
) -> Result<(), Error> {
    let mut saveint: c_int = 0;

    crate::SAVEINT!(saveint);
    let caught = crate::expand::restore_handler_expandarg(redirect(sh, redir, flags).err());
    if let Some(e) = caught {
        /* The C's `longjmp` from `restore_handler_expandarg` left before
         * the `RESTOREINT` below; so does this. */
        return Err(e);
    }
    crate::RESTOREINT!(saveint);

    Ok(())
}

// [spec:dash:def:redir.unwindredir-fn]
// [spec:dash:sem:redir.unwindredir-fn]
/// `stop` was the `redirtab *` to unwind back to; a stack in a vector says
/// the same thing with the depth to unwind back to.
pub unsafe fn unwindredir(sh: &mut Shell, stop: usize) {
    while sh.redirs.list.len() != stop {
        popredir(sh, 0);
    }
}

// [spec:dash:def:redir.pushredir-fn]
// [spec:dash:sem:redir.pushredir-fn]
pub unsafe fn pushredir(sh: &mut Shell, redir: &[Node]) -> usize {
    let q: usize;

    q = sh.redirs.list.len();
    if redir.is_empty() {
        return q; /* goto out */
    }

    sh.redirs.list.push(redirtab {
        renamed: [EMPTY; 10],
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

    use crate::error::Error;
    use crate::expand::restore_handler_expandarg;

    fn diagnostic() -> Error {
        Error::Other {
            line: 7,
            status: 2,
            message: bstr::BString::from(&b"Bad substitution"[..]),
        }
    }

    /// Nothing went wrong: nothing comes back.
    // [spec:dash:sem:expand.restore-handler-expandarg-fn/test]
    #[test]
    fn a_clean_frame_returns_nothing() {
        let _guard = crate::testutil::lock();
        unsafe {
            assert!(restore_handler_expandarg(None).is_none());
        }
    }

    /// A diagnostic is handed straight back, text, status and line
    /// intact -- the arm that used to be `exception == EXERROR`.
    // [spec:dash:sem:expand.restore-handler-expandarg-fn/test]
    #[test]
    fn caught_diagnostic_comes_back() {
        let _guard = crate::testutil::lock();
        unsafe {
            let got = restore_handler_expandarg(Some(diagnostic()))
                .expect("the caught diagnostic is the frame's to return");
            assert_eq!(got.message(), "Bad substitution");
            assert_eq!(got.status(), 2);
            assert_eq!(got.line(), 7);
            assert!(!got.is_interrupt());
        }
    }

    /// An interrupt comes back too, and is *not* the same arm: the C
    /// re-raised it from here rather than swallowing it, and the frames
    /// above must be able to tell the two apart. Getting this wrong is a
    /// shell that stops answering `^C`.
    // [spec:dash:sem:expand.restore-handler-expandarg-fn/test]
    #[test]
    fn an_interrupt_comes_back_as_one() {
        let _guard = crate::testutil::lock();
        unsafe {
            let got = restore_handler_expandarg(Some(Error::Interrupted {
                signal: libc::SIGINT,
            }))
            .expect("an interrupt must not be swallowed by this frame");
            assert!(got.is_interrupt());
            assert_eq!(got.status(), libc::SIGINT + 128);
        }
    }
}
