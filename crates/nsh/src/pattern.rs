//! Shell pattern matching for case patterns, parameter trimming, and globbing.
//!
//! Patterns retain a quote-protection bit beside every source byte. Matching
//! therefore never needs control-byte escapes or multibyte framing: `*`, `?`,
//! and `[` are operators exactly when their byte is unquoted, while literal
//! and locale-multibyte characters remain ordinary byte-preserving slices.

use std::collections::HashMap;

use bstr::BString;

mod extended;

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

    /// Whether this pattern holds an operator that can make it match a
    /// name other than its own text, and so has to be matched against
    /// the filenames that exist.
    ///
    /// A `[` counts only when a `]` follows it in the same
    /// slash-delimited component. That is POSIX's own trigger -- a `*`,
    /// `?` or `[` "that will be treated as special", where a `[` that
    /// introduces nothing stands for itself -- and it is Bash's test
    /// exactly. dash's is the same but for a leading `!`, which it steps
    /// over, so `[!]` is ordinary there and a pattern here; both answer
    /// without opening a directory. Reaching the same answer by
    /// generating the names and finding none of them matched is not the
    /// same answer: under `nullglob` the word is then dropped, under
    /// `failglob` it is an error, and under `nocaseglob` a file whose
    /// name differs from the word only in case replaces it.
    ///
    /// The test deliberately under-reads the bracket syntax that
    /// `Matcher::bracket` reads in full: it asks only whether a `]` is
    /// there to close the list, not whether what lies between them is a
    /// well-formed member. Erring that way costs a directory read for a
    /// word like `[!]`, which the matcher then settles as literal text
    /// and which comes back as itself either way. Erring the other way,
    /// by calling a real bracket expression ordinary, would drop an
    /// expansion, so the loose test is the safe one and is what Bash
    /// itself does.
    ///
    /// A `/` closes the question, because pathname expansion identifies
    /// slashes before bracket expressions: `a[b/c]d` matches the name
    /// `a[b/c]d` and nothing else. Every `/` byte counts, quoted or not,
    /// because that is how the walk splits a pattern into components.
    // [spec:posix:req:pattern.filename-expansion-trigger]
    // [spec:posix:req:pattern.no-special-chars-unchanged]
    // [spec:posix:sem:pattern.left-bracket-literal]
    // [spec:posix:syn:pattern.slash-terminates-bracket]
    pub(crate) fn has_meta(&self) -> bool {
        let mut opened = false;
        for (at, byte) in self.bytes.iter().enumerate() {
            if *byte == b'/' {
                opened = false;
                continue;
            }
            if self.quoted[at] {
                continue;
            }
            match byte {
                b'*' | b'?' => return true,
                b'[' => opened = true,
                b']' if opened => return true,
                b'+' | b'@' | b'!' if self.options.extended && self.active(at + 1, b'(') => {
                    return true;
                }
                _ => {}
            }
        }
        false
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
            spans: Memo::default(),
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

    fn matcher<'a, 'b>(
        &'a self,
        characters: &'a mut Characters<'b>,
        pattern_start: usize,
        budget: &'a mut u64,
        memo: &'a mut Memo,
        goal: Goal,
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
            goal,
            ends: Vec::new(),
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
    /// The same, for the furthest-end question a substitution asks from
    /// every offset. It is a memo of its own because the two hold
    /// different answers under the same key: one says whether the
    /// subject runs out with the pattern, the other how far the pattern
    /// can reach from there.
    spans: Memo,
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
        let matched = self.ask(&mut memo, from, Goal::Whole).0.is_some();
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
        let mut ends = self.ask(&mut memo, from, Goal::Every).1;
        ends.sort_unstable();
        ends.dedup();
        ends
    }

    /// The furthest offset at which the pattern, read from its first
    /// byte, runs out over `subject[from..]`, counting only offsets that
    /// begin a character.
    ///
    /// An unanchored substitution asks this from every offset of its
    /// value, and it is one number per state that does not depend on
    /// which start reached the state -- so every start reads one memo and
    /// the whole operation costs a traversal rather than one apiece. The
    /// set of ends cannot be shared that way, which is what `ends_from`
    /// records and why this is a question of its own rather than its
    /// last element.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) fn furthest_end(&mut self, from: usize) -> Option<usize> {
        let mut spans = std::mem::take(&mut self.spans);
        let reach = self.ask(&mut spans, from, Goal::Furthest).0;
        // A question that ran out of budget abandoned branches it had not
        // finished, and what it wrote for those is not an answer the next
        // question may read.
        if self.budget > 0 {
            self.spans = spans;
        }
        reach
    }

    fn ask(&mut self, memo: &mut Memo, from: usize, goal: Goal) -> (Option<usize>, Vec<usize>) {
        self.budget = self.pattern.budget();
        let mut matcher =
            self.pattern
                .matcher(&mut self.characters, 0, &mut self.budget, memo, goal);
        let reach = matcher.reach_from(0, from);
        let ends = std::mem::take(&mut matcher.ends);
        self.spent += self.pattern.budget() - self.budget;
        (reach, ends)
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
    /// Which subject offsets begin a character, built on the first
    /// question that has to know and empty until then. Only
    /// `Goal::Furthest` asks, and it asks because a bracket expression
    /// can consume a collating element wider than the character beside
    /// it and so run the pattern out in the middle of the next one.
    subject_boundaries: Vec<bool>,
}

