//! Literal port of `src/input.c` / `src/input.h`.
//! Rules: `docs/spec/port/src/input.md`.
//!
//! Configuration: `SMALL` is *not* defined, so `IS_DEFINED_SMALL` is false and
//! the `#ifndef SMALL` arms (`lleft`, libedit, history) are the live ones.
//! Both `IS_DEFINED_SMALL` arms are carried, exactly as the C carries them.

use libc::{c_char, c_int, c_long, c_uint, c_void, off_t, size_t, tcflag_t};
use core::ptr::{addr_of_mut, null_mut};

use crate::alias::alias;
use crate::error::{INTOFF, INTON};
use crate::memalloc::{ckfree, ckmalloc};
use crate::syntax::PEOF;

/* PEOF (the end of file marker) is defined in syntax.h */
pub const PEOA: c_int = PEOF - 1;

/// `MB_LEN_MAX > 16 ? MB_LEN_MAX : 16` — 16 on glibc.
pub const PUNGETC_MAX: usize = 16;
/// stdio's `BUFSIZ`.
pub const BUFSIZ: c_int = 8192;
pub const IBUFSIZ: usize = BUFSIZ as usize + PUNGETC_MAX + 1;

/// `#ifdef SMALL / #define IS_DEFINED_SMALL 1 #else 0` — this port is !SMALL.
pub const IS_DEFINED_SMALL: bool = false;

/*
 * config.h knobs used by this file.  The reference build has `HAVE_TEE 1` /
 * `USE_TEE 1`, so `tee(2)` comes from glibc and system.h's
 * `#ifndef HAVE_TEE` stub is not compiled.
 */
pub const USE_TEE: c_int = 1;

pub const INPUT_PUSH_FILE: c_int = 1;
pub const INPUT_NOFILE_OK: c_int = 2;

// [spec:dash:def:input.strpush]
#[repr(C)]
pub struct strpush {
    pub prev: *mut strpush,      /* preceding string on stack */
    pub prevstring: *mut c_char,
    pub prevnleft: c_int,
    pub ap: *mut alias,          /* if push was associated with an alias */
    pub string: *mut c_char,     /* remember the string since it may change */
    /* Delay freeing so we can stop nested aliases. */
    pub spfree: *mut strpush,
    /* Number of outstanding calls to pungetc. */
    pub unget: c_int,
}

/*
 * The parsefile structure pointed to by the global variable parsefile
 * contains information about the current file being read.
 */

// [spec:dash:def:input.parsefile]
#[repr(C)]
pub struct parsefile {
    pub prev: *mut parsefile,    /* preceding file on stack */
    pub linno: c_int,            /* current line */
    pub fd: c_int,               /* file descriptor (or -1 if string) */
    pub nleft: c_int,            /* number of chars left in this line */
    pub eof: c_int,              /* do not read again once we hit EOF */
    pub nextc: *mut c_char,      /* next char in buffer */
    pub buf: *mut c_char,        /* input buffer */
    pub strpush: *mut strpush,   /* for pushing strings at this level */
    pub basestrpush: strpush,    /* so pushing one is fast */
    /* Delay freeing so we can stop nested aliases. */
    pub spfree: *mut strpush,
    /* #ifndef SMALL */
    pub lleft: c_int, /* number of chars left in this buffer */
    /* Number of outstanding calls to pungetc. */
    pub unget: c_int,
}

const EMPTY_STRPUSH: strpush = strpush {
    prev: null_mut(),
    prevstring: null_mut(),
    prevnleft: 0,
    ap: null_mut(),
    string: null_mut(),
    spfree: null_mut(),
    unget: 0,
};

// [spec:dash:def:input.stdin-state]
/// `MKINIT struct stdin_state { … }` — absent from the port manifest because
/// the `MKINIT` marker defeated the extractor.
#[repr(C)]
pub struct stdin_state_t {
    pub seekable: off_t,
    pub pip: [c_int; 2],
    pub pending: c_int,
    pub bufferable: tcflag_t,
}

