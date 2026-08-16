//! Literal port of `src/jobs.c` / `src/jobs.h`.
//! Rules: `docs/spec/port/src/jobs.md`.
//!
//! Translation notes (literal, bug-for-bug):
//!   * `JOBS` is 1 in the default build (`src/shell.h`), so everything
//!     under `#if JOBS` is compiled. The `JOBS` constant is kept so the
//!     `!JOBS ||` / `! JOBS &&` expressions read as they do in C.
//!   * `struct job`'s C bitfields (`state:8, sigint:1, …`) are expanded
//!     into separate fields of the same widths. Nothing in dash depends
//!     on the packing: `memset(jp, 0, sizeof *jp)` is the only thing
//!     that spoke about the layout, and it is an assignment here.
//!   * The job table is a `Vec<Job>` and a job is named by its index, so
//!     `curjob` and `prev_job` are indices too. The C's `growjobtab`
//!     relocation pass — which existed because `realloc` moved the
//!     array out from under `curjob`, every `prev_job`, and every `ps`
//!     that pointed at its own job's inline `ps0` — has nothing left to
//!     relocate.
//!   * C `goto`s are reproduced with labelled blocks; a `goto` *into*
//!     the middle of a loop becomes an entry flag, and the two backward
//!     `goto`s in `cmdtxt` become an explicit label program counter.
//!   * `TRACE(...)` compiles to nothing without `DEBUG` and is dropped.

use bstr::{BStr, BString, ByteSlice};
use core::ptr::{addr_of_mut, null_mut};
use libc::{c_char, c_int, c_uint, pid_t};
use std::ffi::CStr;
use std::io::Write as _;

use crate::error::{Error, INTOFF, INTON};
use crate::nodes::Node;
use crate::nodes::{
    NAND, NAPPEND, NARG, NBACKGND, NCASE, NCLOBBER, NCMD, NDEFUN, NFOR, NFROM, NFROMFD, NFROMTO,
    NHERE, NIF, NNOT, NOR, NREDIR, NSEMI, NSUBSHELL, NTO, NTOFD, NUNTIL, NWHILE, NXHERE,
};
use crate::output::Dest;
use crate::parser::{VSLENGTH, VSNORMAL, VSNUL, VSTYPE};

/// Copy an already-rendered ASCII fragment into the bounded C scratch
/// buffers retained by the job display code.  The return value preserves
/// `fmtstr`'s historical clamp-to-capacity convention.
unsafe fn copy_ascii_cstr(out: *mut c_char, capacity: usize, text: &str) -> c_int {
    debug_assert!(text.is_ascii());
    let copied = text.len().min(capacity.saturating_sub(1));
    if capacity != 0 {
        core::ptr::copy_nonoverlapping(text.as_ptr(), out as *mut u8, copied);
        *out.add(copied) = 0;
    }
    text.len().min(capacity) as c_int
}

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
pub struct ProcStat {
    pub pid: pid_t,    /* process id */
    pub status: c_int, /* last process status from wait() */
    /* text of command being run. The C points this at the shared
     * `nullstr` when there is none and at a `savestr` copy otherwise,
     * and `freejob` tells the two apart by address; an owned text that
     * is empty says the same thing without the comparison. */
    pub cmd: BString,
}

// [spec:dash:def:jobs.job]
//
// The C original packs the counters and flags into one `uint32_t` of
// bitfields; the widths are preserved here as separate fields. `nprocs`
// is not among them: the C counts the processes it has filled into `ps`
// separately from the array it sized for them, and an owned `Vec` is
// both at once, so `ps.len()` is `nprocs` everywhere.
pub struct Job {
    /* status of the processes; one per pipeline element that has been
     * forked so far. The C keeps one inline `ps0` for the single-process
     * case and `ckmalloc`s otherwise, with `ps` pointing at whichever is
     * in use — a self-reference the table could not be moved without
     * repairing. */
    pub ps: Vec<ProcStat>,
    pub stopstatus: c_int, /* status of a stopped job (#if JOBS) */
    pub state: u8,
    pub sigint: u8,              /* job was killed by SIGINT (#if JOBS) */
    pub jobctl: u8,              /* job running under job control (#if JOBS) */
    pub waited: u8,              /* true if this entry has been waited for */
    pub used: u8,                /* true if this entry is in used */
    pub changed: u8,             /* true if status has changed */
    pub prev_job: Option<usize>, /* previous job */
}

impl Job {
    /* The C reaches this state with `memset(jp, 0, sizeof *jp)`. */
    const fn new() -> Job {
        Job {
            ps: Vec::new(),
            stopstatus: 0,
            state: JOBRUNNING as u8,
            sigint: 0,
            jobctl: 0,
            waited: 0,
            used: 0,
            changed: 0,
            prev_job: None,
        }
    }
}

// ---------------------------------------------------------------------
// src/jobs.c module state
// ---------------------------------------------------------------------

/* mode flags for set_curjob */
const CUR_DELETE: c_uint = 2;
pub(crate) const CUR_RUNNING: c_uint = 1;
const CUR_STOPPED: c_uint = 0;

/* mode flags for dowait */
const DOWAIT_NONBLOCK: c_int = 0;
const DOWAIT_BLOCK: c_int = 1;
pub(crate) const DOWAIT_WAITCMD: c_int = 2;
pub(crate) const DOWAIT_WAITCMD_ALL: c_int = 4;

const _PATH_TTY: &[u8] = b"/dev/tty\0";
const _PATH_DEVNULL: &[u8] = b"/dev/null\0";

/// The shell's jobs, and the terminal state job control needs.
///
/// `docs/api-design.md` 5 groups these under one field and they belong
/// together: `setjobctl` writes three of them in one breath, and
/// `makejob` reads `sh.jobs.jobctl` to decide what to put in `tab`.
///
/// **`vforked` is deliberately not here**, and is not on `Shell`
/// either: it is in the signal inbox, `siginbox.rs`. `onsig` reads it
/// and a handler has no receiver, but the deciding reason is that it
/// describes an address space rather than a shell -- see the comment at
/// its old declaration below.
pub struct JobTable {
    /// The jobs themselves.
    ///
    /// A borrow of an element is taken fresh at each access and never
    /// held across a call, because `freejob`, `set_curjob` and
    /// `showpipe` are all reached from the middle of a walk over it.
    ///
    /// `pub(crate)` where `RedirStack` and `AliasTable` keep their
    /// contents private, and the exception is deliberate rather than
    /// drift: `fg`, `bg`, `wait`, `jobs` and `kill` all index the table
    /// and read `Job`'s fields directly, so hiding it behind accessors
    /// is a rewrite of five builtins and not part of moving it. Worth
    /// doing later; recorded on the node rather than smuggled in here.
    pub(crate) tab: Vec<Job>,
    /// current job
    pub(crate) curjob: Option<usize>,
    /// true if doing job control
    pub(crate) jobctl: c_int,
    /// pgrp of shell on invocation
    initialpgrp: c_int,
    /// control terminal
    ttyfd: c_int,
    /// user was warned about stopped jobs
    pub(crate) job_warning: c_int,
}

impl JobTable {
    /// What the six statics were declared with.
    pub(crate) const fn new() -> Self {
        JobTable {
            tab: Vec::new(),
            curjob: None,
            jobctl: 0,
            initialpgrp: 0,
            ttyfd: -1,
            job_warning: 0,
        }
    }
}