impl<'a> Characters<'a> {
    fn of(locale: &'a nsh_platform::Locale, pattern: &'a [u8], subject: &'a [u8]) -> Self {
        Self {
            pattern: crate::characters::Characters::of(locale, pattern),
            subject: crate::characters::Characters::of(locale, subject),
            subject_boundaries: Vec::new(),
        }
    }

    fn subject_begins_character(&mut self, at: usize) -> bool {
        if self.subject_boundaries.is_empty() {
            let mut flags = vec![false; self.subject.bytes.len() + 1];
            for boundary in crate::characters::boundaries(self.subject.locale, self.subject.bytes) {
                if let Some(flag) = flags.get_mut(boundary) {
                    *flag = true;
                }
            }
            self.subject_boundaries = flags;
        }
        self.subject_boundaries.get(at).copied().unwrap_or(false)
    }
}

/// What a walk is being asked for, which is the whole of the difference
/// between the three questions one pattern is put to.
// [spec:nsh:req:compat.bash.expansion-globbing]
#[derive(Clone, Copy, Eq, PartialEq)]
enum Goal {
    /// Whether the pattern runs out exactly where the subject does. One
    /// answer settles it, so a walk stops at the first branch that has
    /// one.
    Whole,
    /// Every offset the pattern can run out at. Nothing is settled
    /// early, and the ends are recorded as the walk reaches them rather
    /// than returned from a state -- which is why this walk's memo
    /// cannot be read by a later start, and gets a fresh one each time.
    Every,
    /// The furthest offset the pattern can run out at that begins a
    /// character. One number per state, independent of the start that
    /// reached it, and therefore shareable across every start.
    Furthest,
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
    goal: Goal,
    /// Where the pattern ran out, for `Goal::Every`. Running out then
    /// records the offset and answers "no match", so no branch is cut
    /// short and every reachable end is seen; the memo turns into the
    /// visited set that keeps the walk linear in its states.
    ends: Vec<usize>,
}

