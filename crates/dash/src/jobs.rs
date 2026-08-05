//! Literal port of `src/jobs.c` / `src/jobs.h`.
//! Rules: `docs/spec/port/src/jobs.md`.
//!
//! Translation notes (literal, bug-for-bug):
//!   * `JOBS` is 1 in the default build (`src/shell.h`), so everything
//!     under `#if JOBS` is compiled. The `JOBS` constant is kept so the
//!     `!JOBS ||` / `! JOBS &&` expressions read as they do in C.
//!   * `struct job`'s C bitfields (`nprocs:16, state:8, sigint:1, …`)
//!     are expanded into separate fields of the same widths. Nothing in
//!     dash depends on the packing — `memset(jp, 0, sizeof *jp)` and
//!     `growjobtab`'s relocation are both expressed in terms of
//!     `sizeof(struct job)` — and the field widths (and therefore the
//!     truncation behaviour of `nprocs`) are preserved.
//!   * C `goto`s are reproduced with labelled blocks; a `goto` *into*
//!     the middle of a loop becomes an entry flag, and the two backward
//!     `goto`s in `cmdtxt` become an explicit label program counter.
//!   * `TRACE(...)` compiles to nothing without `DEBUG` and is dropped.

use core::ptr::{addr_of_mut, null_mut};
use libc::{c_char, c_int, c_uint, c_void, pid_t, size_t};

use crate::error::{INTOFF, INTON};
use crate::eval::exitstatus;
use crate::memalloc::{ckfree, ckmalloc, ckrealloc, makestrspace, savestr, stackblock};
use crate::nodes::{nodelist, Node};
use crate::nodes::{
    NAND, NAPPEND, NARG, NBACKGND, NCASE, NCLOBBER, NCMD, NDEFUN, NFOR, NFROM, NFROMFD, NFROMTO,
    NHERE, NIF, NNOT, NOR, NREDIR, NSEMI, NSUBSHELL, NTO, NTOFD, NUNTIL, NWHILE, NXHERE,
};
use crate::output::{out1, out2, output};
use crate::parser::{VSLENGTH, VSNORMAL, VSNUL, VSTYPE};

/* src/shell.h: JOBS defaults to 1 */
const JOBS: c_int = 1;

// ---------------------------------------------------------------------
// src/jobs.h
// ---------------------------------------------------------------------

/* Mode argument to forkshell.  Don't change FORK_FG or FORK_BG. */
pub const FORK_FG: c_int = 0;
pub const FORK_BG: c_int = 1;
pub const FORK_NOJOB: c_int = 2;

/* mode flags for showjob(s) */
pub const SHOW_PGID: c_int = 0x01; /* only show pgid - for jobs -p */
pub const SHOW_PID: c_int = 0x04; /* include process pid */
pub const SHOW_CHANGED: c_int = 0x08; /* only jobs whose state has changed */

/* job states */
pub const JOBRUNNING: c_int = 0; /* at least one proc running */
pub const JOBSTOPPED: c_int = 1; /* all procs are stopped */
pub const JOBDONE: c_int = 2; /* all procs are completed */

// [spec:dash:def:jobs.procstat]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct procstat {
    pub pid: pid_t,       /* process id */
    pub status: c_int,    /* last process status from wait() */
    pub cmd: *mut c_char, /* text of command being run */
}

// [spec:dash:def:jobs.job]
//
// The C original packs the counters and flags into one `uint32_t` of
// bitfields; the widths are preserved here as separate fields.
#[repr(C)]
pub struct job {
    pub ps0: procstat,     /* status of process */
    pub ps: *mut procstat, /* status or processes when more than one */
    pub stopstatus: c_int, /* status of a stopped job (#if JOBS) */
    pub nprocs: u16,       /* number of processes */
    pub state: u8,
    pub sigint: u8,         /* job was killed by SIGINT (#if JOBS) */
    pub jobctl: u8,         /* job running under job control (#if JOBS) */
    pub waited: u8,         /* true if this entry has been waited for */
    pub used: u8,           /* true if this entry is in used */
    pub changed: u8,        /* true if status has changed */
    pub prev_job: *mut job, /* previous job */
}

// ---------------------------------------------------------------------
// src/jobs.c module state
// ---------------------------------------------------------------------

/* mode flags for set_curjob */
const CUR_DELETE: c_uint = 2;
const CUR_RUNNING: c_uint = 1;
const CUR_STOPPED: c_uint = 0;

/* mode flags for dowait */
const DOWAIT_NONBLOCK: c_int = 0;
const DOWAIT_BLOCK: c_int = 1;
const DOWAIT_WAITCMD: c_int = 2;
const DOWAIT_WAITCMD_ALL: c_int = 4;

const _PATH_TTY: &[u8] = b"/dev/tty\0";
const _PATH_DEVNULL: &[u8] = b"/dev/null\0";

/* array of jobs */
static mut jobtab: *mut job = null_mut();
/* size of array */
static mut njobs: c_uint = 0;
/* pid of last background process */
pub static mut backgndpid: pid_t = 0;

/* pgrp of shell on invocation */
static mut initialpgrp: c_int = 0;
/* control terminal */
static mut ttyfd: c_int = -1;

/* current job */
static mut curjob: *mut job = null_mut();

/* Set if we are in the vforked child */
pub static mut vforked: c_int = 0;

/* true if doing job control */
pub static mut jobctl: c_int = 0;

/* user was warned about stopped jobs */
pub static mut job_warning: c_int = 0;

#[inline]
unsafe fn errno() -> c_int {
    *libc::__errno_location()
}

/* src/options.h: `#define iflag optlist[3]` and friends. */
#[inline]
unsafe fn iflag() -> c_int {
    crate::options::optlist[crate::options::iflag] as c_int
}
#[inline]
unsafe fn pipefail() -> c_int {
    crate::options::optlist[crate::options::pipefail] as c_int
}

// [spec:dash:def:jobs.onsigchild-fn]
// [spec:dash:sem:jobs.onsigchild-fn]
//
// `STATIC int onsigchild(void);` is declared under `#ifdef SYSV` in
// src/jobs.c (line 117) and is never defined anywhere in the tree — a
// vestige of System V SIGCHLD handling that was removed. There is no
// body to port; this is the annotated placeholder that records the
// omission. `#[cfg(any())]` mirrors the never-satisfied `#ifdef SYSV`.
#[cfg(any())]
unsafe fn onsigchild() -> c_int {
    unimplemented!("declared under #ifdef SYSV, never defined in dash")
}

// [spec:dash:def:jobs.set-curjob-fn]
// [spec:dash:sem:jobs.set-curjob-fn]
unsafe fn set_curjob(jp: *mut job, mode: c_uint) {
    let mut jp1: *mut job;
    let mut jpp: *mut *mut job;
    let curp: *mut *mut job;

    /* first remove from list */
    jpp = addr_of_mut!(curjob);
    curp = jpp;
    loop {
        jp1 = *jpp;
        if jp1 == jp {
            break;
        }
        jpp = addr_of_mut!((*jp1).prev_job);
    }
    *jpp = (*jp1).prev_job;

    /* Then re-insert in correct position */
    jpp = curp;
    match mode {
        CUR_RUNNING => {
            /* newly created job or backgrounded job,
            put after all stopped jobs. */
            loop {
                jp1 = *jpp;
                if JOBS == 0 || jp1.is_null() || (*jp1).state as c_int != JOBSTOPPED {
                    break;
                }
                jpp = addr_of_mut!((*jp1).prev_job);
            }
            /* FALLTHROUGH into CUR_STOPPED */
            (*jp).prev_job = *jpp;
            *jpp = jp;
        }
        CUR_STOPPED => {
            /* newly stopped job - becomes curjob */
            (*jp).prev_job = *jpp;
            *jpp = jp;
        }
        /* `default:` (DEBUG: abort()) falls through into CUR_DELETE:
         * the job is being deleted, so it is not re-inserted. */
        _ /* default, CUR_DELETE */ => {
            let _ = CUR_DELETE;
        }
    }
}

