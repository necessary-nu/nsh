//! Shell pattern matching for case patterns, parameter trimming, and globbing.
//!
//! Patterns retain a quote-protection bit beside every source byte. Matching
//! therefore never needs control-byte escapes or multibyte framing: `*`, `?`,
//! and `[` are operators exactly when their byte is unquoted, while literal
//! and locale-multibyte characters remain ordinary byte-preserving slices.

use std::collections::HashMap;

use bstr::BString;

/// A quote-aware shell pattern.
// [spec:nsh:sem:idiom.typed-expansion]
#[derive(Clone, Debug)]
pub(crate) struct Pattern {
    bytes: BString,
    quoted: Vec<bool>,
}

impl Pattern {
    pub(crate) fn new(bytes: BString, quoted: Vec<bool>) -> Self {
        debug_assert_eq!(bytes.len(), quoted.len());
        Self { bytes, quoted }
    }

    pub(crate) fn unquoted(bytes: impl Into<BString>) -> Self {
        let bytes = bytes.into();
        let quoted = vec![false; bytes.len()];
        Self { bytes, quoted }
    }

    pub(crate) fn as_bytes(&self) -> &[u8] {
        &self.bytes
    }

    pub(crate) fn has_meta(&self) -> bool {
        self.bytes
            .iter()
            .enumerate()
            .any(|(at, byte)| !self.quoted[at] && matches!(byte, b'*' | b'?' | b'['))
    }

    pub(crate) fn starts_with_literal_dot(&self) -> bool {
        self.bytes.first() == Some(&b'.')
    }

    pub(crate) fn slice(&self, range: std::ops::Range<usize>) -> Self {
        Self {
            bytes: BString::from(&self.bytes[range.clone()]),
            quoted: self.quoted[range].to_vec(),
        }
    }

    // [spec:dash:sem:expand.pmatch-fn]
    pub(crate) fn matches(&self, locale: &nsh_platform::Locale, subject: &[u8]) -> bool {
        Matcher {
            locale,
            pattern: self,
            subject,
            memo: HashMap::new(),
        }
        .matches_from(0, 0)
    }

    fn active(&self, at: usize, byte: u8) -> bool {
        self.bytes.get(at) == Some(&byte) && !self.quoted.get(at).copied().unwrap_or(true)
    }
}

struct Matcher<'a> {
    locale: &'a nsh_platform::Locale,
    pattern: &'a Pattern,
    subject: &'a [u8],
    memo: HashMap<(usize, usize), bool>,
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
        let result = self.match_uncached(pattern_at, subject_at);
        self.memo.insert((pattern_at, subject_at), result);
        result
    }

    fn match_uncached(&mut self, mut pattern_at: usize, mut subject_at: usize) -> bool {
        loop {
            if pattern_at == self.pattern.bytes.len() {
                return subject_at == self.subject.len();
            }

            if self.pattern.active(pattern_at, b'*') {
                while self.pattern.active(pattern_at, b'*') {
                    pattern_at += 1;
                }
                if pattern_at == self.pattern.bytes.len() {
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
            if self.pattern.bytes[pattern_at..pattern_end] != self.subject[subject_at..subject_end]
            {
                return false;
            }
            pattern_at = pattern_end;
            subject_at = subject_end;
        }
    }

    /// Return the continuation and every subject width matched by one
    /// well-formed bracket expression. `None` means `[` is an ordinary byte.
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
                    let subject = self.subject[subject_at];
                    if (self.pattern.bytes[member_start]..=self.pattern.bytes[range_end_start])
                        .contains(&subject)
                        && !matched_widths.contains(&1)
                    {
                        matched_widths.push(1);
                    }
                }
                at = range_end;
                continue;
            }

            if let Some(width) = subject_width
                && self.pattern.bytes[member_start..member_end]
                    == self.subject[subject_at..subject_at + width]
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
// [spec:dash:sem:expand.patmatch-fn]
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
    fn long_literals_do_not_recurse() {
        let locale = nsh_platform::Locale::c().unwrap();
        let subject = vec![b'x'; 131_072];
        let pattern = Pattern::unquoted(BString::from(subject.clone()));

        assert!(pattern.matches(&locale, &subject));

        let mut different = subject;
        different.push(b'y');
        assert!(!pattern.matches(&locale, &different));
    }
}
