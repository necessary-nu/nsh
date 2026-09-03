//! Shell pattern matching for case patterns, parameter trimming, and globbing.
//!
//! Patterns retain a quote-protection bit beside every source byte. Matching
//! therefore never needs control-byte escapes or multibyte framing: `*`, `?`,
//! and `[` are operators exactly when their byte is unquoted, while literal
//! and locale-multibyte characters remain ordinary byte-preserving slices.

use std::collections::{HashMap, HashSet};

use bstr::BString;

/// Matching behaviour a pattern inherits from shell options rather than
/// from its own bytes.
///
/// The bits travel with the pattern because the matcher is reached from
/// `case`, `[[ ]]`, parameter trimming, and pathname expansion alike, and
/// each of those decides from shell state whether `shopt -s extglob` and
/// the case-insensitive options are in force.
// [spec:nsh:req:compat.bash.expansion-globbing]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct PatternOptions {
    /// `shopt -s extglob`: `?(…)`, `*(…)`, `+(…)`, `@(…)`, and `!(…)`.
    pub(crate) extended: bool,
    /// `shopt -s nocaseglob` / `nocasematch`.
    pub(crate) ignore_case: bool,
}

impl PatternOptions {
    pub(crate) const NONE: Self = Self {
        extended: false,
        ignore_case: false,
    };
}

/// A quote-aware shell pattern.
// [spec:nsh:sem:idiom.typed-expansion]
#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    bytes: BString,
    quoted: Vec<bool>,
    options: PatternOptions,
}

/// One `X(alternative|…)` extended-glob group and where the pattern
/// continues after it.
// [spec:nsh:req:compat.bash.expansion-globbing]
struct ExtendedGroup {
    kind: u8,
    /// Where `X(` begins. The group is a function of this offset and the
    /// pattern, so it is also the half of `Matcher::group_ends`'s key
    /// that names *which* group an end set belongs to.
    start: usize,
    alternatives: Vec<core::ops::Range<usize>>,
    next: usize,
}

impl Pattern {
    pub(crate) fn new(bytes: BString, quoted: Vec<bool>) -> Self {
        debug_assert_eq!(bytes.len(), quoted.len());
        Self {
            bytes,
            quoted,
            options: PatternOptions::NONE,
        }
    }

    pub(crate) fn unquoted(bytes: impl Into<BString>) -> Self {
        let bytes = bytes.into();
        let quoted = vec![false; bytes.len()];
        Self {
            bytes,
            quoted,
            options: PatternOptions::NONE,
        }
    }

    /// Read a pattern written as ordinary shell text, where a backslash
    /// protects the byte that follows it. `GLOBIGNORE` holds its patterns
    /// this way: the value is already a string, so the quoting a word
    /// carried during expansion is no longer available beside it.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) fn from_escaped_text(text: &[u8], options: PatternOptions) -> Self {
        let mut bytes = Vec::with_capacity(text.len());
        let mut quoted = Vec::with_capacity(text.len());
        let mut at = 0;
        while at < text.len() {
            let escaped = text[at] == b'\\' && at + 1 < text.len();
            if escaped {
                at += 1;
            }
            bytes.push(text[at]);
            quoted.push(escaped);
            at += 1;
        }
        Self {
            bytes: BString::from(bytes),
            quoted,
            options,
        }
    }

    pub(crate) fn with_options(mut self, options: PatternOptions) -> Self {
        self.options = options;
        self
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    /// The quote bit beside each byte, which the regular-expression
    /// compiler needs for the same reason the matcher does.
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    pub(crate) fn quote_bits(&self) -> &[bool] {
        &self.quoted
    }

    pub(crate) fn has_meta(&self) -> bool {
        self.bytes.iter().enumerate().any(|(at, byte)| {
            !self.quoted[at]
                && (matches!(byte, b'*' | b'?' | b'[')
                    || (self.options.extended
                        && matches!(byte, b'+' | b'@' | b'!')
                        && self.active(at + 1, b'(')))
        })
    }

    pub(crate) fn starts_with_literal_dot(&self) -> bool {
        self.bytes.first() == Some(&b'.')
    }

    pub(crate) fn slice(&self, range: std::ops::Range<usize>) -> Self {
        Self {
            bytes: BString::from(&self.bytes[range.clone()]),
            quoted: self.quoted[range].to_vec(),
            options: self.options,
        }
    }

    // [spec:dash:sem:expand.pmatch-fn]
    pub(crate) fn matches(&self, locale: &nsh_platform::Locale, subject: &[u8]) -> bool {
        self.trial(locale, subject).matches_from(0)
    }

    /// This pattern put to one subject, ready to be asked about it more
    /// than once.
    ///
    /// A trim asks about every offset of its value and a substitution
    /// asks from every offset, so what those questions share is what the
    /// whole operation costs: the locale's answers about the subject's
    /// characters, and -- for the questions that are yes-or-no -- the
    /// memo, whose key names a state rather than the question that
    /// reached it.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) fn trial<'a>(
        &'a self,
        locale: &'a nsh_platform::Locale,
        subject: &'a [u8],
    ) -> Trial<'a> {
        Trial {
            pattern: self,
            characters: Characters::of(locale, &self.bytes, subject),
            memo: Memo::default(),
            budget: 0,
            spent: 0,
        }
    }

    /// What one question about this pattern may cost.
    ///
    /// A pattern without extended groups is answered by a memoized walk
    /// whose work is bounded by pattern length times subject length, so
    /// it needs no budget and POSIX matching is unchanged by this
    /// existing.
    fn budget(&self) -> u64 {
        if self.options.extended {
            EXTENDED_MATCH_BUDGET
        } else {
            u64::MAX
        }
    }

    /// Every subject offset at which this pattern, read from `from`, runs
    /// out — the set `{ end : this pattern matches subject[from..end] }`.
    ///
    /// One traversal answers for every `end` at once, which is the whole
    /// point of asking it this way round: the same question put once per
    /// candidate `end` re-walks the same states with a fresh memo each
    /// time and costs a factor of the subject's length more.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn ends_within(
        &self,
        characters: &mut Characters<'_>,
        pattern_start: usize,
        from: usize,
        budget: &mut u64,
    ) -> Vec<usize> {
        let mut memo = Memo::default();
        let mut matcher = self.matcher(characters, pattern_start, budget, &mut memo, true);
        matcher.matches_from(0, from);
        matcher.ends.unwrap_or_default()
    }

    fn matcher<'a, 'b>(
        &'a self,
        characters: &'a mut Characters<'b>,
        pattern_start: usize,
        budget: &'a mut u64,
        memo: &'a mut Memo,
        collect: bool,
    ) -> Matcher<'a, 'b> {
        let locale = characters.subject.locale;
        let subject = characters.subject.bytes;
        Matcher {
            locale,
            pattern: self,
            pattern_start,
            subject,
            characters,
            memo,
            budget,
            ends: collect.then(Vec::new),
        }
    }

    fn active(&self, at: usize, byte: u8) -> bool {
        self.bytes.get(at) == Some(&byte) && !self.quoted.get(at).copied().unwrap_or(true)
    }

    /// The answer, and how much of the extended budget it took to reach.
    /// A test can then fence the work a shape costs rather than the wall
    /// clock it happens to take on the machine running it.
    #[cfg(test)]
    fn match_cost(&self, locale: &nsh_platform::Locale, subject: &[u8]) -> (bool, u64) {
        let mut trial = self.trial(locale, subject);
        let matched = trial.matches_from(0);
        (matched, trial.spent)
    }
}