/*
 * Turn job control on and off.
 *
 * Note:  This code assumes that the third arg to ioctl is a character
 * pointer, which is true on Berkeley systems but not System V.  Since
 * System V doesn't have job control yet, this isn't a problem now.
 *
 * Called with interrupts off.
 */

// [spec:dash:def:jobs.xxtcsetpgrp-fn]
// [spec:dash:sem:jobs.xxtcsetpgrp-fn]
unsafe fn xxtcsetpgrp(pgrp: pid_t) {
    let fd: c_int = ttyfd;

    if fd < 0 {
        return;
    }

    xtcsetpgrp(fd, pgrp);
}

// [spec:dash:def:jobs.setjobctl-fn]
// [spec:dash:sem:jobs.setjobctl-fn]
pub unsafe fn setjobctl(on: c_int) {
    let mut on: c_int = on;
    let mut pgrp: c_int = -1;
    let mut fd: c_int;

    if on == jobctl || crate::shellmain::rootshell() == 0 {
        return;
    }
    if on != 0 {
        let ofd: c_int;
        ofd = crate::redir::sh_open(_PATH_TTY.as_ptr() as *const c_char, libc::O_RDWR, 1);
        fd = ofd;
        'after_dowhile: {
            'out_lbl: {
                'close_lbl: {
                    if fd < 0 {
                        fd += 3;
                        while libc::isatty(fd) == 0 {
                            fd -= 1;
                            if fd < 0 {
                                break 'out_lbl; // goto out
                            }
                        }
                    }
                    fd = crate::redir::savefd(fd, ofd);
                    loop {
                        /* while we are in the background */
                        pgrp = libc::tcgetpgrp(fd);
                        if pgrp < 0 {
                            break 'close_lbl; // goto close
                        }
                        if pgrp == libc::getpgrp() {
                            break 'after_dowhile; // `break` of the do/while
                        }
                        if iflag() == 0 {
                            break 'close_lbl; // goto close
                        }
                        libc::killpg(0, libc::SIGTTIN);
                    }
                }
                // close:
                libc::close(fd);
                fd = -1;
                // falls through into out:
            }
            // out:
            if iflag() == 0 {
                break 'after_dowhile; // `break` of the do/while
            }
            crate::sh_warnx!(
                b"can't access tty; job control turned off\0".as_ptr() as *const c_char
            );
            crate::options::optlist[crate::options::mflag] = 0;
            on = 0;
            let _ = on;
            return;
        }
        initialpgrp = pgrp;
        pgrp = crate::shellmain::rootpid;
    } else {
        /* turning job control off */
        fd = ttyfd;
        pgrp = initialpgrp;
    }

    crate::trap::setsignal(libc::SIGTSTP);
    crate::trap::setsignal(libc::SIGTTOU);
    crate::trap::setsignal(libc::SIGTTIN);
    if fd >= 0 {
        libc::setpgid(0, pgrp);
        xtcsetpgrp(fd, pgrp);

        if on == 0 {
            libc::close(fd);
            fd = -1;
        }
    }

    ttyfd = fd;
    jobctl = on;
}

// [spec:dash:def:jobs.killcmd-fn]
// [spec:dash:sem:jobs.killcmd-fn]
pub unsafe fn killcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    /* the `usage:` label is a backward goto whose body only calls the
     * noreturn sh_error, so it is reproduced as two calls with the
     * same message. */
    const USAGE: &[u8] =
        b"Usage: kill [-s sigspec | -signum | -sigspec] [pid | job]... or\nkill -l [exitstatus]\0";
    let mut argv: *mut *mut c_char = argv;
    let mut signo: c_int = -1;
    let mut list: c_int = 0;
    let mut i: c_int;
    let mut pid: pid_t;
    let mut jp: *mut job;

    if argc <= 1 {
        // usage:
        crate::sh_error!(USAGE.as_ptr() as *const c_char);
    }

    argv = argv.add(1);
    if **argv == b'-' as c_char {
        signo = crate::trap::decode_signal((*argv).add(1), 1);
        if signo < 0 {
            let mut c: c_int;

            loop {
                c = crate::options::nextopt(b"ls:\0".as_ptr() as *const c_char);
                if c == 0 {
                    break;
                }
                match c as u8 {
                    b's' => {
                        signo = crate::trap::decode_signal(crate::options::optionarg, 1);
                        if signo < 0 {
                            crate::sh_error!(
                                b"invalid signal number or name: %s\0".as_ptr()
                                    as *const c_char,
                                crate::options::optionarg
                            );
                        }
                    }
                    /* `default:` (DEBUG: abort()) falls through into 'l' */
                    _ /* default, 'l' */ => {
                        list = 1;
                    }
                }
            }
            argv = crate::options::argptr;
        } else {
            argv = argv.add(1);
        }
    }

    if list == 0 && signo < 0 {
        signo = libc::SIGTERM;
    }

    if (((signo < 0 || (*argv).is_null()) as c_int) ^ list) != 0 {
        // goto usage
        crate::sh_error!(USAGE.as_ptr() as *const c_char);
    }

    if list != 0 {
        let out: *mut output;

        out = out1;
        if (*argv).is_null() {
            crate::output::outstr(b"0\n\0".as_ptr() as *const c_char, out);
            i = 1;
            while i < crate::signames::NSIG as c_int {
                crate::outfmt!(
                    out,
                    (core::ptr::addr_of!(crate::mystring::snlfmt) as *const c_char),
                    crate::signames::signal_names[i as usize].as_ptr()
                );
                i += 1;
            }
            return 0;
        }
        signo = crate::mystring::number(*argv);
        if signo > 128 {
            signo -= 128;
        }
        if 0 < signo && signo < crate::signames::NSIG as c_int {
            crate::outfmt!(
                out,
                (core::ptr::addr_of!(crate::mystring::snlfmt) as *const c_char),
                crate::signames::signal_names[signo as usize].as_ptr()
            );
        } else {
            crate::sh_error!(
                b"invalid signal number or exit status: %s\0".as_ptr() as *const c_char,
                *argv
            );
        }
        return 0;
    }

    i = 0;
    loop {
        if **argv == b'%' as c_char {
            jp = getjob(*argv, 0);
            pid = -(*(*jp).ps.offset(0)).pid;
        } else {
            pid = if **argv == b'-' as c_char {
                -crate::mystring::number((*argv).add(1))
            } else {
                crate::mystring::number(*argv)
            };
        }
        if libc::kill(pid, signo) != 0 {
            crate::sh_warnx!(
                (core::ptr::addr_of!(crate::mystring::snlfmt) as *const c_char),
                libc::strerror(errno())
            );
            i = 1;
        }
        argv = argv.add(1);
        if (*argv).is_null() {
            break;
        }
    }

    i
}

// [spec:dash:def:jobs.jobno-fn]
// [spec:dash:sem:jobs.jobno-fn]
unsafe fn jobno(jp: *const job) -> c_int {
    ((jp as usize - jobtab as usize) / core::mem::size_of::<job>()) as c_int + 1
}

// [spec:dash:def:jobs.fgcmd-fn]
// [spec:dash:sem:jobs.fgcmd-fn]
pub unsafe fn fgcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut argv: *mut *mut c_char = argv;
    let mut jp: *mut job;
    let out: *mut output;
    let mode: c_int;
    let mut retval: c_int;

    mode = if **argv == b'f' as c_char {
        FORK_FG
    } else {
        FORK_BG
    };
    crate::options::nextopt((core::ptr::addr_of!(crate::shell::nullstr) as *const c_char));
    argv = crate::options::argptr;
    out = out1;
    loop {
        jp = getjob(*argv, 1);
        if mode == FORK_BG {
            set_curjob(jp, CUR_RUNNING);
            crate::outfmt!(out, b"[%d] \0".as_ptr() as *const c_char, jobno(jp));
        }
        crate::output::outstr((*(*jp).ps).cmd, out);
        showpipe(jp, out);
        retval = restartjob(jp, mode);

        if (*argv).is_null() {
            break;
        }
        argv = argv.add(1);
        if (*argv).is_null() {
            break;
        }
    }
    retval
}

