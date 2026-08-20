//! Literal port of `src/mail.c` / `src/mail.h`.
//! Rules: `docs/spec/port/src/mail.md`.

use bstr::BStr;
use nsh_platform::ShellBytesExt as _;

use crate::variables::{mail_path_is_set, mail_path_value, mail_value};

/// What `$MAILPATH` checking remembers between prompts.
///
/// Another group §5 does not list. The times are per-shell by
/// construction -- they are compared against the mailboxes of *this*
/// shell's `$MAILPATH`, which another shell may have set differently.
// [spec:nsh:req:idiom.no-artificial-limits]
pub struct MailState {
    /* times of mailboxes */
    mailtime: Vec<i64>,
    /* Set if MAIL or MAILPATH is changed. */
    changed: bool,
}

impl MailState {
    pub(crate) const fn new() -> Self {
        MailState {
            mailtime: Vec::new(),
            changed: false,
        }
    }

    fn check(&mut self, mail_path: &BStr) -> Vec<Vec<u8>> {
        let mut notices = Vec::new();
        let mut component_count = 0;
        for (index, component) in mail_path.split(|&byte| byte == b':').enumerate() {
            component_count = index + 1;
            if self.mailtime.len() <= index {
                self.mailtime.resize(index + 1, 0);
            }
            let (path, message) = component
                .iter()
                .position(|&byte| byte == b'%')
                .map_or((component, None), |at| {
                    (&component[..at], Some(&component[at + 1..]))
                });
            if path.is_empty() {
                self.mailtime[index] = 0;
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
                if !self.changed && modified != self.mailtime[index] {
                    let mut notice =
                        message.map_or_else(|| b"you have mail".to_vec(), <[u8]>::to_vec);
                    notice.push(b'\n');
                    notices.push(notice);
                }
                self.mailtime[index] = modified;
            }
        }
        self.mailtime.truncate(component_count);
        self.changed = false;
        notices
    }
}

/*
 * Print appropriate message(s) if mail has arrived.  If changed is set,
 * then the value of MAIL has changed, so we just update the values.
 */

// [spec:dash:sem:mail.chkmail-fn]
pub fn check_mail(shell: &mut crate::context::Shell) -> Result<(), crate::error::Error> {
    let mail_path = if mail_path_is_set(shell) {
        mail_path_value(shell)
    } else {
        mail_value(shell)
    }
    .to_owned();
    let notices = shell.mail.check(BStr::new(mail_path.as_slice()));
    for notice in notices {
        shell.write_output(crate::output::OutputDestination::Stderr, &notice)?;
    }
    Ok(())
}

// [spec:dash:sem:mail.changemail-fn]
pub fn reset_mail_state(mail: &mut MailState, _value: &BStr) {
    mail.changed = true;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_mailbox_is_tracked() {
        let path = (0..12)
            .map(|index| format!("nsh-missing-mailbox-{index}"))
            .collect::<Vec<_>>()
            .join(":");
        let mut state = MailState::new();

        drop(state.check(BStr::new(path.as_bytes())));

        assert_eq!(state.mailtime.len(), 12);
    }
}
