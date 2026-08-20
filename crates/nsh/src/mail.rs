//! Literal port of `src/mail.c` / `src/mail.h`.
//! Rules: `docs/spec/port/src/mail.md`.

use bstr::BStr;
use core::ffi::c_int;
use nsh_platform::ShellBytesExt as _;
use std::io::Write;

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

    fn check(&mut self, mail_path: &BStr, errors: &mut crate::output::Output) {
        for (index, component) in mail_path
            .split(|&byte| byte == b':')
            .take(MAXMBOXES)
            .enumerate()
        {
            let (path, message) = component
                .iter()
                .position(|&byte| byte == b'%')
                .map_or((component, None), |at| {
                    (&component[..at], Some(&component[at + 1..]))
                });
            if path.is_empty() {
                continue;
            }
            let modified = path
                .try_to_path_buf()
                .and_then(|path| nsh_platform::path_metadata(&path, true))
                .map(|metadata| metadata.modified_seconds)
                .unwrap_or(0);
            if modified == 0 {
                self.mailtime[index] = 0;
            } else {
                if self.changed == 0 && modified != self.mailtime[index] {
                    let mut notice =
                        message.map_or_else(|| b"you have mail".to_vec(), <[u8]>::to_vec);
                    notice.push(b'\n');
                    let _ = errors.write_all(&notice);
                }
                self.mailtime[index] = modified;
            }
        }
        self.changed = 0;
    }
}

/*
 * Print appropriate message(s) if mail has arrived.  If changed is set,
 * then the value of MAIL has changed, so we just update the values.
 */

// [spec:dash:def:mail.chkmail-fn]
// [spec:dash:sem:mail.chkmail-fn]
pub fn chkmail(sh: &mut crate::context::Shell) {
    let mail_path = if mpathset(sh) {
        mpathval(sh)
    } else {
        mailval(sh)
    }
    .to_owned();
    sh.mail
        .check(BStr::new(mail_path.as_slice()), sh.io.stderr());
}

// [spec:dash:def:mail.changemail-fn]
// [spec:dash:sem:mail.changemail-fn]
pub fn changemail(mail: &mut MailState, _value: &BStr) {
    mail.changed += 1;
}
