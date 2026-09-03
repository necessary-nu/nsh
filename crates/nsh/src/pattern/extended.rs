//! The extended-glob groups `shopt -s extglob` gives `?(…)`, `*(…)`,
//! `+(…)`, `@(…)` and `!(…)`.
//!
//! A child module rather than a sibling because it reads `Matcher`'s
//! fields and calls its walk: the group machinery is one caller of the
//! plain matcher rather than a layer over it, and a child sees its
//! parent's private items without a single visibility widened for the
//! move.
//!
//! `Pattern::ends_within` is here because nothing else asks it. Every
//! other entry to the walk goes through `Trial`, which holds a memo
//! across the questions one operation asks; an alternative is a pattern
//! of its own put to one offset, so it gets a fresh memo and a matcher
//! of its own.

use std::collections::HashSet;

use super::{Characters, Goal, Matcher, Memo, Pattern};

/// One `X(alternative|…)` extended-glob group and where the pattern
/// continues after it.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) struct ExtendedGroup {
    kind: u8,
    /// Where `X(` begins. The group is a function of this offset and the
    /// pattern, so it is also the half of `Matcher::group_ends`'s key
    /// that names *which* group an end set belongs to.
    start: usize,
    alternatives: Vec<core::ops::Range<usize>>,
    next: usize,
}

impl Pattern {
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
        let mut matcher = self.matcher(characters, pattern_start, budget, &mut memo, Goal::Every);
        matcher.reach_from(0, from);
        matcher.ends
    }
}

impl Matcher<'_, '_> {
    /// Read the extended-glob group that starts at `at`, when the option
    /// that gives `X(` its meaning is on and the group is well formed.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(super) fn extended_group(&self, at: usize) -> Option<ExtendedGroup> {
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
    pub(super) fn reach_group(
        &mut self,
        group: &ExtendedGroup,
        subject_at: usize,
    ) -> Option<usize> {
        match group.kind {
            b'!' => self.reach_group_excluded(group, subject_at),
            b'?' => self.reach_group_once(group, subject_at, true),
            b'@' => self.reach_group_once(group, subject_at, false),
            b'*' => self.reach_group_repeated(group, subject_at, true),
            _ => self.reach_group_repeated(group, subject_at, false),
        }
    }

    /// The best the pattern after a group reaches from any offset the
    /// group can leave the subject at.
    fn best_reach(&mut self, next: usize, ends: Vec<usize>) -> Option<usize> {
        let mut best = None;
        for end in ends {
            let reach = self.reach_from(next, end);
            if self.settles(reach) {
                return reach;
            }
            best = best.max(reach);
        }
        best
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

    fn reach_group_once(
        &mut self,
        group: &ExtendedGroup,
        subject_at: usize,
        optional: bool,
    ) -> Option<usize> {
        let mut ends = self.alternative_ends(group, subject_at);
        if optional && !ends.contains(&subject_at) {
            ends.push(subject_at);
        }
        self.best_reach(group.next, ends)
    }

    fn reach_group_repeated(
        &mut self,
        group: &ExtendedGroup,
        subject_at: usize,
        optional: bool,
    ) -> Option<usize> {
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
        self.best_reach(group.next, ends)
    }

    /// `!(list)` consumes any run of subject characters that no
    /// alternative matches, then the pattern continues after the group.
    fn reach_group_excluded(&mut self, group: &ExtendedGroup, subject_at: usize) -> Option<usize> {
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
        self.best_reach(group.next, candidates)
    }
}

#[cfg(test)]
mod tests {
    use bstr::BString;

    use super::super::tests::COST_ALLOWANCE;
    use super::super::{Pattern, PatternOptions};

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
