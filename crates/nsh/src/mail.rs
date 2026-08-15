//! Literal port of `src/mail.c` / `src/mail.h`.
//! Rules: `docs/spec/port/src/mail.md`.

use core::ptr::{addr_of, addr_of_mut};
use libc::{c_char, c_int, time_t};
use std::ffi::CStr;
use std::io::Write;

use crate::mystring::nullstr;
use crate::var::{mailval, mpathset, mpathval};

const MAXMBOXES: usize = 10;

/// What `$MAILPATH` checking remembers between prompts.
///
/// Another group §5 does not list. The times are per-shell by
/// construction -- they are compared against the mailboxes of *this*
/// shell's `$MAILPATH`, which another shell may have set differently.
pub struct MailState {
    /* times of mailboxes */
    mailtime: [time_t; MAXMBOXES],
    /* Set if MAIL or MAILPATH is changed. */
    changed: c_int,
}

impl MailState {
    pub(crate) const fn new() -> Self {
        MailState {
            mailtime: [0; MAXMBOXES],
            changed: 0,
        }
    }
}

/*
 * Print appropriate message(s) if mail has arrived.  If changed is set,
 * then the value of MAIL has changed, so we just update the values.
 */

// [spec:dash:def:mail.chkmail-fn]
// [spec:dash:sem:mail.chkmail-fn]
pub unsafe fn chkmail(sh: &mut crate::context::Shell) {
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
    mtp = addr_of_mut!(sh.mail.mailtime) as *mut time_t;
    while mtp < (addr_of_mut!(sh.mail.mailtime) as *mut time_t).add(MAXMBOXES) {
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
        if sh.mail.changed == 0 && statb.st_mtime != *mtp {
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
    sh.mail.changed = 0;
}

// [spec:dash:def:mail.changemail-fn]
// [spec:dash:sem:mail.changemail-fn]
pub unsafe fn changemail(sh: &mut crate::context::Shell, val: *const c_char) {
    sh.mail.changed += 1;
}
