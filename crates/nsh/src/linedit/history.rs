//! Shell-owned policy layered over nshedit's native history store.

use super::{text_from_bytes, text_to_bytes};
use bstr::BString;
use core::ffi::c_int;
use nshedit::domain::{Direction, EditingMode, Text, TextUnit};
use nshedit::editor::effect::{
    HistoryMatch, HistoryPosition, HistoryResponse, HistoryWordPosition, HistoryWordResponse,
};
use nshedit::history::{HistoryCursor, HistoryEntry, HistoryId, HistoryStore, PushResult};
use std::error::Error as StdError;
use std::fmt;

/// A displayed shell-history number.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
struct EventNumber(c_int);

#[derive(Debug, Clone, PartialEq, Eq)]
struct EntryMetadata {
    number: EventNumber,
    /// Exact input bytes for `fc`; the logical `Text` is the editor view.
    bytes: BString,
}

/// An owned history record safe to retain across evaluation re-entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct HistoryEvent {
    pub number: c_int,
    pub line: BString,
}

/// Failure to retain a new shell-history record.
#[derive(Debug)]
pub enum HistoryError {
    NumberExhausted,
    Store(Box<str>),
}

impl fmt::Display for HistoryError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NumberExhausted => formatter.write_str("shell history numbers are exhausted"),
            Self::Store(error) => formatter.write_str(error),
        }
    }
}

impl StdError for HistoryError {}

/// Shell-owned history built on nshedit's native store.
///
/// The shell adds two policies that deliberately do not belong in the
/// general-purpose store: displayed `fc` numbers and the multiline append
/// target. Its configured limit is applied on the next insertion, matching
/// the historical `HISTSIZE` behaviour.
#[derive(Debug)]
pub struct History {
    store: HistoryStore<EntryMetadata>,
    limit: usize,
    next_number: Option<c_int>,
    append_target: Option<HistoryId>,
}

impl Default for History {
    fn default() -> Self {
        Self::new()
    }
}

impl History {
    #[must_use]
    pub fn new() -> Self {
        Self {
            store: HistoryStore::new(),
            limit: 0,
            next_number: Some(1),
            append_target: None,
        }
    }

    /// Change the retained-entry limit. Shrinking takes effect on the next
    /// insertion, as it did for the shell before this native integration.
    pub fn set_limit(&mut self, limit: usize) {
        self.limit = limit;
    }

    #[must_use]
    pub fn len(&self) -> usize {
        self.store.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.store.is_empty()
    }

    /// Insert a complete first physical line and make it the append target.
    pub fn enter(&mut self, bytes: &[u8]) -> Result<c_int, HistoryError> {
        let number = self.next_number.ok_or(HistoryError::NumberExhausted)?;
        self.next_number = number.checked_add(1);
        let metadata = EntryMetadata {
            number: EventNumber(number),
            bytes: BString::from(bytes),
        };
        let id = match self
            .store
            .push_with(text_from_bytes(bytes), metadata)
            .map_err(|error| HistoryError::Store(error.to_string().into_boxed_str()))?
        {
            PushResult::Inserted { id, .. } => id,
            PushResult::Duplicate { .. } => {
                unreachable!("shell history retains consecutive duplicates")
            }
        };
        self.append_target = Some(id);
        self.enforce_limit();
        Ok(number)
    }

    /// Append a continuation physical line to the last entry created by
    /// [`Self::enter`], independently of every traversal cursor.
    pub fn append(&mut self, bytes: &[u8]) -> bool {
        let Some(id) = self.append_target else {
            return false;
        };
        let Some(entry) = self.store.get_mut(id) else {
            return false;
        };
        entry
            .line_mut()
            .extend(text_from_bytes(bytes).as_units().iter().copied());
        entry.metadata_mut().bytes.extend_from_slice(bytes);
        true
    }

    fn enforce_limit(&mut self) {
        while self.store.len() > self.limit {
            let Some(id) = self.store.oldest().map(HistoryEntry::id) else {
                break;
            };
            let _ = self.store.remove(id);
        }
    }

    #[must_use]
    pub fn newest(&self) -> Option<HistoryEvent> {
        self.store.newest().map(history_event)
    }

    #[must_use]
    pub fn oldest(&self) -> Option<HistoryEvent> {
        self.store.oldest().map(history_event)
    }

