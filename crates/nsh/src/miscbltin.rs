//! Literal port of `src/miscbltin.c` / `src/miscbltin.h`.
//! Rules: `docs/spec/port/src/miscbltin.md`.
//!
//! Translation notes (literal, bug-for-bug):
//!   * The `ulimit` half is `#ifdef HAVE_GETRLIMIT`, and the `limits[]`
//!     table is `#ifdef`-guarded per resource. This port fixes the
//!     configuration to the Linux/glibc one, where every `RLIMIT_*`
//!     listed in the C is present, and notes the guards in comments.
//!   * C `goto`s are reproduced with labelled blocks; `readcmd`'s label
//!     graph (`start` is jumped into from *before* the loop) is
//!     expressed as an explicit label program counter.
//!   * `src/miscbltin.c` `#undef`s `rflag` so `readcmd` can use the
//!     name for its own local; here the local simply shadows nothing,
//!     since the shell option is reached as `optlist[...]`.

use core::ptr::null_mut;
use libc::{c_char, c_int, c_uint};
use std::ffi::{CStr, CString};
use std::io::Write as _;

use bstr::{BStr, BString};

use crate::error::{INTOFF, INTON};
use crate::expand::arglist;

/* glibc <limits.h> */
const MB_LEN_MAX: usize = 16;

/// `readcmd`'s `CHECKSTRSPACE((MB_LEN_MAX > 16 ? MB_LEN_MAX : 16) + 4, p)`.
///
/// `getmbc` writes through the bare `char *` this makes room for, so the
/// number has to stay the C's: with `mode` 0 it puts the character's bytes at
/// `out + 2` and the closing length and marker at `out + 2 + ml` and
/// `out + 3 + ml`, which for `ml == MB_LEN_MAX` is the twentieth byte and not
/// one fewer.
const READ_MBSLOP: usize = (if MB_LEN_MAX > 16 { MB_LEN_MAX } else { 16 }) + 4;

// ---------------------------------------------------------------------

/** handle one line of the read command.
 *  more fields than variables -> remainder shall be part of last variable.
 *  less fields than variables -> remaining variables unset.
 *
 *  @param line complete line of input
 *  @param ac argument count
 *  @param ap argument (variable) list
 *  @param len length of line including trailing '\0'
 */

// [spec:dash:def:miscbltin.readcmd-handle-line-fn]
// [spec:dash:sem:miscbltin.readcmd-handle-line-fn]
unsafe fn readcmd_handle_line(line: &mut BString, names: &[&BStr]) {
    let mut arglist: arglist = arglist::new();

    /* `s = grabstackstr(s)`.  The C is handed the cursor one *past* the
     * terminator and turns it into the block's base, which both names the
     * line and reserves it so that `ifsbreakup`'s `stalloc`s land above it.
     * An owned line is already its own base and there is nothing to reserve;
     * the fields `ifsbreakup` builds copy out of it rather than pointing
     * into it, so the line only has to outlive that one call. */
    let s: *mut c_char = line.as_mut_ptr() as *mut c_char;
    debug_assert!(!line.is_empty(), "readcmd always pushes the terminator");

    crate::expand::ifsbreakup(s, names.len() as c_int, &mut arglist);
    crate::expand::ifsfree();

    /* The C walks the names and the fields with two cursors that advance
     * together, so the field for a name is the field at its index; a name
     * past the last field is the "nullify remaining arguments" case. */
    for (index, name) in names.iter().enumerate() {
        let name = crate::shell::cstring(name);
        match arglist.list.get_mut(index) {
            None => {
                crate::var::setvar(
                    name.as_ptr(),
                    core::ptr::addr_of!(crate::shell::nullstr) as *const c_char,
                    0,
                );
            }
            Some(field) => {
                /* set variable to field */
                field.rmescapes();
                crate::var::setvar(name.as_ptr(), field.textp(), 0);
            }
        }
    }
}

/*
 * The read builtin.  The -e option causes backslashes to escape the
 * following character. The -p option followed by an argument prompts
 * with the argument.
 *
 * This uses unbuffered input, which may be avoidable in some cases.
 */

