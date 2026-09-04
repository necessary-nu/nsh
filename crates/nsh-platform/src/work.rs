//! The unit of locale work the shell can be asked to report.
//!
//! A cost rule constrains an implementation rather than an answer: the
//! fast shell and the slow shell print the same bytes, so no
//! differential corpus separates them and only a count of work done can.
//! These are the two expensive things a locale-sensitive operation does
//! -- select the thread locale, and ask the host about a character --
//! and exporting them is what lets a check name a cost without naming a
//! duration.
// [spec:nsh:req:cost.asserted-as-work]

use std::cell::Cell;

thread_local! {
    static SELECTIONS: Cell<u64> = const { Cell::new(0) };
    static CHARACTER_QUERIES: Cell<u64> = const { Cell::new(0) };
}

/// Locale-sensitive work one thread has done since it started.
///
/// Per thread rather than per process, because both counts are of work
/// this thread did and both are read by a caller that just made the
/// thread do it. A process-wide counter would have to be atomic, and
/// would answer with another thread's expansions mixed in.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub struct LocaleWork {
    /// Thread-locale selections: one `uselocale` pair each, and a
    /// thread-global state change that every other locale-sensitive
    /// operation must then be ordered against.
    pub selections: u64,
    /// Questions put to the host about one character or one byte
    /// position -- `mbrlen`, `mbrtowc`, `wcrtomb` and the wide
    /// classifications on a POSIX host, the equivalent table lookups
    /// where there is no C library to ask.
    pub character_queries: u64,
}

impl LocaleWork {
    /// What was done between an earlier reading and this one.
    ///
    /// Counters run for the life of the thread, so every caller wants a
    /// difference rather than a total, and subtracting is the whole of
    /// the reset a thread-local counter needs: nothing else can have
    /// moved it.
    #[must_use]
    pub fn since(self, earlier: Self) -> Self {
        Self {
            selections: self.selections.saturating_sub(earlier.selections),
            character_queries: self
                .character_queries
                .saturating_sub(earlier.character_queries),
        }
    }
}

/// What this thread has spent on locale-sensitive work so far.
#[must_use]
pub fn locale_work() -> LocaleWork {
    LocaleWork {
        selections: SELECTIONS.get(),
        character_queries: CHARACTER_QUERIES.get(),
    }
}

/// One thread-locale selection is about to happen.
pub(crate) fn record_selection() {
    SELECTIONS.set(SELECTIONS.get().saturating_add(1));
}

/// `count` questions about a character are about to be answered.
pub(crate) fn record_character_queries(count: u64) {
    CHARACTER_QUERIES.set(CHARACTER_QUERIES.get().saturating_add(count));
}