/// A job that has not forked yet has no `ProcStat` at all; the C reads
/// its zeroed inline `ps0`. That is reachable: `evalpipe` calls
/// `makejob` before it opens the pipe, so a failing `pipe(2)` leaves a
/// used, zero-process job on the current-job chain for `jobs`, `kill`
/// and `wait` to find. Every reader the C writes as an unconditional
/// `ps[i]` goes through these two. `ps_pid` answers with the zero the C
/// reads out of `ps0`; `ps_cmd` answers with the empty text, where the
/// C reads `ps0.cmd`, a null pointer it then hands to `%s`.
#[inline]
pub(crate) unsafe fn ps_pid(sh: &crate::context::Shell, jp: usize, i: usize) -> pid_t {
    sh.jobs.tab[jp].ps.get(i).map_or(0, |p| p.pid)
}

#[inline]
unsafe fn ps_cmd(sh: &crate::context::Shell, jp: usize, i: usize) -> &BStr {
    sh.jobs.tab[jp]
        .ps
        .get(i)
        .map_or(BStr::new(b""), |p| p.cmd.as_bstr())
}

/// `%s` of a command text. The bytes are the shell's own — the parser
/// puts control bytes 0x81-0x88 in them — so they go out as bytes and
/// not through a `char *`.
#[inline]
pub(crate) unsafe fn outcmd(sh: &mut crate::context::Shell, jp: usize, i: usize, dest: Dest) {
    /* The lookup is spelled out here rather than going through `ps_cmd`,
     * which is otherwise these same three lines. The two borrows have to
     * be *field*-disjoint: the write takes `sh.io` mutably and the text is
     * read out of `sh.jobs`, and the compiler can see those are different
     * fields only when both are direct field paths. `ps_cmd` borrows the
     * whole shell, so writing through it becomes a conflict the moment
     * `io` becomes a field. It stays because `getjob`'s command-text
     * search still uses it, and that one only reads. */
    let cmd = sh.jobs.tab[jp]
        .ps
        .get(i)
        .map_or(BStr::new(b""), |p| p.cmd.as_bstr());
    let _ = sh.io.get(dest).write_all(cmd);
}

/* Set if we are in the vforked child.
 *
 * Lives in the signal inbox (`siginbox.rs`), not here and not on
 * `Shell`. `trap.rs`'s `onsig` reads it and a handler has no receiver --
 * but the reason it is not a field is stronger than that. The *parent*
 * sets it before `vfork` and clears it after; the child reads it out of
 * the address space it shares to learn that it is the child. That is a
 * property of an address space, and a `sh.vforked` would be one shell's
 * field written by one process and read by another that only accidentally
 * shares it. See `docs/api-design.md` 5.3 and 6. */

pub(crate) use crate::system::errno;

