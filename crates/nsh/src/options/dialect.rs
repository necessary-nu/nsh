use super::{ShellOption, ShellOptions};
// [spec:nsh:def:idiom.shell-options]

// [spec:nsh:def:compat.bash.mode]
/// The grammar and runtime profile selected for one shell input unit.
///
/// This is a compact view of [`ShellOptions`], not independent mutable
/// state. Parser entries copy it so an option change cannot reinterpret a
/// unit whose parsing is already in progress.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum Dialect {
    /// The established POSIX and documented `nsh` language.
    Posix,
    /// The opt-in GNU Bash compatibility profile.
    Bash,
}

impl Dialect {
    /// The status a refusal takes where the dialect decides the boundary
    /// but the diagnostic is not the shell's own `$0: line: ` spine.
    ///
    /// `crate::error::Diagnostics::dialect_error` answers the same
    /// question for a refusal that writes through the spine, and cannot
    /// serve here: a built-in that has already fixed its own wording --
    /// `.: NAME: not found`, `export: NAME: is read only` -- needs the
    /// number without the prefix. XCU 2.8.1 makes both fatal in the POSIX
    /// dialect and leaves the status unspecified, so the 2 is dash's and
    /// the 1 is what Bash reports before carrying on.
    // [spec:nsh:req:compat.bash.error-boundary]
    pub(crate) fn refusal_status(self) -> crate::status::ExitStatus {
        match self {
            Dialect::Posix => crate::status::ExitStatus::ERROR,
            Dialect::Bash => crate::status::ExitStatus::FAILURE,
        }
    }
}

impl ShellOptions {
    // [spec:nsh:req:compat.bash.default-isolation]
    /// The dialect selected by this shell's own option table.
    pub(crate) fn dialect(&self) -> Dialect {
        if !self.enabled(ShellOption::Bash) {
            Dialect::Posix
        } else {
            Dialect::Bash
        }
    }
}