// [spec:dash:def:jobs.bgcmd-fn]
// [spec:dash:sem:jobs.bgcmd-fn]
//
// The same function as `fgcmd` — `__attribute__((alias("fgcmd")))`
// where the compiler supports it; the portable fallback, reproduced
// here, forwards. `fgcmd` distinguishes the two by `argv[0]`.
pub unsafe fn bgcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    fgcmd(argc, argv)
}

// [spec:dash:def:jobs.restartjob-fn]
// [spec:dash:sem:jobs.restartjob-fn]
unsafe fn restartjob(jp: *mut job, mode: c_int) -> c_int {
    let mut ps: *mut procstat;
    let mut i: c_int;
    let status: c_int;
    let pgid: pid_t;

    INTOFF();
    'out_lbl: {
        if (*jp).state as c_int == JOBDONE {
            break 'out_lbl;
        }
        (*jp).state = JOBRUNNING as u8;
        pgid = (*(*jp).ps).pid;
        if mode == FORK_FG {
            xxtcsetpgrp(pgid);
        }
        libc::killpg(pgid, libc::SIGCONT);
        ps = (*jp).ps;
        i = (*jp).nprocs as c_int;
        loop {
            if libc::WIFSTOPPED((*ps).status) {
                (*ps).status = -1;
            }
            ps = ps.add(1);
            i -= 1;
            if i == 0 {
                break;
            }
        }
    }
    // out:
    status = if mode == FORK_FG { waitforjob(jp) } else { 0 };
    INTON();
    status
}

// [spec:dash:def:jobs.sprint-status-fn]
// [spec:dash:sem:jobs.sprint-status-fn]
unsafe fn sprint_status(os: *mut c_char, status: c_int, sigonly: c_int) -> c_int {
    let mut s: *mut c_char = os;
    let mut st: c_int;

    'out_lbl: {
        st = libc::WEXITSTATUS(status);
        if !libc::WIFEXITED(status) {
            st = libc::WSTOPSIG(status);
            if !libc::WIFSTOPPED(status) {
                st = libc::WTERMSIG(status);
            }
            if sigonly != 0 {
                if st == libc::SIGINT || st == libc::SIGPIPE {
                    break 'out_lbl;
                }
                if libc::WIFSTOPPED(status) {
                    break 'out_lbl;
                }
            }
            s = libc::stpncpy(s, libc::strsignal(st), 32);
            if libc::WCOREDUMP(status) {
                s = libc::stpcpy(s, b" (core dumped)\0".as_ptr() as *const c_char);
            }
        } else if sigonly == 0 {
            if st != 0 {
                s = s.offset(
                    crate::fmtstr!(s, 16, b"Done(%d)\0".as_ptr() as *const c_char, st) as isize,
                );
            } else {
                s = libc::stpcpy(s, b"Done\0".as_ptr() as *const c_char);
            }
        }
    }
    // out:
    (s as usize - os as usize) as c_int
}

// [spec:dash:def:jobs.showjob-fn]
// [spec:dash:sem:jobs.showjob-fn]
unsafe fn showjob(out: *mut output, jp: *mut job, mode: c_int) {
    let mut ps: *mut procstat;
    let psend: *mut procstat;
    let mut col: c_int;
    let indent: c_int;
    let mut s: [c_char; 80] = [0; 80];

    ps = (*jp).ps;

    if (mode & SHOW_PGID) != 0 {
        /* just output process (group) id of pipeline */
        crate::outfmt!(out, b"%d\n\0".as_ptr() as *const c_char, (*ps).pid);
        return;
    }

    col = crate::fmtstr!(
        s.as_mut_ptr(),
        16,
        b"[%d]   \0".as_ptr() as *const c_char,
        jobno(jp)
    );
    indent = col;

    if jp == curjob {
        s[(col - 2) as usize] = b'+' as c_char;
    } else if !curjob.is_null() && jp == (*curjob).prev_job {
        s[(col - 2) as usize] = b'-' as c_char;
    }

    if (mode & SHOW_PID) != 0 {
        col += crate::fmtstr!(
            s.as_mut_ptr().offset(col as isize),
            16,
            b"%d \0".as_ptr() as *const c_char,
            (*ps).pid
        );
    }

    psend = (*jp).ps.add((*jp).nprocs as usize);

    if (*jp).state as c_int == JOBRUNNING {
        /* scopy("Running", s + col) */
        libc::strcpy(
            s.as_mut_ptr().offset(col as isize),
            b"Running\0".as_ptr() as *const c_char,
        );
        col += 7; /* strlen("Running") */
    } else {
        let mut status: c_int = (*psend.offset(-1)).status;
        if (*jp).state as c_int == JOBSTOPPED {
            status = (*jp).stopstatus;
        }
        col += sprint_status(s.as_mut_ptr().offset(col as isize), status, 0);
    }

    /* `goto start` enters the do/while below at the `start:` label */
    let mut at_start = true;
    loop {
        if !at_start {
            /* for each process */
            col = crate::fmtstr!(
                s.as_mut_ptr(),
                48,
                b" |\n%*c%d \0".as_ptr() as *const c_char,
                indent,
                ' ' as c_int,
                (*ps).pid
            ) - 3;
        }
        at_start = false;

        // start:
        crate::outfmt!(
            out,
            b"%s%*c%s\0".as_ptr() as *const c_char,
            s.as_ptr(),
            if 33 - col >= 0 { 33 - col } else { 0 },
            ' ' as c_int,
            (*ps).cmd
        );
        if (mode & SHOW_PID) == 0 {
            showpipe(jp, out);
            break;
        }
        ps = ps.add(1);
        if ps == psend {
            crate::output::outcslow('\n' as c_int, out);
            break;
        }
    }

    (*jp).changed = 0;

    if (*jp).state as c_int == JOBDONE {
        /* TRACE(("showjob: freeing job %d\n", jobno(jp))); */
        freejob(jp);
    }
}

// [spec:dash:def:jobs.jobscmd-fn]
// [spec:dash:sem:jobs.jobscmd-fn]
pub unsafe fn jobscmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut argv: *mut *mut c_char = argv;
    let mut mode: c_int;
    let mut m: c_int;
    let out: *mut output;

    mode = 0;
    loop {
        m = crate::options::nextopt(b"lp\0".as_ptr() as *const c_char);
        if m == 0 {
            break;
        }
        if m == 'l' as c_int {
            mode = SHOW_PID;
        } else {
            mode = SHOW_PGID;
        }
    }

    out = out1;
    argv = crate::options::argptr;
    if !(*argv).is_null() {
        loop {
            showjob(out, getjob(*argv, 0), mode);
            argv = argv.add(1);
            if (*argv).is_null() {
                break;
            }
        }
    } else {
        showjobs(out, mode);
    }

    0
}

/*
 * Print a list of jobs.  If "change" is nonzero, only print jobs whose
 * statuses have changed since the last call to showjobs.
 */

// [spec:dash:def:jobs.showjobs-fn]
// [spec:dash:sem:jobs.showjobs-fn]
pub unsafe fn showjobs(out: *mut output, mode: c_int) {
    let mut jp: *mut job;

    /* TRACE(("showjobs(%x) called\n", mode)); */

    /* If not even one job changed, there is nothing to do */
    dowait(DOWAIT_NONBLOCK, null_mut());

    jp = curjob;
    while !jp.is_null() {
        if (mode & SHOW_CHANGED) == 0 || (*jp).changed != 0 {
            showjob(out, jp, mode);
        }
        jp = (*jp).prev_job;
    }
}

/*
 * Mark a job structure as unused.
 */

// [spec:dash:def:jobs.freejob-fn]
// [spec:dash:sem:jobs.freejob-fn]
unsafe fn freejob(jp: *mut job) {
    let mut ps: *mut procstat;
    let mut i: c_int;

    INTOFF();
    i = (*jp).nprocs as c_int;
    ps = (*jp).ps;
    loop {
        i -= 1;
        if i < 0 {
            break;
        }
        if (*ps).cmd != (core::ptr::addr_of!(crate::shell::nullstr) as *mut c_char) {
            ckfree((*ps).cmd as *mut c_void);
        }
        ps = ps.add(1);
    }
    if (*jp).ps != addr_of_mut!((*jp).ps0) {
        ckfree((*jp).ps as *mut c_void);
    }
    (*jp).used = 0;
    set_curjob(jp, CUR_DELETE);
    INTON();
}