/// What one walk has already decided, and may be asked again.
#[derive(Default)]
struct Memo {
    /// What each state answered. The value is what the goal asked for
    /// there: the subject's end or nothing for `Whole`, the furthest end
    /// that begins a character for `Furthest`, and nothing at all for
    /// `Every`, whose walk uses the key alone to say a state was
    /// visited.
    reaches: HashMap<(usize, usize), Option<usize>>,
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
    /// What the pattern read from `pattern_at` reaches over the subject
    /// read from `subject_at`, in whatever the goal counts as reaching.
    fn reach_from(&mut self, pattern_at: usize, subject_at: usize) -> Option<usize> {
        if let Some(reach) = self.memo.reaches.get(&(pattern_at, subject_at)) {
            return *reach;
        }
        if *self.budget == 0 {
            return None;
        }
        *self.budget -= 1;
        let reach = self.reach_uncached(pattern_at, subject_at);
        self.memo.reaches.insert((pattern_at, subject_at), reach);
        reach
    }

    /// Whether an answer settles the question, so a walk holding one need
    /// not take the branches it has not taken yet.
    fn settles(&self, reach: Option<usize>) -> bool {
        reach.is_some() && self.goal == Goal::Whole
    }

    /// What the pattern running out at `subject_at` is worth to the
    /// question being asked.
    fn ran_out(&mut self, subject_at: usize) -> Option<usize> {
        match self.goal {
            Goal::Whole => (subject_at == self.subject.len()).then_some(subject_at),
            Goal::Furthest => self
                .characters
                .subject_begins_character(subject_at)
                .then_some(subject_at),
            Goal::Every => {
                if !self.ends.contains(&subject_at) {
                    self.ends.push(subject_at);
                }
                None
            }
        }
    }

