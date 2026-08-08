//! Literal port of `src/mail.c` / `src/mail.h`.
//! Rules: `docs/spec/port/src/mail.md`.

use libc::{c_char, c_int, time_t};
use core::ptr::{addr_of, addr_of_mut};

use crate::memalloc::{popstackmark, setstackmark, stackmark};
use crate::mystring::nullstr;
use crate::output::VaArg;
use crate::var::{mailval, mpathset, mpathval};

const MAXMBOXES: usize = 10;

/* times of mailboxes */
static mut mailtime: [time_t; MAXMBOXES] = [0; MAXMBOXES];
/* Set if MAIL or MAILPATH is changed. */
static mut changed: c_int = 0;

/*
 * Print appropriate message(s) if mail has arrived.  If changed is set,
 * then the value of MAIL has changed, so we just update the values.
 */

// [spec:dash:def:mail.chkmail-fn]
// [spec:dash:sem:mail.chkmail-fn]
pub unsafe fn chkmail() {
    let mut mpath: *const c_char;
    let mut q: *mut c_char;
    let mut mtp: *mut time_t;
    let mut smark: stackmark = core::mem::zeroed();
    let mut statb: libc::stat64 = core::mem::zeroed();

    setstackmark(&mut smark);
    mpath = if mpathset() != 0 { mpathval() } else { mailval() };
    mtp = addr_of_mut!(mailtime) as *mut time_t;
    while mtp < (addr_of_mut!(mailtime) as *mut time_t).add(MAXMBOXES) {
        let len: c_int;

        len = crate::exec::padvance_magic(
            &mut mpath,
            addr_of!(nullstr) as *const c_char,
            2,
        );
        if len < 0 {
            break;
        }
        let p_blk = crate::exec::padvance_result();
        if *p_blk == b'\0' as c_char {
            mtp = mtp.add(1);
            continue;
        }
        q = p_blk;
        while *q != 0 {
            q = q.add(1);
        }
        if crate::shell::DEBUG && *q.offset(-1) != b'/' as c_char {
            libc::abort();
        }
        *q.offset(-1) = b'\0' as c_char; /* delete trailing '/' */
        if libc::stat64(p_blk, &mut statb) < 0 {
            *mtp = 0;
            mtp = mtp.add(1);
            continue;
        }
        if changed == 0 && statb.st_mtime != *mtp {
            crate::output::outfmt(
                addr_of_mut!(crate::output::errout),
                addr_of!(crate::mystring::snlfmt) as *const c_char,
                &[VaArg::Str(if !crate::exec::pathopt.is_null() {
                    crate::exec::pathopt
                } else {
                    crate::shell::cstr(b"you have mail\0")
                })],
            );
        }
        *mtp = statb.st_mtime;
        mtp = mtp.add(1);
    }
    changed = 0;
    popstackmark(&mut smark);
}

// [spec:dash:def:mail.changemail-fn]
// [spec:dash:sem:mail.changemail-fn]
pub unsafe fn changemail(val: *const c_char) {
    changed += 1;
}
