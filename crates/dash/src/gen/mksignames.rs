//! Literal port of `src/mksignames.c`.
//! Rules: `docs/spec/port/src/mksignames.md`.
//!
//! `signames.c` -- Create and write `signames.c', which contains an array of
//! signal names.  Imported from GNU Bash and therefore GPL-2+, unlike the
//! rest of the tree.
//!
//! It is a build-time code generator, not part of the shell; it is ported so
//! that `crate::signames` stays derivable.  The C selects signals with
//! `#if defined (SIG…)`; this port is compiled for one target at a time, so
//! the arms that Linux/glibc does not define (the AIX, SunOS5, HP-UX, BeOS
//! and 4.4BSD ones — `SIGLOST`, `SIGMSG`, `SIGDANGER`, `SIGMIGRATE`,
//! `SIGPRE`, `SIGVIRT`, `SIGALRM1`, `SIGWAITING`, `SIGGRANT`, `SIGKAP`,
//! `SIGRETRACT`, `SIGSOUND`, `SIGSAK`, `SIGLWP`, `SIGFREEZE`, `SIGTHAW`,
//! `SIGCANCEL`, `SIGDIL`, `SIGWINDOW`, `SIGEMT`, `SIGINFO`, `SIGKILLTHR`)
//! are simply absent.  The order of the arms that remain is unchanged,
//! because it is load-bearing: later assignments overwrite earlier ones.

use core::ptr;

use libc::{c_char, c_int, FILE};

/// `#if !defined (NSIG) #  define NSIG 64 #endif` — glibc defines it as 65.
const NSIG: usize = 65;

/*
 * Special traps:
 *	EXIT == 0
 */
const LASTSIG: usize = NSIG - 1;

static mut signal_names: [*mut c_char; 2 * NSIG + 3] = [ptr::null_mut(); 2 * NSIG + 3];

const signal_names_size: usize = 2 * NSIG + 3;

static mut progname: *const c_char = ptr::null();

/* SIGRTMIN and SIGRTMAX are both defined on Linux. */
const RTLEN: usize = 14;
const RTLIM: c_int = 256;