// [spec:dash:def:jobs.waitcmd-fn]
// [spec:dash:sem:jobs.waitcmd-fn]
pub unsafe fn waitcmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut argv: *mut *mut c_char = argv;
    let mut jobp: *mut job;
    let mut retval: c_int;
    let mut jp: *mut job;

    crate::options::nextopt((core::ptr::addr_of!(crate::shell::nullstr) as *const c_char));
    retval = 0;

    argv = crate::options::argptr;
    'out_lbl: {
        if (*argv).is_null() {
            /* wait for all jobs */
            loop {
                jp = curjob;
                loop {
                    if jp.is_null() {
                        /* no running procs */
                        break 'out_lbl;
                    }
                    if (*jp).state as c_int == JOBRUNNING {
                        break;
                    }
                    (*jp).waited = 1;
                    jp = (*jp).prev_job;
                }
                if dowait(DOWAIT_WAITCMD_ALL, null_mut()) == 0 {
                    // sigout:
                    retval = 128 + crate::trap::pending_sig;
                    break 'out_lbl;
                }
            }
        }

        retval = 127;
        loop {
            'repeat: {
                if **argv != b'%' as c_char {
                    let pid: pid_t = crate::mystring::number(*argv);
                    jobp = curjob;
                    /* `goto start` enters the do/while at `start:` */
                    let mut at_start = true;
                    loop {
                        if !at_start {
                            /* C indexes `job->ps[job->nprocs - 1]` with the
                             * bitfield promoted to `int`, so nprocs == 0
                             * reads `ps[-1]`; the signed offset keeps that
                             * (bug-for-bug) instead of trapping. */
                            if (*(*jobp).ps.offset((*jobp).nprocs as c_int as isize - 1)).pid == pid
                            {
                                break;
                            }
                            jobp = (*jobp).prev_job;
                        }
                        at_start = false;
                        // start:
                        if jobp.is_null() {
                            break 'repeat;
                        }
                    }
                } else {
                    jobp = getjob(*argv, 0);
                }
                /* loop until process terminated or stopped */
                if dowait(DOWAIT_WAITCMD, jobp) == 0 {
                    // sigout:
                    retval = 128 + crate::trap::pending_sig;
                    break 'out_lbl;
                }
                (*jobp).waited = 1;
                retval = getstatus(jobp);
            }
            // repeat:
            argv = argv.add(1);
            if (*argv).is_null() {
                break;
            }
        }
    }
    // out:
    retval
}

/*
 * Convert a job name to a job structure.
 */

// [spec:dash:def:jobs.getjob-fn]
// [spec:dash:sem:jobs.getjob-fn]
unsafe fn getjob(name: *const c_char, getctl: c_int) -> *mut job {
    let mut jp: *mut job;
    let mut found: *mut job;
    let mut err_msg: *const c_char = b"No such job: %s\0".as_ptr() as *const c_char;
    let num: c_uint;
    let c: c_int;
    let mut p: *const c_char;
    /* C: `char *(*match)(const char *, const char *)`, assigned either
     * `prefix` or `strstr`. `strstr` is `extern "C"`, so it is reached
     * through a thin Rust-ABI wrapper to keep the two assignable to one
     * variable. */
    unsafe fn match_strstr(a: *const c_char, b: *const c_char) -> *mut c_char {
        libc::strstr(a, b)
    }
    let mut matchfn: unsafe fn(*const c_char, *const c_char) -> *mut c_char;

    'err_lbl: {
        'gotit_lbl: {
            'check_lbl: {
                'currentjob_lbl: {
                    jp = curjob;
                    p = name;
                    if p.is_null() {
                        break 'currentjob_lbl; // goto currentjob
                    }

                    if *p != b'%' as c_char {
                        break 'err_lbl; // goto err
                    }

                    p = p.add(1);
                    c = *p as c_int;
                    if c == 0 {
                        break 'currentjob_lbl; // goto currentjob
                    }

                    if *p.offset(1) == 0 {
                        if c == '+' as c_int || c == '%' as c_int {
                            break 'currentjob_lbl; // the currentjob: label body
                        } else if c == '-' as c_int {
                            if !jp.is_null() {
                                jp = (*jp).prev_job;
                            }
                            err_msg = b"No previous job\0".as_ptr() as *const c_char;
                            break 'check_lbl; // the check: label body
                        }
                    }

                    if crate::mystring::is_number(p) != 0 {
                        num = libc::atoi(p) as c_uint;
                        if num > 0 && num <= njobs {
                            jp = jobtab.add((num - 1) as usize);
                            if (*jp).used != 0 {
                                break 'gotit_lbl; // goto gotit
                            }
                            break 'err_lbl; // goto err
                        }
                    }

                    matchfn = crate::mystring::prefix;
                    if *p == b'?' as c_char {
                        matchfn = match_strstr;
                        p = p.add(1);
                    }

                    found = null_mut();
                    while !jp.is_null() {
                        if !matchfn((*(*jp).ps.offset(0)).cmd, p).is_null() {
                            if !found.is_null() {
                                break 'err_lbl; // goto err
                            }
                            found = jp;
                            err_msg = b"%s: ambiguous\0".as_ptr() as *const c_char;
                        }
                        jp = (*jp).prev_job;
                    }

                    if found.is_null() {
                        break 'err_lbl; // goto err
                    }
                    jp = found;

                    break 'gotit_lbl; /* fall through to gotit: */
                }
                // currentjob:
                err_msg = b"No current job\0".as_ptr() as *const c_char;
                // goto check
            }
            // check:
            if jp.is_null() {
                break 'err_lbl; // goto err
            }
            // goto gotit
        }
        // gotit:
        err_msg = b"job %s not created under job control\0".as_ptr() as *const c_char;
        if getctl != 0 && (*jp).jobctl == 0 {
            break 'err_lbl; // goto err
        }
        return jp;
    }
    // err:
    crate::sh_error!(err_msg, name);
}

/*
 * Return a new job structure.
 * Called with interrupts off.
 */

// [spec:dash:def:jobs.makejob-fn]
// [spec:dash:sem:jobs.makejob-fn]
pub unsafe fn makejob(nprocs: c_int) -> *mut job {
    let mut ps: *mut procstat;
    let mut jp: *mut job;
    let mut i: c_int;

    i = njobs as c_int;
    jp = jobtab;
    loop {
        i -= 1;
        if i < 0 {
            jp = growjobtab();
            break;
        }
        if (*jp).used == 0 {
            break;
        }
        if (*jp).state as c_int != JOBDONE || (*jp).waited == 0 {
            jp = jp.add(1);
            continue;
        }
        if jobctl != 0 {
            jp = jp.add(1);
            continue;
        }
        freejob(jp);
        break;
    }
    libc::memset(jp as *mut c_void, 0, core::mem::size_of::<job>());
    ps = addr_of_mut!((*jp).ps0);
    if nprocs > 1 {
        ps = ckmalloc(nprocs as size_t * core::mem::size_of::<procstat>()) as *mut procstat;
    }
    if jobctl != 0 {
        (*jp).jobctl = 1;
    }
    (*jp).prev_job = curjob;
    curjob = jp;
    (*jp).used = 1;
    (*jp).ps = ps;
    /* TRACE(("makejob(%d) returns %%%d\n", nprocs, jobno(jp))); */
    jp
}