// [spec:dash:def:miscbltin.readcmd-fn]
// [spec:dash:sem:miscbltin.readcmd-fn]
pub unsafe fn readcmd(args: &[&BStr]) -> c_int {
    let mut prompt: Option<CString>;
    let mut startloc: c_int = 0;
    let mut newloc: c_int = 0;
    let mut status: c_int;
    let mut rflag: c_int;

    rflag = 0;
    prompt = None;
    let mut opts = crate::options::Options::new(args);
    while let Some(i) = opts.next(b"p:r") {
        if i == b'p' {
            prompt = Some(crate::shell::cstring(opts.arg()));
        } else {
            rflag = 1;
        }
    }
    if let Some(prompt) = &prompt {
        if libc::isatty(crate::streams::streams().stdin) != 0 {
            let _ = (&mut *crate::output::stderr()).write_all(prompt.as_bytes());
        }
    }
    let names = opts.operands();
    if names.is_empty() {
        crate::error::sh_error(b"arg count");
    }

    status = 0;
    /* `STARTSTACKSTR(p)`.  The line is an owned buffer, so the C's cursor is
     * its length and `stackblock()` its base: every `p - stackblock()` below
     * is `line.len()`, and `USTPUTC` is `push`. */
    let mut line = BString::default();

    crate::input::pushstdin();

    /* The C body is a `for (;;)` entered by `goto start`, with the
     * labels `put`, `record` and `start` inside it. The label graph is
     * reproduced with an explicit program counter. */
    const L_BODY: c_int = 0;
    const L_PUT: c_int = 1;
    const L_RECORD: c_int = 2;
    const L_START: c_int = 3;

    let mut pc: c_int = L_START; /* goto start */
    let mut c: c_int = 0;

    loop {
        if pc == L_BODY {
            let ml: c_uint;

            /* CHECKSTRSPACE((MB_LEN_MAX > 16 ? MB_LEN_MAX : 16) + 4, p) —
             * the room `getmbc` writes into through the raw cursor below. */
            line.reserve(READ_MBSLOP);
            c = crate::input::pgetc();
            if c == crate::syntax::PEOF {
                status = 1;
                break;
            }
            if c == '\0' as c_int {
                pc = L_BODY;
                continue;
            }
            let at = line.len();
            ml = crate::parser::getmbc(c, line.as_mut_ptr().add(at) as *mut c_char, 0);
            if ml != 0 {
                /* `p += ml` is the commit of what `getmbc` wrote past the
                 * cursor; a zero return leaves the scribble uncommitted, for
                 * the next write to overwrite exactly as the C's does. */
                debug_assert!(ml as usize <= READ_MBSLOP);
                line.set_len(at + ml as usize);
                pc = L_RECORD; /* goto record */
            } else if newloc >= startloc {
                if c == '\n' as c_int {
                    pc = L_RECORD; /* goto record */
                } else {
                    pc = L_PUT; /* goto put */
                }
            } else if rflag == 0 && c == '\\' as c_int {
                newloc = line.len() as c_int;
                pc = L_BODY;
                continue;
            } else if c == '\n' as c_int {
                break;
            } else {
                pc = L_PUT; /* fall through to put: */
            }
        }
        if pc == L_PUT {
            // put:
            if !libc::strchr(
                (core::ptr::addr_of!(crate::mystring::cqchars) as *const c_char).add(1),
                c,
            )
            .is_null()
            {
                /* USTPUTC(CTLESC, p) */
                line.push(crate::parser::CTLESC as u8);
            }
            /* USTPUTC(c, p) */
            line.push(c as u8);
            pc = L_RECORD;
        }
        if pc == L_RECORD {
            // record:
            if newloc >= startloc {
                crate::expand::recordregion(startloc, newloc, 0);
                pc = L_START;
            } else {
                pc = L_BODY; /* end of the for body */
                continue;
            }
        }
        if pc == L_START {
            // start:
            startloc = line.len() as c_int;
            newloc = startloc - 1;
            pc = L_BODY; /* end of the for body */
        }
    }
    crate::input::popfile();
    crate::expand::recordregion(startloc, line.len() as c_int, 0);
    /* `STACKSTRNUL(p)` writes the terminator without advancing, and the call
     * below then passes `p + 1` — the length *including* it.  Pushing is both
     * halves at once. */
    line.push(b'\0');
    readcmd_handle_line(&mut line, names);
    status
}

/*
 * umask builtin
 *
 * This code was ripped from pdksh 5.2.14 and hacked for use with
 * dash by Herbert Xu.
 *
 * Public domain.
 */

