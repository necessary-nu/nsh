//! Literal port of `src/mail.c` / `src/mail.h`.
//! Rules: `docs/spec/port/src/mail.md`.

use core::ptr::{addr_of, addr_of_mut};
use libc::{c_char, c_int, time_t};
use std::ffi::CStr;
use std::io::Write;

use crate::mystring::nullstr;
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
pub unsafe fn chkmail(sh: &crate::context::Shell) {
    let mut mpath: *const c_char;
    let mut q: *mut c_char;
    let mut mtp: *mut time_t;
    let mut statb: libc::stat64 = core::mem::zeroed();

    /* `setstackmark`/`popstackmark` bounded the candidate paths
     * `padvance` built in the region; it builds them in its own buffer. */
    mpath = if mpathset(sh) != 0 {
        mpathval(sh)
    } else {
        mailval(sh)
    };
    mtp = addr_of_mut!(mailtime) as *mut time_t;
    while mtp < (addr_of_mut!(mailtime) as *mut time_t).add(MAXMBOXES) {
        let len: c_int;

        len = crate::exec::padvance_magic(&mut mpath, addr_of!(nullstr) as *const c_char, 2);
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
            std::process::abort();
        }
        *q.offset(-1) = b'\0' as c_char; /* delete trailing '/' */
        if libc::stat64(p_blk, &mut statb) < 0 {
            *mtp = 0;
            mtp = mtp.add(1);
            continue;
        }
        if changed == 0 && statb.st_mtime != *mtp {
            let text = if !crate::exec::pathopt.is_null() {
                crate::exec::pathopt
            } else {
                crate::shell::cstr(b"you have mail\0")
            };
            let mut message = CStr::from_ptr(text).to_bytes().to_vec();
            message.push(b'\n');
            let _ = (*crate::output::stderr()).write_all(&message);
        }
        *mtp = statb.st_mtime;
        mtp = mtp.add(1);
    }
    changed = 0;
}

// [spec:dash:def:mail.changemail-fn]
// [spec:dash:sem:mail.changemail-fn]
pub unsafe fn changemail(_sh: &mut crate::context::Shell, val: *const c_char) {
    changed += 1;
}