    /// Resolve `-N` in `fc`: zero is the newest record (the current `fc`
    /// command), one is the preceding record.
    #[must_use]
    pub fn relative(&self, older_by: usize) -> Option<HistoryEvent> {
        self.store.iter().nth(older_by).map(history_event)
    }

    #[must_use]
    pub fn numbered(&self, number: c_int) -> Option<HistoryEvent> {
        self.store
            .iter()
            .find(|entry| entry.metadata().number.0 == number)
            .map(history_event)
    }

    #[must_use]
    pub fn prefixed(&self, prefix: &[u8]) -> Option<HistoryEvent> {
        self.store
            .iter()
            .find(|entry| entry.metadata().bytes.starts_with(prefix))
            .map(history_event)
    }

    /// Snapshot an inclusive range in the direction from `first` to `last`.
    /// The owned result permits `fc -s` to re-enter evaluation without
    /// retaining a borrow into history.
    #[must_use]
    pub fn range(&self, first: c_int, last: c_int) -> Vec<HistoryEvent> {
        let entries: Vec<&HistoryEntry<EntryMetadata>> = self.store.iter().collect();
        let Some(first_index) = entries
            .iter()
            .position(|entry| entry.metadata().number.0 == first)
        else {
            return Vec::new();
        };
        let Some(last_index) = entries
            .iter()
            .position(|entry| entry.metadata().number.0 == last)
        else {
            return Vec::new();
        };
        if first_index <= last_index {
            entries[first_index..=last_index]
                .iter()
                .map(|entry| history_event(entry))
                .collect()
        } else {
            entries[last_index..=first_index]
                .iter()
                .rev()
                .map(|entry| history_event(entry))
                .collect()
        }
    }

    fn cursor_index(&self, cursor: &mut HistoryCursor) -> Option<usize> {
        let current = cursor.current()?;
        let position = self.store.iter().position(|entry| entry.id() == current);
        if position.is_none() {
            cursor.reset();
        }
        position
    }

    fn select_editor_index(&self, cursor: &mut HistoryCursor, index: usize) -> Option<Text> {
        let id = self.store.iter().nth(index)?.id();
        let entry = self.store.select(cursor, id)?;
        Some(editor_history_text(entry.line()))
    }

    pub(super) fn current_editor_text(&self, cursor: &HistoryCursor) -> Option<Text> {
        cursor
            .current()
            .and_then(|id| self.store.get(id))
            .map(|entry| editor_history_text(entry.line()))
    }

    pub(super) fn navigate_editor(
        &self,
        cursor: &mut HistoryCursor,
        direction: Direction,
        count: usize,
        mode: EditingMode,
    ) -> HistoryResponse {
        let original = cursor.current();
        let current = self.cursor_index(cursor);
        match direction {
            Direction::Previous => {
                let target =
                    current.map_or(count.saturating_sub(1), |index| index.saturating_add(count));
                if let Some(line) = self.select_editor_index(cursor, target) {
                    return HistoryResponse::entry(line);
                }
                if mode == EditingMode::Vi {
                    if let Some(id) = original
                        && let Some(entry) = self.store.select(cursor, id)
                    {
                        return HistoryResponse::entry(editor_history_text(entry.line()))
                            .at_boundary();
                    }
                    cursor.reset();
                    return HistoryResponse::live().at_boundary();
                }
                let Some(oldest_index) = self.store.len().checked_sub(1) else {
                    return HistoryResponse::boundary();
                };
                self.select_editor_index(cursor, oldest_index)
                    .map_or_else(HistoryResponse::boundary, |line| {
                        HistoryResponse::entry(line).at_boundary()
                    })
            }
            Direction::Next => {
                let Some(index) = current else {
                    return HistoryResponse::live().at_boundary();
                };
                let depth = index + 1;
                if count >= depth {
                    cursor.reset();
                    let response = HistoryResponse::live();
                    return if count > depth {
                        response.at_boundary()
                    } else {
                        response
                    };
                }
                self.select_editor_index(cursor, index - count)
                    .map_or_else(HistoryResponse::boundary, HistoryResponse::entry)
            }
        }
    }

