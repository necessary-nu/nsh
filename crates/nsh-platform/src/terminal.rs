//! Safe terminal-settings snapshots.

use std::os::fd::AsFd;

/// An opaque snapshot of every setting associated with a terminal.
///
/// Keeping the host representation private lets the shell retain and restore
/// terminal state without depending on raw descriptors or termios details.
pub struct TerminalSettings(rustix::termios::Termios);

impl TerminalSettings {
    /// Capture the terminal settings currently associated with `fd`.
    pub fn capture(fd: impl AsFd) -> std::io::Result<Self> {
        let fd = fd.as_fd();
        loop {
            match rustix::termios::tcgetattr(fd) {
                Ok(attributes) => return Ok(Self(attributes)),
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(std::io::Error::from(error)),
            }
        }
    }

    /// Apply this snapshot to `fd` immediately.
    pub fn apply(&self, fd: impl AsFd) -> std::io::Result<()> {
        let fd = fd.as_fd();
        loop {
            match rustix::termios::tcsetattr(fd, rustix::termios::OptionalActions::Now, &self.0) {
                Ok(()) => return Ok(()),
                Err(rustix::io::Errno::INTR) => {}
                Err(error) => return Err(std::io::Error::from(error)),
            }
        }
    }
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