/// One pattern's answers about one subject, kept across the questions an
/// operation over a whole value asks.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(crate) struct Trial<'a> {
    pattern: &'a Pattern,
    characters: Characters<'a>,
    /// Shared by every yes-or-no question, because a state's answer does
    /// not depend on which question reached it.
    memo: Memo,
    /// What the question being asked has left to spend. Each gets the
    /// whole of the pattern's budget, as it did when each was a match of
    /// its own.
    budget: u64,
    /// What every question so far has cost, in states walked. The budget
    /// left says nothing about that, having been refilled between the
    /// questions, and it is what a test fences a whole operation on.
    spent: u64,
}

impl Trial<'_> {
    /// Whether the pattern matches the whole of `subject[from..]`.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) fn matches_from(&mut self, from: usize) -> bool {
        let mut memo = std::mem::take(&mut self.memo);
        let matched = self.ask(&mut memo, from, false).0;
        // A question that ran out of budget abandoned branches it had not
        // finished, and the `false` it wrote for those is not an answer
        // the next question may read.
        if self.budget > 0 {
            self.memo = memo;
        }
        matched
    }

    /// Every offset at which the pattern, read from its first byte, runs
    /// out over `subject[from..]` -- the set
    /// `{ end : the pattern matches subject[from..end] }`, in order.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) fn ends_from(&mut self, from: usize) -> Vec<usize> {
        // A collecting walk answers "no match" wherever the pattern runs
        // out, so what its memo holds is not what a yes-or-no walk would
        // hold at the same state, and it gets a memo of its own.
        let mut memo = Memo::default();
        let mut ends = self.ask(&mut memo, from, true).1;
        ends.sort_unstable();
        ends.dedup();
        ends
    }

    fn ask(&mut self, memo: &mut Memo, from: usize, collect: bool) -> (bool, Vec<usize>) {
        self.budget = self.pattern.budget();
        let mut matcher =
            self.pattern
                .matcher(&mut self.characters, 0, &mut self.budget, memo, collect);
        let matched = matcher.matches_from(0, from);
        let ends = matcher.ends.take().unwrap_or_default();
        self.spent += self.pattern.budget() - self.budget;
        (matched, ends)
    }
}

/// How many pattern positions one extended-glob match may visit.
///
/// `!(…)` asks whether *no* alternative matches at every subject offset,
/// and a nested group repeats that question inside itself, so a pattern
/// can demand enormous work while producing no output. The budget is
/// charged for attempted work rather than for results, and running out
/// answers "no match" instead of running on.
///
/// What the budget is *not* is the thing that keeps one group cheap. A
/// group at one subject offset costs a walk of its alternatives over the
/// subject, so a pattern whose groups do not nest stays a polynomial in
/// pattern length times subject length and never comes near this number;
/// the budget is left holding only the depth that nesting multiplies.
/// Asking it to hold more than that is how four four-hundred-byte inputs
/// came to spend the whole four million and take a minute and a half.
// [spec:nsh:req:compat.bash.expansion-globbing]
const EXTENDED_MATCH_BUDGET: u64 = 4_000_000;

