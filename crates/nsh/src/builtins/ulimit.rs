//! `ulimit`.
//!
//! Port of `ulimitcmd` and its table from `src/miscbltin.c`.
//!
//! Like `umask` the state is the process's own, so the whole builtin is
//! here: the table naming each resource, the option letter that selects
//! it, and the factor its units are reported in.

use core::ptr::null_mut;
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io::Write as _;

use bstr::BStr;

use crate::options::Options;

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
        if limitarg.as_bytes() == b"unlimited" {
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
                CStr::from_ptr(libc::strerror(crate::system::errno())).to_bytes(),
            );
            message.push(b')');
            crate::error::sh_error(&message);
        }
    } else {
        printlim(how, &limit, l);
    }
    0
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The lookup that follows the option scan is unbounded -- it walks
    /// the table until the option letter matches, trusting that the scan
    /// rejected anything absent. That trust is only warranted while the
    /// option string and the table say the same thing, which is what this
    /// checks.
    #[test]
    fn every_option_letter_has_a_row() {
        const SCANNED: &[u8] = b"tfdscmlpnvwr";
        for &letter in SCANNED {
            assert!(
                limits.iter().any(|l| l.option as u8 == letter),
                "-{} is scanned for but names no resource",
                letter as char
            );
        }
    }

    /// And the other way, with the table's shape: every row but the last
    /// is selectable, and the last is the sentinel `-a` stops at. A row
    /// whose letter nothing scans for could never be reached at all.
    #[test]
    fn the_table_ends_at_its_sentinel() {
        const SCANNED: &[u8] = b"tfdscmlpnvwr";
        let (sentinel, rows) = limits.split_last().expect("the table is not empty");
        assert_eq!(sentinel.option, 0, "the last row is the sentinel");
        assert!(sentinel.name.is_null());
        for l in rows {
            assert!(
                SCANNED.contains(&(l.option as u8)),
                "a resource is selected by -{}, which is not scanned for",
                l.option as u8 as char
            );
        }
    }

    /// `-H` and `-S` choose which of the two limits to report, and are
    /// not resource letters -- so they must not collide with one.
    #[test]
    fn hard_and_soft_are_not_resources() {
        for l in limits.iter() {
            assert!(l.option as u8 != b'H' && l.option as u8 != b'S');
        }
    }

    /// Two rows answering to one letter would make the search's result
    /// depend on the table's order.
    #[test]
    fn no_letter_appears_twice() {
        let mut seen = Vec::new();
        for l in limits.iter().filter(|l| l.option != 0) {
            assert!(!seen.contains(&l.option), "-{} appears twice", l.option as u8 as char);
            seen.push(l.option);
        }
    }
}