// [spec:dash:def:miscbltin.umaskcmd-fn]
// [spec:dash:sem:miscbltin.umaskcmd-fn]
pub unsafe fn umaskcmd(args: &[&BStr]) -> c_int {
    let mut ap: *mut c_char;
    let mut mask: c_int;
    let mut i: c_int;
    let mut symbolic_mode: c_int = 0;

    let mut opts = crate::options::Options::new(args);
    while opts.next(b"S").is_some() {
        symbolic_mode = 1;
    }
    /* The mode is walked as a cursor, so it stays a C string for the
     * length of the walk. */
    let mode = opts.operands().first().map(|w| crate::shell::cstring(w));

    INTOFF();
    mask = libc::umask(0) as c_int;
    libc::umask(mask as libc::mode_t);
    INTON();

    ap = mode
        .as_ref()
        .map_or(null_mut(), |mode| mode.as_ptr() as *mut c_char);
    if ap.is_null() {
        if symbolic_mode != 0 {
            let mut buf: [c_char; 18] = [0; 18];
            let mut j: c_int;

            mask = !mask;
            ap = buf.as_mut_ptr();
            i = 0;
            while i < 3 {
                *ap = b"ugo"[i as usize] as c_char;
                ap = ap.add(1);
                *ap = b'=' as c_char;
                ap = ap.add(1);
                j = 0;
                while j < 3 {
                    if (mask & (1 << (8 - (3 * i + j)))) != 0 {
                        *ap = b"rwx"[j as usize] as c_char;
                        ap = ap.add(1);
                    }
                    j += 1;
                }
                *ap = b',' as c_char;
                ap = ap.add(1);
                i += 1;
            }
            *ap.offset(-1) = b'\0' as c_char;
            let mut record = CStr::from_ptr(buf.as_ptr()).to_bytes().to_vec();
            record.push(b'\n');
            let _ = (&mut *crate::output::stdout()).write_all(&record);
        } else {
            let _ = writeln!(&mut *crate::output::stdout(), "{mask:04o}");
        }
    } else {
        let mut new_mask: c_int;

        if libc::isdigit(*ap as libc::c_uchar as c_int) != 0 {
            new_mask = 0;
            loop {
                if *ap >= b'8' as c_char || *ap < b'0' as c_char {
                    let mut message = b"Illegal number: ".to_vec();
                    message.extend_from_slice(mode.as_ref().expect("a mode to walk").as_bytes());
                    crate::error::sh_error(&message);
                }
                new_mask = (new_mask << 3) + (*ap as c_int - '0' as c_int);
                ap = ap.add(1);
                if *ap == b'\0' as c_char {
                    break;
                }
            }
        } else {
            let mut positions: c_int;
            let mut new_val: c_int;
            let mut op: c_char;

            mask = !mask;
            new_mask = mask;
            positions = 0;
            'sym: {
                'error_lbl: {
                    while *ap != 0 {
                        while *ap != 0
                            && !libc::strchr(b"augo\0".as_ptr() as *const c_char, *ap as c_int)
                                .is_null()
                        {
                            let ch = *ap;
                            ap = ap.add(1);
                            match ch as u8 {
                                b'a' => positions |= 0o111,
                                b'u' => positions |= 0o100,
                                b'g' => positions |= 0o010,
                                b'o' => positions |= 0o001,
                                _ => {}
                            }
                        }
                        if positions == 0 {
                            positions = 0o111; /* default is a */
                        }
                        op = *ap;
                        if op == 0 {
                            break 'error_lbl; // goto error
                        }
                        if libc::strchr(b"=+-\0".as_ptr() as *const c_char, op as c_int).is_null() {
                            break;
                        }
                        ap = ap.add(1);
                        new_val = 0;
                        while *ap != 0
                            && !libc::strchr(b"rwxugoXs\0".as_ptr() as *const c_char, *ap as c_int)
                                .is_null()
                        {
                            let ch = *ap;
                            ap = ap.add(1);
                            match ch as u8 {
                                b'r' => new_val |= 0o4,
                                b'w' => new_val |= 0o2,
                                b'x' => new_val |= 0o1,
                                b'u' => new_val |= mask >> 6,
                                b'g' => new_val |= mask >> 3,
                                b'o' => new_val |= mask >> 0,
                                b'X' => {
                                    if (mask & 0o111) != 0 {
                                        new_val |= 0o1;
                                    }
                                }
                                b's' => { /* ignored */ }
                                _ => {}
                            }
                        }
                        new_val = (new_val & 0o7) * positions;
                        match op as u8 {
                            b'-' => {
                                new_mask &= !new_val;
                            }
                            b'=' => {
                                new_mask = new_val | (new_mask & !(positions * 0o7));
                            }
                            b'+' => {
                                new_mask |= new_val;
                            }
                            _ => {}
                        }
                        if *ap == b',' as c_char {
                            positions = 0;
                            ap = ap.add(1);
                        } else if libc::strchr(b"=+-\0".as_ptr() as *const c_char, *ap as c_int)
                            .is_null()
                        {
                            break;
                        }
                    }
                    if *ap != 0 {
                        break 'error_lbl; // fall into error:
                    }
                    new_mask = !new_mask;
                    break 'sym;
                }
                // error:
                let mut message = b"Illegal mode: ".to_vec();
                message.extend_from_slice(CStr::from_ptr(*crate::options::argptr).to_bytes());
                crate::error::sh_error(&message);
                /* return 1; -- NOTREACHED, sh_error does not return */
            }
        }
        libc::umask(new_mask as libc::mode_t);
    }
    0
}