// [spec:dash:def:mksignames.initialize-signames-fn]
// [spec:dash:sem:mksignames.initialize-signames-fn]
pub unsafe fn initialize_signames() {
    let mut i: c_int;
    let rtmin: c_int;
    let rtmax: c_int;
    let mut rtcnt: c_int;

    i = 1;
    while (i as usize) < signal_names_size {
        signal_names[i as usize] = ptr::null_mut();
        i += 1;
    }

    /* `signal' 0 is what we do on exit. */
    signal_names[0] = c"EXIT".as_ptr() as *mut c_char;

    /* Place signal names which can be aliases for more common signal
       names first.  This allows (for example) SIGABRT to overwrite SIGLOST. */

    /* POSIX 1003.1b-1993 real time signals, but take care of incomplete
       implementations. Acoording to the standard, both, SIGRTMIN and
       SIGRTMAX must be defined, SIGRTMIN must be stricly less than
       SIGRTMAX, and the difference must be at least 7, that is, there
       must be at least eight distinct real time signals. */

    /* The generated signal names are SIGRTMIN, SIGRTMIN+1, ...,
       SIGRTMIN+x, SIGRTMAX-x, ..., SIGRTMAX-1, SIGRTMAX. If the number
       of RT signals is odd, there is an extra SIGRTMIN+(x+1).
       These names are the ones used by ksh and /usr/xpg4/bin/sh on SunOS5. */

    rtmin = libc::SIGRTMIN();
    signal_names[rtmin as usize] = c"RTMIN".as_ptr() as *mut c_char;

    rtmax = libc::SIGRTMAX();
    signal_names[rtmax as usize] = c"RTMAX".as_ptr() as *mut c_char;

    if rtmax > rtmin {
        rtcnt = (rtmax - rtmin - 1) / 2;
        /* croak if there are too many RT signals */
        if rtcnt >= RTLIM / 2 {
            rtcnt = RTLIM / 2 - 1;
            eprintln!(
                "{}: error: more than {} real time signals, fix `{}'",
                core::ffi::CStr::from_ptr(progname).to_string_lossy(),
                RTLIM,
                core::ffi::CStr::from_ptr(progname).to_string_lossy()
            );
        }

        i = 1;
        while i <= rtcnt {
            signal_names[(rtmin + i) as usize] = libc::malloc(RTLEN) as *mut c_char;
            if !signal_names[(rtmin + i) as usize].is_null() {
                libc::snprintf(
                    signal_names[(rtmin + i) as usize],
                    RTLEN,
                    c"RTMIN+%d".as_ptr(),
                    i,
                );
            }
            signal_names[(rtmax - i) as usize] = libc::malloc(RTLEN) as *mut c_char;
            if !signal_names[(rtmax - i) as usize].is_null() {
                libc::snprintf(
                    signal_names[(rtmax - i) as usize],
                    RTLEN,
                    c"RTMAX-%d".as_ptr(),
                    i,
                );
            }
            i += 1;
        }

        if rtcnt < RTLIM / 2 - 1 && rtcnt != (rtmax - rtmin) / 2 {
            /* Need an extra RTMIN signal */
            signal_names[(rtmin + rtcnt + 1) as usize] = libc::malloc(RTLEN) as *mut c_char;
            if !signal_names[(rtmin + rtcnt + 1) as usize].is_null() {
                libc::snprintf(
                    signal_names[(rtmin + rtcnt + 1) as usize],
                    RTLEN,
                    c"RTMIN+%d".as_ptr(),
                    rtcnt + 1,
                );
            }
        }
    }

    /* System V */
    /* SIGCLD: Like SIGCHLD.  glibc has no separate constant; it is SIGCHLD. */
    signal_names[libc::SIGCHLD as usize] = c"CLD".as_ptr() as *mut c_char;

    /* power state indication */
    signal_names[libc::SIGPWR as usize] = c"PWR".as_ptr() as *mut c_char;

    /* Pollable event (for streams) */
    signal_names[libc::SIGPOLL as usize] = c"POLL".as_ptr() as *mut c_char;

    /* Common */
    /* hangup */
    signal_names[libc::SIGHUP as usize] = c"HUP".as_ptr() as *mut c_char;

    /* interrupt */
    signal_names[libc::SIGINT as usize] = c"INT".as_ptr() as *mut c_char;

    /* quit */
    signal_names[libc::SIGQUIT as usize] = c"QUIT".as_ptr() as *mut c_char;

    /* illegal instruction (not reset when caught) */
    signal_names[libc::SIGILL as usize] = c"ILL".as_ptr() as *mut c_char;

    /* trace trap (not reset when caught) */
    signal_names[libc::SIGTRAP as usize] = c"TRAP".as_ptr() as *mut c_char;

    /* SIGIOT: IOT instruction.  glibc has no separate constant; it is SIGABRT. */
    signal_names[libc::SIGABRT as usize] = c"IOT".as_ptr() as *mut c_char;

    /* Cause current process to dump core. */
    signal_names[libc::SIGABRT as usize] = c"ABRT".as_ptr() as *mut c_char;

    /* floating point exception */
    signal_names[libc::SIGFPE as usize] = c"FPE".as_ptr() as *mut c_char;

    /* kill (cannot be caught or ignored) */
    signal_names[libc::SIGKILL as usize] = c"KILL".as_ptr() as *mut c_char;

    /* bus error */
    signal_names[libc::SIGBUS as usize] = c"BUS".as_ptr() as *mut c_char;

    /* segmentation violation */
    signal_names[libc::SIGSEGV as usize] = c"SEGV".as_ptr() as *mut c_char;

    /* bad argument to system call */
    signal_names[libc::SIGSYS as usize] = c"SYS".as_ptr() as *mut c_char;

    /* write on a pipe with no one to read it */
    signal_names[libc::SIGPIPE as usize] = c"PIPE".as_ptr() as *mut c_char;

    /* alarm clock */
    signal_names[libc::SIGALRM as usize] = c"ALRM".as_ptr() as *mut c_char;

    /* software termination signal from kill */
    signal_names[libc::SIGTERM as usize] = c"TERM".as_ptr() as *mut c_char;

    /* urgent condition on IO channel */
    signal_names[libc::SIGURG as usize] = c"URG".as_ptr() as *mut c_char;

    /* sendable stop signal not from tty */
    signal_names[libc::SIGSTOP as usize] = c"STOP".as_ptr() as *mut c_char;

    /* stop signal from tty */
    signal_names[libc::SIGTSTP as usize] = c"TSTP".as_ptr() as *mut c_char;

    /* continue a stopped process */
    signal_names[libc::SIGCONT as usize] = c"CONT".as_ptr() as *mut c_char;

    /* to parent on child stop or exit */
    signal_names[libc::SIGCHLD as usize] = c"CHLD".as_ptr() as *mut c_char;

    /* to readers pgrp upon background tty read */
    signal_names[libc::SIGTTIN as usize] = c"TTIN".as_ptr() as *mut c_char;

    /* like TTIN for output if (tp->t_local&LTOSTOP) */
    signal_names[libc::SIGTTOU as usize] = c"TTOU".as_ptr() as *mut c_char;

    /* input/output possible signal */
    signal_names[libc::SIGIO as usize] = c"IO".as_ptr() as *mut c_char;

    /* exceeded CPU time limit */
    signal_names[libc::SIGXCPU as usize] = c"XCPU".as_ptr() as *mut c_char;

    /* exceeded file size limit */
    signal_names[libc::SIGXFSZ as usize] = c"XFSZ".as_ptr() as *mut c_char;

    /* virtual time alarm */
    signal_names[libc::SIGVTALRM as usize] = c"VTALRM".as_ptr() as *mut c_char;

    /* profiling time alarm */
    signal_names[libc::SIGPROF as usize] = c"PROF".as_ptr() as *mut c_char;

    /* window changed */
    signal_names[libc::SIGWINCH as usize] = c"WINCH".as_ptr() as *mut c_char;

    /* user defined signal 1 */
    signal_names[libc::SIGUSR1 as usize] = c"USR1".as_ptr() as *mut c_char;

    /* user defined signal 2 */
    signal_names[libc::SIGUSR2 as usize] = c"USR2".as_ptr() as *mut c_char;

    i = 0;
    while (i as usize) < NSIG {
        if signal_names[i as usize].is_null() {
            signal_names[i as usize] = libc::malloc(18) as *mut c_char;
            if !signal_names[i as usize].is_null() {
                libc::snprintf(signal_names[i as usize], 18, c"%d".as_ptr(), i);
            }
        }
        i += 1;
    }
}

