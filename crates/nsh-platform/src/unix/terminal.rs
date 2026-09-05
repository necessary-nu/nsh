//! Terminal-settings snapshots, and the two questions asked of a
//! descriptor before one is taken.
//!
//! `is_terminal` and `terminal_canonical_mode` are the same `tcgetattr`
//! call the snapshot makes, asked for an answer rather than for the
//! settings: whether there is a terminal there at all, and whether it is
//! in the line-buffered mode that decides how the shell reads from it.

use crate::AsDescriptor;

/// An opaque snapshot of every setting associated with a terminal.
///
/// Keeping the host representation private lets the shell retain and restore
/// terminal state without depending on raw descriptors or termios details.
pub struct TerminalSettings(rustix::termios::Termios);

impl TerminalSettings {
    /// Capture the terminal settings currently associated with `fd`.
    pub fn capture(fd: &impl AsDescriptor) -> std::io::Result<Self> {
        let fd = fd.as_platform_descriptor().0;
        loop {
            match rustix::termios::tcgetattr(fd) {
                Ok(attributes) => return Ok(Self(attributes)),
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(std::io::Error::from(error)),
            }
        }
    }

    /// The same settings with terminal echo turned off, for `read -s`.
    ///
    /// A copy rather than a mutation, so the caller still holds the
    /// snapshot it has to put back.
    #[must_use]
    pub fn without_echo(&self) -> Self {
        let mut quiet = self.0.clone();
        quiet.local_modes.remove(rustix::termios::LocalModes::ECHO);
        Self(quiet)
    }

    /// The same settings out of canonical input mode, for `read -n`.
    ///
    /// A terminal in canonical mode holds every character until the line
    /// is complete, and the wait belongs to the kernel rather than to
    /// the reader -- so a `read` bounded by a character count cannot get
    /// its first character by reading harder. `VMIN` of one with no
    /// timer is the mode that hands over as soon as a character is
    /// there. Echo is left alone: `-s` is a separate request, and a
    /// terminal that stops showing what is typed for every `read -n1` is
    /// not what the reference does.
    #[must_use]
    pub fn without_canonical_input(&self) -> Self {
        let mut immediate = self.0.clone();
        immediate
            .local_modes
            .remove(rustix::termios::LocalModes::ICANON);
        immediate.special_codes[rustix::termios::SpecialCodeIndex::VMIN] = 1;
        immediate.special_codes[rustix::termios::SpecialCodeIndex::VTIME] = 0;
        Self(immediate)
    }

    /// Apply this snapshot to `fd` immediately.
    pub fn apply(&self, fd: &impl AsDescriptor) -> std::io::Result<()> {
        let fd = fd.as_platform_descriptor().0;
        loop {
            match rustix::termios::tcsetattr(fd, rustix::termios::OptionalActions::Now, &self.0) {
                Ok(()) => return Ok(()),
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(std::io::Error::from(error)),
            }
        }
    }
}

/// Whether a terminal descriptor is in canonical input mode. `None` means
/// the descriptor is not a terminal (or its attributes cannot be queried).
pub fn terminal_canonical_mode(fd: &impl AsDescriptor) -> Option<bool> {
    let attributes = rustix::termios::tcgetattr(fd.as_platform_descriptor().0).ok()?;
    Some(
        attributes
            .local_modes
            .contains(rustix::termios::LocalModes::ICANON),
    )
}

/// Whether an endpoint is attached to a terminal.
pub fn is_terminal(fd: &impl AsDescriptor) -> bool {
    rustix::termios::tcgetattr(fd.as_platform_descriptor().0).is_ok()
}

/// How many columns wide the terminal behind `fd` is right now.
///
/// `None` where there is no terminal, and also where the terminal
/// reports a width of zero -- which a pseudo-terminal nobody has sized
/// does, and which is not a width. The caller cannot tell the two apart
/// and has no use for the distinction: neither is a number to publish.
pub fn terminal_width(fd: &impl AsDescriptor) -> Option<usize> {
    let size = rustix::termios::tcgetwinsize(fd.as_platform_descriptor().0).ok()?;
    (size.ws_col > 0).then(|| usize::from(size.ws_col))
}

#[cfg(test)]
mod tests {
    use super::*;
    use rustix::pty::OpenptFlags;
    use rustix::termios::LocalModes;
    use std::ffi::OsStr;
    use std::os::unix::ffi::OsStrExt as _;

    #[test]
    fn terminal_settings_restore_a_snapshot() {
        let controller = rustix::pty::openpt(OpenptFlags::RDWR | OpenptFlags::NOCTTY).unwrap();
        rustix::pty::grantpt(&controller).unwrap();
        rustix::pty::unlockpt(&controller).unwrap();
        let slave_name = rustix::pty::ptsname(&controller, Vec::new()).unwrap();
        let slave = std::fs::OpenOptions::new()
            .read(true)
            .write(true)
            .open(OsStr::from_bytes(slave_name.to_bytes()))
            .unwrap();

        let snapshot = TerminalSettings::capture(&slave).unwrap();
        let original_echo = snapshot.0.local_modes.contains(LocalModes::ECHO);
        let mut changed = snapshot.0.clone();
        changed.local_modes.toggle(LocalModes::ECHO);
        rustix::termios::tcsetattr(&slave, rustix::termios::OptionalActions::Now, &changed)
            .unwrap();

        snapshot.apply(&slave).unwrap();
        let restored = rustix::termios::tcgetattr(&slave).unwrap();
        assert_eq!(
            restored.local_modes.contains(LocalModes::ECHO),
            original_echo
        );
    }
}
