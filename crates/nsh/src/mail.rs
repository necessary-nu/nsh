//! Literal port of `src/mail.c` / `src/mail.h`.
//! Rules: `docs/spec/port/src/mail.md`.

use core::ffi::c_int;
use bstr::BStr;
use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;
use std::os::unix::fs::MetadataExt;

use crate::var::{mailval, mpathset, mpathval};

const MAXMBOXES: usize = 10;

/// What `$MAILPATH` checking remembers between prompts.
///
/// Another group §5 does not list. The times are per-shell by
/// construction -- they are compared against the mailboxes of *this*
/// shell's `$MAILPATH`, which another shell may have set differently.
pub struct MailState {
    /* times of mailboxes */
    mailtime: [i64; MAXMBOXES],
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
pub fn chkmail(sh: &mut crate::context::Shell) {
    let mail_path = if mpathset(sh) != 0 {
        mpathval(sh)
    } else {
        mailval(sh)
    };
    for (index, component) in mail_path
        .split(|&byte| byte == b':')
        .take(MAXMBOXES)
        .enumerate()
    {
        let (path, message) = component
            .iter()
            .position(|&byte| byte == b'%')
            .map_or((component, None), |at| (&component[..at], Some(&component[at + 1..])));
        if path.is_empty() {
            continue;
        }
        let modified = std::fs::metadata(OsStr::from_bytes(path))
            .map(|metadata| metadata.mtime())
            .unwrap_or(0);
        if modified == 0 {
            sh.mail.mailtime[index] = 0;
        } else {
            if sh.mail.changed == 0 && modified != sh.mail.mailtime[index] {
                let mut notice = message.map_or_else(
                    || b"you have mail".to_vec(),
                    <[u8]>::to_vec,
                );
                notice.push(b'\n');
                let _ = sh.io.stderr().write_all(&notice);
            }
            sh.mail.mailtime[index] = modified;
        }
    }
    sh.mail.changed = 0;
}

// [spec:dash:def:mail.changemail-fn]
// [spec:dash:sem:mail.changemail-fn]
pub fn changemail(sh: &mut crate::context::Shell, _value: &BStr) {
    sh.mail.changed += 1;
}