/// The two strings one match reads, and what the locale has said about
/// their characters so far.
///
/// `mbrlen` has no locale-taking form, so an answer costs the thread
/// locale being selected and restored around it, and asking one
/// character at a time is how a single match came to ask 482,532 times
/// about a 293-byte subject: a repeated group re-asks the same offsets
/// at every offset it can be entered at. One of these is built per trial
/// -- one operation over one value, however many questions that takes --
/// and shared by every matcher those questions create, including the
/// ones an extended group makes for its alternatives, so an offset is
/// asked about once and the locale is selected a handful of times.
// [spec:nsh:req:compat.bash.expansion-globbing]
struct Characters<'a> {
    /// The whole pattern. Every alternative is a slice of it, which is
    /// why one table serves them all.
    pattern: crate::characters::Characters<'a>,
    subject: crate::characters::Characters<'a>,
}

impl<'a> Characters<'a> {
    fn of(locale: &'a nsh_platform::Locale, pattern: &'a [u8], subject: &'a [u8]) -> Self {
        Self {
            pattern: crate::characters::Characters::of(locale, pattern),
            subject: crate::characters::Characters::of(locale, subject),
        }
    }
}

/// One pattern matched against one subject.
///
/// `memo` is keyed on `(pattern_at, subject_at)` alone, and that key is
/// complete because nothing else can vary between two visits to the same
/// pair: `pattern`, `subject` and `ends` are fixed for the matcher's
/// life, and `PatternOptions` belongs to the pattern and is immutable. In
/// particular `!(…)` does not put the matcher into a negated mode — its
/// alternatives are walked by separate matchers over sliced patterns, and
/// `matches_from` is only ever asked the plain question "does the rest of
/// the pattern match the rest of the subject".
///
/// `characters` is the one field that does change, and it does not
/// weaken that argument: it only ever grows, and what it accumulates are
/// the locale's answers about bytes that do not move, so a second visit
/// to a pair reads exactly the widths the first one did.
// [spec:nsh:req:compat.bash.expansion-globbing]
struct Matcher<'a, 'b> {
    locale: &'b nsh_platform::Locale,
    pattern: &'a Pattern,
    /// Where this matcher's pattern begins in the whole one. An
    /// alternative is a slice of the pattern its group was read from, so
    /// its offsets index the shared width table once shifted by this.
    pattern_start: usize,
    subject: &'b [u8],
    /// What the locale has already said about the pattern's and the
    /// subject's characters, shared with every other matcher of the same
    /// match.
    characters: &'a mut Characters<'b>,
    /// What this walk has already decided. Borrowed rather than owned so
    /// that a caller asking about many offsets of one subject -- which
    /// is what a trim does -- can hold one across the lot.
    memo: &'a mut Memo,
    budget: &'a mut u64,
    /// `Some` while the walk is collecting ends rather than answering a
    /// yes-or-no question. Running out of pattern then records where the
    /// subject had got to and reports "no match", so no branch is cut
    /// short and every reachable end is seen. The memo turns into the
    /// visited set that keeps the walk linear in its states.
    ends: Option<Vec<usize>>,
}

/// What one walk has already decided, and may be asked again.
#[derive(Default)]
struct Memo {
    matches: HashMap<(usize, usize), bool>,
    /// Where each group, entered at each subject offset, can end. Keyed
    /// on `(group start, from)`, which is complete for the same reason
    /// `matches`'s key is. Its size is bounded by the budget rather than
    /// by the subject: producing an end costs at least one budget unit,
    /// so the table cannot hold more entries than the walk was allowed
    /// to pay for.
    group_ends: HashMap<(usize, usize), Vec<usize>>,
}

impl Matcher<'_, '_> {
    /// Where the character beginning at `at` in the subject runs out.
    fn subject_end(&mut self, at: usize) -> usize {
        self.characters.subject.end(at)
    }

    /// Where the character beginning at `at` in this matcher's own
    /// pattern runs out. Both offsets are that pattern's, and the table
    /// underneath is the whole pattern's.
    fn pattern_end(&mut self, at: usize) -> usize {
        let start = self.pattern_start;
        let limit = start + self.pattern.bytes.len();
        self.characters.pattern.end_within(start + at, limit) - start
    }

    // [spec:posix:def:pattern.notation-purpose]
    // [spec:posix:req:pattern.invalid-byte-sequence-unspecified]
    // [spec:posix:req:pattern.match-by-bit-pattern]
    // [spec:posix:syn:pattern.single-character-patterns]
    // [spec:posix:def:pattern.ordinary-character]
    // [spec:posix:def:pattern.special-pattern-characters]
    // [spec:posix:sem:pattern.question-mark]
    // [spec:posix:sem:pattern.asterisk]
    // [spec:posix:syn:pattern.bracket-expression]
    // [spec:posix:sem:pattern.left-bracket-literal]
    // [spec:posix:sem:pattern.asterisk-matches-any-string]
    // [spec:posix:syn:pattern.concatenation]
    // [spec:posix:sem:pattern.asterisk-longest-match]
    fn matches_from(&mut self, pattern_at: usize, subject_at: usize) -> bool {
        if let Some(result) = self.memo.matches.get(&(pattern_at, subject_at)) {
            return *result;
        }
        if *self.budget == 0 {
            return false;
        }
        *self.budget -= 1;
        let result = self.match_uncached(pattern_at, subject_at);
        self.memo.matches.insert((pattern_at, subject_at), result);
        result
    }

