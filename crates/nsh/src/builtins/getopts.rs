//! `getopts`.
//!
//! Port of `getoptscmd` and its engine from `src/options.c`.
//!
//! This is the option scanner a *script* runs, which makes it the one
//! that cannot be `crate::options::Options`: it has to survive between
//! invocations, because the loop calling it is in the script. Its
//! position lives in `shellparam` as `OPTIND` plus a byte offset, and it
//! reports through the shell variables `OPTIND` and `OPTARG` rather than
//! by returning.
//!
//! It walks a `char **` for the same reason: the array it is given is
//! either the positional parameters or its own operands, and the offset
//! it remembers has to mean the same thing next time.

use bstr::BStr;
use core::ptr::{addr_of, null_mut};
use libc::{c_char, c_int, c_uint, size_t};
use std::ffi::CString;
use std::io::Write;

use crate::mystring::nullstr;
use crate::options::{Options, shellparam, shellparam_p};
use crate::var::{VNOFUNC, setvar, setvarint, unsetvar};

// [spec:dash:def:options.getoptscmd-fn]
// [spec:dash:sem:options.getoptscmd-fn]
pub unsafe fn getoptscmd(args: &[&BStr]) -> c_int {
    let optbase: *mut *mut c_char;

    let mut opts = Options::new(args);
    opts.next(b"");
    let operands = opts.operands();
    if operands.len() < 2 {
        crate::error::sh_error(b"Usage: getopts optstring var [arg...]");
    }
    let optstr = crate::shell::cstring(operands[0]);
    let optvar = crate::shell::cstring(operands[1]);

    /* `getopts` walks a `char **` and remembers where it got to as an
     * index and a byte offset, so the words it is given have to be an
     * array for the length of the call. The positional parameters
     * already are one; explicit operands are built into one here, which
     * is what `evalcommand` was doing for every builtin. */
    let explicit: Vec<CString>;
    let mut ptrs: Vec<*mut c_char>;
    if operands.len() == 2 {
        optbase = shellparam_p();
        if (shellparam.optind as c_uint) > (shellparam.nparam + 1) as c_uint {
            shellparam.optind = 1;
            shellparam.optoff = -1;
        }
    } else {
        explicit = operands[2..]
            .iter()
            .map(|w| crate::shell::cstring(w))
            .collect();
        ptrs = explicit.iter().map(|w| w.as_ptr() as *mut c_char).collect();
        ptrs.push(null_mut());
        optbase = ptrs.as_mut_ptr();
        if (shellparam.optind as c_uint) > (operands.len() - 1) as c_uint {
            shellparam.optind = 1;
            shellparam.optoff = -1;
        }
    }

    getopts(
        optstr.as_ptr() as *mut c_char,
        optvar.as_ptr() as *mut c_char,
        optbase,
    )
}