pub static mut basepf: parsefile = parsefile {
    prev: null_mut(),
    linno: 0,
    fd: 0,
    nleft: 0,
    eof: 0,
    nextc: null_mut(),
    buf: null_mut(),
    strpush: null_mut(),
    basestrpush: EMPTY_STRPUSH,
    spfree: null_mut(),
    lleft: 0,
    unget: 0,
}; /* top level input file */
pub static mut basebuf: [c_char; IBUFSIZ] = [0; IBUFSIZ]; /* buffer for top level input file */
pub static mut toppf: *mut parsefile = addr_of_mut!(basepf);
pub static mut stdin_state: stdin_state_t = stdin_state_t {
    seekable: 0,
    pip: [0, 0],
    pending: 0,
    bufferable: 0,
};
pub static mut parsefile: *mut parsefile = addr_of_mut!(basepf); /* current input file */
pub static mut whichprompt: c_int = 0; /* 1 == PS1, 2 == PS2 */
pub static mut stdin_istty: c_int = -1;

/// `#define plinno (parsefile->linno)`
#[macro_export]
macro_rules! plinno {
    () => {
        (*$crate::input::parsefile).linno
    };
}

// [spec:dash:def:input.input-get-lleft-fn]
// [spec:dash:sem:input.input-get-lleft-fn]
pub unsafe fn input_get_lleft(pf: *mut parsefile) -> c_int {
    /* #ifdef SMALL return 0; #else */
    (*pf).lleft
}

// [spec:dash:def:input.input-set-lleft-fn]
// [spec:dash:sem:input.input-set-lleft-fn]
pub unsafe fn input_set_lleft(pf: *mut parsefile, len: c_int) {
    /* #ifndef SMALL */
    (*pf).lleft = len;
}

#[inline]
unsafe fn errno() -> c_int {
    *libc::__errno_location()
}

#[inline]
unsafe fn set_errno(e: c_int) {
    *libc::__errno_location() = e;
}

/* mkinit INIT fragment from src/input.c:96-99. */
pub unsafe fn mkinit_init() {
    basepf.buf = addr_of_mut!(basebuf) as *mut c_char;
    basepf.nextc = basepf.buf;
    basepf.linno = 1;
    /* Not in the C: `basepf` is statically `.fd = 0` there because the
     * shell reads descriptor 0 by definition. Here the base parse file
     * reads whatever the frontend gave us -- which is 0 unless it said
     * otherwise. See [dec:nsh:host-owns-streams]. */
    basepf.fd = crate::streams::streams().stdin;
}

/* mkinit RESET fragment from src/input.c:101-112. */
pub unsafe fn mkinit_reset() {
    let mut c: c_int;

    /* clear input buffer */
    popallfiles();

    c = PEOF;
    if ((*toppf).nextc as isize - (*toppf).buf as isize) > (*toppf).unget as isize {
        c = *(*toppf).nextc.offset(-((*toppf).unget as isize) - 1) as c_int;
    }
    while c != b'\n' as c_int && c != PEOF && crate::error::int_pending() == 0 {
        c = pgetc();
    }
}

/* mkinit FORKRESET fragment from src/input.c:114-125. */
pub unsafe fn mkinit_forkreset() {
    popallfiles();
    /* The C tests `> 0`, meaning "an open file that is not stdin". With a
     * frontend-supplied stdin the second half of that is no longer implied
     * by the first, and getting it wrong would close the shell's own
     * input. */
    let sin: c_int = crate::streams::streams().stdin;
    if (*parsefile).fd > 0 && (*parsefile).fd != sin {
        libc::close((*parsefile).fd);
        (*parsefile).fd = sin;
    }
    if stdin_state.pip[0] != 0 {
        libc::close(stdin_state.pip[0]);
        libc::close(stdin_state.pip[1]);
        libc::memset(
            addr_of_mut!(stdin_state.pip) as *mut c_void,
            0,
            core::mem::size_of::<[c_int; 2]>() as size_t,
        );
    }
}

/* mkinit POSTEXITRESET fragment from src/input.c:127-129. */
pub unsafe fn mkinit_postexitreset() {
    flush_input();
}