    fn match_uncached(&mut self, mut pattern_at: usize, mut subject_at: usize) -> bool {
        loop {
            if pattern_at == self.pattern.bytes.len() {
                let Some(ends) = self.ends.as_mut() else {
                    return subject_at == self.subject.len();
                };
                if !ends.contains(&subject_at) {
                    ends.push(subject_at);
                }
                return false;
            }

            if let Some(group) = self.extended_group(pattern_at) {
                return self.match_group(&group, subject_at);
            }

            if self.pattern.active(pattern_at, b'*') {
                let star_at = pattern_at;
                while self.pattern.active(pattern_at, b'*') {
                    pattern_at += 1;
                }
                // A trailing `*` answers a yes-or-no question at once, but
                // a collecting walk still has to visit every offset it
                // reaches, so it goes round the candidate loop below.
                if pattern_at == self.pattern.bytes.len() && self.ends.is_none() {
                    return true;
                }
                return self.match_star(star_at, pattern_at, subject_at);
            }

            if self.pattern.active(pattern_at, b'?') {
                if subject_at == self.subject.len() {
                    return false;
                }
                pattern_at += 1;
                subject_at = self.subject_end(subject_at);
                continue;
            }

            if self.pattern.active(pattern_at, b'[')
                && let Some((next_pattern, consumed)) = self.bracket(pattern_at + 1, subject_at)
            {
                let mut consumed = consumed.into_iter();
                let Some(first) = consumed.next() else {
                    return false;
                };
                if consumed.any(|count| self.matches_from(next_pattern, subject_at + count)) {
                    return true;
                }
                pattern_at = next_pattern;
                subject_at += first;
                continue;
            }

            if subject_at == self.subject.len() {
                return false;
            }
            let pattern_end = self.pattern_end(pattern_at);
            let subject_end = self.subject_end(subject_at);
            if !self.same_character(
                &self.pattern.bytes[pattern_at..pattern_end],
                &self.subject[subject_at..subject_end],
            ) {
                return false;
            }
            pattern_at = pattern_end;
            subject_at = subject_end;
        }
    }

    /// Where a `*` and the pattern after it can be satisfied from
    /// `subject_at`.
    ///
    /// `*` is the one operator whose state answers for a whole run of
    /// offsets at once: what it asks is whether the rest of the pattern
    /// is satisfied at `subject_at` or at any offset after it, and the
    /// same star one character further on asks that of a shorter run. So
    /// the walk over candidates settles `(star, c)` for every `c` it
    /// stepped over rather than only the offset it was asked about, and
    /// reads those answers back when a later question steps into them.
    ///
    /// Without that, a caller asking about every offset of its value --
    /// which is exactly what a suffix trim does -- pays for the walk
    /// once per offset, and the whole operation is a square of the
    /// value's length while its states are a multiple of it.
    // [spec:posix:sem:pattern.asterisk-matches-any-string]
    fn match_star(&mut self, star_at: usize, after: usize, subject_at: usize) -> bool {
        let mut candidate = subject_at;
        let mut stepped = Vec::new();
        let matched = loop {
            if let Some(known) = self.memo.matches.get(&(star_at, candidate)) {
                break *known;
            }
            if self.matches_from(after, candidate) {
                break true;
            }
            stepped.push(candidate);
            if candidate == self.subject.len() {
                break false;
            }
            candidate = self.subject_end(candidate);
        };
        // A walk that ran out of budget answered "no match" for branches
        // it never took, and that answer is this question's alone.
        if *self.budget > 0 {
            for at in stepped {
                self.memo.matches.insert((star_at, at), matched);
            }
        }
        matched
    }