// [spec:dash:def:jobs.growjobtab-fn]
// [spec:dash:sem:jobs.growjobtab-fn]
unsafe fn growjobtab() -> *mut job {
    let len: size_t;
    let offset: isize;
    let mut jp: *mut job;
    let mut jq: *mut job;

    len = njobs as size_t * core::mem::size_of::<job>();
    jq = jobtab;
    jp = ckrealloc(jq as *mut c_void, len + 4 * core::mem::size_of::<job>()) as *mut job;

    /* C computes `(char *)jp - (char *)jq`; jq may be NULL on the very
     * first growth, which `offset_from` would not allow, so the
     * subtraction is done on the integer values. */
    offset = (jp as usize).wrapping_sub(jq as usize) as isize;
    if offset != 0 {
        /* Relocate pointers */
        let mut l: size_t = len;

        jq = (jq as *mut c_char).wrapping_add(l) as *mut job;
        while l != 0 {
            l -= core::mem::size_of::<job>();
            jq = jq.wrapping_offset(-1);
            /* joff(p) == (struct job *)((char *)(p) + l) */
            let joff_jp: *mut job = (jp as *mut c_char).add(l) as *mut job;
            /* jmove(p) == (p) = (void *)((char *)(p) + offset) */
            if (*joff_jp).ps == addr_of_mut!((*jq).ps0) {
                (*joff_jp).ps =
                    ((*joff_jp).ps as *mut c_char).wrapping_offset(offset) as *mut procstat;
            }
            if !(*joff_jp).prev_job.is_null() {
                (*joff_jp).prev_job =
                    ((*joff_jp).prev_job as *mut c_char).wrapping_offset(offset) as *mut job;
            }
        }
        if !curjob.is_null() {
            curjob = (curjob as *mut c_char).wrapping_offset(offset) as *mut job;
        }
    }

    njobs += 4;
    jobtab = jp;
    jp = (jp as *mut c_char).add(len) as *mut job;
    jq = jp.add(3);
    loop {
        (*jq).used = 0;
        jq = jq.offset(-1);
        if !(jq >= jp) {
            break;
        }
    }
    jp
}

/*
 * Fork off a subshell.  If we are doing job control, give the subshell its
 * own process group.  Jp is a job structure that the job is to be added to.
 * N is the command that will be evaluated by the child.  Both jp and n may
 * be NULL.  The mode parameter can be one of the following:
 *	FORK_FG - Fork off a foreground process.
 *	FORK_BG - Fork off a background process.
 *	FORK_NOJOB - Like FORK_FG, but don't give the process its own
 *		     process group even if job control is on.
 *
 * When job control is turned off, background processes have their standard
 * input redirected to /dev/null (except for the second and later processes
 * in a pipeline).
 *
 * Called with interrupts off.
 */

// [spec:dash:def:jobs.forkchild-fn]
// [spec:dash:sem:jobs.forkchild-fn]
unsafe fn forkchild(jp: *mut job, n: *mut Node, mode: c_int) {
    let mut jp: *mut job = jp;
    let lvforked: c_int;
    let oldlvl: c_int;

    /* TRACE(("Child shell %d\n", getpid())); */

    oldlvl = crate::shellmain::shlvl;
    lvforked = vforked;

    if lvforked == 0 {
        crate::shellmain::mypid = 0;
        crate::shellmain::shlvl += 1;

        crate::init::forkreset(if mode == FORK_NOJOB { n } else { null_mut() });

        /* do job control only in root shell */
        jobctl = 0;
    }

    if mode != FORK_NOJOB && (*jp).jobctl != 0 && oldlvl == 0 {
        let pgrp: pid_t;

        if (*jp).nprocs == 0 {
            pgrp = libc::getpid();
            crate::shellmain::mypid = pgrp;
        } else {
            pgrp = (*(*jp).ps.offset(0)).pid;
        }
        /* This can fail because we are doing it in the parent also */
        libc::setpgid(0, pgrp);
        if mode == FORK_FG {
            xxtcsetpgrp(pgrp);
        }
        crate::trap::setsignal(libc::SIGTSTP);
        crate::trap::setsignal(libc::SIGTTOU);
    } else if mode == FORK_BG {
        crate::trap::ignoresig(libc::SIGINT);
        crate::trap::ignoresig(libc::SIGQUIT);
        if (*jp).nprocs == 0 {
            libc::close(0);
            crate::redir::sh_open(_PATH_DEVNULL.as_ptr() as *const c_char, libc::O_RDONLY, 0);
            /* Should call reset_input here, but it's harmless
             * for now.
             */
        }
    }
    if oldlvl == 0 && iflag() != 0 {
        crate::trap::setsignal(libc::SIGINT);
        crate::trap::setsignal(libc::SIGQUIT);
        crate::trap::setsignal(libc::SIGTERM);
    }

    if lvforked != 0 {
        return;
    }

    if jp.is_null() {
        return;
    }

    freejob(jp);

    if crate::parser::issimplecmd(n, (*crate::builtins::JOBSCMD).name.as_ptr()) != 0 {
        return;
    }

    jp = curjob;
    while !jp.is_null() {
        freejob(jp);
        jp = (*jp).prev_job;
    }
}

// [spec:dash:def:jobs.forkparent-fn]
// [spec:dash:sem:jobs.forkparent-fn]
unsafe fn forkparent(jp: *mut job, n: *mut Node, mode: c_int, pid: pid_t) {
    if pid < 0 {
        /* TRACE(("Fork failed, errno=%d", errno)); */
        if !jp.is_null() {
            freejob(jp);
        }
        crate::sh_error!(b"Cannot fork\0".as_ptr() as *const c_char);
        /* NOTREACHED */
    }

    /* TRACE(("In parent shell:  child = %d\n", pid)); */
    if jp.is_null() {
        return;
    }
    if mode != FORK_NOJOB && (*jp).jobctl != 0 {
        let pgrp: c_int;

        if (*jp).nprocs == 0 {
            pgrp = pid;
        } else {
            pgrp = (*(*jp).ps.offset(0)).pid;
        }
        /* This can fail because we are doing it in the child also */
        libc::setpgid(pid, pgrp);
    }
    if mode == FORK_BG {
        backgndpid = pid; /* set $! */
        set_curjob(jp, CUR_RUNNING);
        if crate::options::optlist[crate::options::iflag] != 0 {
            crate::output::outfmt(
                crate::output::out2,
                crate::shell::cstr(b"[%d] %d\n\0"),
                &[
                    crate::output::VaArg::Int(jobno(jp)),
                    crate::output::VaArg::Int(pid),
                ],
            );
        }
    }
    if !jp.is_null() {
        let ps: *mut procstat = (*jp).ps.add((*jp).nprocs as usize);
        (*jp).nprocs += 1;
        (*ps).pid = pid;
        (*ps).status = -1;
        (*ps).cmd = (core::ptr::addr_of!(crate::shell::nullstr) as *mut c_char);
        if jobctl != 0 && !n.is_null() {
            (*ps).cmd = commandtext(n);
        }
    }
}

// [spec:dash:def:jobs.forkshell-fn]
// [spec:dash:sem:jobs.forkshell-fn]
pub unsafe fn forkshell(jp: *mut job, n: *mut Node, mode: c_int) -> c_int {
    let pid: c_int;

    /* TRACE(("forkshell(%%%d, %p, %d) called\n", jobno(jp), n, mode)); */

    crate::input::flush_input();

    pid = libc::fork();
    if pid == 0 {
        forkchild(jp, n, mode);
    } else {
        forkparent(jp, n, mode, pid);
    }

    pid
}

// [spec:dash:def:jobs.vforkexec-fn]
// [spec:dash:sem:jobs.vforkexec-fn]
#[allow(deprecated)] /* libc marks vfork deprecated; dash relies on it */
pub unsafe fn vforkexec(
    n: *mut Node,
    argv: *mut *mut c_char,
    path: *const c_char,
    idx: c_int,
) -> *mut job {
    let jp: *mut job;
    let pid: c_int;

    jp = makejob(1);

    if crate::shellmain::mypid == 0 {
        crate::shellmain::mypid = libc::getpid();
    }
    vforked = crate::shellmain::mypid;

    pid = libc::vfork();

    if pid == 0 {
        forkchild(jp, n, FORK_FG);
        crate::exec::shellexec(argv, path, idx);
        /* NOTREACHED */
    }

    vforked = 0;
    forkparent(jp, n, FORK_FG, pid);

    jp
}

/*
 * Wait for job to finish.
 *
 * Under job control we have the problem that while a child process is
 * running interrupts generated by the user are sent to the child but not
 * to the shell.  This means that an infinite loop started by an inter-
 * active user may be hard to kill.  With job control turned off, an
 * interactive user may place an interactive program inside a loop.  If
 * the interactive program catches interrupts, the user doesn't want
 * these interrupts to also abort the loop.  The approach we take here
 * is to have the shell ignore interrupt signals while waiting for a
 * forground process to terminate, and then send itself an interrupt
 * signal if the child process was terminated by an interrupt signal.
 * Unfortunately, some programs want to do a bit of cleanup and then
 * exit on interrupt; unless these processes terminate themselves by
 * sending a signal to themselves (instead of calling exit) they will
 * confuse this approach.
 *
 * Called with interrupts off.
 */