// [spec:dash:def:input.input-init-fn]
// [spec:dash:sem:input.input-init-fn]
pub unsafe fn input_init() {
    let st: *mut stdin_state_t = addr_of_mut!(stdin_state);
    let mut tios: libc::termios = core::mem::zeroed();
    let istty: c_int;

    let sin: c_int = crate::streams::streams().stdin;

    istty = libc::tcgetattr(sin, &mut tios) + 1;
    stdin_istty = istty;
    if istty != 0 {
        (*st).bufferable = tios.c_lflag & libc::ICANON;
    } else {
        (*st).seekable = libc::lseek(sin, 0, libc::SEEK_CUR) + 1;
        (*st).bufferable = ((*st).seekable != 0) as tcflag_t;
    }
}

// [spec:dash:def:input.stdin-bufferable-fn]
// [spec:dash:sem:input.stdin-bufferable-fn]
unsafe fn stdin_bufferable() -> bool {
    let st: *mut stdin_state_t = addr_of_mut!(stdin_state);

    if stdin_istty < 0 {
        input_init();
    }

    (*st).bufferable != 0
}

// [spec:dash:def:input.flush-tee-fn]
// [spec:dash:sem:input.flush-tee-fn]
unsafe fn flush_tee(buf: *mut c_void, nr: c_int, mut pending: c_int) {
    while pending > 0 {
        let err: c_int;

        err = libc::read(
            crate::streams::streams().stdin,
            buf,
            if nr > pending { pending } else { nr } as size_t,
        ) as c_int;
        if err > 0 {
            pending -= err;
        }
    }
}

// [spec:dash:def:input.stdin-tee-fn]
// [spec:dash:sem:input.stdin-tee-fn]
unsafe fn stdin_tee(buf: *mut c_void, nr: c_int) -> c_int {
    let err: c_int;

    if stdin_state.pip[0] == 0 {
        crate::redir::sh_pipe(addr_of_mut!(stdin_state.pip) as *mut c_int, 0);
        if stdin_state.pip[0] < 10 {
            stdin_state.pip[0] = crate::redir::savefd(stdin_state.pip[0], stdin_state.pip[0]);
        }
        if stdin_state.pip[1] < 10 {
            stdin_state.pip[1] = crate::redir::savefd(stdin_state.pip[1], stdin_state.pip[1]);
        }
    }

    flush_tee(buf, nr, stdin_state.pending);

    if USE_TEE != 0 {
        err = libc::tee(0, stdin_state.pip[1], nr as size_t, 0) as c_int;
    } else {
        set_errno(libc::EINVAL);
        err = -1;
    }
    stdin_state.pending = err;
    err
}

// [spec:dash:def:input.freestrings-fn]
// [spec:dash:sem:input.freestrings-fn]
unsafe fn freestrings(mut sp: *mut strpush) {
    INTOFF();
    loop {
        let psp: *mut strpush;

        if !(*sp).ap.is_null() {
            (*(*sp).ap).flag &= !crate::alias::ALIASINUSE;
            if ((*(*sp).ap).flag & crate::alias::ALIASDEAD) != 0 {
                crate::alias::unalias((*(*sp).ap).name);
            }
        }

        psp = sp;
        sp = (*sp).spfree;

        if psp != addr_of_mut!((*parsefile).basestrpush) {
            ckfree(psp as *mut c_void);
        }
        if sp.is_null() {
            break;
        }
    }

    (*parsefile).spfree = null_mut();
    INTON();
}

/*
 * Read a character from the script, returning PEOF on end of file.
 * Nul characters in the input are silently discarded.
 */

// [spec:dash:def:input.pgetc-fn]
// [spec:dash:sem:input.pgetc-fn]
pub unsafe fn pgetc() -> c_int {
    let sp: *mut strpush = (*parsefile).spfree;
    let mut c: c_int;

    if !sp.is_null() {
        freestrings(sp);
    }

    'again: loop {
        if (*parsefile).unget != 0 {
            let old = (*parsefile).unget;
            (*parsefile).unget -= 1;
            let unget: c_long = -((old as c_uint) as c_long);

            return *(*parsefile).nextc.offset(unget as isize) as i8 as c_int;
        }

        'nextc: loop {
            if (*parsefile).nleft > 0 {
                (*parsefile).nleft -= 1;
                c = *(*parsefile).nextc as i8 as c_int;
                (*parsefile).nextc = (*parsefile).nextc.add(1);
            } else if !(*parsefile).strpush.is_null() {
                popstring();
                /* The freestrings call must be delayed til the next
                 * pgetc call for PEOA to work properly.
                 */
                continue 'again;
            } else {
                c = preadbuffer();
            }

            /* delete nul characters */
            if IS_DEFINED_SMALL && c == 0 {
                (*parsefile).nextc = libc::memmove(
                    (*parsefile).nextc.offset(-1) as *mut c_void,
                    (*parsefile).nextc as *const c_void,
                    (*parsefile).nleft as size_t,
                ) as *mut c_char;
                continue 'nextc;
            }

            return c;
        }
    }
}