/*
 * ulimit builtin
 *
 * This code, originally by Doug Gwyn, Doug Kingston, Eric Gisin, and
 * Michael Rendell was ripped from pdksh 5.0.8 and hacked for use with
 * ash by J.T. Conklin.
 *
 * Public domain.
 */

// [spec:dash:def:miscbltin.limits]
#[repr(C)]
pub struct limits {
    pub name: *const c_char,
    pub cmd: c_int,
    pub factor: c_int, /* multiply by to get rlim_{cur,max} values */
    pub option: c_char,
}

unsafe impl Sync for limits {}

/* Each entry is `#ifdef RLIMIT_*`-guarded in the C; all of them exist
 * on Linux/glibc, so the table is complete here. */
static limits: [limits; 13] = [
    limits {
        name: b"time(seconds)\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_CPU as c_int,
        factor: 1,
        option: b't' as c_char,
    },
    limits {
        name: b"file(blocks)\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_FSIZE as c_int,
        factor: 512,
        option: b'f' as c_char,
    },
    limits {
        name: b"data(kbytes)\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_DATA as c_int,
        factor: 1024,
        option: b'd' as c_char,
    },
    limits {
        name: b"stack(kbytes)\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_STACK as c_int,
        factor: 1024,
        option: b's' as c_char,
    },
    limits {
        name: b"coredump(blocks)\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_CORE as c_int,
        factor: 512,
        option: b'c' as c_char,
    },
    limits {
        name: b"memory(kbytes)\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_RSS as c_int,
        factor: 1024,
        option: b'm' as c_char,
    },
    limits {
        name: b"locked memory(kbytes)\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_MEMLOCK as c_int,
        factor: 1024,
        option: b'l' as c_char,
    },
    limits {
        name: b"process\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_NPROC as c_int,
        factor: 1,
        option: b'p' as c_char,
    },
    limits {
        name: b"nofiles\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_NOFILE as c_int,
        factor: 1,
        option: b'n' as c_char,
    },
    limits {
        name: b"vmemory(kbytes)\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_AS as c_int,
        factor: 1024,
        option: b'v' as c_char,
    },
    limits {
        name: b"locks\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_LOCKS as c_int,
        factor: 1,
        option: b'w' as c_char,
    },
    limits {
        name: b"rtprio\0".as_ptr() as *const c_char,
        cmd: libc::RLIMIT_RTPRIO as c_int,
        factor: 1,
        option: b'r' as c_char,
    },
    limits {
        name: core::ptr::null(), /* (char *) 0 */
        cmd: 0,
        factor: 0,
        option: b'\0' as c_char,
    },
];

// [spec:dash:def:miscbltin.limtype]
//
// C: `enum limtype { SOFT = 0x1, HARD = 0x2 };`. The values are used as
// a bit mask (`how = SOFT | HARD`), which a Rust `enum` cannot express,
// so the enumeration is carried as an integer type plus constants.
pub type limtype = c_int;
pub const SOFT: limtype = 0x1;
pub const HARD: limtype = 0x2;

// [spec:dash:def:miscbltin.printlim-fn]
// [spec:dash:sem:miscbltin.printlim-fn]
unsafe fn printlim(how: limtype, limit: *const libc::rlimit, l: *const limits) {
    let mut val: libc::rlim_t;

    val = (*limit).rlim_max;
    if (how & SOFT) != 0 {
        val = (*limit).rlim_cur;
    }

    if val == libc::RLIM_INFINITY {
        let _ = writeln!(&mut *crate::output::stdout(), "unlimited");
    } else {
        val /= (*l).factor as libc::rlim_t;
        let signed = val as libc::intmax_t;
        let _ = writeln!(&mut *crate::output::stdout(), "{signed}");
    }
}