// [spec:dash:def:jobs.waitforjob-fn]
// [spec:dash:sem:jobs.waitforjob-fn]
pub unsafe fn waitforjob(jp: *mut job) -> c_int {
    let st: c_int;

    /* TRACE(("waitforjob(%%%d) called\n", jp ? jobno(jp) : 0)); */
    dowait(
        if !jp.is_null() {
            DOWAIT_BLOCK
        } else {
            DOWAIT_NONBLOCK
        },
        jp,
    );
    if jp.is_null() {
        return exitstatus;
    }

    st = getstatus(jp);
    if (*jp).jobctl != 0 {
        xxtcsetpgrp(crate::shellmain::rootpid);
        /*
         * This is truly gross.
         * If we're doing job control, then we did a TIOCSPGRP which
         * caused us (the shell) to no longer be in the controlling
         * session -- so we wouldn't have seen any ^C/SIGINT.  So, we
         * intuit from the subprocess exit status whether a SIGINT
         * occurred, and if so interrupt ourselves.  Yuck.  - mycroft
         */
        if (*jp).sigint != 0 {
            libc::raise(libc::SIGINT);
        }
    }
    if JOBS == 0 || (*jp).state as c_int == JOBDONE {
        freejob(jp);
    }
    st
}

/*
 * Wait for a process to terminate.
 */

// [spec:dash:def:jobs.waitone-fn]
// [spec:dash:sem:jobs.waitone-fn]
unsafe fn waitone(block: c_int, jobp: *mut job) -> c_int {
    let pid: c_int;
    let mut status: c_int = 0;
    let mut jp: *mut job;
    let mut thisjob: *mut job = null_mut();
    let mut state: c_int = 0;

    INTOFF();
    /* TRACE(("dowait(%d) called\n", block)); */
    pid = waitproc(block, &mut status);
    /* TRACE(("wait returns pid %d, status=%d\n", pid, status)); */
    'out_lbl: {
        if pid <= 0 {
            break 'out_lbl;
        }

        'gotjob: {
            jp = curjob;
            while !jp.is_null() {
                let mut sp: *mut procstat;
                let spend: *mut procstat;
                if (*jp).state as c_int == JOBDONE {
                    jp = (*jp).prev_job;
                    continue;
                }
                state = JOBDONE;
                spend = (*jp).ps.add((*jp).nprocs as usize);
                sp = (*jp).ps;
                loop {
                    if (*sp).pid == pid {
                        /* TRACE(("Job %d: changing status of proc %d ...")); */
                        (*sp).status = status;
                        thisjob = jp;
                    }
                    'contin: {
                        if (*sp).status == -1 {
                            state = JOBRUNNING;
                        }
                        if state == JOBRUNNING {
                            break 'contin;
                        }
                        if libc::WIFSTOPPED((*sp).status) {
                            (*jp).stopstatus = (*sp).status;
                            state = JOBSTOPPED;
                        }
                    }
                    sp = sp.add(1);
                    if !(sp < spend) {
                        break;
                    }
                }
                if !thisjob.is_null() {
                    break 'gotjob;
                }
                jp = (*jp).prev_job;
            }
            break 'out_lbl;
        }
        // gotjob:
        if state != JOBRUNNING {
            (*thisjob).changed = 1;

            if (*thisjob).state as c_int != state {
                /* TRACE(("Job %d: changing state from %d to %d\n", ...)); */
                (*thisjob).state = state as u8;
                if state == JOBSTOPPED {
                    set_curjob(thisjob, CUR_STOPPED);
                }
            }
        }
    }
    // out:
    INTON();

    if !thisjob.is_null() && thisjob == jobp {
        let mut s: [c_char; 48 + 1] = [0; 49];
        let len: c_int;

        len = sprint_status(s.as_mut_ptr(), status, 1);
        if len != 0 {
            s[len as usize] = b'\n' as c_char;
            s[(len + 1) as usize] = 0;
            crate::output::outstr(s.as_ptr(), out2);
        }
    }
    pid
}

// [spec:dash:def:jobs.dowait-fn]
// [spec:dash:sem:jobs.dowait-fn]
unsafe fn dowait(block: c_int, jp: *mut job) -> c_int {
    let gotchld: c_int = core::ptr::read_volatile(addr_of_mut!(crate::trap::gotsigchld));
    let mut rpid: c_int;
    let mut pid: c_int;
    let mut block: c_int = block;

    if !jp.is_null() && (*jp).state as c_int != JOBRUNNING {
        block = DOWAIT_NONBLOCK;
    }

    if block == DOWAIT_NONBLOCK && gotchld == 0 {
        return 1;
    }

    rpid = 1;

    loop {
        pid = waitone(block, jp);
        rpid &= (pid != 0) as c_int;

        block &= !DOWAIT_WAITCMD_ALL;
        if pid == 0 || (!jp.is_null() && (*jp).state as c_int != JOBRUNNING) {
            block = DOWAIT_NONBLOCK;
        }
        if !(pid >= 0) {
            break;
        }
    }

    rpid
}

/*
 * Do a wait system call.  If block is zero, we return -1 rather than
 * blocking.  If block is DOWAIT_WAITCMD, we return 0 when a signal
 * other than SIGCHLD interrupted the wait.
 *
 * We use sigsuspend in conjunction with a non-blocking wait3 in
 * order to ensure that waitcmd exits promptly upon the reception
 * of a signal.
 *
 * For code paths other than waitcmd we either use a blocking wait3
 * or a non-blocking wait3.  For the latter case the caller of dowait
 * must ensure that it is called over and over again until all dead
 * children have been reaped.  Otherwise zombies may linger.
 */

// [spec:dash:def:jobs.waitproc-fn]
// [spec:dash:sem:jobs.waitproc-fn]
unsafe fn waitproc(block: c_int, status: *mut c_int) -> c_int {
    let mut oldmask: libc::sigset_t = core::mem::zeroed();
    let mut flags: c_int = if block == DOWAIT_BLOCK {
        0
    } else {
        libc::WNOHANG
    };
    let mut err: c_int;

    if jobctl != 0 {
        flags |= libc::WUNTRACED;
    }

    /* HAVE_WAIT3; the fallback is `waitpid((pid_t)-1, status, flags, NULL)`.
     * `wait3` has no binding in the `libc` crate, so it is declared here. */
    extern "C" {
        fn wait3(status: *mut c_int, options: c_int, rusage: *mut libc::rusage) -> pid_t;
    }

    /* `gotsigchld` and `pending_sig` are `volatile sig_atomic_t` in C, so
     * every plain C access below is a volatile access; spell that out. */
    loop {
        core::ptr::write_volatile(addr_of_mut!(crate::trap::gotsigchld), 0);
        loop {
            err = wait3(status, flags, null_mut());
            if !(err < 0 && errno() == libc::EINTR) {
                break;
            }
        }

        if err != 0 {
            break;
        }
        err = -((block == 0) as c_int);
        if err != 0 {
            break;
        }

        crate::trap::sigblockall(&mut oldmask);

        while core::ptr::read_volatile(addr_of_mut!(crate::trap::gotsigchld)) == 0
            && core::ptr::read_volatile(addr_of_mut!(crate::trap::pending_sig)) == 0
        {
            libc::sigsuspend(&oldmask);
        }

        crate::system::sigclearmask();

        if core::ptr::read_volatile(addr_of_mut!(crate::trap::gotsigchld)) == 0 {
            break;
        }
    }

    err
}

/*
 * return 1 if there are stopped jobs, otherwise 0
 */