// [spec:dash:def:input.pgetc-eoa-fn]
// [spec:dash:sem:input.pgetc-eoa-fn]
pub unsafe fn pgetc_eoa() -> c_int {
    if !(*parsefile).strpush.is_null()
        && (*parsefile).nleft == -1
        && !(*(*parsefile).strpush).ap.is_null()
    {
        PEOA
    } else {
        pgetc()
    }
}

// [spec:dash:def:input.stdin-clear-nonblock-fn]
// [spec:dash:sem:input.stdin-clear-nonblock-fn]
unsafe fn stdin_clear_nonblock() -> c_int {
    let sin: c_int = crate::streams::streams().stdin;
    let mut flags: c_int = libc::fcntl(sin, libc::F_GETFL, 0);

    if flags >= 0 {
        flags &= !libc::O_NONBLOCK;
        flags = libc::fcntl(sin, libc::F_SETFL, flags);
    }

    flags
}

/// `const char *el_gets(EditLine *, int *)` — the libedit entry point
/// `preadfd` uses.  `crate::histedit::libedit` shims the rest of the libedit
/// API the same way; this one has no call site there, so it lives here.
unsafe fn el_gets(e: *mut crate::histedit::EditLine, n: *mut c_int) -> *const c_char {
    crate::linedit::el_gets(e, crate::histedit::hist, n)
}

// [spec:dash:def:input.preadfd-fn]
// [spec:dash:sem:input.preadfd-fn]
unsafe fn preadfd() -> c_int {
    let mut buf: *mut c_char = (*parsefile).buf;
    let mut fd: c_int = (*parsefile).fd;
    let mut use_tee: bool;
    let mut unget: c_int;
    let mut pnr: c_int;
    let mut nr: c_int;

    nr = input_get_lleft(parsefile);

    unget = ((*parsefile).nextc as isize - buf as isize) as c_int;
    if unget > PUNGETC_MAX as c_int {
        unget = PUNGETC_MAX as c_int;
    }

    libc::memmove(
        buf as *mut c_void,
        (*parsefile).nextc.offset(-(unget as isize)) as *const c_void,
        (unget + nr) as size_t,
    );
    buf = buf.offset(unget as isize);
    (*parsefile).nextc = buf;
    buf = buf.offset(nr as isize);

    nr = BUFSIZ - nr;
    if !IS_DEFINED_SMALL && nr == 0 {
        return nr;
    }

    /* The C's `fd == 0` means "this parse file is the shell's standard
     * input", which is the condition for line editing and for teeing --
     * not descriptor 0 for its own sake. */
    let sin: c_int = crate::streams::streams().stdin;

    use_tee = fd == sin
        /* #ifndef SMALL */
        && crate::histedit::el.is_null()
        && !stdin_bufferable();

    pnr = nr;
    'retry: loop {
        nr = pnr;
        /* #ifndef SMALL */
        if fd == sin && !crate::histedit::el.is_null() {
            static mut rl_cp: *const c_char = null_mut();
            static mut el_len: c_int = 0;

            if rl_cp.is_null() {
                /* `pushstackmark(&smark, stackblocksize())` around
                 * `el_gets`, whose prompt callback reaches `expandstr`.
                 * That prompt is an owned buffer now. */
                rl_cp = el_gets(crate::histedit::el, addr_of_mut!(el_len));
            }
            if rl_cp.is_null() {
                nr = 0;
            } else {
                if nr > el_len {
                    nr = el_len;
                }
                libc::memcpy(buf as *mut c_void, rl_cp as *const c_void, nr as size_t);
                if nr != el_len {
                    el_len -= nr;
                    rl_cp = rl_cp.offset(nr as isize);
                } else {
                    rl_cp = null_mut();
                }
            }

            return nr;
        }

        if use_tee {
            nr = stdin_tee(buf as *mut c_void, nr);
            if nr >= 0 {
                fd = stdin_state.pip[0];
            } else if errno() == libc::EINVAL {
                use_tee = false;
                pnr = 1;
                nr = 1;
            }
        }

        if nr > 0 {
            nr = libc::read(fd, buf as *mut c_void, nr as size_t) as c_int;
        }

        if nr < 0 {
            if errno() == libc::EINTR
                && !(!basepf.prev.is_null() && crate::trap::pending_sig != 0)
            {
                continue 'retry;
            }
            if fd == 0 && errno() == libc::EWOULDBLOCK && stdin_clear_nonblock() >= 0 {
                crate::output::out2str(
                    b"sh: turning off NDELAY mode\n\0".as_ptr() as *const c_char,
                );
                continue 'retry;
            }
        }
        break 'retry;
    }
    nr
}

