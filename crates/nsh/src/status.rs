//! Exit status and signal values shared by the evaluator, builtins, traps,
//! jobs, and the public shell API.
//!
//! A raw number exists only while parsing a shell operand or crossing the
//! operating-system boundary. Once accepted, the value travels through the
//! core as one of these types.

use bstr::BStr;

/// A shell exit status: `$?`.
///
/// A `u8`, because that is the range `$?` has. `exit 300` leaves 44 — in
/// dash and in this port — and the type says so rather than leaving a
/// `i32` that can hold values the shell cannot produce.
// [spec:nsh:req:idiom.status-flow-signal]
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExitStatus(u8);

impl ExitStatus {
    /// Zero.
    pub const SUCCESS: ExitStatus = ExitStatus(0);

    /// A generic unsuccessful command status.
    pub const FAILURE: ExitStatus = ExitStatus(1);

    /// The conventional status for a shell-language error.
    pub const ERROR: ExitStatus = ExitStatus(2);

    /// The status a command that was not found produces.
    pub const NOT_FOUND: ExitStatus = ExitStatus(127);

    /// The status a command that could not be executed produces.
    pub const NOT_EXECUTABLE: ExitStatus = ExitStatus(126);

    /// The status of an unrecoverable error while reading shell commands.
    pub const UNRECOVERABLE_READ: ExitStatus = ExitStatus(128);

    /// Convert a shell-language status number to its eight-bit value.
    ///
    /// Truncating, and that is the shell's own arithmetic rather than a
    /// convenience: `exit 300` is 44 because the wait status carries eight
    /// bits. A negative status wraps the same way `exit -1` does.
    pub const fn from_code(status: i32) -> ExitStatus {
        ExitStatus(status as u8)
    }

    /// The status as a number.
    pub const fn code(self) -> u8 {
        self.0
    }

    /// Whether the status is zero.
    pub const fn success(self) -> bool {
        self.0 == 0
    }

    /// The signal a command died from, under the shell's `128 + n`
    /// convention.
    ///
    /// A command that merely *exited* 130 is indistinguishable from one
    /// killed by SIGINT, because in a shell it is: the convention is
    /// lossy in the language, not only in this type. `None` for a status
    /// below 129, and for one that no signal number reaches.
    pub fn signal(self) -> Option<Signal> {
        let signal_number = i32::from(self.0) - 128;
        if signal_number > 0 && (signal_number as usize) < crate::signal_names::SIGNAL_SLOT_COUNT {
            Signal::from_number(signal_number)
        } else {
            None
        }
    }
}

impl From<i32> for ExitStatus {
    fn from(status: i32) -> Self {
        Self::from_code(status)
    }
}

impl From<u8> for ExitStatus {
    fn from(status: u8) -> Self {
        Self(status)
    }
}

impl From<ExitStatus> for u8 {
    fn from(status: ExitStatus) -> u8 {
        status.0
    }
}

impl core::fmt::Display for ExitStatus {
    fn fmt(&self, formatter: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        self.code().fmt(formatter)
    }
}

/// A signal number.
///
/// A newtype over the number rather than an enum: signal numbers are
/// platform-dependent, the shell has to carry ones it does not know a
/// name for (`SIGRTMIN+n`, and the slots glibc reserves), and an enum
/// would need an `Other(i32)` arm anyway — at which point it is this with
/// more ceremony.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Signal(nsh_platform::Signal);

impl Signal {
    /// Validate a signal number parsed at a shell-language boundary.
    pub const fn from_number(number: i32) -> Option<Signal> {
        match nsh_platform::Signal::new(number) {
            Some(signal) => Some(Signal(signal)),
            None => None,
        }
    }

    /// The platform signal value.
    pub const fn platform(self) -> nsh_platform::Signal {
        self.0
    }

    /// The positive signal number used by the shell language.
    pub const fn number(self) -> i32 {
        self.0.number()
    }