    fn reach_uncached(&mut self, mut pattern_at: usize, mut subject_at: usize) -> Option<usize> {
        // What the branches this loop stepped past reached. The loop
        // carries on down one of them and answers the rest here, so every
        // way out of it folds these back in.
        let mut aside = None;
        loop {
            if pattern_at == self.pattern.bytes.len() {
                return self.ran_out(subject_at).max(aside);
            }

            if let Some(group) = self.extended_group(pattern_at) {
                return self.reach_group(&group, subject_at).max(aside);
            }

            if self.pattern.active(pattern_at, b'*') {
                let star_at = pattern_at;
                while self.pattern.active(pattern_at, b'*') {
                    pattern_at += 1;
                }
                // A `*` that ends the pattern reaches the end of the
                // subject, and the two questions with a single answer can
                // give it without walking there. `Every` cannot: each
                // offset the star steps over is an end of its own.
                if pattern_at == self.pattern.bytes.len() && self.goal != Goal::Every {
                    return Some(self.subject.len()).max(aside);
                }
                return self.reach_star(star_at, pattern_at, subject_at).max(aside);
            }

            if self.pattern.active(pattern_at, b'?') {
                if subject_at == self.subject.len() {
                    return aside;
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
                    return aside;
                };
                for count in consumed {
                    let reach = self.reach_from(next_pattern, subject_at + count);
                    if self.settles(reach) {
                        return reach;
                    }
                    aside = aside.max(reach);
                }
                pattern_at = next_pattern;
                subject_at += first;
                continue;
            }

            if subject_at == self.subject.len() {
                return aside;
            }
            let pattern_end = self.pattern_end(pattern_at);
            let subject_end = self.subject_end(subject_at);
            if !self.same_character(
                &self.pattern.bytes[pattern_at..pattern_end],
                &self.subject[subject_at..subject_end],
            ) {
                return aside;
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
    fn reach_star(&mut self, star_at: usize, after: usize, subject_at: usize) -> Option<usize> {
        let mut candidate = subject_at;
        let mut stepped = Vec::new();
        let mut best = loop {
            if let Some(known) = self.memo.reaches.get(&(star_at, candidate)) {
                break *known;
            }
            let reach = self.reach_from(after, candidate);
            if self.settles(reach) {
                break reach;
            }
            stepped.push((candidate, reach));
            if candidate == self.subject.len() {
                break None;
            }
            candidate = self.subject_end(candidate);
        };
        // A walk that ran out of budget answered for branches it never
        // took, and that answer is this question's alone.
        let keep = *self.budget > 0;
        for (at, reach) in stepped.into_iter().rev() {
            best = best.max(reach);
            if keep {
                self.memo.reaches.insert((star_at, at), best);
            }
        }
        best
    }

    /// Whether one pattern character stands for one subject character,
    /// honouring the case-insensitive shell options.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn same_character(&self, pattern: &[u8], subject: &[u8]) -> bool {
        pattern == subject
            || (self.pattern.options.ignore_case && fold_case(pattern) == fold_case(subject))
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

    /// A work assertion rather than a timing one, which is what makes it
    /// worth anything on a loaded machine: a word that is not a candidate
    /// performs no directory read at all, so `[ "$i" -lt 3 ]` costs the
    /// current directory nothing however large it is.
    // [spec:posix:req:pattern.filename-expansion-trigger/test]
    // [spec:posix:req:pattern.no-special-chars-unchanged/test]
    // [spec:posix:sem:pattern.left-bracket-literal/test]
    // [spec:posix:syn:pattern.slash-terminates-bracket/test]
    #[test]
    fn an_unclosed_bracket_is_not_a_pathname_candidate() {
        for text in [
            "[", "[abc", "a[b", "[a-", "[[", "]", "a]b", "]a[",
            /* A slash is identified before a bracket expression, so
             * neither component of these holds a closed list. */
            "a[b/c]d", "sub/[", "[/x", "x/[",
        ] {
            assert!(
                !Pattern::unquoted(text).has_meta(),
                "`{text}` reads the directory it should have been left beside"
            );
        }
        for text in [
            "*",
            "?",
            "[a]",
            "[]",
            "[^]",
            "[!]",
            "[]]",
            "a*[",
            "*/[",
            "[[:alpha:]",
            /* The bracket closes inside one component even though the
             * word spans two. */
            "a/[b]c",
        ] {
            assert!(
                Pattern::unquoted(text).has_meta(),
                "`{text}` is a pattern and has to be matched against the names that exist"
            );
        }
        /* Quoting the bracket takes the question away entirely, and
         * quoting the `]` leaves the `[` with nothing to close it. */
        assert!(!Pattern::new(BString::from("[a]"), vec![true, false, false]).has_meta());
        assert!(!Pattern::new(BString::from("[a]"), vec![false, false, true]).has_meta());
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
    pub(super) const COST_ALLOWANCE: u64 = 16;

    /// What one operation over a whole value is allowed to cost.
    ///
    /// `${v#*zz}` asked whether the pattern matched every prefix of its
    /// value in turn, `${v%*zz}` the same of every suffix, and
    /// `${v/*zz/X}` asked the second question from every offset -- so a
    /// trim was a square of the value's length and a substitution a
    /// cube. Measured interleaved at loads 9 to 18, one operation each
    /// on a 2047-byte value: a trim 0.27s and a substitution 175.75s.
    /// What the collecting walk answers for every end at once, and the
    /// two memos the questions share, make each of the three one walk of
    /// the value -- so all three rows below hold to the same allowance,
    /// and a substitution costs what a trim does.
    #[test]
    // [spec:nsh:req:compat.bash.expansion-globbing/test]
    fn an_operation_over_a_value_costs_one_walk() {
        let locale = nsh_platform::Locale::c().unwrap();
        let pattern = Pattern::unquoted(BString::from("*zz"));
        let subject = vec![b'a'; 2048];
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

        // `${v/*zz/X}` needs the furthest end from each offset, and the
        // whole of it costs one traversal rather than one apiece: how far
        // a state reaches does not depend on which start reached it, so
        // every start reads the answers the one before it left.
        let mut spans = pattern.trial(&locale, &subject);
        assert!((0..=subject.len()).all(|at| spans.furthest_end(at).is_none()));
        let cost = spans.spent;
        assert!(cost < walk, "span cost {cost} exceeds allowance {walk}");
    }
}
