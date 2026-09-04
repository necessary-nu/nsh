//! The editor module's surface for a build without the `edit` feature.
//!
//! `edit` is what links `nshedit` and `nshterm`. Without it there is no
//! line editor to read through and no native store to retain history in,
//! so this module answers the same questions [`crate::editor`] does with
//! the answers a shell that cannot edit gives: editing is not active,
//! history is not active, and nothing is retained.
//!
//! The point of answering rather than disappearing is that the call sites
//! stay identical. `input.rs`, `parser.rs`, `trap.rs`, `options.rs`,
//! `variables.rs` and the `fc` and `history` built-ins are written once,
//! against one API, and neither reads a `cfg`. What varies is which of the
//! two modules `lib.rs` binds to the name -- see
//! [`dec:nsh:shell-as-library`], which asks that the frontend hold what a
//! library may not assume, and a terminal is one of those things.
//!
//! `fc` remains a built-in here. POSIX describes it over a history list
//! the shell need not have retained -- a non-interactive shell retains
//! nothing even in a full build -- so "no history" is a state `fc`
//! already had to answer for, not a capability removed from the language.

use bstr::{BStr, BString};

/// One retained command as `fc` reports it.
///
/// Structurally identical to the `edit` build's: `fc` formats these, and
/// the formatting is not the editor's to vary.
pub struct HistoryEvent {
    pub number: i32,
    pub line: BString,
}

/// The shell's history list, in a build with nowhere to keep one.
///
/// Every query answers empty. Nothing constructs one outside
/// [`History::new`], and [`history_mut`] never hands one out, so the
/// queries exist to keep `fc` compiling against a single API rather than
/// to be reached. There is no insertion side: [`record_history_line`]
/// retains nothing, so nothing ever needs storing.
pub struct History {
    _private: (),
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    #[must_use]
    pub fn new() -> Self {
        Self { _private: () }
    }

    pub fn discard_input_entry(&mut self) {}

    #[must_use]
    /// The newest entry, which `$HISTCMD` numbers. There is never one
    /// without an editor, which is why a non-interactive shell answers
    /// `0` for that name here and in the reference.
    pub fn newest(&self) -> Option<HistoryEvent> {
        None
    }

    pub fn oldest(&self) -> Option<HistoryEvent> {
        None
    }

    #[must_use]
    pub fn relative(&self, _older_by: usize) -> Option<HistoryEvent> {
        None
    }

    #[must_use]
    pub fn numbered(&self, _number: i32) -> Option<HistoryEvent> {
        None
    }

    #[must_use]
    pub fn prefixed(&self, _prefix: &[u8]) -> Option<HistoryEvent> {
        None
    }

    #[must_use]
    pub fn range(&self, _first: i32, _last: i32) -> Vec<HistoryEvent> {
        Vec::new()
    }

    #[must_use]
    pub fn file_contents(&self) -> Vec<u8> {
        Vec::new()
    }
}

/// What reading through an editor would have failed with.
///
/// [`read_edit_line`] cannot fail here because [`editing_active`] never
/// lets it be reached, but `input.rs` matches on the `Result` either way.
#[derive(Debug)]
pub enum LineEditorError {}

impl std::fmt::Display for LineEditorError {
    fn fmt(&self, _formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match *self {}
    }
}

impl std::error::Error for LineEditorError {}

/// History and line-editor resources owned by one shell.
///
/// `fc_depth` is not the editor's -- it bounds `fc -s` re-execution, which
/// is evaluator recursion, and it is kept here so `fc` reads one field in
/// both builds.
pub(crate) struct EditorState {
    pub(crate) fc_depth: usize,
}

impl EditorState {
    pub(crate) fn new() -> Self {
        Self { fc_depth: 0 }
    }

    /// `!` in a prompt is the next history number, which is 1 forever here.
    // [spec:posix:req:param.ps1-exclamation-expansion]
    pub(crate) fn expand_prompt_exclamation_marks(&self, prompt: &BStr) -> BString {
        let mut expanded = Vec::with_capacity(prompt.len());
        let mut index = 0;
        while index < prompt.len() {
            if prompt[index] != b'!' {
                expanded.push(prompt[index]);
                index += 1;
            } else if prompt.get(index + 1) == Some(&b'!') {
                expanded.push(b'!');
                index += 2;
            } else {
                expanded.push(b'1');
                index += 1;
            }
        }
        BString::from(expanded)
    }
}

#[inline]
pub(crate) fn history_mut(_shell: &mut crate::context::Shell) -> Option<&mut History> {
    None
}

#[must_use]
pub fn history_active(_shell: &crate::context::Shell) -> bool {
    false
}

#[must_use]
pub fn editing_active(_shell: &crate::context::Shell) -> bool {
    false
}

/// Never reached: `input.rs` asks [`editing_active`] first.
pub fn read_edit_line(
    _shell: &mut crate::context::Shell,
    _destination: &mut [u8],
) -> Result<usize, LineEditorError> {
    Ok(0)
}

// [spec:posix:req:edit.history-list]
pub fn record_history_line(
    _shell: &mut crate::context::Shell,
    _bytes: &[u8],
    _first: bool,
    _from_input: bool,
) {
}

// [spec:posix:req:builtin.fc.env-histfile]
// [spec:posix:req:builtin.fc.env-histsize]
// [spec:posix:req:edit.history-list]
pub fn refresh_editor_configuration(_shell: &mut crate::context::Shell) {}

pub(crate) fn save_history(_shell: &mut crate::context::Shell) {}

// [spec:posix:req:builtin.fc.env-histsize]
pub fn set_history_size(_shell: &mut crate::context::Shell, _hs: &BStr) {}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nothing_edits_and_nothing_is_retained() {
        let shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        assert!(!editing_active(&shell));
        assert!(!history_active(&shell));
    }

    // [spec:posix:req:param.ps1-exclamation-expansion/test]
    #[test]
    fn prompt_exclamation_marks_report_the_first_number() {
        let state = EditorState::new();
        assert_eq!(
            state.expand_prompt_exclamation_marks(BStr::new(b"[!][!!][!!!]")),
            BString::from("[1][!][!1]")
        );
    }
}