/* src/options.h: `#define iflag optlist[3]` and friends. */
#[inline]
unsafe fn iflag(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::iflag) as c_int
}
#[inline]
unsafe fn pipefail(sh: &crate::context::Shell) -> c_int {
    sh.options.flag(crate::options::pipefail) as c_int
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

/// Where the next link of the current-job chain lives. The C walks the
/// chain through a `struct job **` so that it can rewrite the link it
/// arrived by, and that pointer is either `&sh.jobs.curjob` or `&jp->prev_job`.
#[derive(Clone, Copy)]
enum Link {
    Head,
    Prev(usize),
}

#[inline]
unsafe fn link_get(sh: &mut crate::context::Shell, l: Link) -> Option<usize> {
    match l {
        Link::Head => sh.jobs.curjob,
        Link::Prev(i) => sh.jobs.tab[i].prev_job,
    }
}

#[inline]
unsafe fn link_set(sh: &mut crate::context::Shell, l: Link, v: Option<usize>) {
    match l {
        Link::Head => sh.jobs.curjob = v,
        Link::Prev(i) => sh.jobs.tab[i].prev_job = v,
    }
}

// [spec:dash:def:jobs.set-curjob-fn]
// [spec:dash:sem:jobs.set-curjob-fn]
pub(crate) unsafe fn set_curjob(sh: &mut crate::context::Shell, jp: usize, mode: c_uint) {
    let mut jp1: Option<usize>;
    let mut jpp: Link;
    let curp: Link;

    /* first remove from list */
    jpp = Link::Head;
    curp = jpp;
    loop {
        jp1 = link_get(sh, jpp);
        if jp1 == Some(jp) {
            break;
        }
        /* The C walks off the end of the chain and dereferences NULL if
         * `jp` is not on it; every caller has just linked it or is
         * deleting one that is linked. */
        jpp = Link::Prev(jp1.expect("job is not on the current-job chain"));
    }
    link_set(sh, jpp, sh.jobs.tab[jp].prev_job);

    /* Then re-insert in correct position */
    jpp = curp;
    match mode {
        CUR_RUNNING => {
            /* newly created job or backgrounded job,
            put after all stopped jobs. */
            loop {
                jp1 = link_get(sh, jpp);
                match jp1 {
                    Some(i) if JOBS != 0 && sh.jobs.tab[i].state as c_int == JOBSTOPPED => {
                        jpp = Link::Prev(i);
                    }
                    _ => break,
                }
            }
            /* FALLTHROUGH into CUR_STOPPED */
            sh.jobs.tab[jp].prev_job = link_get(sh, jpp);
            link_set(sh, jpp, Some(jp));
        }
        CUR_STOPPED => {
            /* newly stopped job - becomes the current job */
            sh.jobs.tab[jp].prev_job = link_get(sh, jpp);
            link_set(sh, jpp, Some(jp));
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
pub(crate) unsafe fn xxtcsetpgrp(sh: &mut crate::context::Shell, pgrp: pid_t) -> Result<(), Error> {
    let fd: c_int = sh.jobs.ttyfd;

    if fd < 0 {
        return Ok(());
    }

    xtcsetpgrp(sh, fd, pgrp)
}

// [spec:dash:def:jobs.setjobctl-fn]
// [spec:dash:sem:jobs.setjobctl-fn]
/// Turn job control on or off.
///
/// Returns its diagnostic rather than raising it. Two of its three
/// callers are teardown -- `exitshell`, and `optschanged` when
/// `poplocalvars` restores a `local -` option set -- and 4.3's rule is
/// that teardown does not become fallible; the `Result` is here so the
/// callers that *are* ordinary code (`set -m`, `exec`, `procargs`) keep
/// dash's behaviour of abandoning the command, and the teardown callers
/// drop it where the C already swallowed it.
pub unsafe fn setjobctl(sh: &mut crate::context::Shell, on: c_int) -> Result<(), Error> {
    let mut on: c_int = on;
    let mut pgrp: c_int = -1;
    let mut fd: c_int;

    if on == sh.jobs.jobctl || crate::shellmain::rootshell() == 0 {
        return Ok(());
    }
    if on != 0 {
        let ofd: c_int;
        /* `setjobctl` is reached from `exitshell`'s job-control teardown as
         * well as from `optschanged`, so it stays infallible and bridges:
         * a failure here longjmps exactly as the C's `sh_open` did. Making
         * teardown fallible is the shape docs/errors-are-values.md 4.3
         * argues against. */
        /* `mayfail = 1`, so the only thing this can hand back is an
         * interrupt taken at its EINTR poll. */
        ofd = crate::redir::sh_open(sh, _PATH_TTY.as_ptr() as *const c_char, libc::O_RDWR, 1)?;
        fd = ofd;
        'after_dowhile: {
            'out_lbl: {
                'close_lbl: {
                    if fd < 0 {
                        /* `/dev/tty` would not open, so fall back to
                         * whichever of the shell's own streams is a
                         * terminal. The C writes this as `fd += 3` from
                         * -1 and counts down -- descriptors 2, 1, 0, in
                         * that order -- which is the shell's stderr,
                         * stdout and stdin, not the numbers for their
                         * own sake. */
                        let s = sh.streams;
                        let candidates = [s.stderr, s.stdout, s.stdin];
                        let mut i: usize = 0;
                        fd = -1;
                        while i < candidates.len() {
                            if libc::isatty(candidates[i]) != 0 {
                                fd = candidates[i];
                                break;
                            }
                            i += 1;
                        }
                        if fd < 0 {
                            break 'out_lbl; // goto out
                        }
                    }
                    fd = crate::redir::savefd(sh, fd, ofd)?;
                    loop {
                        /* while we are in the background */
                        loop {
                            pgrp = libc::tcgetpgrp(fd);
                            /* The 8.3 audit's one real finding: this is
                             * the only EINTR-capable syscall in this file
                             * whose -1 is handled without ever reading
                             * `errno`, and a -1 here turns job control off
                             * for the rest of the session with a warning.
                             * Before step F an interrupt arriving during
                             * `setjobctl` left by longjmp and never
                             * reached this test; now the call returns
                             * EINTR and would be read as "can't access
                             * tty". Retry, which is what every other
                             * EINTR site in the shell does when the
                             * interrupt is not yet due. There is no poll
                             * here because `setjobctl` is infallible and
                             * stays so -- it is reached from `exitshell`'s
                             * teardown -- so the interrupt waits for the
                             * next real poll site, which is where it would
                             * have waited anyway. */
                            if !(pgrp < 0 && errno() == libc::EINTR) {
                                break;
                            }
                        }
                        if pgrp < 0 {
                            break 'close_lbl; // goto close
                        }
                        if pgrp == libc::getpgrp() {
                            break 'after_dowhile; // `break` of the do/while
                        }
                        if iflag(sh) == 0 {
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
            if iflag(sh) == 0 {
                break 'after_dowhile; // `break` of the do/while
            }
            sh.sh_warnx(b"can't access tty; job control turned off");
            sh.options.set_flag(crate::options::mflag, 0);
            on = 0;
            let _ = on;
            return Ok(());
        }
        sh.jobs.initialpgrp = pgrp;
        pgrp = crate::shellmain::rootpid;
    } else {
        /* turning job control off */
        fd = sh.jobs.ttyfd;
        pgrp = sh.jobs.initialpgrp;
    }

    crate::trap::setsignal(sh, libc::SIGTSTP);
    crate::trap::setsignal(sh, libc::SIGTTOU);
    crate::trap::setsignal(sh, libc::SIGTTIN);
    if fd >= 0 {
        libc::setpgid(0, pgrp);
        xtcsetpgrp(sh, fd, pgrp)?;

        if on == 0 {
            libc::close(fd);
            fd = -1;
        }
    }

    sh.jobs.ttyfd = fd;
    sh.jobs.jobctl = on;
    Ok(())
}

// [spec:dash:def:jobs.jobno-fn]
// [spec:dash:sem:jobs.jobno-fn]
//
// The C recovers the index by subtracting `jobtab` from the pointer.
pub(crate) fn jobno(jp: usize) -> c_int {
    jp as c_int + 1
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
            /* `stpncpy(s, …, 32)` copies at most 32 bytes and NUL-pads
             * the rest of them, which is why the callers' buffers are
             * sized for 32 whatever the signal is called. `strsignal` is
             * locale text, not ASCII, so the bytes are copied rather than
             * routed through `copy_ascii_cstr`. */
            let name = CStr::from_ptr(libc::strsignal(st)).to_bytes();
            let n = name.len().min(32);
            core::ptr::copy_nonoverlapping(name.as_ptr(), s as *mut u8, n);
            core::ptr::write_bytes((s as *mut u8).add(n), 0, 32 - n);
            s = s.add(n);
            if libc::WCOREDUMP(status) {
                s = s.offset(copy_ascii_cstr(s, 15, " (core dumped)") as isize);
            }
        } else if sigonly == 0 {
            if st != 0 {
                let status = format!("Done({st})");
                s = s.offset(copy_ascii_cstr(s, 16, &status) as isize);
            } else {
                s = s.offset(copy_ascii_cstr(s, 5, "Done") as isize);
            }
        }
    }
    // out:
    (s as usize - os as usize) as c_int
}

// [spec:dash:def:jobs.showjob-fn]
// [spec:dash:sem:jobs.showjob-fn]
pub(crate) unsafe fn showjob(sh: &mut crate::context::Shell, dest: Dest, jp: usize, mode: c_int) {
    let mut ps: usize;
    let psend: usize;
    let mut col: c_int;
    let indent: c_int;
    let mut s: [c_char; 80] = [0; 80];

    ps = 0;

    if (mode & SHOW_PGID) != 0 {
        /* just output process (group) id of pipeline */
        /* The pid is read out before the write starts rather than inside
         * its argument list: `ps_pid` borrows the shell and the write
         * borrows `sh.io`, and evaluating one inside the other is the
         * conflict `Dest` exists to keep out of these functions. */
        let pid = ps_pid(sh, jp, ps);
        let _ = writeln!(sh.io.get(dest), "{pid}");
        return;
    }

    let heading = format!("[{}]   ", jobno(jp));
    col = copy_ascii_cstr(s.as_mut_ptr(), 16, &heading);
    indent = col;

    if Some(jp) == sh.jobs.curjob {
        s[(col - 2) as usize] = b'+' as c_char;
    } else if sh.jobs.curjob.map_or(false, |c| sh.jobs.tab[c].prev_job == Some(jp)) {
        s[(col - 2) as usize] = b'-' as c_char;
    }

    if (mode & SHOW_PID) != 0 {
        let pid = format!("{} ", ps_pid(sh, jp, ps));
        col += copy_ascii_cstr(s.as_mut_ptr().offset(col as isize), 16, &pid);
    }

    psend = sh.jobs.tab[jp].ps.len();

    if sh.jobs.tab[jp].state as c_int == JOBRUNNING {
        /* scopy("Running", s + col) */
        col += copy_ascii_cstr(s.as_mut_ptr().offset(col as isize), 8, "Running");
    } else {
        /* `psend[-1]`: a job leaves JOBRUNNING only through `waitone`,
         * which needs a process to have exited to do it. */
        let mut status: c_int = sh.jobs.tab[jp].ps[psend - 1].status;
        if sh.jobs.tab[jp].state as c_int == JOBSTOPPED {
            status = sh.jobs.tab[jp].stopstatus;
        }
        col += sprint_status(s.as_mut_ptr().offset(col as isize), status, 0);
    }

    /* `goto start` enters the do/while below at the `start:` label */
    let mut at_start = true;
    loop {
        if !at_start {
            /* for each process */
            let continuation = format!(
                " |\n{space:>width$}{} ",
                ps_pid(sh, jp, ps),
                space = ' ',
                width = indent.max(0) as usize,
            );
            col = copy_ascii_cstr(s.as_mut_ptr(), 48, &continuation) - 3;
        }
        at_start = false;

        // start:
        let mut record = CStr::from_ptr(s.as_ptr()).to_bytes().to_vec();
        let width = (33 - col).max(0) as usize;
        record.resize(record.len() + width.max(1), b' ');
        let _ = sh.io.get(dest).write_all(&record);
        outcmd(sh, jp, ps, dest);
        if (mode & SHOW_PID) == 0 {
            showpipe(sh, jp, dest);
            break;
        }
        ps += 1;
        if ps == psend {
            let _ = sh.io.get(dest).write_all(b"\n");
            break;
        }
    }

    sh.jobs.tab[jp].changed = 0;

    if sh.jobs.tab[jp].state as c_int == JOBDONE {
        /* TRACE(("showjob: freeing job %d\n", jobno(jp))); */
        freejob(sh, jp);
    }
}

/*
 * Print a list of jobs.  If "change" is nonzero, only print jobs whose
 * statuses have changed since the last call to showjobs.
 */

// [spec:dash:def:jobs.showjobs-fn]
// [spec:dash:sem:jobs.showjobs-fn]
pub unsafe fn showjobs(sh: &mut crate::context::Shell, dest: Dest, mode: c_int) -> Result<(), Error> {
    let mut jp: Option<usize>;

    /* TRACE(("showjobs(%x) called\n", mode)); */

    /* If not even one job changed, there is nothing to do */
    /* `DOWAIT_NONBLOCK`, so `wait3` cannot block and the poll inside it
     * has nothing to notice; the `?` is the type saying so rather than a
     * path anyone expects to take. */
    dowait(sh, DOWAIT_NONBLOCK, None)?;

    jp = sh.jobs.curjob;
    /* `showjob` may `freejob` the entry this walk is standing on.
     * `freejob` unlinks the job from the chain but leaves its own
     * `prev_job` alone, which is what keeps the next step valid. */
    while let Some(i) = jp {
        if (mode & SHOW_CHANGED) == 0 || sh.jobs.tab[i].changed != 0 {
            showjob(sh, dest, i, mode);
        }
        jp = sh.jobs.tab[i].prev_job;
    }
    Ok(())
}

/*
 * Mark a job structure as unused.
 */

// [spec:dash:def:jobs.freejob-fn]
// [spec:dash:sem:jobs.freejob-fn]
unsafe fn freejob(sh: &mut crate::context::Shell, jp: usize) {
    INTOFF();
    /* The C `ckfree`s each `ps[i].cmd` that is not the shared null
     * string and leaves `nprocs` alone, so freeing the same job twice
     * frees them twice; dropping the array releases each text once and
     * makes the second call the no-op the C only gets away with by
     * never making it. */
    sh.jobs.tab[jp].ps.clear();
    sh.jobs.tab[jp].used = 0;
    set_curjob(sh, jp, CUR_DELETE);
    INTON();
}

/*
 * Convert a job name to a job structure.
 */

// [spec:dash:def:jobs.getjob-fn]
// [spec:dash:sem:jobs.getjob-fn]
pub(crate) unsafe fn getjob(sh: &mut crate::context::Shell, name: *const c_char, getctl: c_int) -> Result<usize, Error> {
    enum JobError {
        NoSuch,
        NoPrevious,
        Ambiguous,
        NoCurrent,
        NoControl,
    }

    let mut jp: Option<usize>;
    let mut found: Option<usize>;
    let mut job_error = JobError::NoSuch;
    let num: c_uint;
    let c: c_int;
    let mut p: *const c_char;
    /* C: `char *(*match)(const char *, const char *)`, assigned either
     * `prefix` or `strstr`; the two differ only in whether the pattern
     * has to start at the beginning of the command text. */
    let mut substring: bool;

    'err_lbl: {
        'gotit_lbl: {
            'check_lbl: {
                'currentjob_lbl: {
                    jp = sh.jobs.curjob;
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
                            if let Some(i) = jp {
                                jp = sh.jobs.tab[i].prev_job;
                            }
                            job_error = JobError::NoPrevious;
                            break 'check_lbl; // the check: label body
                        }
                    }

                    if crate::mystring::is_number(p) != 0 {
                        num = libc::atoi(p) as c_uint;
                        if num > 0 && num as usize <= sh.jobs.tab.len() {
                            let i = (num - 1) as usize;
                            jp = Some(i);
                            if sh.jobs.tab[i].used != 0 {
                                break 'gotit_lbl; // goto gotit
                            }
                            break 'err_lbl; // goto err
                        }
                    }

                    substring = false;
                    if *p == b'?' as c_char {
                        substring = true;
                        p = p.add(1);
                    }

                    let pat: &[u8] = CStr::from_ptr(p).to_bytes();
                    found = None;
                    while let Some(i) = jp {
                        let cmd = ps_cmd(sh, i, 0);
                        let hit = if substring {
                            cmd.contains_str(pat)
                        } else {
                            cmd.starts_with(pat)
                        };
                        if hit {
                            if found.is_some() {
                                break 'err_lbl; // goto err
                            }
                            found = Some(i);
                            job_error = JobError::Ambiguous;
                        }
                        jp = sh.jobs.tab[i].prev_job;
                    }

                    if found.is_none() {
                        break 'err_lbl; // goto err
                    }
                    jp = found;

                    break 'gotit_lbl; /* fall through to gotit: */
                }
                // currentjob:
                job_error = JobError::NoCurrent;
                // goto check
            }
            // check:
            if jp.is_none() {
                break 'err_lbl; // goto err
            }
            // goto gotit
        }
        // gotit:
        job_error = JobError::NoControl;
        let i = jp.unwrap();
        if getctl != 0 && sh.jobs.tab[i].jobctl == 0 {
            break 'err_lbl; // goto err
        }
        return Ok(i);
    }
    // err:
    let mut message = Vec::new();
    match job_error {
        JobError::NoSuch => {
            message.extend_from_slice(b"No such job: ");
            message.extend_from_slice(CStr::from_ptr(name).to_bytes());
        }
        JobError::NoPrevious => message.extend_from_slice(b"No previous job"),
        JobError::Ambiguous => {
            message.extend_from_slice(CStr::from_ptr(name).to_bytes());
            message.extend_from_slice(b": ambiguous");
        }
        JobError::NoCurrent => message.extend_from_slice(b"No current job"),
        JobError::NoControl => {
            message.extend_from_slice(b"job ");
            if name.is_null() {
                message.extend_from_slice(b"(null)");
            } else {
                message.extend_from_slice(CStr::from_ptr(name).to_bytes());
            }
            message.extend_from_slice(b" not created under job control");
        }
    }
    Err(sh.sh_error_value(&message))
}