// [spec:dash:def:jobs.stoppedjobs-fn]
// [spec:dash:sem:jobs.stoppedjobs-fn]
pub unsafe fn stoppedjobs() -> c_int {
    let jp: *mut job;
    let mut retval: c_int;

    retval = 0;
    'out_lbl: {
        if JOBS == 0 {
            break 'out_lbl;
        }
        if job_warning != 0 {
            break 'out_lbl;
        }
        jp = curjob;
        if !jp.is_null() && (*jp).state as c_int == JOBSTOPPED {
            crate::output::out2str(b"You have stopped jobs.\n\0".as_ptr() as *const c_char);
            job_warning = 2;
            retval += 1;
        }
    }
    // out:
    retval
}

/*
 * Return a string identifying a command (to be printed by the
 * jobs command).
 */

static mut cmdnextc: *mut c_char = null_mut();

// [spec:dash:def:jobs.commandtext-fn]
// [spec:dash:sem:jobs.commandtext-fn]
unsafe fn commandtext(n: *mut Node) -> *mut c_char {
    let name: *mut c_char;

    /* STARTSTACKSTR(cmdnextc) */
    cmdnextc = stackblock() as *mut c_char;
    cmdtxt(n);
    name = stackblock() as *mut c_char;
    /* TRACE(("commandtext: name %p, end %p\n", name, cmdnextc)); */
    savestr(name)
}

// [spec:dash:def:jobs.cmdtxt-fn]
// [spec:dash:sem:jobs.cmdtxt-fn]
//
// `cmdtxt` has two *backward* gotos (`goto dodo` from NFOR and
// `goto donode` from the redirection tail), so the label graph is
// expressed as an explicit program counter rather than as nested
// labelled blocks.
unsafe fn cmdtxt(n: *mut Node) {
    let mut n: *mut Node = n;
    let mut np: *mut Node;
    let mut lp: *mut nodelist;
    let mut p: *const c_char = core::ptr::null();
    let mut s: [c_char; 2] = [0; 2];

    const L_SWITCH: c_int = 0;
    const L_BINOP: c_int = 1;
    const L_DONODE: c_int = 2;
    const L_UNTIL: c_int = 3;
    const L_DODO: c_int = 4;
    const L_DOTAIL: c_int = 5;
    const L_DOTAIL2: c_int = 6;
    const L_REDIR: c_int = 7;

    if n.is_null() {
        return;
    }

    let mut pc: c_int = L_SWITCH;
    loop {
        match pc {
            L_SWITCH => match (*n).r#type {
                NSEMI => {
                    p = b"; \0".as_ptr() as *const c_char;
                    pc = L_BINOP;
                }
                NAND => {
                    p = b" && \0".as_ptr() as *const c_char;
                    pc = L_BINOP;
                }
                NOR => {
                    p = b" || \0".as_ptr() as *const c_char;
                    pc = L_BINOP;
                }
                NREDIR | NBACKGND => {
                    n = (*n).nredir.n;
                    pc = L_DONODE;
                }
                NNOT => {
                    cmdputs(b"!\0".as_ptr() as *const c_char);
                    n = (*n).nnot.com;
                    pc = L_DONODE;
                }
                NIF => {
                    cmdputs(b"if \0".as_ptr() as *const c_char);
                    cmdtxt((*n).nif.test);
                    cmdputs(b"; then \0".as_ptr() as *const c_char);
                    if !(*n).nif.elsepart.is_null() {
                        cmdtxt((*n).nif.ifpart);
                        cmdputs(b"; else \0".as_ptr() as *const c_char);
                        n = (*n).nif.elsepart;
                    } else {
                        n = (*n).nif.ifpart;
                    }
                    p = b"; fi\0".as_ptr() as *const c_char;
                    pc = L_DOTAIL;
                }
                NSUBSHELL => {
                    cmdputs(b"(\0".as_ptr() as *const c_char);
                    n = (*n).nredir.n;
                    p = b")\0".as_ptr() as *const c_char;
                    pc = L_DOTAIL;
                }
                NWHILE => {
                    p = b"while \0".as_ptr() as *const c_char;
                    pc = L_UNTIL;
                }
                NUNTIL => {
                    p = b"until \0".as_ptr() as *const c_char;
                    pc = L_UNTIL;
                }
                NFOR => {
                    cmdputs(b"for \0".as_ptr() as *const c_char);
                    cmdputs((*n).nfor.var);
                    cmdputs(b" in \0".as_ptr() as *const c_char);
                    cmdlist((*n).nfor.args, 1);
                    n = (*n).nfor.body;
                    p = b"; done\0".as_ptr() as *const c_char;
                    pc = L_DODO;
                }
                NDEFUN => {
                    cmdputs((*n).ndefun.text);
                    p = b"() { ... }\0".as_ptr() as *const c_char;
                    pc = L_DOTAIL2;
                }
                NCMD => {
                    cmdlist((*n).ncmd.args, 1);
                    cmdlist((*n).ncmd.redirect, 0);
                    return;
                }
                NARG => {
                    p = (*n).narg.text;
                    pc = L_DOTAIL2;
                }
                NHERE | NXHERE => {
                    p = b"<<...\0".as_ptr() as *const c_char;
                    pc = L_DOTAIL2;
                }
                NCASE => {
                    cmdputs(b"case \0".as_ptr() as *const c_char);
                    cmdputs((*(*n).ncase.expr).narg.text);
                    cmdputs(b" in \0".as_ptr() as *const c_char);
                    np = (*n).ncase.cases;
                    while !np.is_null() {
                        cmdtxt((*np).nclist.pattern);
                        cmdputs(b") \0".as_ptr() as *const c_char);
                        cmdtxt((*np).nclist.body);
                        cmdputs(b";; \0".as_ptr() as *const c_char);
                        np = (*np).nclist.next;
                    }
                    p = b"esac\0".as_ptr() as *const c_char;
                    pc = L_DOTAIL2;
                }
                NTO => {
                    p = b">\0".as_ptr() as *const c_char;
                    pc = L_REDIR;
                }
                NCLOBBER => {
                    p = b">|\0".as_ptr() as *const c_char;
                    pc = L_REDIR;
                }
                NAPPEND => {
                    p = b">>\0".as_ptr() as *const c_char;
                    pc = L_REDIR;
                }
                NTOFD => {
                    p = b">&\0".as_ptr() as *const c_char;
                    pc = L_REDIR;
                }
                NFROM => {
                    p = b"<\0".as_ptr() as *const c_char;
                    pc = L_REDIR;
                }
                NFROMFD => {
                    p = b"<&\0".as_ptr() as *const c_char;
                    pc = L_REDIR;
                }
                NFROMTO => {
                    p = b"<>\0".as_ptr() as *const c_char;
                    pc = L_REDIR;
                }
                /* `default:` is empty outside DEBUG, so an unrecognised
                 * node type falls straight through into `case NPIPE:`.
                 * Reproduced bug-for-bug. */
                _ /* default, NPIPE */ => {
                    lp = (*n).npipe.cmdlist;
                    loop {
                        cmdtxt((*lp).n);
                        lp = (*lp).next;
                        if lp.is_null() {
                            break;
                        }
                        cmdputs(b" | \0".as_ptr() as *const c_char);
                    }
                    return;
                }
            },
            L_BINOP => {
                // binop:
                cmdtxt((*n).nbinary.ch1);
                cmdputs(p);
                n = (*n).nbinary.ch2;
                pc = L_DONODE;
            }
            L_DONODE => {
                // donode:
                cmdtxt(n);
                return;
            }
            L_UNTIL => {
                // until:
                cmdputs(p);
                cmdtxt((*n).nbinary.ch1);
                n = (*n).nbinary.ch2;
                p = b"; done\0".as_ptr() as *const c_char;
                pc = L_DODO;
            }
            L_DODO => {
                // dodo:
                cmdputs(b"; do \0".as_ptr() as *const c_char);
                pc = L_DOTAIL;
            }
            L_DOTAIL => {
                // dotail:
                cmdtxt(n);
                pc = L_DOTAIL2;
            }
            L_DOTAIL2 => {
                // dotail2:
                cmdputs(p);
                return;
            }
            _ /* L_REDIR */ => {
                // redir:
                s[0] = ((*n).nfile.fd + '0' as c_int) as c_char;
                s[1] = b'\0' as c_char;
                cmdputs(s.as_ptr());
                cmdputs(p);
                if (*n).r#type == NTOFD || (*n).r#type == NFROMFD {
                    s[0] = ((*n).ndup.dupfd + '0' as c_int) as c_char;
                    p = s.as_ptr();
                    pc = L_DOTAIL2;
                } else {
                    n = (*n).nfile.fname;
                    pc = L_DONODE;
                }
            }
        }
    }
}