    /// Whether one pattern character stands for one subject character,
    /// honouring the case-insensitive shell options.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn same_character(&self, pattern: &[u8], subject: &[u8]) -> bool {
        pattern == subject
            || (self.pattern.options.ignore_case && fold_case(pattern) == fold_case(subject))
    }

    /// Read the extended-glob group that starts at `at`, when the option
    /// that gives `X(` its meaning is on and the group is well formed.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn extended_group(&self, at: usize) -> Option<ExtendedGroup> {
        if !self.pattern.options.extended {
            return None;
        }
        let kind = *self.pattern.bytes.get(at)?;
        if !matches!(kind, b'?' | b'*' | b'+' | b'@' | b'!')
            || self.pattern.quoted.get(at).copied().unwrap_or(true)
            || !self.pattern.active(at + 1, b'(')
        {
            return None;
        }

        let mut alternatives = Vec::new();
        let mut start = at + 2;
        let mut depth = 1usize;
        let mut cursor = start;
        while cursor < self.pattern.bytes.len() {
            if self.pattern.quoted[cursor] {
                cursor += 1;
                continue;
            }
            match self.pattern.bytes[cursor] {
                b'(' => depth += 1,
                b')' => {
                    depth -= 1;
                    if depth == 0 {
                        alternatives.push(start..cursor);
                        return Some(ExtendedGroup {
                            kind,
                            start: at,
                            alternatives,
                            next: cursor + 1,
                        });
                    }
                }
                b'|' if depth == 1 => {
                    alternatives.push(start..cursor);
                    start = cursor + 1;
                }
                _ => {}
            }
            cursor += 1;
        }
        None
    }

    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn match_group(&mut self, group: &ExtendedGroup, subject_at: usize) -> bool {
        match group.kind {
            b'!' => self.match_group_excluded(group, subject_at),
            b'?' => self.match_group_once(group, subject_at, true),
            b'@' => self.match_group_once(group, subject_at, false),
            b'*' => self.match_group_repeated(group, subject_at, true),
            _ => self.match_group_repeated(group, subject_at, false),
        }
    }

    /// Every subject position one alternative can reach from `from`.
    ///
    /// Each alternative is a pattern in its own right, walked over the
    /// same subject by its own matcher; the work budget is the one thing
    /// they share, so a nested group cannot escape it.
    ///
    /// The answer depends on nothing but the group and `from`, and a
    /// repeated group asks it about every offset it can reach — from
    /// every offset it can be entered at. Remembering it is what stops
    /// that being a cube of the subject's length.
    fn alternative_ends(&mut self, group: &ExtendedGroup, from: usize) -> Vec<usize> {
        if let Some(ends) = self.memo.group_ends.get(&(group.start, from)) {
            return ends.clone();
        }
        let mut ends = Vec::new();
        for range in &group.alternatives {
            let alternative = self.pattern.slice(range.clone());
            let start = self.pattern_start + range.start;
            ends.extend(alternative.ends_within(self.characters, start, from, self.budget));
        }
        ends.sort_unstable();
        ends.dedup();
        self.memo
            .group_ends
            .insert((group.start, from), ends.clone());
        ends
    }

    fn match_group_once(
        &mut self,
        group: &ExtendedGroup,
        subject_at: usize,
        optional: bool,
    ) -> bool {
        let mut ends = self.alternative_ends(group, subject_at);
        if optional && !ends.contains(&subject_at) {
            ends.push(subject_at);
        }
        ends.into_iter()
            .any(|end| self.matches_from(group.next, end))
    }

    fn match_group_repeated(
        &mut self,
        group: &ExtendedGroup,
        subject_at: usize,
        optional: bool,
    ) -> bool {
        // A repetition reaches the closure of the group's ends, so an
        // offset is worth expanding once. `seen` says which have been,
        // and it is a set rather than a scanned list because the closure
        // can hold an offset for every position in the subject.
        let mut seen = HashSet::new();
        let mut ends = Vec::new();
        let mut pending: Vec<usize> = self
            .alternative_ends(group, subject_at)
            .into_iter()
            .filter(|end| seen.insert(*end))
            .collect();
        while let Some(at) = pending.pop() {
            ends.push(at);
            if at > subject_at {
                let reached = self.alternative_ends(group, at);
                pending.extend(
                    reached
                        .into_iter()
                        .filter(|end| *end > at && seen.insert(*end)),
                );
            }
        }
        if optional && seen.insert(subject_at) {
            ends.push(subject_at);
        }
        ends.into_iter()
            .any(|end| self.matches_from(group.next, end))
    }

    /// `!(list)` consumes any run of subject characters that no
    /// alternative matches, then the pattern continues after the group.
    fn match_group_excluded(&mut self, group: &ExtendedGroup, subject_at: usize) -> bool {
        let excluded: HashSet<usize> = self
            .alternative_ends(group, subject_at)
            .into_iter()
            .collect();
        let mut candidates = Vec::new();
        let mut end = subject_at;
        loop {
            if !excluded.contains(&end) {
                candidates.push(end);
            }
            if end == self.subject.len() {
                break;
            }
            end = self.subject_end(end);
        }
        candidates
            .into_iter()
            .any(|end| self.matches_from(group.next, end))
    }

    /// Return the continuation and every subject width matched by one
    /// well-formed bracket expression. `None` means `[` is an ordinary byte.
    ///
    /// `first_member` is why `[]]` and `[^]]` both hold a literal `]`: a
    /// `]` that opens the list is a member, not the terminator. POSIX
    /// states that rule for `[!...]` and leaves `[^...]` undefined for
    /// shell patterns, so the choice is ours -- and it is to apply one
    /// rule to both, in every context a pattern appears in.
    ///
    /// Bash does not. `case a in [^]]` matches there, as it does here,
    /// but `${s//[^]]/z}` matches *nothing* in Bash -- its substitution
    /// path closes the list at the first `]` and is then left requiring a
    /// second one. One spelling means two things depending on where it is
    /// written. Reproducing that would mean carrying two bracket parsers
    /// and choosing between them by call site. Recorded in
    /// docs/divergences.md; costs `var-op-patsub.test.sh:23`.
    // [spec:dash:sem:expand.ccmatch-fn]
    // [spec:posix:req:pattern.unmatched-open-bracket-unspecified]
    fn bracket(&mut self, mut at: usize, subject_at: usize) -> Option<(usize, Vec<usize>)> {
        let inverted = (self.pattern.active(at, b'!') || self.pattern.active(at, b'^'))
            .then(|| at += 1)
            .is_some();
        let subject_width =
            (subject_at < self.subject.len()).then(|| self.subject_end(subject_at) - subject_at);
        let mut matched_widths = Vec::new();
        let mut first_member = true;

        loop {
            if at >= self.pattern.bytes.len() {
                return None;
            }
            if self.pattern.active(at, b']') && !first_member {
                let next = at + 1;
                if inverted {
                    return Some((
                        next,
                        subject_width
                            .filter(|width| !matched_widths.contains(width))
                            .into_iter()
                            .collect(),
                    ));
                }
                return Some((next, matched_widths));
            }

            if self.pattern.active(at, b'[')
                && let Some((next, widths)) = self.nested_member(at, subject_at, subject_width)
            {
                first_member = false;
                for width in widths {
                    if !matched_widths.contains(&width) {
                        matched_widths.push(width);
                    }
                }
                at = next;
                continue;
            }

            let member_start = at;
            let member_end = self.pattern_end(at);
            at = member_end;
            first_member = false;

            if self.pattern.active(at, b'-')
                && at + 1 < self.pattern.bytes.len()
                && !self.pattern.active(at + 1, b']')
            {
                let range_end_start = at + 1;
                let range_end = self.pattern_end(range_end_start);
                if let Some(width) = subject_width
                    && member_end - member_start == 1
                    && range_end - range_end_start == 1
                    && width == 1
                {
                    let range =
                        self.pattern.bytes[member_start]..=self.pattern.bytes[range_end_start];
                    let subject = self.subject[subject_at];
                    // A case-insensitive range accepts either case of the
                    // subject byte, which is what folding one character
                    // means where the member is a range rather than a
                    // character.
                    let folded = self.pattern.options.ignore_case
                        && (range.contains(&subject.to_ascii_lowercase())
                            || range.contains(&subject.to_ascii_uppercase()));
                    if (range.contains(&subject) || folded) && !matched_widths.contains(&1) {
                        matched_widths.push(1);
                    }
                }
                at = range_end;
                continue;
            }

            if let Some(width) = subject_width
                && self.same_character(
                    &self.pattern.bytes[member_start..member_end],
                    &self.subject[subject_at..subject_at + width],
                )
                && !matched_widths.contains(&width)
            {
                matched_widths.push(width);
            }
        }
    }

    fn nested_member(
        &self,
        at: usize,
        subject_at: usize,
        subject_width: Option<usize>,
    ) -> Option<(usize, Vec<usize>)> {
        let delimiter_at = at + 1;
        let delimiter = *self.pattern.bytes.get(delimiter_at)?;
        if self
            .pattern
            .quoted
            .get(delimiter_at)
            .copied()
            .unwrap_or(true)
            || !matches!(delimiter, b':' | b'.' | b'=')
        {
            return None;
        }

        let mut close = delimiter_at + 1;
        while close + 1 < self.pattern.bytes.len() {
            if self.pattern.active(close, delimiter) && self.pattern.active(close + 1, b']') {
                break;
            }
            close += 1;
        }
        if close + 1 >= self.pattern.bytes.len() {
            return None;
        }
        let body = &self.pattern.bytes[delimiter_at + 1..close];
        if body.is_empty() {
            return None;
        }
        let continuation = close + 2;
        let mut widths = Vec::new();

        if delimiter == b':' {
            if let Some(width) = subject_width
                && self
                    .locale
                    .wide_class_matches(body, &self.subject[subject_at..], width)
                    == Some(true)
            {
                widths.push(width);
            }
            return Some((continuation, widths));
        }

        let mut expression = Vec::with_capacity(body.len() + 5);
        expression.extend_from_slice(b"[[");
        expression.push(delimiter);
        expression.extend_from_slice(body);
        expression.extend_from_slice(&[delimiter, b']', b']']);

        for width in subject_width.into_iter().chain(std::iter::once(body.len())) {
            if width != 0
                && subject_at + width <= self.subject.len()
                && self.locale.collating_bracket_matches(
                    &expression,
                    &self.subject[subject_at..subject_at + width],
                )
                && !widths.contains(&width)
            {
                widths.push(width);
            }
        }
        Some((continuation, widths))
    }
}