/*
 * Return a new job structure.
 * Called with interrupts off.
 */

// [spec:dash:def:jobs.makejob-fn]
// [spec:dash:sem:jobs.makejob-fn]
pub unsafe fn makejob(sh: &mut crate::context::Shell, nprocs: c_int) -> usize {
    let jp: usize;
    let mut i: usize;

    i = 0;
    jp = loop {
        if i >= sh.jobs.tab.len() {
            break growjobtab(sh);
        }
        if sh.jobs.tab[i].used == 0 {
            break i;
        }
        if sh.jobs.tab[i].state as c_int != JOBDONE || sh.jobs.tab[i].waited == 0 {
            i += 1;
            continue;
        }
        if sh.jobs.jobctl != 0 {
            i += 1;
            continue;
        }
        freejob(sh, i);
        break i;
    };
    /* C: memset(jp, 0, sizeof *jp) */
    sh.jobs.tab[jp] = Job::new();
    /* The C picks the inline `ps0` for a single process and `ckmalloc`s
     * an array otherwise; all that decided was where the room came from,
     * so it is the capacity here and the processes are pushed as
     * `forkparent` forks them. */
    if nprocs > 0 {
        sh.jobs.tab[jp].ps.reserve_exact(nprocs as usize);
    }
    if sh.jobs.jobctl != 0 {
        sh.jobs.tab[jp].jobctl = 1;
    }
    sh.jobs.tab[jp].prev_job = sh.jobs.curjob;
    sh.jobs.curjob = Some(jp);
    sh.jobs.tab[jp].used = 1;
    /* TRACE(("makejob(%d) returns %%%d\n", nprocs, jobno(jp))); */
    jp
}