    pub(super) fn search_editor(
        &self,
        cursor: &mut HistoryCursor,
        pattern: &Text,
        direction: Direction,
        matching: HistoryMatch,
    ) -> HistoryResponse {
        let current = self.cursor_index(cursor);
        let found_index = match direction {
            Direction::Previous => self
                .store
                .iter()
                .enumerate()
                .skip(current.map_or(0, |index| index + 1))
                .find(|(_, entry)| history_matches(entry.line(), pattern, matching))
                .map(|(index, _)| index),
            Direction::Next => {
                let Some(index) = current else {
                    return HistoryResponse::boundary();
                };
                (0..index).rev().find(|candidate| {
                    self.store
                        .iter()
                        .nth(*candidate)
                        .is_some_and(|entry| history_matches(entry.line(), pattern, matching))
                })
            }
        };
        let Some(index) = found_index else {
            return HistoryResponse::boundary();
        };
        self.select_editor_index(cursor, index)
            .map_or_else(HistoryResponse::boundary, HistoryResponse::entry)
    }

    pub(super) fn select_editor_line(
        &self,
        cursor: &mut HistoryCursor,
        position: HistoryPosition,
    ) -> HistoryResponse {
        match position {
            HistoryPosition::Current => {
                cursor.reset();
                HistoryResponse::live()
            }
            HistoryPosition::Oldest => {
                self.store
                    .len()
                    .checked_sub(1)
                    .map_or_else(HistoryResponse::boundary, |index| {
                        self.select_editor_index(cursor, index)
                            .map_or_else(HistoryResponse::boundary, HistoryResponse::entry)
                    })
            }
            HistoryPosition::Number(number) => {
                let Ok(number) = c_int::try_from(number.get()) else {
                    return HistoryResponse::boundary();
                };
                let Some(index) = self
                    .store
                    .iter()
                    .position(|entry| entry.metadata().number.0 == number)
                else {
                    return HistoryResponse::boundary();
                };
                self.select_editor_index(cursor, index)
                    .map_or_else(HistoryResponse::boundary, HistoryResponse::entry)
            }
        }
    }

    pub(super) fn newest_word(&self, position: HistoryWordPosition) -> HistoryWordResponse {
        let Some(entry) = self.store.newest() else {
            return HistoryWordResponse::Missing;
        };
        let line = editor_history_text(entry.line());
        let words: Vec<&[TextUnit]> = line
            .as_units()
            .split(is_history_space)
            .filter(|word| !word.is_empty())
            .collect();
        let selected = match position {
            HistoryWordPosition::Last => words.last().copied(),
            HistoryWordPosition::Number(number) => words.get(number.get() - 1).copied(),
        };
        selected.map_or(HistoryWordResponse::Missing, |word| {
            HistoryWordResponse::Word(word.iter().copied().collect())
        })
    }
}

fn history_event(entry: &HistoryEntry<EntryMetadata>) -> HistoryEvent {
    HistoryEvent {
        number: entry.metadata().number.0,
        line: entry.metadata().bytes.clone(),
    }
}

fn editor_history_text(line: &Text) -> Text {
    let units = line.as_units();
    let mut end = units.len();
    if units.get(end.wrapping_sub(1)) == Some(&TextUnit::Scalar('\n')) {
        end -= 1;
    }
    if units.get(end.wrapping_sub(1)) == Some(&TextUnit::Scalar(' ')) {
        end -= 1;
    }
    units[..end].iter().copied().collect()
}

fn history_matches(line: &Text, pattern: &Text, matching: HistoryMatch) -> bool {
    let line = editor_history_text(line);
    if pattern.is_empty() {
        return true;
    }
    match matching {
        HistoryMatch::Prefix => line.as_units().starts_with(pattern.as_units()) && line != *pattern,
        // The editor's generic rule for search commands is literal-then-regex,
        // but this host's history search speaks shell pattern notation — the
        // dash/libedit contract the pty corpora pin. Both variants resolve to
        // the shell's matcher.
        HistoryMatch::Contains | HistoryMatch::LiteralOrRegex => {
            shell_history_pattern_matches(&line, pattern)
        }
    }
}

/// Vi history searches use shell pattern notation against any part of a
/// history line. A leading `^` removes that implicit leading wildcard.
fn shell_history_pattern_matches(line: &Text, pattern: &Text) -> bool {
    let Ok(line) = text_to_bytes(line) else {
        return false;
    };
    let Ok(pattern) = text_to_bytes(pattern) else {
        return false;
    };
    if line.contains(&0) || pattern.contains(&0) {
        return false;
    }
    let (anchored, pattern) = match pattern.strip_prefix(b"^") {
        Some(pattern) => (true, pattern),
        None => (false, pattern.as_slice()),
    };
    let mut expression = Vec::with_capacity(pattern.len() + 3);
    if !anchored {
        expression.push(b'*');
    }
    expression.extend_from_slice(pattern);
    expression.push(b'*');
    crate::pmatch::pmatch_slices(&expression, &line) != 0
}