/*
 * Refill the input buffer and return the next input character:
 *
 * 1) If a string was pushed back on the input, pop it;
 * 2) If we are reading from a string we can't refill the buffer, return EOF.
 * 3) If there is more stuff in this buffer, use it else call read to fill it.
 * 4) Process input up to the next newline, deleting nul characters.
 */

// [spec:dash:def:input.preadbuffer-fn]
// [spec:dash:sem:input.preadbuffer-fn]
unsafe fn preadbuffer() -> c_int {
    let first: c_int = (whichprompt == 1) as c_int;
    let mut something: c_int;
    let mut savec: c_char = 0;
    let mut more: c_int;
    let mut q: *mut c_char;
    let mut nr: c_int;
    let mut save = false;

    if ((*parsefile).eof & 2) != 0 {
        /* eof: */
        (*parsefile).eof = 3;
        return PEOF;
    }
    crate::output::flushall();

    q = (*parsefile).nextc;
    something = (first == 0) as c_int;

    more = input_get_lleft(parsefile);

    INTOFF();
    'outer: loop {
        if more <= 0 {
            /* again: */
            nr = (q as isize - (*parsefile).nextc as isize) as c_int;
            input_set_lleft(parsefile, nr);
            more = preadfd();
            q = (*parsefile).nextc.offset(nr as isize);
            if more <= 0 {
                (*parsefile).nleft = 0;
                input_set_lleft(parsefile, 0);
                if !IS_DEFINED_SMALL && nr > 0 {
                    save = true;
                    break 'outer; /* goto save */
                }
                INTON();
                /* goto eof */
                (*parsefile).eof = 3;
                return PEOF;
            }
        }

        if IS_DEFINED_SMALL {
            q = q.offset(more as isize);
            more = 0;
            break 'outer; /* goto done */
        }

        /* delete nul characters */
        loop {
            let c: c_int;

            more -= 1;
            c = *q as c_int;

            if c == 0 {
                libc::memmove(
                    q as *mut c_void,
                    q.offset(1) as *const c_void,
                    more as size_t,
                );
                /* goto check */
            } else {
                q = q.add(1);

                if c == b'\n' as c_int {
                    break 'outer; /* goto done */
                }
                if c != b'\t' as c_int && c != b' ' as c_int {
                    something = 1;
                }
            }

            /* check: */
            if more <= 0 {
                continue 'outer; /* goto again */
            }
        }
    }

    if !save {
        /* done: */
        input_set_lleft(parsefile, more);
    }

    /* save: */
    (*parsefile).nleft = ((q as isize - (*parsefile).nextc as isize) - 1) as c_int;
    if !IS_DEFINED_SMALL {
        savec = *q;
    }
    *q = b'\0' as c_char;

    if (*parsefile).fd == crate::streams::streams().stdin
        && !crate::histedit::hist.is_null()
        && something != 0
    {
        let mut he: crate::histedit::HistEvent = core::mem::zeroed();
        crate::histedit::history(
            crate::histedit::hist,
            &mut he,
            if first != 0 {
                crate::histedit::libedit::H_ENTER
            } else {
                crate::histedit::libedit::H_APPEND
            },
            (*parsefile).nextc,
        );
    }
    INTON();

    if crate::options::optlist[crate::options::vflag] != 0 {
        crate::output::out2str((*parsefile).nextc);
        /* #ifdef FLUSHERR flushout(out2); */
    }

    if !IS_DEFINED_SMALL {
        *q = savec;
    }

    let r = *(*parsefile).nextc as i8 as c_int;
    (*parsefile).nextc = (*parsefile).nextc.add(1);
    r
}