// [spec:dash:def:jobs.growjobtab-fn]
// [spec:dash:sem:jobs.growjobtab-fn]
//
// The C's second half — relocating `curjob`, every `prev_job` and every
// `ps` that pointed at its own job's `ps0`, because `ckrealloc` may have
// moved the array — has no counterpart: a job is named by its index and
// owns its process array, so nothing points into the table.
unsafe fn growjobtab(sh: &mut crate::context::Shell) -> usize {
    let len: usize = sh.jobs.tab.len();

    for _ in 0..4 {
        sh.jobs.tab.push(Job::new());
    }
    len
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

/// What `forkchild` does with a diagnostic it cannot return.
///
/// `forkchild` runs in the child, and under `vforkexec` in a child that
/// *shares the parent's stack*. `docs/errors-are-values.md` §2.5 calls
/// that a hard boundary rather than a wrinkle: an `Err` returned from
/// here would travel through frames the parent owns and unwind them under
/// it. So this is a terminus, and it is the same terminus the C had --
/// `exraise`'s two arms, written where the boundary is.
///
/// Vforked: `_exit` immediately, before anything can run a destructor on
/// the parent's objects. Otherwise: the child ends the way `main`'s
/// handler ends every forked child, which `forkchild`'s own `shlvl += 1`
/// is what guarantees (see `shellmain::exit_from_child`). The diagnostic
/// has already been written either way.
#[cold]
unsafe fn forkchild_fatal(sh: &mut crate::context::Shell, e: Error) -> ! {
    if crate::siginbox::signals().vforked() != 0 {
        /* The `_exit` below is this frame's own ending, so this frame
         * writes the status the error took. */
        sh.status = e.status();
        drop(e);
        crate::shell::flush_coverage();
        libc::_exit(sh.status);
    }
    crate::shellmain::exit_from_child(sh, Err(e))
}

// [spec:dash:def:jobs.forkchild-fn]
// [spec:dash:sem:jobs.forkchild-fn]
//
// Under `vfork` this runs in the parent's address space, so everything
// before the `lvforked` return must stay allocation- and destructor-free
// (§4.12 of docs/std-replacements.md); it reads the job table and writes
// nothing but process-global state, and `vforkexec` passes FORK_FG, so
// the `/dev/null` branch — the one that would open a descriptor — is not
// on that path either.
unsafe fn forkchild(
    sh: &mut crate::context::Shell,
    jp: Option<usize>,
    n: Option<&Node>,
    mode: c_int,
) {
    let lvforked: c_int;
    let oldlvl: c_int;

    /* TRACE(("Child shell %d\n", getpid())); */

    crate::shell::reset_coverage();

    oldlvl = crate::shellmain::shlvl;
    lvforked = crate::siginbox::signals().vforked();

    if lvforked == 0 {
        crate::shellmain::mypid = 0;
        crate::shellmain::shlvl += 1;

        crate::init::forkreset(sh, if mode == FORK_NOJOB { n } else { None });

        /* do job control only in root shell */
        sh.jobs.jobctl = 0;
    }

    /* The C tests `jp->jobctl` without checking `jp`; `jp` is NULL only
     * under FORK_NOJOB, which the first conjunct has already excluded. */
    let ownpgrp = mode != FORK_NOJOB && oldlvl == 0 && jp.map_or(false, |i| sh.jobs.tab[i].jobctl != 0);
    if ownpgrp {
        let pgrp: pid_t;
        let ji: usize = jp.unwrap();

        if sh.jobs.tab[ji].ps.is_empty() {
            pgrp = libc::getpid();
            /* The C writes `mypid = pgrp = getpid()` unguarded
             * (`src/jobs.c:891`), and under `vfork` that is a store into
             * the *parent's* address space -- the one place on the
             * vforked child's prologue where the parent reads the
             * location again. Every other write in this function is
             * already gated on `lvforked == 0`: the block above, and
             * `setsignal`/`ignoresig`'s `sigmode` stores, which carry
             * their own guard (`trap.rs:440`, `:469`). This one was
             * missed, so it joins them.
             *
             * What it costs the C: after a foreground external command
             * under `set -m`, the parent's `mypid` holds its *child's*
             * pid. `vforkexec`'s `if mypid == 0` then never re-reads it,
             * so the next `vfork` publishes a stale pid as `vforked`, and
             * `onsig`'s `getpid() != vforked` answers *true for the
             * parent* -- which drops any signal arriving between `vfork`
             * returning and `set_vforked(0)`. Narrow, but it is a
             * dropped signal, and `[dec:nsh:we-own-the-defects]` says a
             * defect in the Rust is fixed in the Rust.
             *
             * The child does not need the value: it reaches `execve`
             * without reading `mypid` again. */
            if lvforked == 0 {
                crate::shellmain::mypid = pgrp;
            }
        } else {
            pgrp = sh.jobs.tab[ji].ps[0].pid;
        }
        /* This can fail because we are doing it in the parent also */
        libc::setpgid(0, pgrp);
        if mode == FORK_FG {
            xxtcsetpgrp(sh, pgrp).unwrap_or_else(|e| forkchild_fatal(sh, e));
        }
        crate::trap::setsignal_in_child(sh, libc::SIGTSTP);
        crate::trap::setsignal_in_child(sh, libc::SIGTTOU);
    } else if mode == FORK_BG {
        crate::trap::ignoresig_in_child(sh, libc::SIGINT);
        crate::trap::ignoresig_in_child(sh, libc::SIGQUIT);
        if jp.map_or(false, |i| sh.jobs.tab[i].ps.is_empty()) {
            /* The C closes descriptor 0 and reopens /dev/null, relying on
             * `open` returning the lowest free descriptor to land back on
             * 0. That only works when the shell's stdin *is* 0, so put it
             * where it belongs when the frontend said otherwise. */
            let sin: c_int = sh.streams.stdin;
            libc::close(sin);
            let f: c_int =
                crate::redir::sh_open(sh, _PATH_DEVNULL.as_ptr() as *const c_char, libc::O_RDONLY, 0)
                    .unwrap_or_else(|e| forkchild_fatal(sh, e));
            if f != sin {
                libc::dup2(f, sin);
                libc::close(f);
            }
            /* Should call reset_input here, but it's harmless
             * for now.
             */
        }
    }
    if oldlvl == 0 && iflag(sh) != 0 {
        crate::trap::setsignal_in_child(sh, libc::SIGINT);
        crate::trap::setsignal_in_child(sh, libc::SIGQUIT);
        crate::trap::setsignal_in_child(sh, libc::SIGTERM);
    }

    if lvforked != 0 {
        return;
    }

    let Some(ji) = jp else {
        return;
    };

    freejob(sh, ji);

    if crate::parser::issimplecmd(n, (*crate::builtins::JOBSCMD).name.as_ptr()) != 0 {
        return;
    }

    /* as in `showjobs`, the walk steps through jobs `freejob` has just
     * unlinked, using the `prev_job` it leaves behind */
    let mut jq = sh.jobs.curjob;
    while let Some(i) = jq {
        freejob(sh, i);
        jq = sh.jobs.tab[i].prev_job;
    }
}

// [spec:dash:def:jobs.forkparent-fn]
// [spec:dash:sem:jobs.forkparent-fn]
unsafe fn forkparent(
    sh: &mut crate::context::Shell,
    jp: Option<usize>,
    n: Option<&Node>,
    mode: c_int,
    pid: pid_t,
) -> Result<(), Error> {
    if pid < 0 {
        /* TRACE(("Fork failed, errno=%d", errno)); */
        if let Some(i) = jp {
            freejob(sh, i);
        }
        return Err(sh.sh_error_value(b"Cannot fork"));
    }

    /* TRACE(("In parent shell:  child = %d\n", pid)); */
    let Some(ji) = jp else {
        return Ok(());
    };
    if mode != FORK_NOJOB && sh.jobs.tab[ji].jobctl != 0 {
        let pgrp: c_int;

        if sh.jobs.tab[ji].ps.is_empty() {
            pgrp = pid;
        } else {
            pgrp = sh.jobs.tab[ji].ps[0].pid;
        }
        /* This can fail because we are doing it in the child also */
        libc::setpgid(pid, pgrp);
    }
    if mode == FORK_BG {
        sh.backgndpid = pid; /* set $! */
        set_curjob(sh, ji, CUR_RUNNING);
        if sh.options.flag(crate::options::iflag) != 0 {
            let _ = writeln!(sh.io.stderr(), "[{}] {pid}", jobno(ji));
        }
    }
    /* the C's second `if (jp)` is dead after the early return above */
    sh.jobs.tab[ji].ps.push(ProcStat {
        pid,
        status: -1,
        cmd: BString::new(Vec::new()),
    });
    if sh.jobs.jobctl != 0 && n.is_some() {
        let cmd = commandtext(n.unwrap());
        let last = sh.jobs.tab[ji].ps.len() - 1;
        sh.jobs.tab[ji].ps[last].cmd = cmd;
    }
    Ok(())
}

// [spec:dash:def:jobs.forkshell-fn]
// [spec:dash:sem:jobs.forkshell-fn]
pub unsafe fn forkshell(
    sh: &mut crate::context::Shell,
    jp: Option<usize>,
    n: Option<&Node>,
    mode: c_int,
) -> Result<c_int, Error> {
    let pid: c_int;

    /* TRACE(("forkshell(%%%d, %p, %d) called\n", jobno(jp), n, mode)); */

    crate::input::flush_input(sh);

    pid = libc::fork();
    if pid == 0 {
        forkchild(sh, jp, n, mode);
    } else {
        forkparent(sh, jp, n, mode, pid)?;
    }

    Ok(pid)
}

// [spec:dash:def:jobs.vforkexec-fn]
// [spec:dash:sem:jobs.vforkexec-fn]
#[allow(deprecated)] /* libc marks vfork deprecated; dash relies on it */
pub unsafe fn vforkexec(
    sh: &mut crate::context::Shell,
    n: &Node,
    argv: *mut *mut c_char,
    path: *const c_char,
    idx: c_int,
) -> Result<usize, Error> {
    let jp: usize;
    let pid: c_int;

    jp = makejob(sh, 1);

    if crate::shellmain::mypid == 0 {
        crate::shellmain::mypid = libc::getpid();
    }
    crate::siginbox::signals().set_vforked(crate::shellmain::mypid);

    pid = libc::vfork();

    if pid == 0 {
        /* Shared address space until `execve`: nothing between here and
         * it may allocate, free or drop. `forkchild` returns at its
         * `lvforked` test without touching the job table's storage.
         *
         * The rule the prologue is audited against is
         * [dec:nsh:fork-child-is-a-terminus]: **a vforked child writes no
         * location the parent reads again.** The audit, in the order the
         * child runs them:
         *
         *   * the `mypid = 0` / `shlvl += 1` / `forkreset` / `jobctl = 0`
         *     block -- skipped, `lvforked != 0`;
         *   * `mypid = getpid()` in the job-control arm -- guarded there
         *     now; it was the one violation and it is dash's;
         *   * `setpgid`, `tcsetpgrp` -- syscalls on the child, not
         *     memory;
         *   * `setsignal`/`ignoresig` -- their `sigmode` stores carry
         *     their own `lvforked` guard, and `sigaction` is per-process,
         *     so the child's dispositions are the child's;
         *   * the `freejob` walk -- skipped, `lvforked != 0`.
         *
         * Two writes are *permitted* and named rather than removed:
         * `forkchild_fatal`'s and `shellexec`'s `sh.status = ...`, both
         * immediately before their own `_exit`. The parent overwrites
         * `sh.status` from the child's wait status in `waitforjob` before
         * anything reads it, and `shellexec`'s failure path has to build
         * and write dash's diagnostic, which allocates in the shared heap
         * whatever is done about the field. Removing the store would buy
         * nothing the allocation does not give back. */
        forkchild(sh, Some(jp), Some(n), FORK_FG);
        /* `shellexec` either replaces the image or, in a vforked child,
         * `_exit`s at the failure site. It cannot come back here: this
         * frame belongs to the parent, and returning through it would
         * unwind the parent's stack from inside the child.
         * docs/errors-are-values.md 2.5 is the boundary, and the `_exit`
         * that enforces it is at `shellexec`'s own failure site now
         * rather than inside `exraise`. */
        drop(crate::exec::shellexec(sh, argv, path, idx));
        std::process::abort();
    }

    crate::siginbox::signals().set_vforked(0);
    forkparent(sh, Some(jp), Some(n), FORK_FG, pid)?;

    Ok(jp)
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
pub unsafe fn waitforjob(sh: &mut crate::context::Shell, jp: Option<usize>) -> Result<c_int, Error> {
    let st: c_int;

    /* TRACE(("waitforjob(%%%d) called\n", jp ? jobno(jp) : 0)); */
    dowait(sh, 
        if jp.is_some() {
            DOWAIT_BLOCK
        } else {
            DOWAIT_NONBLOCK
        },
        jp,
    )?;
    let Some(jp) = jp else {
        return Ok(sh.status);
    };

    st = getstatus(sh, jp);
    if sh.jobs.tab[jp].jobctl != 0 {
        xxtcsetpgrp(sh, crate::shellmain::rootpid)?;
        /*
         * This is truly gross.
         * If we're doing job control, then we did a TIOCSPGRP which
         * caused us (the shell) to no longer be in the controlling
         * session -- so we wouldn't have seen any ^C/SIGINT.  So, we
         * intuit from the subprocess exit status whether a SIGINT
         * occurred, and if so interrupt ourselves.  Yuck.  - mycroft
         */
        if sh.jobs.tab[jp].sigint != 0 {
            libc::raise(libc::SIGINT);
        }
    }
    if JOBS == 0 || sh.jobs.tab[jp].state as c_int == JOBDONE {
        freejob(sh, jp);
    }
    Ok(st)
}

/*
 * Wait for a process to terminate.
 */

// [spec:dash:def:jobs.waitone-fn]
// [spec:dash:sem:jobs.waitone-fn]
unsafe fn waitone(sh: &mut crate::context::Shell, block: c_int, jobp: Option<usize>) -> Result<c_int, Error> {
    let pid: c_int;
    let mut status: c_int = 0;
    let mut jp: Option<usize>;
    let mut thisjob: Option<usize> = None;
    let mut state: c_int = 0;

    INTOFF();
    /* TRACE(("dowait(%d) called\n", block)); */
    pid = waitproc(sh, block, &mut status)?;
    /* TRACE(("wait returns pid %d, status=%d\n", pid, status)); */
    'out_lbl: {
        if pid <= 0 {
            break 'out_lbl;
        }

        'gotjob: {
            jp = sh.jobs.curjob;
            while let Some(ji) = jp {
                if sh.jobs.tab[ji].state as c_int == JOBDONE {
                    jp = sh.jobs.tab[ji].prev_job;
                    continue;
                }
                state = JOBDONE;
                /* the C's `do { … } while (sp < spend)` reads `ps[0]`
                 * before it compares, so a job that has not forked yet
                 * costs it one read of its zeroed `ps0`; that read can
                 * match no pid and `state` is only consulted once one
                 * has, so making the loop test first decides nothing */
                let spend: usize = sh.jobs.tab[ji].ps.len();
                let mut sp: usize = 0;
                while sp < spend {
                    if sh.jobs.tab[ji].ps[sp].pid == pid {
                        /* TRACE(("Job %d: changing status of proc %d ...")); */
                        sh.jobs.tab[ji].ps[sp].status = status;
                        thisjob = Some(ji);
                    }
                    'contin: {
                        if sh.jobs.tab[ji].ps[sp].status == -1 {
                            state = JOBRUNNING;
                        }
                        if state == JOBRUNNING {
                            break 'contin;
                        }
                        if libc::WIFSTOPPED(sh.jobs.tab[ji].ps[sp].status) {
                            sh.jobs.tab[ji].stopstatus = sh.jobs.tab[ji].ps[sp].status;
                            state = JOBSTOPPED;
                        }
                    }
                    sp += 1;
                }
                if thisjob.is_some() {
                    break 'gotjob;
                }
                jp = sh.jobs.tab[ji].prev_job;
            }
            break 'out_lbl;
        }
        // gotjob:
        if state != JOBRUNNING {
            let tj = thisjob.unwrap();
            sh.jobs.tab[tj].changed = 1;

            if sh.jobs.tab[tj].state as c_int != state {
                /* TRACE(("Job %d: changing state from %d to %d\n", ...)); */
                sh.jobs.tab[tj].state = state as u8;
                if state == JOBSTOPPED {
                    set_curjob(sh, tj, CUR_STOPPED);
                }
            }
        }
    }
    // out:
    INTON();

    if thisjob.is_some() && thisjob == jobp {
        let mut s: [c_char; 48 + 1] = [0; 49];
        let len: c_int;

        len = sprint_status(s.as_mut_ptr(), status, 1);
        if len != 0 {
            s[len as usize] = b'\n' as c_char;
            s[(len + 1) as usize] = 0;
            let _ =
                sh.io.stderr().write_all(CStr::from_ptr(s.as_ptr()).to_bytes());
        }
    }
    /* This frame brackets the whole wait in INTOFF/INTON, so the poll
     * inside `waitproc` cannot fire under it -- and the C did not deliver
     * there either. The C delivered at the `INTON` above, when the
     * counter reached zero, and this is that instruction. Putting the
     * poll at the call site rather than inside `INTON` is what keeps
     * `INTON` infallible (§4.3) without moving the delivery point for the
     * one path where it would have been visible: a ^C during a foreground
     * command that does not itself die of the signal. */
    if let Some(e) = crate::error::poll_interrupt(sh) {
        return Err(e);
    }
    Ok(pid)
}

