//! `umask`.
//!
//! Port of `umaskcmd` from `src/miscbltin.c`.
//!
//! "This code was ripped from pdksh 5.2.14 and hacked for use with dash
//! by Herbert Xu. Public domain."
//!
//! The mask is the process's, not the shell's: there is nothing to keep
//! here, so the builtin reads it from the kernel and writes it back.

use crate::context::Shell;
use crate::error::Error;
use core::ptr::null_mut;
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io::Write as _;

use bstr::BStr;

use crate::error::{INTOFF, INTON};
use crate::eval::Flow;
use crate::options::Options;

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
pub unsafe fn umaskcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut ap: *mut c_char;
    let mut mask: c_int;
    let mut i: c_int;
    let mut symbolic_mode: c_int = 0;

    let mut opts = crate::options::Options::new(args);
    while opts.next(sh, b"S")?.is_some() {
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
                    return Err(sh.sh_error_value(&message));
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
                        while *ap != 0 && b"augo".contains(&(*ap as u8)) {
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
                        if !b"=+-".contains(&(op as u8)) {
                            break;
                        }
                        ap = ap.add(1);
                        new_val = 0;
                        while *ap != 0 && b"rwxugoXs".contains(&(*ap as u8)) {
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
                        /* The terminator stays in the set here, and only
                         * here: the three scans above run under `*ap != 0`,
                         * but this one can see the end of the mode, where
                         * `strchr` matches the NUL and the C falls out
                         * through the loop condition rather than through
                         * this break. */
                        } else if !b"=+-\0".contains(&(*ap as u8)) {
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
                message.extend_from_slice(mode.as_ref().expect("a mode to walk").as_bytes());
                /* The C's `return 1` after this is unreachable because its
                 * `sh_error` longjmps; the error is the return now. */
                return Err(sh.sh_error_value(&message));
            }
        }
        libc::umask(new_mask as libc::mode_t);
    }
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testutil::lock;

    /// The umask is the process's, so a test has to put back what it
    /// found -- and reading it is itself a write, which is why the
    /// builtin sets zero and then restores.
    fn with_mask<T>(body: impl FnOnce() -> T) -> T {
        let _guard = lock();
        let saved = unsafe { libc::umask(0) };
        unsafe { libc::umask(saved) };
        let out = body();
        unsafe { libc::umask(saved) };
        out
    }

    fn set(mode: &[u8]) -> libc::mode_t {
        unsafe {
            let sh = &mut Shell::new();
            assert_eq!(
                umaskcmd(sh, &[BStr::new("umask"), BStr::new(mode)]).unwrap(),
                Flow::Done(0)
            );
            let now = libc::umask(0);
            libc::umask(now);
            now
        }
    }

    #[test]
    fn an_octal_operand_sets_the_mask() {
        with_mask(|| {
            assert_eq!(set(b"027"), 0o027);
            assert_eq!(set(b"0"), 0);
            /* A leading zero is not special: the number is octal either way. */
            assert_eq!(set(b"0022"), 0o022);
        });
    }

    /// The symbolic form says which bits to *allow*, so the mask it
    /// produces is their complement.
    #[test]
    fn a_symbolic_operand_is_complemented() {
        with_mask(|| {
            assert_eq!(set(b"a=rx"), 0o222);
            assert_eq!(set(b"a="), 0o777);
            assert_eq!(set(b"a=rwx"), 0);
        });
    }

    /// `+` and `-` adjust what the current mask allows rather than
    /// replacing it.
    #[test]
    fn plus_and_minus_adjust() {
        with_mask(|| {
            set(b"a=rwx");
            assert_eq!(set(b"go-w"), 0o022);
            assert_eq!(set(b"go+w"), 0);
        });
    }

    #[test]
    fn a_bad_operand_raises() {
        /* Returned rather than raised, per [dec:nsh:errors-are-values];
         * the text and the status are dash's, unchanged. */
        let _guard = lock();
        for (mode, text) in [
            ("999", &b"Illegal number: 999"[..]),
            ("q=r", &b"Illegal mode: q=r"[..]),
        ] {
            let e = unsafe { umaskcmd(&mut Shell::new(), &[BStr::new("umask"), BStr::new(mode)]) }
                .expect_err("a bad mode fails");
            assert_eq!(e.message().to_vec(), text.to_vec());
            assert_eq!(e.status(), 2);
        }
    }
}