// [spec:dash:def:input.pungetn-fn]
// [spec:dash:sem:input.pungetn-fn]
pub unsafe fn pungetn(n: c_int) {
    (*parsefile).unget += n;
}

/*
 * Undo a call to pgetc.  Only two characters may be pushed back.
 * PEOF may be pushed back.
 */

// [spec:dash:def:input.pungetc-fn]
// [spec:dash:sem:input.pungetc-fn]
pub unsafe fn pungetc() {
    pungetn(1 - ((*parsefile).eof & 1));
    (*parsefile).eof &= !1;
}

/*
 * Push a string back onto the input at this current parsefile level.
 * We handle aliases this way.
 */

// [spec:dash:def:input.pushstring-fn]
// [spec:dash:sem:input.pushstring-fn]
pub unsafe fn pushstring(s: *mut c_char, ap: *mut c_void) {
    let sp: *mut strpush;
    let len: size_t;

    len = libc::strlen(s) as size_t;
    INTOFF();
    /*dprintf("*** calling pushstring: %s, %d\n", s, len);*/
    if (((*parsefile).strpush as libc::c_ulong) | ((*parsefile).spfree as libc::c_ulong)) != 0 {
        sp = ckmalloc(core::mem::size_of::<strpush>() as size_t) as *mut strpush;
        (*sp).prev = (*parsefile).strpush;
        (*parsefile).strpush = sp;
    } else {
        sp = addr_of_mut!((*parsefile).basestrpush);
        (*parsefile).strpush = sp;
    }
    (*sp).prevstring = (*parsefile).nextc;
    (*sp).prevnleft = (*parsefile).nleft;
    (*sp).unget = (*parsefile).unget;
    (*sp).spfree = (*parsefile).spfree;
    (*sp).ap = ap as *mut alias;
    if !ap.is_null() {
        (*(ap as *mut alias)).flag |= crate::alias::ALIASINUSE;
        (*sp).string = (*(ap as *mut alias)).name;
    }
    (*parsefile).nextc = s;
    (*parsefile).nleft = len as c_int;
    (*parsefile).unget = 0;
    (*parsefile).spfree = null_mut();
    INTON();
}

// [spec:dash:def:input.popstring-fn]
// [spec:dash:sem:input.popstring-fn]
unsafe fn popstring() {
    let sp: *mut strpush = (*parsefile).strpush;

    INTOFF();
    if !(*sp).ap.is_null() && (*parsefile).nextc > (*sp).string {
        if *(*parsefile).nextc.offset(-1) == b' ' as c_char
            || *(*parsefile).nextc.offset(-1) == b'\t' as c_char
        {
            crate::parser::checkkwd |= crate::parser::CHKALIAS;
        }
        if (*sp).string != (*(*sp).ap).name {
            ckfree((*sp).string as *mut c_void);
        }
    }
    (*parsefile).nextc = (*sp).prevstring;
    (*parsefile).nleft = (*sp).prevnleft;
    (*parsefile).unget = (*sp).unget;
    /*dprintf("*** calling popstring: restoring to '%s'\n", parsenextc);*/
    (*parsefile).strpush = (*sp).prev;
    (*parsefile).spfree = sp;
    INTON();
}

/*
 * Set the input to take input from a file.  If push is set, push the
 * old input onto the stack first.
 */

// [spec:dash:def:input.setinputfile-fn]
// [spec:dash:sem:input.setinputfile-fn]
pub unsafe fn setinputfile(fname: *const c_char, flags: c_int) -> c_int {
    let mut fd: c_int;

    INTOFF();
    fd = crate::redir::sh_open(fname, libc::O_RDONLY, flags & INPUT_NOFILE_OK);
    if fd < 0 {
        INTON();
        return fd; /* goto out */
    }
    if fd < 10 {
        fd = crate::redir::savefd(fd, fd);
    }
    setinputfd(fd, flags & INPUT_PUSH_FILE);
    INTON();
    fd
}

