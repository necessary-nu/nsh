//! The two value types an embedder reads results through.
//!
//! `docs/api-design.md` §2 lists them together — "`u8` and a numeric
//! newtype" — and `[dec:nsh:public-surface]` carries both on its surface
//! list. They are here rather than in the module that produces them
//! because neither belongs to one: a status comes out of the evaluator,
//! a builtin, a reaped child or a trap, and a signal number arrives from
//! the inbox, a `kill` operand or a job's death.
//!
//! ## Why these are not the evaluator's currency
//!
//! `[dec:nsh:public-surface]`'s `(+1)` amendment draws the line and it is
//! worth restating where the types live rather than only where the
//! decision does:
//!
//! * **[`ExitStatus`] is the surface.** `Shell::run` returns one,
//!   `Shell::status` answers one, and the evaluator's `Flow::Exit` maps
//!   onto one at the API boundary.
//! * **`Flow` is the inside.** It is `pub(crate)`, it carries what
//!   `ExitStatus` deliberately cannot — that the shell is *stopping*, and
//!   why — and it stays the builtin table's return type because `exit` is
//!   a builtin and `[dec:nsh:errors-are-values]` step E put control flow
//!   in the `Ok` position on purpose.
//!
//! Collapsing `Flow` onto `ExitStatus` is correct in exactly one place,
//! the boundary, because an embedder asking *what happened* wants the
//! status and not the evaluator's reason for stopping. `Shell::has_exited`
//! carries the part that does not survive the collapse.
//!
//! Neither type is wired through the evaluator, and that is deliberate:
//! `docs/api-design.md` §3.5 keeps the status a `c_int` field precisely so
//! that turning it into a return value is not a second refactor riding on
//! the first. These are the types the surface is built from, introduced
//! ahead of the surface that uses them.

use bstr::BStr;
use core::ffi::c_int;

/// A shell exit status: `$?`.
///
/// A `u8`, because that is the range `$?` has. `exit 300` leaves 44 — in
/// dash and in this port — and the type says so rather than leaving a
/// `c_int` that can hold values the shell cannot produce.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct ExitStatus(u8);

impl ExitStatus {
    /// Zero.
    pub const SUCCESS: ExitStatus = ExitStatus(0);

    /// The status a command that was not found produces.
    pub const NOT_FOUND: ExitStatus = ExitStatus(127);

    /// The status a command that could not be executed produces.
    pub const NOT_EXECUTABLE: ExitStatus = ExitStatus(126);

    /// The status of an unrecoverable error while reading shell commands.
    pub const UNRECOVERABLE_READ: ExitStatus = ExitStatus(128);

    /// From a raw status.
    ///
    /// Truncating, and that is the shell's own arithmetic rather than a
    /// convenience: `exit 300` is 44 because the wait status carries eight
    /// bits. A negative status wraps the same way `exit -1` does.
    pub fn from_raw(status: c_int) -> ExitStatus {
        ExitStatus(status as u8)
    }

    /// The status as a number.
    pub fn code(self) -> u8 {
        self.0
    }

    /// Whether the status is zero.
    pub fn success(self) -> bool {
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
        let n = c_int::from(self.0) - 128;
        if n > 0 && (n as usize) < crate::signames::NSIG {
            Some(Signal(n))
        } else {
            None
        }
    }
}

impl From<ExitStatus> for u8 {
    fn from(status: ExitStatus) -> u8 {
        status.0
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
pub struct Signal(c_int);

impl Signal {
    /// From a raw number.
    pub fn from_raw(number: c_int) -> Signal {
        Signal(number)
    }

    /// The raw number.
    pub fn number(self) -> c_int {
        self.0
    }

    /// The name without the `SIG` prefix — `INT`, `TERM` — as `trap -l`
    /// prints it.
    ///
    /// `None` for a number outside the table. Note that a slot no
    /// `#if defined(SIG…)` claimed answers with its own decimal digits
    /// rather than `None`, because that is what `kill -l` prints and this
    /// reads the same table: `signames.rs` is the generator's output and
    /// index 0 is `EXIT`, the pseudo-signal the exit trap uses.
    pub fn name(self) -> Option<&'static BStr> {
        let index = usize::try_from(self.0).ok()?;
        if index >= crate::signames::NSIG {
            return None;
        }
        Some(BStr::new(crate::signames::signal_names[index].to_bytes()))
    }

    /// The status a command killed by this signal produces: `128 + n`.
    pub fn as_status(self) -> ExitStatus {
        ExitStatus::from_raw(self.0 + 128)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `$?` is eight bits, and the type is what says so.
    #[test]
    fn a_status_is_eight_bits() {
        assert_eq!(ExitStatus::from_raw(300).code(), 44);
        assert_eq!(ExitStatus::from_raw(0), ExitStatus::SUCCESS);
        assert!(ExitStatus::SUCCESS.success());
        assert!(!ExitStatus::from_raw(1).success());
    }

    /// The `128 + n` convention, and the ambiguity it carries.
    #[test]
    fn a_signal_and_the_status_it_makes_round_trip() {
        let int = Signal::from_raw(nsh_platform::interrupt_signal());
        assert_eq!(int.as_status().code(), 130);
        assert_eq!(int.as_status().signal(), Some(int));

        /* Not a bug being pinned, a property: `exit 130` and "killed by
         * SIGINT" are the same status, and the shell language cannot tell
         * them apart either. */
        assert_eq!(ExitStatus::from_raw(130).signal(), Some(int));

        assert_eq!(ExitStatus::SUCCESS.signal(), None);
        assert_eq!(ExitStatus::from_raw(127).signal(), None);
    }

    /// The names are `signames.rs`'s, which is the generator's output, so
    /// this is checking the seam rather than the table.
    #[test]
    fn a_signal_names_itself_without_the_prefix() {
        assert_eq!(
            Signal::from_raw(nsh_platform::interrupt_signal()).name(),
            Some(BStr::new("INT"))
        );
        assert_eq!(
            Signal::from_raw(nsh_platform::kill_signal()).name(),
            Some(BStr::new("KILL"))
        );
        /* Index 0 is the exit trap's pseudo-signal, not a signal. */
        assert_eq!(Signal::from_raw(0).name(), Some(BStr::new("EXIT")));
        assert_eq!(
            Signal::from_raw(crate::signames::NSIG as c_int).name(),
            None
        );
        assert_eq!(Signal::from_raw(-1).name(), None);
    }
}