// [spec:dash:def:miscbltin.ulimitcmd-fn]
// [spec:dash:sem:miscbltin.ulimitcmd-fn]
pub unsafe fn ulimitcmd(args: &[&BStr]) -> c_int {
    let mut c: c_int;
    let mut val: libc::rlim_t = 0;
    let mut how: limtype = SOFT | HARD;
    let mut l: *const limits;
    let set: c_int;
    let mut all: c_int = 0;
    let mut optc: c_int;
    let mut what: c_int;
    let mut limit: libc::rlimit = core::mem::zeroed();

    what = 'f' as c_int;
    /* "HSa" plus one letter per resource the platform supports; each
     * letter is `#ifdef RLIMIT_*`-guarded in the C source. */
    let mut opts = crate::options::Options::new(args);
    loop {
        let Some(o) = opts.next(b"HSatfdscmlpnvwr") else {
            break;
        };
        optc = o as c_int;
        match o {
            b'H' => {
                how = HARD;
            }
            b'S' => {
                how = SOFT;
            }
            b'a' => {
                all = 1;
            }
            _ => {
                what = optc;
            }
        }
    }

    /* Unbounded search: nextopt has already rejected any letter that is
     * not in the option string, so a mismatch cannot occur. */
    l = limits.as_ptr();
    while (*l).option as c_int != what {
        l = l.add(1);
    }

    let operands = opts.operands();
    let limitarg = operands.first().map(|w| crate::shell::cstring(w));
    set = limitarg.is_some() as c_int;
    if let Some(limitarg) = &limitarg {
        let mut p: *mut c_char = limitarg.as_ptr() as *mut c_char;

        if all != 0 || operands.len() > 1 {
            crate::error::sh_error(b"too many arguments");
        }
        if libc::strcmp(p, b"unlimited\0".as_ptr() as *const c_char) == 0 {
            val = libc::RLIM_INFINITY;
        } else {
            val = 0 as libc::rlim_t;

            loop {
                c = *p as c_int;
                p = p.add(1);
                if !(c >= '0' as c_int && c <= '9' as c_int) {
                    break;
                }
                /* `rlim_t` is unsigned, so C's `val * 10` and `+ digit`
                 * wrap modulo 2**64 rather than trapping; the wrapping
                 * ops are the literal translation. */
                val = (val.wrapping_mul(10))
                    .wrapping_add((c - '0' as c_int) as libc::c_long as libc::rlim_t);
                /* `rlim_t` is unsigned, so this overflow guard can
                 * never fire. Reproduced as-is (bug-for-bug). */
                if val < (0 as libc::rlim_t) {
                    break;
                }
            }
            if c != 0 {
                crate::error::sh_error(b"bad number");
            }
            val = val.wrapping_mul((*l).factor as libc::rlim_t);
        }
    }
    if all != 0 {
        l = limits.as_ptr();
        while !(*l).name.is_null() {
            libc::getrlimit((*l).cmd as libc::__rlimit_resource_t, &mut limit);
            let name = CStr::from_ptr((*l).name).to_bytes();
            let mut record = Vec::with_capacity(name.len().max(20) + 1);
            record.extend_from_slice(name);
            if record.len() < 20 {
                record.resize(20, b' ');
            }
            record.push(b' ');
            let _ = (&mut *crate::output::stdout()).write_all(&record);
            printlim(how, &limit, l);
            l = l.add(1);
        }
        return 0;
    }

    libc::getrlimit((*l).cmd as libc::__rlimit_resource_t, &mut limit);
    if set != 0 {
        if (how & HARD) != 0 {
            limit.rlim_max = val;
        }
        if (how & SOFT) != 0 {
            limit.rlim_cur = val;
        }
        if libc::setrlimit((*l).cmd as libc::__rlimit_resource_t, &limit) < 0 {
            let mut message = b"error setting limit (".to_vec();
            message.extend_from_slice(
                CStr::from_ptr(libc::strerror(*libc::__errno_location())).to_bytes(),
            );
            message.push(b')');
            crate::error::sh_error(&message);
        }
    } else {
        printlim(how, &limit, l);
    }
    0
}
