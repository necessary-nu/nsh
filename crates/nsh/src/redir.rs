//! Literal port of `src/redir.c` / `src/redir.h`.
//! Rules: `docs/spec/port/src/redir.md`.

use libc::{c_char, c_int, c_uint, c_void, size_t};
use core::ptr::{addr_of_mut, null_mut};

use crate::error::{jmploc, INTOFF, INTON};
use crate::output::VaArg;
use crate::shell::cstr;
use crate::nodes::{
    Node, NAPPEND, NCLOBBER, NFROM, NFROMFD, NFROMTO, NTO, NTOFD, NXHERE,
};

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

/// One frame per redirection scope, innermost last. A frame's *index* is
/// what outlives a call here, never a borrow: `openredirect` can reach
/// command substitution, which pushes and pops frames of its own and can
/// move the vector out from under a reference.
pub static mut redirlist: Vec<redirtab> = Vec::new();

#[inline]
unsafe fn redirlist_mut() -> &'static mut Vec<redirtab> {
    &mut *addr_of_mut!(redirlist)
}

/* Bit map of currently closed file descriptors. */
static mut closed_redirs: c_uint = 0;

#[inline]
unsafe fn errno() -> c_int {
    *libc::__errno_location()
}

// [spec:dash:def:redir.update-closed-redirs-fn]
// [spec:dash:sem:redir.update-closed-redirs-fn]
unsafe fn update_closed_redirs(fd: c_int, nfd: c_int) -> c_uint {
    let val: c_uint = closed_redirs;
    let bit: c_uint = 1u32 << fd;

    if nfd >= 0 {
        closed_redirs &= !bit;
    } else {
        closed_redirs |= bit;
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
pub unsafe fn redirect(redir: &[Node], flags: c_int) {
    let sv: Option<usize>;
    let mut i: c_int;
    let mut fd: c_int;
    let mut newfd: c_int;

    /* #if notyet — the `memory[10]` in-memory sink is not compiled. */
    if redir.is_empty() {
        return;
    }
    INTOFF();
    /* `sv = redirlist` — the frame `pushredir` just pushed, and NULL when
     * there is none, which is what `checked_sub` says. */
    sv = if (flags & REDIR_PUSH) != 0 {
        redirlist_mut().len().checked_sub(1)
    } else {
        None
    };
    /* The C walks the list through `n->nfile.next`, which is the same offset
     * in every redirection arm; the list is a `Vec` now. */
    for n in redir {
        newfd = openredirect(n);
        if newfd >= -1 {
            fd = n.redir_fd();
            /* The C's `fd == 0` is "this redirection replaced the shell's
             * own input", which is what makes the buffered parse state
             * stale -- not descriptor 0 for its own sake. */
            if fd == crate::streams::streams().stdin {
                crate::input::reset_input();
            }

            if let Some(svi) = sv {
                let closed: c_uint;

                /* The C takes `p = &sv->renamed[fd]` before `fd` becomes -1,
                 * so the write below lands in the slot the read came from. */
                let p_slot = fd as usize;
                i = redirlist_mut()[svi].renamed[p_slot];

                closed = update_closed_redirs(fd, newfd);

                if i == EMPTY {
                    i = CLOSED;
                    if fd != newfd && closed == 0 {
                        i = savefd(fd, fd);
                        fd = -1;
                    }
                }

                redirlist_mut()[svi].renamed[p_slot] = i;
            }

            if fd != newfd {
                dupredirect(n, newfd);
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
            let renamed = redirlist_mut()[svi].renamed;
            if (serr as usize) < renamed.len() && renamed[serr as usize] >= 0 {
                crate::output::preverrout.fd = renamed[serr as usize];
            }
        }
    }
}

// [spec:dash:def:redir.sh-open-fail-fn]
// [spec:dash:sem:redir.sh-open-fail-fn]
unsafe fn sh_open_fail(pathname: *const c_char, flags: c_int, e: c_int) -> ! {
    let mut word: *const c_char;
    let mut action: c_int;

    word = b"open\0".as_ptr() as *const c_char;
    action = crate::error::E_OPEN;
    if (flags & libc::O_CREAT) != 0 {
        word = b"create\0".as_ptr() as *const c_char;
        action = crate::error::E_CREAT;
    }

    crate::error::sh_error(
        cstr(b"cannot %s %s: %s\0"),
        &[
            VaArg::Str(word),
            VaArg::Str(pathname),
            VaArg::Str(crate::error::errmsg(e, action)),
        ],
    )
}

// [spec:dash:def:redir.sh-open-fn]
// [spec:dash:sem:redir.sh-open-fn]
pub unsafe fn sh_open(pathname: *const c_char, flags: c_int, mayfail: c_int) -> c_int {
    let mut fd: c_int;
    let mut e: c_int;

    loop {
        fd = libc::open64(pathname, flags, 0o666);
        e = errno();
        if !(fd < 0 && e == libc::EINTR && crate::trap::pending_sig == 0) {
            break;
        }
    }

    if mayfail != 0 || fd >= 0 {
        return fd;
    }

    sh_open_fail(pathname, flags, e);
}

// [spec:dash:def:redir.openredirect-fn]
// [spec:dash:sem:redir.openredirect-fn]
unsafe fn openredirect(redir: &Node) -> c_int {
    let mut sb: libc::stat64 = core::mem::zeroed();
    let mut fname: *mut c_char = null_mut();
    let mut flags: c_int;
    let f: c_int;

    match redir.node_type() {
        NFROM => {
            flags = libc::O_RDONLY;
            /* do_open: */
            f = sh_open(redir.nfile().expfname_ptr(), flags, 0);
        }
        NFROMTO => {
            flags = libc::O_RDWR | libc::O_CREAT;
            f = sh_open(redir.nfile().expfname_ptr(), flags, 0);
        }
        NTO | NCLOBBER => {
            let mut fell_through = true;
            let mut fv: c_int = 0;
            if redir.node_type() == NTO {
                /* Take care of noclobber mode. */
                if crate::options::optlist[crate::options::Cflag] != 0 {
                    fname = redir.nfile().expfname_ptr();
                    if libc::stat64(fname, &mut sb) < 0 {
                        flags = libc::O_WRONLY | libc::O_CREAT | libc::O_EXCL;
                        /* goto do_open */
                        return sh_open(fname, flags, 0);
                    }

                    if (sb.st_mode & libc::S_IFMT) == libc::S_IFREG {
                        /* goto ecreate */
                        sh_open_fail(fname, libc::O_CREAT, libc::EEXIST);
                    }

                    fv = sh_open(fname, libc::O_WRONLY, 0);
                    if libc::fstat64(fv, &mut sb) == 0
                        && (sb.st_mode & libc::S_IFMT) == libc::S_IFREG
                    {
                        libc::close(fv);
                        /* goto ecreate */
                        sh_open_fail(fname, libc::O_CREAT, libc::EEXIST);
                    }
                    fell_through = false;
                }
                /* FALLTHROUGH */
            }
            if fell_through {
                flags = libc::O_WRONLY | libc::O_CREAT | libc::O_TRUNC;
                f = sh_open(redir.nfile().expfname_ptr(), flags, 0);
            } else {
                f = fv;
            }
        }
        NAPPEND => {
            flags = libc::O_WRONLY | libc::O_CREAT | libc::O_APPEND;
            f = sh_open(redir.nfile().expfname_ptr(), flags, 0);
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
                libc::abort();
            }
            f = openhere(redir);
        }
    }

    f
}

// [spec:dash:def:redir.sh-dup2-fn]
// [spec:dash:sem:redir.sh-dup2-fn]
unsafe fn sh_dup2(ofd: c_int, nfd: c_int, cfd: c_int) -> c_int {
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
        crate::error::sh_error(
            cstr(b"%d: %s\0"),
            &[VaArg::Int(ofd), VaArg::Str(libc::strerror(errno()))],
        );
    }

    nfd
}

// [spec:dash:def:redir.dupredirect-fn]
// [spec:dash:sem:redir.dupredirect-fn]
/// The extracted `def` signature carries a stray `#endif`; the real signature
/// (outside `#ifdef notyet`) is `static void dupredirect(union node *, int)`.
unsafe fn dupredirect(redir: &Node, f: c_int) {
    let fd: c_int = redir.redir_fd();

    if redir.node_type() == NTOFD || redir.node_type() == NFROMFD {
        /* if not ">&-" */
        if f >= 0 {
            sh_dup2(f, fd, -1);
            return;
        }
        libc::close(fd);
    } else {
        sh_dup2(f, fd, f);
    }
}

// [spec:dash:def:redir.sh-pipe-fn]
// [spec:dash:sem:redir.sh-pipe-fn]
pub unsafe fn sh_pipe(pip: *mut c_int, memfd: c_int) -> c_int {
    if memfd != 0 {
        *pip.offset(0) = if USE_MEMFD_CREATE != 0 {
            libc::memfd_create(b"dash\0".as_ptr() as *const c_char, 0)
        } else {
            -1
        };
        if *pip.offset(0) >= 0 {
            *pip.offset(1) = sh_dup2(*pip.offset(0), -1, *pip.offset(0));
            return 1;
        }
    }

    if libc::pipe(pip) < 0 {
        crate::error::sh_error(cstr(b"Pipe call failed\0"), &[]);
    }

    0
}

/*
 * Handle here documents.  Normally we fork off a process to write the
 * data to a pipe.  If the document is short, we can stuff the data in
 * the pipe without forking.
 */

// [spec:dash:def:redir.openhere-fn]
// [spec:dash:sem:redir.openhere-fn]
unsafe fn openhere(redir: &Node) -> c_int {
    let len: size_t;
    let mut pip: [c_int; 2] = [0; 2];
    let memfd: c_int;
    let mut p: *mut c_char;

    /* `redir->nhere.doc` is the slot `parseheredoc` filled; the C would have
     * dereferenced a null pointer had it not run. */
    let doc: &Node = redir.nhere().doc.get().unwrap();
    p = doc.narg().text.as_ptr();
    if redir.node_type() == NXHERE {
        crate::expand::expandarg(doc, None, crate::expand::EXP_QUOTED);
        /* The C reads the expansion back out of the region as
         * `stackblock()`.  The expansion buffer is owned now, so the read is
         * named.  Two consequences, both in the port's favour: the bytes
         * cannot be moved by the `sh_pipe`/`forkshell` allocations below —
         * the C's were only safe because neither happens to `stalloc` — and
         * they are still NUL-terminated by `argstr`, which is what the
         * `strlen` on the next line needs. */
        p = crate::expand::expansion_result();
    }

    len = libc::strlen(p) as size_t;
    memfd = sh_pipe(pip.as_mut_ptr(), (len > PIPESIZE) as c_int);

    if memfd != 0 || len <= PIPESIZE {
        crate::output::xwrite(pip[1], p as *const c_void, len);
        libc::lseek(pip[1], 0, libc::SEEK_SET);
        /* goto out */
        libc::close(pip[1]);
        return pip[0];
    }

    if crate::jobs::forkshell(None, None, crate::jobs::FORK_NOJOB) == 0 {
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
    pip[0]
}

/*
 * Undo the effects of the last redirection.
 */

// [spec:dash:def:redir.popredir-fn]
// [spec:dash:sem:redir.popredir-fn]
pub unsafe fn popredir(drop: c_int) {
    let rp: usize;
    let mut i: c_int;

    INTOFF();
    rp = redirlist_mut().len() - 1;
    i = 0;
    while i < 10 {
        let closed: c_uint;
        let renamed: c_int = redirlist_mut()[rp].renamed[i as usize];

        if renamed == EMPTY {
            i += 1;
            continue;
        }

        closed = if drop != 0 {
            1
        } else {
            update_closed_redirs(i, renamed)
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
                        crate::input::reset_input();
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
    redirlist_mut().truncate(rp);
    INTON();
}

/*
 * Undo all redirections.  Called on error or interrupt.
 */

/* mkinit EXITRESET fragment from src/redir.c:443-448. */
pub unsafe fn mkinit_exitreset() {
    /*
     * Discard all saved file descriptors.
     */
    unwindredir(0);
}

/* mkinit FORKRESET fragment from src/redir.c:450-452. */
pub unsafe fn mkinit_forkreset() {
    /* `redirlist = NULL`: the frames are abandoned, not popped, so no
     * descriptor is restored or closed.  The slots are plain integers, so
     * clearing the vector abandons them the same way. */
    redirlist_mut().clear();
}

/*
 * Move a file descriptor to > 10.  Invokes sh_error on error unless
 * the original file dscriptor is not open.
 */

// [spec:dash:def:redir.savefd-fn]
// [spec:dash:sem:redir.savefd-fn]
pub unsafe fn savefd(from: c_int, ofd: c_int) -> c_int {
    let newfd: c_int;
    let err: c_int;

    /* #if HAVE_F_DUPFD_CLOEXEC */
    newfd = libc::fcntl(from, libc::F_DUPFD_CLOEXEC, 10);

    err = if newfd < 0 { errno() } else { 0 };
    if err != libc::EBADF {
        libc::close(ofd);
        if err != 0 {
            crate::error::sh_error(
                cstr(b"%d: %s\0"),
                &[VaArg::Int(from), VaArg::Str(libc::strerror(err))],
            );
        } else if HAVE_F_DUPFD_CLOEXEC == 0 {
            libc::fcntl(newfd, libc::F_SETFD, libc::FD_CLOEXEC);
        }
    }

    newfd
}

// [spec:dash:def:redir.redirectsafe-fn]
// [spec:dash:sem:redir.redirectsafe-fn]
pub unsafe fn redirectsafe(redir: &[Node], flags: c_int) -> c_int {
    let err: c_int;
    let mut saveint: c_int = 0;
    let savehandler: *mut jmploc = crate::error::handler;
    let mut jmploc: jmploc = jmploc::new();
    let jl: *mut jmploc = addr_of_mut!(jmploc);

    crate::SAVEINT!(saveint);
    err = crate::eval::setjmp_catch(jl, || {
        crate::error::handler = jl;
        redirect(redir, flags);
    }) * 2;
    crate::expand::restore_handler_expandarg(savehandler, err);
    crate::RESTOREINT!(saveint);
    err
}

// [spec:dash:def:redir.unwindredir-fn]
// [spec:dash:sem:redir.unwindredir-fn]
/// `stop` was the `redirtab *` to unwind back to; a stack in a vector says
/// the same thing with the depth to unwind back to.
pub unsafe fn unwindredir(stop: usize) {
    while redirlist_mut().len() != stop {
        popredir(0);
    }
}

// [spec:dash:def:redir.pushredir-fn]
// [spec:dash:sem:redir.pushredir-fn]
pub unsafe fn pushredir(redir: &[Node]) -> usize {
    let q: usize;

    q = redirlist_mut().len();
    if redir.is_empty() {
        return q; /* goto out */
    }

    redirlist_mut().push(redirtab {
        renamed: [EMPTY; 10],
    });

    q
}