fn is_history_space(unit: &TextUnit) -> bool {
    match unit {
        TextUnit::Scalar(character) => character.is_whitespace(),
        TextUnit::RawByte(byte) => byte.is_ascii_whitespace(),
        TextUnit::OpaqueCodePoint(_) => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use nshedit::editor::effect::HistorySelection;

    fn history(lines: &[&[u8]]) -> History {
        let mut history = History::new();
        history.set_limit(100);
        for line in lines {
            history.enter(line).unwrap();
        }
        history
    }

    #[test]
    fn history_numbers_and_ranges_are_semantic() {
        let history = history(&[b"one\n", b"two\n", b"three\n"]);
        assert_eq!(history.newest().unwrap().number, 3);
        assert_eq!(history.oldest().unwrap().number, 1);
        assert_eq!(
            history
                .range(1, 3)
                .into_iter()
                .map(|event| event.number)
                .collect::<Vec<_>>(),
            vec![1, 2, 3]
        );
        assert_eq!(
            history
                .range(3, 1)
                .into_iter()
                .map(|event| event.number)
                .collect::<Vec<_>>(),
            vec![3, 2, 1]
        );
    }

    #[test]
    fn history_limit_shrinks_on_insert() {
        let mut history = history(&[b"one\n", b"two\n", b"three\n"]);
        history.set_limit(1);
        assert_eq!(history.len(), 3);
        assert_eq!(history.enter(b"four\n").unwrap(), 4);
        assert_eq!(history.len(), 1);
        assert_eq!(history.newest().unwrap().line, BString::from("four\n"));
        history.set_limit(0);
        assert_eq!(history.enter(b"five\n").unwrap(), 5);
        assert!(history.is_empty());
        history.set_limit(1);
        assert_eq!(history.enter(b"six\n").unwrap(), 6);
    }

    #[test]
    fn append_targets_entered_entry() {
        let mut history = history(&[b"one\n"]);
        history.enter(b"if true\n").unwrap();
        assert!(history.append(b"then echo yes\n"));
        assert_eq!(
            history.newest().unwrap().line,
            BString::from("if true\nthen echo yes\n")
        );
    }

    #[test]
    fn history_preserves_raw_bytes() {
        let bytes = b"echo \xff \n";
        let history = history(&[bytes]);
        assert_eq!(history.newest().unwrap().line.as_slice(), bytes);

        let mut cursor = HistoryCursor::new();
        let selected =
            history.navigate_editor(&mut cursor, Direction::Previous, 1, EditingMode::Emacs);
        let HistorySelection::Entry(editor) = selected.selection() else {
            panic!("newest history entry was not selected");
        };
        assert_eq!(text_to_bytes(editor).unwrap(), b"echo \xff");
    }

    #[test]
    fn history_patterns_use_shell_globs() {
        let line = Text::from("printf beta");
        assert!(shell_history_pattern_matches(&line, &Text::from("b?t*")));
        assert!(shell_history_pattern_matches(&line, &Text::from("^printf")));
        assert!(!shell_history_pattern_matches(&line, &Text::from("^beta")));
        assert!(!shell_history_pattern_matches(
            &text_from_bytes(b"has\0nul"),
            &Text::from("nul")
        ));
    }

    #[test]
    fn history_cursor_restores_live_line() {
        let history = history(&[b"one\n", b"two\n", b"three\n"]);
        let mut cursor = HistoryCursor::new();

        let previous =
            history.navigate_editor(&mut cursor, Direction::Previous, 1, EditingMode::Emacs);
        assert_eq!(
            previous.selection(),
            &HistorySelection::Entry(Text::from("three"))
        );
        assert!(!previous.reached_boundary());

        let next = history.navigate_editor(&mut cursor, Direction::Next, 1, EditingMode::Emacs);
        assert_eq!(next.selection(), &HistorySelection::Live);
        assert!(!next.reached_boundary());

        let clamped =
            history.navigate_editor(&mut cursor, Direction::Previous, 9, EditingMode::Emacs);
        assert_eq!(
            clamped.selection(),
            &HistorySelection::Entry(Text::from("one"))
        );
        assert!(clamped.reached_boundary());
    }
}