/// Fold one character for the case-insensitive shell options.
///
/// A single byte folds by ASCII rules, which is all the C locale has; a
/// complete UTF-8 character folds by Unicode's simple lowercase mapping.
// [spec:nsh:req:compat.bash.expansion-globbing]
fn fold_case(bytes: &[u8]) -> Vec<u8> {
    if bytes.len() == 1 {
        return vec![bytes[0].to_ascii_lowercase()];
    }
    match core::str::from_utf8(bytes) {
        Ok(text) => text.to_lowercase().into_bytes(),
        Err(_) => bytes.to_vec(),
    }
}

/// Compatibility entry for callers whose pattern contains no quoted bytes.
///
/// Only the editor's history search reaches this outside the tests below;
/// every other caller has a `Pattern` already.
// [spec:dash:sem:expand.patmatch-fn]
#[cfg(any(feature = "edit", test))]
pub(crate) fn pattern_matches(
    locale: &nsh_platform::Locale,
    pattern: &[u8],
    subject: &[u8],
) -> bool {
    Pattern::unquoted(BString::from(pattern)).matches(locale, subject)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(locale: &nsh_platform::Locale, pattern: &[u8], subject: &[u8]) -> bool {
        pattern_matches(locale, pattern, subject)
    }

    #[test]
    fn nested_bracket_members_are_atomic() {
        let locale = nsh_platform::Locale::c().unwrap();
        for (pattern, subject) in [
            (b"[[.-.]]".as_slice(), b"-".as_slice()),
            (b"[[.].]]", b"]"),
            (b"[[=-=]]", b"-"),
            (b"[[=]=]]", b"]"),
            (b"[[:alpha:]]", b"a"),
        ] {
            assert!(matches(&locale, pattern, subject));
        }
    }

    #[test]
    fn bracket_members_preserve_pattern_continuation() {
        let locale = nsh_platform::Locale::c().unwrap();
        assert!(matches(&locale, b"[[.-.]]x", b"-x"));
        assert!(matches(&locale, b"*[[=]=]]", b"prefix]"));
        assert!(matches(&locale, b"[![:digit:]]", b"a"));
        assert!(!matches(&locale, b"[![:digit:]]", b"7"));
        assert!(!matches(&locale, b"[[.zz.]]", b"zz"));
    }

    #[test]
    fn quote_bits_disable_pattern_operators() {
        let locale = nsh_platform::Locale::c().unwrap();
        let quoted_star = Pattern::new(BString::from("*"), vec![true]);
        assert!(quoted_star.matches(&locale, b"*"));
        assert!(!quoted_star.matches(&locale, b"anything"));
    }

    #[test]
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    fn extended_groups_repeat_and_negate() {
        let locale = nsh_platform::Locale::c().unwrap();
        let extended = PatternOptions {
            extended: true,
            ignore_case: false,
        };
        let matches = |pattern: &[u8], subject: &[u8]| {
            Pattern::unquoted(BString::from(pattern))
                .with_options(extended)
                .matches(&locale, subject)
        };

        assert!(matches(b"@(foo|bar)", b"foo"));
        assert!(!matches(b"@(foo|bar)", b"foobar"));
        assert!(matches(b"?(foo)", b""));
        assert!(matches(b"*(foo)", b"foofoofoo"));
        assert!(!matches(b"+(foo)", b""));
        assert!(matches(b"+(foo)bar", b"foofoobar"));
        assert!(matches(b"!(foo)", b"bar"));
        assert!(!matches(b"!(foo)", b"foo"));
        assert!(matches(b"--@(help|verbose=@(1|2))", b"--verbose=2"));
        // The same continuation is reached from inside and outside a
        // negated group; the memo answers each with its own question.
        assert!(!matches(b"!(a)x", b"ax"));
        assert!(matches(b"!(a)x", b"aax"));
        // Without the option the group is ordinary pattern text.
        assert!(Pattern::unquoted(BString::from("@(foo|bar)")).matches(&locale, b"@(foo|bar)"));
    }

    #[test]
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    fn extended_matching_work_is_bounded() {
        let locale = nsh_platform::Locale::c().unwrap();
        // Nested negation over a long subject can demand unbounded work
        // while producing no output; the budget answers "no match"
        // rather than running on.
        let pattern =
            Pattern::unquoted(BString::from("!(!(!(!(!(a*b))))))")).with_options(PatternOptions {
                extended: true,
                ignore_case: false,
            });
        let subject = vec![b'a'; 4096];
        assert!(!pattern.matches(&locale, &subject));
    }

    #[test]
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    fn case_folding_is_a_pattern_option() {
        let locale = nsh_platform::Locale::c().unwrap();
        let folded = PatternOptions {
            extended: false,
            ignore_case: true,
        };
        assert!(
            Pattern::unquoted(BString::from("A*C"))
                .with_options(folded)
                .matches(&locale, b"abc")
        );
        assert!(
            Pattern::unquoted(BString::from("[a-c]"))
                .with_options(folded)
                .matches(&locale, b"B")
        );
        assert!(!Pattern::unquoted(BString::from("A*C")).matches(&locale, b"abc"));
    }

    #[test]
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    fn escaped_text_patterns_keep_their_backslashes() {
        let locale = nsh_platform::Locale::c().unwrap();
        let pattern = Pattern::from_escaped_text(b"escape\\*.txt", PatternOptions::NONE);
        assert!(pattern.matches(&locale, b"escape*.txt"));
        assert!(!pattern.matches(&locale, b"escape-10.txt"));
    }

    #[test]
    fn long_literals_do_not_recurse() {
        let locale = nsh_platform::Locale::c().unwrap();
        let subject = vec![b'x'; 131_072];
        let pattern = Pattern::unquoted(BString::from(subject.clone()));

        assert!(pattern.matches(&locale, &subject));

        let mut different = subject;
        different.push(b'y');
        assert!(!pattern.matches(&locale, &different));
    }

    /// What one extended-glob match is allowed to cost, as a multiple of
    /// pattern length times subject length.
    ///
    /// The fence is the work done rather than the clock, because the
    /// clock says nothing about why, and because the inputs below moved
    /// by a factor of three within an hour as this machine's load did.
    /// The number is loose on purpose: the shapes below were over it by a
    /// factor of seven and of twenty-five when they were found, because
    /// what each had was a whole factor of the subject's length. A fence
    /// that only just held would be measuring the constant instead.
    const COST_ALLOWANCE: u64 = 16;

    /// The shape the `matcher` fuzz target found on 2026-09-01: a leading
    /// `*`, one `+(…)` whose alternatives are mostly empty, one
    /// alternative holding a parenthesised run of `*`, and a subject made
    /// of runs. Four such inputs of about four hundred bytes took between
    /// eleven and ninety-two seconds, because `alternative_ends` asked
    /// "does this alternative match `subject[from..end]`" once per
    /// candidate `end` and threw the memo away between the questions.
    #[test]
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    fn extended_alternation_costs_a_multiple_of_its_input() {
        let locale = nsh_platform::Locale::c().unwrap();
        let mut bytes = b"*+(\x9f\x9d\xd6\xff\x9e\x9d\x9e(".to_vec();
        bytes.extend(std::iter::repeat_n(b'*', 34));
        bytes.extend_from_slice(b")**$*****");
        bytes.extend(std::iter::repeat_n(b'|', 18));
        bytes.extend(std::iter::repeat_n(b'*', 4));
        bytes.extend(std::iter::repeat_n(0xff, 30));
        bytes.extend_from_slice(b"aaa)+");
        let pattern = Pattern::unquoted(BString::from(bytes)).with_options(PatternOptions {
            extended: true,
            ignore_case: false,
        });

        let mut subject = vec![b'a'; 14];
        for (byte, run) in [
            (b'*', 4),
            (0x9a, 37),
            (b'*', 33),
            (0xff, 30),
            (0xd6, 40),
            (0xff, 60),
            (b'*', 13),
            (b'a', 12),
        ] {
            subject.extend(std::iter::repeat_n(byte, run));
        }

        let (matched, cost) = pattern.match_cost(&locale, &subject);
        assert!(!matched);
        let allowance = COST_ALLOWANCE * pattern.as_bytes().len() as u64 * subject.len() as u64;
        assert!(
            cost < allowance,
            "cost {cost} exceeds allowance {allowance}"
        );
    }

    /// What one operation over a whole value is allowed to cost.
    ///
    /// `${v#*zz}` asked whether the pattern matched every prefix of its
    /// value in turn, `${v%*zz}` the same of every suffix, and
    /// `${v/*zz/X}` asked the second question from every offset -- so a
    /// trim was a square of the value's length and a substitution a
    /// cube. Measured interleaved at loads 9 to 18, one operation each
    /// on a 2047-byte value: a trim 0.27s and a substitution 175.75s.
    /// What the collecting walk answers for every end at once, and the
    /// memo the questions now share, make a trim one walk of the value
    /// and a substitution one walk from each of its offsets.
    #[test]
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    fn an_operation_over_a_value_costs_one_walk() {
        let locale = nsh_platform::Locale::c().unwrap();
        let pattern = Pattern::unquoted(BString::from("*zz"));
        let subject = vec![b'a'; 2048];
        let offsets = subject.len() as u64 + 1;
        let walk = COST_ALLOWANCE * pattern.as_bytes().len() as u64 * subject.len() as u64;

        // `${v#*zz}` and `${v##*zz}` are the two ends of one traversal.
        let mut prefixes = pattern.trial(&locale, &subject);
        assert!(prefixes.ends_from(0).is_empty());
        let cost = prefixes.spent;
        assert!(cost < walk, "prefix cost {cost} exceeds allowance {walk}");

        // `${v%*zz}` and `${v%%*zz}` ask one question per offset, and
        // the memo they share is what keeps the lot to one walk.
        let mut suffixes = pattern.trial(&locale, &subject);
        assert!(!(0..=subject.len()).any(|at| suffixes.matches_from(at)));
        let cost = suffixes.spent;
        assert!(cost < walk, "suffix cost {cost} exceeds allowance {walk}");

        // `${v/*zz/X}` needs the furthest end from each offset, which is
        // a traversal each and no more than that.
        let mut spans = pattern.trial(&locale, &subject);
        assert!((0..=subject.len()).all(|at| spans.ends_from(at).is_empty()));
        let (cost, allowance) = (spans.spent, walk * offsets);
        assert!(
            cost < allowance,
            "span cost {cost} exceeds allowance {allowance}"
        );
    }

    /// The shape the same target found on 2026-09-02, in the campaign run
    /// to check the first one was closed. A repeated group whose only
    /// alternative is `*` reaches every offset from wherever it starts,
    /// and `match_group_repeated` then asks each of those where the group
    /// can go next — so a group entered at n offsets asked the same n
    /// questions n times over, and the answer to each was recomputed from
    /// nothing. A twenty-six byte pattern against a 388-byte subject
    /// spent the whole budget and took twenty-five seconds to replay.
    #[test]
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    fn a_repeated_group_asks_each_offset_once() {
        let locale = nsh_platform::Locale::c().unwrap();
        let pattern = Pattern::unquoted(BString::from(&b"*aa*(*)aaaa\x8daaaaaaa*(a*)*\x95"[..]))
            .with_options(PatternOptions {
                extended: true,
                ignore_case: false,
            });

        let mut subject = Vec::new();
        for (byte, run) in [
            (b'a', 2),
            (0xff, 1),
            (b'?', 3),
            (0xff, 2),
            (b'?', 1),
            (0xff, 45),
            (b'a', 110),
            (0xff, 1),
            (b'?', 1),
            (0xff, 45),
            (b'a', 118),
            (0xff, 3),
            (b'?', 13),
            (b'a', 8),
            (0xff, 3),
            (b'?', 25),
            (b'a', 7),
        ] {
            subject.extend(std::iter::repeat_n(byte, run));
        }

        let (matched, cost) = pattern.match_cost(&locale, &subject);
        assert!(!matched);
        let allowance = COST_ALLOWANCE * pattern.as_bytes().len() as u64 * subject.len() as u64;
        assert!(
            cost < allowance,
            "cost {cost} exceeds allowance {allowance}"
        );
    }
}