// [spec:dash:def:options.getopts-fn]
// [spec:dash:sem:options.getopts-fn]
unsafe fn getopts(optstr: *mut c_char, optvar: *mut c_char, optfirst: *mut *mut c_char) -> c_int {
    let mut p: *mut c_char;
    let mut q: *mut c_char;
    let mut c: c_char = b'?' as c_char;
    let mut done: c_int = 0;
    let mut s: [c_char; 2] = [0; 2];
    let mut optnext: *mut *mut c_char;
    let mut ind: c_int = shellparam.optind;
    let off: c_int = shellparam.optoff;

    shellparam.optind = -1;
    optnext = optfirst.offset(ind as isize - 1);

    if ind <= 1 || off < 0 || (libc::strlen(*optnext.offset(-1)) as size_t) < off as size_t {
        p = null_mut();
    } else {
        p = (*optnext.offset(-1)).offset(off as isize);
    }
    'out: loop {
        if p.is_null() || *p == b'\0' as c_char {
            /* Current word is done, advance */
            p = *optnext;
            if p.is_null() || *p != b'-' as c_char || {
                p = p.add(1);
                *p == b'\0' as c_char
            } {
                /* atend: */
                p = null_mut();
                done = 1;
                break 'out;
            }
            optnext = optnext.add(1);
            if *p.offset(0) == b'-' as c_char && *p.offset(1) == b'\0' as c_char {
                /* check for "--" — goto atend */
                p = null_mut();
                done = 1;
                break 'out;
            }
        }

        c = *p;
        p = p.add(1);
        q = if *optstr.offset(0) == b':' as c_char {
            optstr.offset(1)
        } else {
            optstr
        };
        while *q != c {
            if *q == b'\0' as c_char {
                if *optstr.offset(0) == b':' as c_char {
                    s[0] = c;
                    s[1] = b'\0' as c_char;
                    setvar(b"OPTARG\0".as_ptr() as *const c_char, s.as_ptr(), 0);
                } else {
                    let mut message = b"Illegal option -".to_vec();
                    message.push(c as u8);
                    message.push(b'\n');
                    let _ = (*crate::output::stderr()).write_all(&message);
                    crate::var::unsetvar(b"OPTARG\0".as_ptr() as *const c_char);
                }
                c = b'?' as c_char;
                break 'out;
            }
            q = q.add(1);
            if *q == b':' as c_char {
                q = q.add(1);
            }
        }

        q = q.add(1);
        if *q == b':' as c_char {
            if *p == b'\0' as c_char && {
                p = *optnext;
                p.is_null()
            } {
                if *optstr.offset(0) == b':' as c_char {
                    s[0] = c;
                    s[1] = b'\0' as c_char;
                    setvar(b"OPTARG\0".as_ptr() as *const c_char, s.as_ptr(), 0);
                    c = b':' as c_char;
                } else {
                    let mut message = b"No arg for -".to_vec();
                    message.push(c as u8);
                    message.extend_from_slice(b" option\n");
                    let _ = (*crate::output::stderr()).write_all(&message);
                    crate::var::unsetvar(b"OPTARG\0".as_ptr() as *const c_char);
                    c = b'?' as c_char;
                }
                break 'out;
            }

            if p == *optnext {
                optnext = optnext.add(1);
            }
            setvar(b"OPTARG\0".as_ptr() as *const c_char, p, 0);
            p = null_mut();
        } else {
            setvar(
                b"OPTARG\0".as_ptr() as *const c_char,
                addr_of!(nullstr) as *const c_char,
                0,
            );
        }
        break 'out;
    }

    /* out: */
    ind = ((optnext as isize - optfirst as isize) / core::mem::size_of::<*mut c_char>() as isize)
        as c_int
        + 1;
    setvarint(
        b"OPTIND\0".as_ptr() as *const c_char,
        ind as libc::intmax_t,
        VNOFUNC,
    );
    s[0] = c;
    s[1] = b'\0' as c_char;
    setvar(optvar, s.as_ptr(), 0);

    shellparam.optoff = if !p.is_null() {
        (p as isize - *optnext.offset(-1) as isize) as c_int
    } else {
        -1
    };
    shellparam.optind = ind;

    done
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testutil::{CStr0, lock};
    use crate::var::lookupvar;

    unsafe fn value(name: &str) -> String {
        let p = lookupvar(CStr0::new(name).p());
        if p.is_null() {
            return String::new();
        }
        String::from_utf8_lossy(std::ffi::CStr::from_ptr(p).to_bytes()).into_owned()
    }

    /// A script's scan has to survive between invocations, so what it
    /// reports is `OPTIND` and `OPTARG` rather than a return value; the
    /// loop below is the shape every `while getopts` has.
    #[test]
    fn a_scan_runs_across_invocations() {
        let _g = lock();
        unsafe {
            shellparam.optind = 1;
            shellparam.optoff = -1;

            let words = ["getopts", "ab:", "o", "-a", "-bVAL", "rest"];
            let args: Vec<&BStr> = words.iter().map(|w| BStr::new(*w)).collect();

            assert_eq!(getoptscmd(&args), 0);
            assert_eq!(value("o"), "a");

            assert_eq!(getoptscmd(&args), 0);
            assert_eq!(value("o"), "b");
            assert_eq!(value("OPTARG"), "VAL");

            /* The operand ends the scan, and OPTIND points at it. */
            assert_ne!(getoptscmd(&args), 0);
            assert_eq!(value("OPTIND"), "3");
        }
    }

    /// A leading `:` in the option string is the quiet form: an unknown
    /// option is reported through `OPTARG` and the variable instead of on
    /// stderr.
    #[test]
    fn a_leading_colon_reports_quietly() {
        let _g = lock();
        unsafe {
            shellparam.optind = 1;
            shellparam.optoff = -1;

            let words = ["getopts", ":a", "o", "-z"];
            let args: Vec<&BStr> = words.iter().map(|w| BStr::new(*w)).collect();

            assert_eq!(getoptscmd(&args), 0);
            assert_eq!(value("o"), "?");
            assert_eq!(value("OPTARG"), "z");
        }
    }
}