// [spec:dash:def:jobs.cmdlist-fn]
// [spec:dash:sem:jobs.cmdlist-fn]
unsafe fn cmdlist(np: *mut Node, sep: c_int) {
    let mut np: *mut Node = np;

    while !np.is_null() {
        if sep == 0 {
            cmdputs((core::ptr::addr_of!(crate::mystring::spcstr) as *const c_char));
        }
        cmdtxt(np);
        if sep != 0 && !(*np).narg.next.is_null() {
            cmdputs((core::ptr::addr_of!(crate::mystring::spcstr) as *const c_char));
        }
        np = (*np).narg.next;
    }
}

// [spec:dash:def:jobs.cmdputs-fn]
// [spec:dash:sem:jobs.cmdputs-fn]
unsafe fn cmdputs(s: *const c_char) {
    const CTLESC_C: c_char = crate::parser::CTLESC as c_char;
    const CTLVAR_C: c_char = crate::parser::CTLVAR as c_char;
    const CTLENDVAR_C: c_char = crate::parser::CTLENDVAR as c_char;
    const CTLBACKQ_C: c_char = crate::parser::CTLBACKQ as c_char;
    const CTLARI_C: c_char = crate::parser::CTLARI as c_char;
    const CTLENDARI_C: c_char = crate::parser::CTLENDARI as c_char;
    const CTLQUOTEMARK_C: c_char = crate::parser::CTLQUOTEMARK as c_char;

    static vstype: [[c_char; 4]; (VSTYPE + 1) as usize] = [
        [0, 0, 0, 0],
        [b'}' as c_char, 0, 0, 0],
        [b'-' as c_char, 0, 0, 0],
        [b'+' as c_char, 0, 0, 0],
        [b'?' as c_char, 0, 0, 0],
        [b'=' as c_char, 0, 0, 0],
        [b'%' as c_char, 0, 0, 0],
        [b'%' as c_char, b'%' as c_char, 0, 0],
        [b'#' as c_char, 0, 0, 0],
        [b'#' as c_char, b'#' as c_char, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
        [0, 0, 0, 0],
    ];

    let mut p: *const c_char;
    let mut str: *const c_char;
    let mut cc: [c_char; 2] = [b' ' as c_char, 0];
    let mut nextc: *mut c_char;
    let mut c: c_char;
    let mut subtype: c_int = 0;
    let mut quoted: c_int = 0;

    nextc = makestrspace((libc::strlen(s) + 1) * 8, cmdnextc);
    p = s;
    'whileloop: loop {
        c = *p;
        p = p.add(1);
        if c == 0 {
            break;
        }
        str = core::ptr::null();
        'dostr: {
            'checkstr: {
                match c {
                    CTLESC_C => {
                        c = *p;
                        p = p.add(1);
                    }
                    CTLVAR_C => {
                        subtype = *p as c_int;
                        p = p.add(1);
                        if (subtype & VSTYPE) == VSLENGTH {
                            str = b"${#\0".as_ptr() as *const c_char;
                        } else {
                            str = b"${\0".as_ptr() as *const c_char;
                        }
                        break 'dostr;
                    }
                    CTLENDVAR_C => {
                        str = b"\"}\0".as_ptr() as *const c_char;
                        str = str.offset(((quoted & 1) == 0) as isize);
                        quoted >>= 1;
                        subtype = 0;
                        break 'dostr;
                    }
                    CTLBACKQ_C => {
                        str = b"$(...)\0".as_ptr() as *const c_char;
                        break 'dostr;
                    }
                    CTLARI_C => {
                        str = b"$((\0".as_ptr() as *const c_char;
                        break 'dostr;
                    }
                    CTLENDARI_C => {
                        str = b"))\0".as_ptr() as *const c_char;
                        break 'dostr;
                    }
                    CTLQUOTEMARK_C => {
                        quoted ^= 1;
                        c = b'"' as c_char;
                    }
                    _ => {
                        if c == b'=' as c_char {
                            if subtype == 0 {
                                /* break out of the switch */
                            } else {
                                if (subtype & VSTYPE) != VSNORMAL {
                                    quoted <<= 1;
                                }
                                str = vstype[(subtype & VSTYPE) as usize].as_ptr();
                                if (subtype & VSNUL) != 0 {
                                    c = b':' as c_char;
                                } else {
                                    break 'checkstr;
                                }
                            }
                        } else if c == b'\'' as c_char
                            || c == b'\\' as c_char
                            || c == b'"' as c_char
                            || c == b'$' as c_char
                        {
                            /* These can only happen inside quotes */
                            cc[0] = c;
                            str = cc.as_ptr();
                            c = b'\\' as c_char;
                        } else {
                            /* default: break */
                        }
                    }
                }
                /* USTPUTC(c, nextc) */
                *nextc = c;
                nextc = nextc.add(1);
            }
            // checkstr:
            if str.is_null() {
                continue 'whileloop;
            }
            // falls into dostr:
        }
        // dostr:
        loop {
            c = *str;
            str = str.add(1);
            if c == 0 {
                break;
            }
            /* USTPUTC(c, nextc) */
            *nextc = c;
            nextc = nextc.add(1);
        }
    }
    if (quoted & 1) != 0 {
        /* USTPUTC('"', nextc) */
        *nextc = b'"' as c_char;
        nextc = nextc.add(1);
    }
    *nextc = 0;
    cmdnextc = nextc;
}

// [spec:dash:def:jobs.showpipe-fn]
// [spec:dash:sem:jobs.showpipe-fn]
unsafe fn showpipe(jp: *mut job, out: *mut output) {
    let mut sp: *mut procstat;
    let spend: *mut procstat;

    spend = (*jp).ps.add((*jp).nprocs as usize);
    sp = (*jp).ps.add(1);
    while sp < spend {
        crate::outfmt!(out, b" | %s\0".as_ptr() as *const c_char, (*sp).cmd);
        sp = sp.add(1);
    }
    crate::output::outcslow('\n' as c_int, out);
    crate::output::flushall();
}

// [spec:dash:def:jobs.xtcsetpgrp-fn]
// [spec:dash:sem:jobs.xtcsetpgrp-fn]
unsafe fn xtcsetpgrp(fd: c_int, pgrp: pid_t) {
    let err: c_int;

    crate::trap::sigblockall(null_mut());
    err = libc::tcsetpgrp(fd, pgrp);
    crate::system::sigclearmask();

    if err != 0 {
        crate::sh_error!(
            b"Cannot set tty process group (%s)\0".as_ptr() as *const c_char,
            libc::strerror(errno())
        );
    }
}

// [spec:dash:def:jobs.getstatus-fn]
// [spec:dash:sem:jobs.getstatus-fn]
unsafe fn getstatus(jobp: *mut job) -> c_int {
    let mut status: c_int;
    let mut retval: c_int;
    let mut ps: *mut procstat;

    /* `job->ps + job->nprocs - 1` in C: the bitfield promotes to `int`, so
     * nprocs == 0 yields `ps - 1` rather than a wrapped `size_t`. */
    ps = (*jobp).ps.offset((*jobp).nprocs as c_int as isize - 1);
    status = (*ps).status;
    if pipefail() != 0 {
        loop {
            if status != 0 {
                break;
            }
            ps = ps.offset(-1);
            if !(ps >= (*jobp).ps) {
                break;
            }
            status = (*ps).status;
        }
    }

    retval = libc::WEXITSTATUS(status);
    if !libc::WIFEXITED(status) {
        retval = libc::WSTOPSIG(status);
        if !libc::WIFSTOPPED(status) {
            /* XXX: limits number of signals */
            retval = libc::WTERMSIG(status);
            if retval == libc::SIGINT {
                (*jobp).sigint = 1;
            }
        }
        retval += 128;
    }
    /* TRACE(("getstatus: job %d, nproc %d, status %x, retval %x\n", ...)); */
    retval
}