/*
 * Like setinputfile, but takes an open file descriptor.  Call this with
 * interrupts off.
 */

// [spec:dash:def:input.setinputfd-fn]
// [spec:dash:sem:input.setinputfd-fn]
unsafe fn setinputfd(fd: c_int, push: c_int) {
    pushfile();
    if push == 0 {
        toppf = parsefile;
    }
    (*parsefile).fd = fd;
    (*parsefile).buf = ckmalloc(IBUFSIZ as size_t) as *mut c_char;
    (*parsefile).nextc = (*parsefile).buf;
}

/*
 * Like setinputfile, but takes input from a string.
 */

// [spec:dash:def:input.setinputstring-fn]
// [spec:dash:sem:input.setinputstring-fn]
pub unsafe fn setinputstring(string: *mut c_char) {
    INTOFF();
    pushfile();
    (*parsefile).nextc = string;
    (*parsefile).nleft = libc::strlen(string) as c_int;
    (*parsefile).eof = 2;
    INTON();
}

/*
 * To handle the "." command, a stack of input files is used.  Pushfile
 * adds a new entry to the stack and popfile restores the previous level.
 */

// [spec:dash:def:input.pushfile-fn]
// [spec:dash:sem:input.pushfile-fn]
unsafe fn pushfile() {
    let pf: *mut parsefile;

    pf = ckmalloc(core::mem::size_of::<parsefile>() as size_t) as *mut parsefile;
    libc::memset(
        pf as *mut c_void,
        0,
        core::mem::size_of::<parsefile>() as size_t,
    );
    (*pf).prev = parsefile;
    (*pf).linno = 1;
    (*pf).fd = -1;
    parsefile = pf;
}

// [spec:dash:def:input.pushstdin-fn]
// [spec:dash:sem:input.pushstdin-fn]
pub unsafe fn pushstdin() {
    INTOFF();
    basepf.prev = parsefile;
    parsefile = addr_of_mut!(basepf);
    INTON();
}

// [spec:dash:def:input.popfile-fn]
// [spec:dash:sem:input.popfile-fn]
pub unsafe fn popfile() {
    let pf: *mut parsefile = parsefile;

    INTOFF();
    parsefile = (*pf).prev;
    (*pf).prev = null_mut();
    if pf == addr_of_mut!(basepf) {
        INTON();
        return; /* goto out */
    }

    if (*pf).fd >= 0 {
        libc::close((*pf).fd);
    }
    ckfree((*pf).buf as *mut c_void);
    if !(*parsefile).spfree.is_null() {
        freestrings((*parsefile).spfree);
    }
    while !(*pf).strpush.is_null() {
        popstring();
        freestrings((*parsefile).spfree);
    }
    ckfree(pf as *mut c_void);

    INTON();
}

// [spec:dash:def:input.unwindfiles-fn]
// [spec:dash:sem:input.unwindfiles-fn]
pub unsafe fn unwindfiles(stop: *mut parsefile) {
    while !basepf.prev.is_null() || parsefile != stop {
        popfile();
    }
}

/*
 * Return to top level.
 */

// [spec:dash:def:input.popallfiles-fn]
// [spec:dash:sem:input.popallfiles-fn]
pub unsafe fn popallfiles() {
    unwindfiles(toppf);
}

// [spec:dash:def:input.flush-input-fn]
// [spec:dash:sem:input.flush-input-fn]
pub unsafe fn flush_input() {
    let left: c_int = basepf.nleft + input_get_lleft(addr_of_mut!(basepf));

    INTOFF();
    if stdin_state.seekable != 0 && left != 0 {
        libc::lseek(
            crate::streams::streams().stdin,
            -(left as off_t),
            libc::SEEK_CUR,
        );
    } else if stdin_state.pending > left {
        flush_tee(
            addr_of_mut!(basebuf) as *mut c_void,
            BUFSIZ,
            stdin_state.pending - left,
        );
        stdin_state.pending = 0;
    }
    basepf.nleft = 0;
    input_set_lleft(addr_of_mut!(basepf), 0);
    INTON();
}

// [spec:dash:def:input.reset-input-fn]
// [spec:dash:sem:input.reset-input-fn]
pub unsafe fn reset_input() {
    stdin_istty = -1;
    basepf.eof = 0;
    flush_input();
}
