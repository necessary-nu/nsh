//! Shell pattern matching for case patterns, parameter trimming, and globbing.
//!
//! Patterns retain a quote-protection bit beside every source byte. Matching
//! therefore never needs control-byte escapes or multibyte framing: `*`, `?`,
//! and `[` are operators exactly when their byte is unquoted, while literal
//! and locale-multibyte characters remain ordinary byte-preserving slices.

use std::collections::HashMap;

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
        // A pattern without extended groups is matched by a memoized
        // walk whose work is bounded by pattern length times subject
        // length, so it needs no budget and POSIX matching is unchanged
        // by this argument existing.
        let mut budget = if self.options.extended {
            EXTENDED_MATCH_BUDGET
        } else {
            u64::MAX
        };
        self.matches_within(locale, subject, &mut budget)
    }

    fn matches_within(
        &self,
        locale: &nsh_platform::Locale,
        subject: &[u8],
        budget: &mut u64,
    ) -> bool {
        self.matcher(locale, subject, budget, None)
            .matches_from(0, 0)
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
        locale: &nsh_platform::Locale,
        subject: &[u8],
        from: usize,
        budget: &mut u64,
    ) -> Vec<usize> {
        let mut matcher = self.matcher(locale, subject, budget, Some(Vec::new()));
        matcher.matches_from(0, from);
        matcher.ends.unwrap_or_default()
    }

    fn matcher<'a>(
        &'a self,
        locale: &'a nsh_platform::Locale,
        subject: &'a [u8],
        budget: &'a mut u64,
        ends: Option<Vec<usize>>,
    ) -> Matcher<'a> {
        Matcher {
            locale,
            pattern: self,
            subject,
            memo: HashMap::new(),
            budget,
            ends,
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
        let mut budget = EXTENDED_MATCH_BUDGET;
        let matched = self.matches_within(locale, subject, &mut budget);
        (matched, EXTENDED_MATCH_BUDGET - budget)
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
// [spec:nsh:req:compat.bash.expansion-globbing]
struct Matcher<'a> {
    locale: &'a nsh_platform::Locale,
    pattern: &'a Pattern,
    subject: &'a [u8],
    memo: HashMap<(usize, usize), bool>,
    budget: &'a mut u64,
    /// `Some` while the walk is collecting ends rather than answering a
    /// yes-or-no question. Running out of pattern then records where the
    /// subject had got to and reports "no match", so no branch is cut
    /// short and every reachable end is seen. The memo turns into the
    /// visited set that keeps the walk linear in its states.
    ends: Option<Vec<usize>>,
}

impl Matcher<'_> {
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
        if let Some(result) = self.memo.get(&(pattern_at, subject_at)) {
            return *result;
        }
        if *self.budget == 0 {
            return false;
        }
        *self.budget -= 1;
        let result = self.match_uncached(pattern_at, subject_at);
        self.memo.insert((pattern_at, subject_at), result);
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
                while self.pattern.active(pattern_at, b'*') {
                    pattern_at += 1;
                }
                // A trailing `*` answers a yes-or-no question at once, but
                // a collecting walk still has to visit every offset it
                // reaches, so it goes round the candidate loop below.
                if pattern_at == self.pattern.bytes.len() && self.ends.is_none() {
                    return true;
                }
                let mut candidate = subject_at;
                loop {
                    if self.matches_from(pattern_at, candidate) {
                        return true;
                    }
                    if candidate == self.subject.len() {
                        return false;
                    }
                    candidate = character_end(self.locale, self.subject, candidate);
                }
            }

            if self.pattern.active(pattern_at, b'?') {
                if subject_at == self.subject.len() {
                    return false;
                }
                pattern_at += 1;
                subject_at = character_end(self.locale, self.subject, subject_at);
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
            let pattern_end = character_end(self.locale, &self.pattern.bytes, pattern_at);
            let subject_end = character_end(self.locale, self.subject, subject_at);
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
    fn alternative_ends(&mut self, group: &ExtendedGroup, from: usize) -> Vec<usize> {
        let locale = self.locale;
        let subject = self.subject;
        let mut ends = Vec::new();
        for range in &group.alternatives {
            let alternative = self.pattern.slice(range.clone());
            for end in alternative.ends_within(locale, subject, from, self.budget) {
                if !ends.contains(&end) {
                    ends.push(end);
                }
            }
        }
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
        let mut ends = Vec::new();
        let mut pending = self.alternative_ends(group, subject_at);
        while let Some(at) = pending.pop() {
            if ends.contains(&at) {
                continue;
            }
            ends.push(at);
            if at > subject_at {
                pending.extend(
                    self.alternative_ends(group, at)
                        .into_iter()
                        .filter(|end| *end > at),
                );
            }
        }
        if optional && !ends.contains(&subject_at) {
            ends.push(subject_at);
        }
        ends.into_iter()
            .any(|end| self.matches_from(group.next, end))
    }

    /// `!(list)` consumes any run of subject characters that no
    /// alternative matches, then the pattern continues after the group.
    fn match_group_excluded(&mut self, group: &ExtendedGroup, subject_at: usize) -> bool {
        let excluded = self.alternative_ends(group, subject_at);
        let mut candidates = Vec::new();
        let mut end = subject_at;
        loop {
            if !excluded.contains(&end) {
                candidates.push(end);
            }
            if end == self.subject.len() {
                break;
            }
            end = character_end(self.locale, self.subject, end);
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
    fn bracket(&self, mut at: usize, subject_at: usize) -> Option<(usize, Vec<usize>)> {
        let inverted = (self.pattern.active(at, b'!') || self.pattern.active(at, b'^'))
            .then(|| at += 1)
            .is_some();
        let subject_width = (subject_at < self.subject.len())
            .then(|| character_end(self.locale, self.subject, subject_at) - subject_at);
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
            let member_end = character_end(self.locale, &self.pattern.bytes, at);
            at = member_end;
            first_member = false;

            if self.pattern.active(at, b'-')
                && at + 1 < self.pattern.bytes.len()
                && !self.pattern.active(at + 1, b']')
            {
                let range_end_start = at + 1;
                let range_end = character_end(self.locale, &self.pattern.bytes, range_end_start);
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

fn character_end(locale: &nsh_platform::Locale, bytes: &[u8], at: usize) -> usize {
    if at >= bytes.len() {
        return at;
    }
    let width = locale
        .multibyte_len(&bytes[at..])
        .filter(|width| *width > 0)
        .unwrap_or(1);
    at.saturating_add(width).min(bytes.len())
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

    /// The shape the `matcher` fuzz target found on 2026-09-01: a leading
    /// `*`, one `+(…)` whose alternatives are mostly empty, one
    /// alternative holding a parenthesised run of `*`, and a subject made
    /// of runs. Four such inputs of about four hundred bytes took between
    /// eleven and ninety-two seconds, because `alternative_ends` asked
    /// "does this alternative match `subject[from..end]`" once per
    /// candidate `end` and threw the memo away between the questions.
    ///
    /// The fence is the work done rather than the clock, because the
    /// clock says nothing about why: an answer costing more than a small
    /// multiple of pattern length times subject length means a factor of
    /// the subject's length has come back.
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
        let allowance = 2 * pattern.as_bytes().len() as u64 * subject.len() as u64;
        assert!(
            cost < allowance,
            "cost {cost} exceeds allowance {allowance}"
        );
    }
}
