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
