//! Dialect-sensitive built-in cache invalidation.

use super::{CmdTable, Command};
use crate::options::Dialect;

impl CmdTable {
    /// Defend every lookup against stale classification even if a future
    /// option-changing path forgets the proactive notification.
    pub(super) fn ensure_dispatch(&mut self, dialect: Dialect) {
        if self.dispatch_dialect != dialect {
            self.invalidate_dispatch(dialect);
        }
    }

    fn invalidate_dispatch(&mut self, dialect: Dialect) {
        self.map
            .retain(|_, entry| !matches!(entry.command, Command::Builtin(_)));
        self.dispatch_dialect = dialect;
    }
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch]
/// Invalidate observations made by built-in discovery under the old option
/// set. Today the dialect is the only dispatch-affecting setting; clearing on
/// every completed option update keeps that contract correct as more Bash
/// options acquire dispatch effects.
pub(crate) fn dispatch_changed(sh: &mut crate::context::Shell) {
    let dialect = sh.options.dialect();
    crate::error::with_interrupts_deferred(sh, |sh| {
        sh.commands.invalidate_dispatch(dialect);
    });
}