    /// The name without the `SIG` prefix — `INT`, `TERM` — as `trap -l`
    /// prints it.
    ///
    /// `None` for a number outside the table. A slot absent from the native
    /// signal-name set answers with its own decimal digits rather than
    /// `None`, because that is what `kill -l` prints. Index 0 is `EXIT`, the
    /// pseudo-signal the exit trap uses.
    pub fn name(self) -> Option<&'static BStr> {
        let index = usize::try_from(self.number()).ok()?;
        if index >= crate::signal_names::SIGNAL_SLOT_COUNT {
            return None;
        }
        Some(BStr::new(
            crate::signal_names::SIGNAL_NAMES[index].to_bytes(),
        ))
    }

    /// The status a command killed by this signal produces: `128 + n`.
    pub fn as_status(self) -> ExitStatus {
        ExitStatus::from_code(self.number() + 128)
    }
}

impl From<nsh_platform::Signal> for Signal {
    fn from(signal: nsh_platform::Signal) -> Self {
        Self(signal)
    }
}

impl From<Signal> for nsh_platform::Signal {
    fn from(signal: Signal) -> Self {
        signal.platform()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `$?` is eight bits, and the type is what says so.
    #[test]
    fn a_status_is_eight_bits() {
        assert_eq!(ExitStatus::from_code(300).code(), 44);
        assert_eq!(ExitStatus::from_code(0), ExitStatus::SUCCESS);
        assert!(ExitStatus::SUCCESS.success());
        assert!(!ExitStatus::FAILURE.success());
    }

    /// The `128 + n` convention, and the ambiguity it carries.
    #[test]
    fn a_signal_and_the_status_it_makes_round_trip() {
        let int = Signal::from(nsh_platform::interrupt_signal());
        assert_eq!(int.as_status().code(), 130);
        assert_eq!(int.as_status().signal(), Some(int));

        /* Not a bug being pinned, a property: `exit 130` and "killed by
         * SIGINT" are the same status, and the shell language cannot tell
         * them apart either. */
        assert_eq!(ExitStatus::from_code(130).signal(), Some(int));

        assert_eq!(ExitStatus::SUCCESS.signal(), None);
        assert_eq!(ExitStatus::from_code(127).signal(), None);
    }

    /// The names are `signames.rs`'s, which is the generator's output, so
    /// this is checking the seam rather than the table.
    #[test]
    fn a_signal_names_itself_without_the_prefix() {
        assert_eq!(
            Signal::from(nsh_platform::interrupt_signal()).name(),
            Some(BStr::new("INT"))
        );
        assert_eq!(
            Signal::from(nsh_platform::kill_signal()).name(),
            Some(BStr::new("KILL"))
        );
        assert!(Signal::from_number(0).is_none());
        let outside = Signal::from_number(crate::signal_names::SIGNAL_SLOT_COUNT as i32).unwrap();
        assert_eq!(outside.name(), None);
        assert!(Signal::from_number(-1).is_none());
    }

    // [spec:nsh:req:idiom.status-flow-signal/test]
    #[test]
    fn core_carries_typed_status_flow_signals() {
        let done = crate::evaluation::Flow::Done((7).into());
        assert!(matches!(
            done,
            crate::evaluation::Flow::Done(status) if status == ExitStatus::from_code(7)
        ));
        let _: fn(&crate::context::Shell) -> ExitStatus = crate::context::Shell::status;
        let _: fn(&mut crate::context::Shell, Signal) = crate::trap::configure_signal;

        let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        for (source, fragments) in [
            ("context.rs", &["pub(crate) status: i32"][..]),
            (
                "evaluation.rs",
                &[
                    "Done(i32)",
                    "Exit { status: Option<i32>",
                    "back_exitstatus: i32",
                    "trap_default_exit_status: Option<i32>",
                ][..],
            ),
            (
                "signal_inbox.rs",
                &[
                    "fn pending_signal(&self) -> i32",
                    "fn signal_pending(&self, signo: i32)",
                ][..],
            ),
            (
                "trap.rs",
                &["fn setsignal(sh: &mut crate::context::Shell, signo: i32)"][..],
            ),
        ] {
            let text = std::fs::read_to_string(root.join(source)).unwrap();
            for fragment in fragments {
                assert!(
                    !text.contains(fragment),
                    "{source} exposes untyped domain value {fragment:?}"
                );
            }
        }
    }
}