// [spec:dash:def:mksignames.write-signames-fn]
// [spec:dash:sem:mksignames.write-signames-fn]
pub unsafe fn write_signames(stream: *mut FILE) {
    let mut i: c_int;

    libc::fprintf(
        stream,
        c"/* This file was automatically created by %s.\n".as_ptr(),
        progname,
    );
    libc::fprintf(
        stream,
        c"   Do not edit.  Edit support/mksignames.c instead. */\n\n".as_ptr(),
    );
    libc::fprintf(stream, c"#include <signal.h>\n\n".as_ptr());
    libc::fprintf(
        stream,
        c"/* A translation list so we can be polite to our users. */\n".as_ptr(),
    );
    libc::fprintf(
        stream,
        c"const char *const signal_names[NSIG + 1] = {\n".as_ptr(),
    );

    i = 0;
    while i <= LASTSIG as c_int {
        libc::fprintf(stream, c"    \"%s\",\n".as_ptr(), signal_names[i as usize]);
        i += 1;
    }

    libc::fprintf(stream, c"    (char *)0x0\n".as_ptr());
    libc::fprintf(stream, c"};\n".as_ptr());
}

// [spec:dash:def:mksignames.main-fn]
// [spec:dash:sem:mksignames.main-fn]
pub fn main_fn(argc: c_int, argv: Vec<String>) -> c_int {
    unsafe {
        let stream_name: std::ffi::CString;
        let stream: *mut FILE;

        let progname_str = std::ffi::CString::new(argv[0].as_str()).unwrap();
        progname = progname_str.as_ptr();

        if argc == 1 {
            stream_name = std::ffi::CString::new("signames.c").unwrap();
        } else if argc == 2 {
            stream_name = std::ffi::CString::new(argv[1].as_str()).unwrap();
        } else {
            eprintln!("Usage: {} [output-file]", argv[0]);
            libc::exit(1);
        }

        stream = libc::fopen(stream_name.as_ptr(), c"w".as_ptr());
        if stream.is_null() {
            eprintln!(
                "{}: {}: cannot open for writing",
                argv[0],
                stream_name.to_string_lossy()
            );
            libc::exit(2);
        }

        initialize_signames();
        write_signames(stream);
        libc::exit(0);
    }
}