// [spec:dash:def:jobs.dowait-fn]
// [spec:dash:sem:jobs.dowait-fn]
pub(crate) unsafe fn dowait(sh: &mut crate::context::Shell, block: c_int, jp: Option<usize>) -> Result<c_int, Error> {
    let gotchld: c_int = core::ptr::read_volatile(addr_of_mut!(crate::trap::gotsigchld));
    let mut rpid: c_int;
    let mut pid: c_int;
    let mut block: c_int = block;

    if jp.map_or(false, |i| sh.jobs.tab[i].state as c_int != JOBRUNNING) {
        block = DOWAIT_NONBLOCK;
    }

    if block == DOWAIT_NONBLOCK && gotchld == 0 {
        return Ok(1);
    }

    rpid = 1;

    loop {
        pid = waitone(sh, block, jp)?;
        rpid &= (pid != 0) as c_int;

        block &= !DOWAIT_WAITCMD_ALL;
        if pid == 0 || jp.map_or(false, |i| sh.jobs.tab[i].state as c_int != JOBRUNNING) {
            block = DOWAIT_NONBLOCK;
        }
        if !(pid >= 0) {
            break;
        }
    }

    Ok(rpid)
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
unsafe fn waitproc(sh: &mut crate::context::Shell, block: c_int, status: *mut c_int) -> Result<c_int, Error> {
    let mut oldmask: libc::sigset_t = core::mem::zeroed();
    let mut flags: c_int = if block == DOWAIT_BLOCK {
        0
    } else {
        libc::WNOHANG
    };
    let mut err: c_int;

    if sh.jobs.jobctl != 0 {
        flags |= libc::WUNTRACED;
    }

    /* HAVE_WAIT3; the fallback is `waitpid((pid_t)-1, status, flags, NULL)`.
     * `wait3` has no binding in the `libc` crate, so it is declared here. */
    unsafe extern "C" {
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
            /* One of the three EINTR sites the C retries blindly, and the
             * one that matters for a ^C during a foreground command that
             * does not itself die of it. */
            if let Some(e) = crate::error::poll_interrupt(sh) {
                return Err(e);
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

    Ok(err)
}

/*
 * return 1 if there are stopped jobs, otherwise 0
 */

// [spec:dash:def:jobs.stoppedjobs-fn]
// [spec:dash:sem:jobs.stoppedjobs-fn]
pub unsafe fn stoppedjobs(sh: &mut crate::context::Shell) -> c_int {
    let jp: Option<usize>;
    let mut retval: c_int;

    retval = 0;
    'out_lbl: {
        if JOBS == 0 {
            break 'out_lbl;
        }
        if sh.jobs.job_warning != 0 {
            break 'out_lbl;
        }
        jp = sh.jobs.curjob;
        if jp.map_or(false, |i| sh.jobs.tab[i].state as c_int == JOBSTOPPED) {
            let _ = sh.io.stderr().write_all(b"You have stopped jobs.\n");
            sh.jobs.job_warning = 2;
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

/// The text `cmdtxt` is building. The C's `cmdnextc` is a `char *` cursor
/// into the stack block; here the cursor is the buffer's length.
static mut cmdbuf: BString = BString::new(Vec::new());

/// A fresh borrow of `cmdbuf` per access, so that a `cmdputs` reached
/// from the middle of `cmdtxt`'s recursion never holds one.
#[inline]
unsafe fn cmdtext() -> &'static mut BString {
    &mut *addr_of_mut!(cmdbuf)
}

// [spec:dash:def:jobs.commandtext-fn]
// [spec:dash:sem:jobs.commandtext-fn]
unsafe fn commandtext(n: &Node) -> BString {
    cmdtext().clear();
    cmdtxt(Some(n));
    /* `cmdtxt` writes nothing at all for a command with no words — `x=1 &`
     * is one — and the C then hands `savestr` an uninitialised stack block,
     * out of which the reference reads a NUL and prints an empty command
     * text. The empty buffer is that, said on purpose. */
    /* TRACE(("commandtext: name %p, end %p\n", name, cmdnextc)); */
    cmdtext().clone()
}

// [spec:dash:def:jobs.cmdtxt-fn]
// [spec:dash:sem:jobs.cmdtxt-fn]
//
// `cmdtxt` has two *backward* gotos (`goto dodo` from NFOR and
// `goto donode` from the redirection tail), so the label graph is
// expressed as an explicit program counter rather than as nested
// labelled blocks.
unsafe fn cmdtxt(n: Option<&Node>) {
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

    /* The C reassigns `n` and jumps; every label that reads a *field* reads
     * one of the node the switch was entered with, and every label that
     * reassigns hands the new node straight to a recursive `cmdtxt` that
     * tolerates NULL. So the entry node and the "next" node are separate
     * bindings here. */
    let cur: &Node = match n {
        Some(n) => n,
        None => return,
    };
    let mut n: Option<&Node> = None;

    let mut pc: c_int = L_SWITCH;
    loop {
        match pc {
            L_SWITCH => match cur.node_type() {
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
                    n = cur.nredir().n.as_deref();
                    pc = L_DONODE;
                }
                NNOT => {
                    cmdputs(b"!\0".as_ptr() as *const c_char);
                    n = cur.nnot().com.as_deref();
                    pc = L_DONODE;
                }
                NIF => {
                    let f = cur.nif();
                    cmdputs(b"if \0".as_ptr() as *const c_char);
                    cmdtxt(f.test.as_deref());
                    cmdputs(b"; then \0".as_ptr() as *const c_char);
                    if f.elsepart.is_some() {
                        cmdtxt(f.ifpart.as_deref());
                        cmdputs(b"; else \0".as_ptr() as *const c_char);
                        n = f.elsepart.as_deref();
                    } else {
                        n = f.ifpart.as_deref();
                    }
                    p = b"; fi\0".as_ptr() as *const c_char;
                    pc = L_DOTAIL;
                }
                NSUBSHELL => {
                    cmdputs(b"(\0".as_ptr() as *const c_char);
                    n = cur.nredir().n.as_deref();
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
                    let f = cur.nfor();
                    cmdputs(b"for \0".as_ptr() as *const c_char);
                    cmdputs(f.var.as_ptr());
                    cmdputs(b" in \0".as_ptr() as *const c_char);
                    cmdlist(&f.args, 1);
                    n = f.body.as_deref();
                    p = b"; done\0".as_ptr() as *const c_char;
                    pc = L_DODO;
                }
                NDEFUN => {
                    cmdputs(cur.ndefun().text.as_ptr());
                    p = b"() { ... }\0".as_ptr() as *const c_char;
                    pc = L_DOTAIL2;
                }
                NCMD => {
                    cmdlist(&cur.ncmd().args, 1);
                    cmdlist(&cur.ncmd().redirect, 0);
                    return;
                }
                NARG => {
                    p = cur.narg().text.as_ptr();
                    pc = L_DOTAIL2;
                }
                NHERE | NXHERE => {
                    p = b"<<...\0".as_ptr() as *const c_char;
                    pc = L_DOTAIL2;
                }
                NCASE => {
                    let c = cur.ncase();
                    cmdputs(b"case \0".as_ptr() as *const c_char);
                    cmdputs(c.expr.as_deref().unwrap().narg().text.as_ptr());
                    cmdputs(b" in \0".as_ptr() as *const c_char);
                    for np in &c.cases {
                        /* the C passes the head of the pattern list, so only
                         * the first pattern of a case ever prints */
                        cmdtxt(np.nclist().pattern.first());
                        cmdputs(b") \0".as_ptr() as *const c_char);
                        cmdtxt(np.nclist().body.as_deref());
                        cmdputs(b";; \0".as_ptr() as *const c_char);
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
                 * NCLIST is the only type left over, and it never reaches
                 * `cmdtxt`: the NCASE arm above hands over its `pattern`
                 * and `body`, never the NCLIST itself. */
                _ /* default, NPIPE */ => {
                    let cl = &cur.npipe().cmdlist;
                    for (i, c) in cl.iter().enumerate() {
                        cmdtxt(Some(c));
                        if i + 1 == cl.len() {
                            break;
                        }
                        cmdputs(b" | \0".as_ptr() as *const c_char);
                    }
                    return;
                }
            },
            L_BINOP => {
                // binop:
                cmdtxt(cur.nbinary().ch1.as_deref());
                cmdputs(p);
                n = cur.nbinary().ch2.as_deref();
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
                cmdtxt(cur.nbinary().ch1.as_deref());
                n = cur.nbinary().ch2.as_deref();
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
                s[0] = (cur.redir_fd() + '0' as c_int) as c_char;
                s[1] = b'\0' as c_char;
                cmdputs(s.as_ptr());
                cmdputs(p);
                if cur.node_type() == NTOFD || cur.node_type() == NFROMFD {
                    s[0] = (cur.ndup().dupfd.get() + '0' as c_int) as c_char;
                    p = s.as_ptr();
                    pc = L_DOTAIL2;
                } else {
                    n = cur.nfile().fname.as_deref();
                    pc = L_DONODE;
                }
            }
        }
    }
}

// [spec:dash:def:jobs.cmdlist-fn]
// [spec:dash:sem:jobs.cmdlist-fn]
unsafe fn cmdlist(np: &[Node], sep: c_int) {
    for (i, node) in np.iter().enumerate() {
        if sep == 0 {
            cmdputs(core::ptr::addr_of!(crate::mystring::spcstr) as *const c_char);
        }
        cmdtxt(Some(node));
        if sep != 0 && i + 1 < np.len() {
            cmdputs(core::ptr::addr_of!(crate::mystring::spcstr) as *const c_char);
        }
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
    let mut c: c_char;
    let mut subtype: c_int = 0;
    let mut quoted: c_int = 0;

    /* The C reserves `(strlen(s) + 1) * 8` — its bound on how far the
     * cursor can run for this one input string — because a `char *`
     * cursor cannot grow the block it walks. Pushing does. */
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
                cmdtext().push(c as u8);
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
            cmdtext().push(c as u8);
        }
    }
    if (quoted & 1) != 0 {
        /* USTPUTC('"', nextc) */
        cmdtext().push(b'"');
    }
    /* The C leaves an unadvanced `*nextc = '\0'` for `commandtext` to
     * read as the end of the text. The length is that. */
}

// [spec:dash:def:jobs.showpipe-fn]
// [spec:dash:sem:jobs.showpipe-fn]
pub(crate) unsafe fn showpipe(sh: &mut crate::context::Shell, jp: usize, dest: Dest) {
    let spend: usize = sh.jobs.tab[jp].ps.len();

    for sp in 1..spend {
        let _ = sh.io.get(dest).write_all(b" | ");
        outcmd(sh, jp, sp, dest);
    }
    let _ = sh.io.get(dest).write_all(b"\n");
    sh.io.flushall();
}

// [spec:dash:def:jobs.xtcsetpgrp-fn]
// [spec:dash:sem:jobs.xtcsetpgrp-fn]
unsafe fn xtcsetpgrp(sh: &mut crate::context::Shell, fd: c_int, pgrp: pid_t) -> Result<(), Error> {
    let err: c_int;

    crate::trap::sigblockall(null_mut());
    err = libc::tcsetpgrp(fd, pgrp);
    crate::system::sigclearmask();

    if err != 0 {
        let mut message = b"Cannot set tty process group (".to_vec();
        message.extend_from_slice(CStr::from_ptr(libc::strerror(errno())).to_bytes());
        message.push(b')');
        return Err(sh.sh_error_value(&message));
    }
    Ok(())
}

// [spec:dash:def:jobs.getstatus-fn]
// [spec:dash:sem:jobs.getstatus-fn]
pub(crate) unsafe fn getstatus(sh: &mut crate::context::Shell, jobp: usize) -> c_int {
    let mut status: c_int;
    let mut retval: c_int;
    let mut ps: usize;

    /* `job->ps + job->nprocs - 1` in C: the bitfield promotes to `int`,
     * so a job that has not forked yet reads `ps[-1]`. It has no status
     * to report; `wait %n` on one answers 0. */
    ps = sh.jobs.tab[jobp].ps.len();
    status = if ps == 0 {
        0
    } else {
        sh.jobs.tab[jobp].ps[ps - 1].status
    };
    if pipefail(sh) != 0 {
        loop {
            if status != 0 {
                break;
            }
            if ps < 2 {
                break;
            }
            ps -= 1;
            status = sh.jobs.tab[jobp].ps[ps - 1].status;
        }
    }

    retval = libc::WEXITSTATUS(status);
    if !libc::WIFEXITED(status) {
        retval = libc::WSTOPSIG(status);
        if !libc::WIFSTOPPED(status) {
            /* XXX: limits number of signals */
            retval = libc::WTERMSIG(status);
            if retval == libc::SIGINT {
                sh.jobs.tab[jobp].sigint = 1;
            }
        }
        retval += 128;
    }
    /* TRACE(("getstatus: job %d, nproc %d, status %x, retval %x\n", ...)); */
    retval
}
