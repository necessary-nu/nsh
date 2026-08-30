//! POSIX extended regular expressions for Bash's `=~` operator.
//!
//! This is deliberately not part of [`crate::pattern`]: shell patterns and
//! EREs share no syntax beyond the bracket expression, and conflating them
//! is how `*` ends up meaning two things in one matcher. The compiler takes
//! the operand's bytes together with the quote bit each byte carries out of
//! word expansion, so a byte the shell quoted is a literal character here
//! without a re-escaping round trip.
//!
//! Matching is leftmost-longest, as POSIX requires: the search takes the
//! first start offset that matches at all, and at that offset explores the
//! whole expression to keep the longest end.
//!
//! Two bounds hold the exploration down, and both are needed. The step
//! budget is spent by the whole search rather than by one attempt, because
//! an attempt is made at every character and a per-attempt budget would
//! multiply by the subject's length instead of bounding it. The depth
//! bound is separate because steps do not measure stack: a quantified
//! group consumes one subject character per nested continuation, so a
//! long subject reaches the budget only after the call stack is already
//! gone. Exceeding either yields a shorter match or none, never a crash.

use bstr::BString;

/// How much of the search space one whole search may visit.
const STEP_BUDGET: u64 = 400_000;

/// How deeply the continuation chain may nest.
///
/// Every nested [`Matcher::matches`] is one continuation frame, and a
/// quantified group spends one per subject character. Measured against a
/// debug build, `(a)+` over an unbroken run exhausts an eight-megabyte
/// stack at roughly five thousand five hundred nested frames; this leaves
/// the bound a factor of two below that, for a shell whose stack is
/// already carrying an evaluator.
const MAX_DEPTH: u32 = 2_000;

/// The largest repetition count an interval may name.
const MAX_REPEAT: u32 = 1000;

/// How deeply the *compiler* may nest a group.
///
/// Separate from the continuation bound above, which holds matching
/// down: `(((...)))` never reaches the matcher at all, because building
/// the tree recurses once per open parenthesis and dropping it recurses
/// the same way. A level costs 575 bytes in a release build, measured,
/// so 256 of them is 0.14 MiB. The figure is the parser's own nesting
/// ceiling, on the same reasoning: a written expression reaches a
/// handful.
// [spec:nsh:req:idiom.bounded-recursion]
const MAX_GROUP_DEPTH: u32 = 256;

/// A compiled extended regular expression.
pub(crate) struct Regex {
    root: Expr,
    group_count: usize,
    /// `shopt -s nocasematch`, which Bash applies to `=~` exactly as it
    /// applies it to `==`.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    ignore_case: bool,
}

/// Byte offsets of the whole match and each capturing group.
pub(crate) struct Captures {
    pub(crate) groups: Vec<Option<(usize, usize)>>,
}

#[derive(Clone, Debug)]
enum Expr {
    Empty,
    Literal(Vec<u8>),
    AnyCharacter,
    Bracket(Bracket),
    Start,
    End,
    Sequence(Vec<Expr>),
    Alternation(Vec<Expr>),
    Repeat {
        body: Box<Expr>,
        min: u32,
        max: Option<u32>,
    },
    Group {
        index: usize,
        body: Box<Expr>,
    },
}

#[derive(Clone, Debug)]
struct Bracket {
    negated: bool,
    members: Vec<Member>,
}

#[derive(Clone, Debug)]
enum Member {
    Character(Vec<u8>),
    Range(u8, u8),
    Class(BString),
    Collating(BString),
}

impl Regex {
    /// Compile one operand. `quoted[i]` marks a byte that shell quoting
    /// already made literal, so it never carries regular-expression syntax.
    pub(crate) fn compile(
        bytes: &[u8],
        quoted: &[bool],
        ignore_case: bool,
    ) -> Result<Self, BString> {
        let mut parser = Parser {
            bytes,
            quoted,
            pos: 0,
            groups: 0,
            depth: 0,
        };
        let root = parser.alternation()?;
        if parser.pos != bytes.len() {
            return Err(BString::from(&b"Unmatched ) or \\)"[..]));
        }
        Ok(Self {
            root,
            group_count: parser.groups,
            ignore_case,
        })
    }

    /// Search `subject` for the leftmost-longest match.
    pub(crate) fn search(&self, locale: &nsh_platform::Locale, subject: &[u8]) -> Option<Captures> {
        /* One budget for every attempt this search makes. Charging each
         * start offset separately would let a subject `n` bytes long buy
         * `n` full budgets, which is the shape that turns a rejected
         * pattern into an unbounded run. */
        let mut steps = 0_u64;
        let mut start = 0;
        loop {
            if let Some(captures) = self.match_at(locale, subject, start, &mut steps) {
                return Some(captures);
            }
            if start >= subject.len() {
                return None;
            }
            start = character_end(locale, subject, start);
        }
    }

    fn match_at(
        &self,
        locale: &nsh_platform::Locale,
        subject: &[u8],
        start: usize,
        steps: &mut u64,
    ) -> Option<Captures> {
        let mut matcher = Matcher {
            locale,
            subject,
            groups: vec![None; self.group_count + 1],
            steps,
            depth: 0,
            ignore_case: self.ignore_case,
        };
        let mut longest: Option<Vec<Option<(usize, usize)>>> = None;
        let mut best_end = 0;
        matcher.matches(&self.root, start, &mut |state, end| {
            if longest.is_none() || end > best_end {
                best_end = end;
                let mut groups = state.groups.clone();
                groups[0] = Some((start, end));
                longest = Some(groups);
            }
            false
        });
        longest.map(|groups| Captures { groups })
    }
}

// ---- parsing ---------------------------------------------------------

struct Parser<'a> {
    bytes: &'a [u8],
    quoted: &'a [bool],
    pos: usize,
    groups: usize,
    /// Open groups, which is how deeply this parse has recursed.
    // [spec:nsh:req:idiom.bounded-recursion]
    depth: u32,
}

impl Parser<'_> {
    fn active(&self, at: usize, byte: u8) -> bool {
        self.bytes.get(at) == Some(&byte) && !self.quoted.get(at).copied().unwrap_or(true)
    }

    fn here(&self, byte: u8) -> bool {
        self.active(self.pos, byte)
    }

    fn alternation(&mut self) -> Result<Expr, BString> {
        let mut branches = vec![self.sequence()?];
        while self.here(b'|') {
            self.pos += 1;
            branches.push(self.sequence()?);
        }
        Ok(if branches.len() == 1 {
            branches.pop().expect("one branch was just built")
        } else {
            Expr::Alternation(branches)
        })
    }

    fn sequence(&mut self) -> Result<Expr, BString> {
        let mut pieces: Vec<Expr> = Vec::new();
        while self.pos < self.bytes.len() && !self.here(b'|') && !self.here(b')') {
            let atom = self.atom()?;
            pieces.push(self.quantified(atom)?);
        }
        Ok(match pieces.len() {
            0 => Expr::Empty,
            1 => pieces.pop().expect("one piece was just built"),
            _ => Expr::Sequence(pieces),
        })
    }

    /// Apply every quantifier that follows one atom.
    fn quantified(&mut self, atom: Expr) -> Result<Expr, BString> {
        let mut body = atom;
        loop {
            let (min, max) = if self.here(b'*') {
                self.pos += 1;
                (0, None)
            } else if self.here(b'+') {
                self.pos += 1;
                (1, None)
            } else if self.here(b'?') {
                self.pos += 1;
                (0, Some(1))
            } else if self.here(b'{') {
                match self.interval()? {
                    Some(bounds) => bounds,
                    None => return Ok(body),
                }
            } else {
                return Ok(body);
            };
            if matches!(body, Expr::Start | Expr::End) {
                return Err(BString::from(&b"Invalid preceding regular expression"[..]));
            }
            body = Expr::Repeat {
                body: Box::new(body),
                min,
                max,
            };
        }
    }

    /// Parse `{n}`, `{n,}` or `{n,m}`. A brace that opens no interval is a
    /// malformed expression, matching glibc rather than treating it as text.
    fn interval(&mut self) -> Result<Option<(u32, Option<u32>)>, BString> {
        let bad = || BString::from(&b"Invalid content of \\{\\}"[..]);
        let mut at = self.pos + 1;
        let Some(min) = self.digits(&mut at) else {
            return Err(bad());
        };
        let max = if self.active(at, b',') {
            at += 1;
            self.digits(&mut at)
        } else {
            Some(min)
        };
        if !self.active(at, b'}') {
            return Err(bad());
        }
        if min > MAX_REPEAT || max.is_some_and(|max| max > MAX_REPEAT) {
            return Err(BString::from(&b"Regular expression too big"[..]));
        }
        if max.is_some_and(|max| max < min) {
            return Err(bad());
        }
        self.pos = at + 1;
        Ok(Some((min, max)))
    }

    fn digits(&self, at: &mut usize) -> Option<u32> {
        let start = *at;
        let mut value = 0u32;
        while self.bytes.get(*at).is_some_and(|byte| {
            byte.is_ascii_digit() && !self.quoted.get(*at).copied().unwrap_or(true)
        }) {
            value = value
                .saturating_mul(10)
                .saturating_add(u32::from(self.bytes[*at] - b'0'));
            *at += 1;
        }
        (*at != start).then_some(value)
    }

    fn atom(&mut self) -> Result<Expr, BString> {
        if self.here(b'(') {
            // [spec:nsh:req:idiom.bounded-recursion]
            if self.depth >= MAX_GROUP_DEPTH {
                return Err(BString::from(&b"Regular expression nested too deeply"[..]));
            }
            self.pos += 1;
            self.groups += 1;
            let index = self.groups;
            self.depth += 1;
            let body = self.alternation();
            self.depth -= 1;
            let body = body?;
            if !self.here(b')') {
                return Err(BString::from(&b"Unmatched ( or \\("[..]));
            }
            self.pos += 1;
            return Ok(Expr::Group {
                index,
                body: Box::new(body),
            });
        }
        if self.here(b'[') {
            return self.bracket();
        }
        if self.here(b'^') {
            self.pos += 1;
            return Ok(Expr::Start);
        }
        if self.here(b'$') {
            self.pos += 1;
            return Ok(Expr::End);
        }
        if self.here(b'.') {
            self.pos += 1;
            return Ok(Expr::AnyCharacter);
        }
        if self.here(b'*') || self.here(b'+') || self.here(b'?') {
            return Err(BString::from(&b"Invalid preceding regular expression"[..]));
        }
        // A brace reaches an atom only when no atom precedes it, so it can
        // never open the interval that would make it syntax.
        if self.here(b'{') {
            return Err(BString::from(&b"Invalid content of \\{\\}"[..]));
        }
        let start = self.pos;
        self.pos = literal_end(self.bytes, self.pos);
        Ok(Expr::Literal(self.bytes[start..self.pos].to_vec()))
    }

    /// Parse one bracket expression.
    ///
    /// Quoting inside the brackets is discarded rather than honoured, so
    /// `["a-z"]` is the range and not three literals. That is not a
    /// simplification: every shell that implements `=~` agrees on it,
    /// because the operand reaches `regcomp` with the shell's quotes
    /// already removed and only the brackets left to interpret.
    // [spec:posix:syn:pattern.bracket-expression]
    fn bracket(&mut self) -> Result<Expr, BString> {
        let unmatched = || BString::from(&b"Unmatched [, [^, [:, [., or [="[..]);
        let mut at = self.pos + 1;
        let negated = self.bytes.get(at) == Some(&b'^');
        if negated {
            at += 1;
        }
        let mut members = Vec::new();
        let mut first = true;
        loop {
            if at >= self.bytes.len() {
                return Err(unmatched());
            }
            if self.bytes[at] == b']' && !first {
                self.pos = at + 1;
                return Ok(Expr::Bracket(Bracket { negated, members }));
            }
            first = false;
            if self.bytes[at] == b'['
                && let Some(next) = self.bracket_member(at, &mut members)
            {
                at = next;
                continue;
            }
            let member_end = literal_end(self.bytes, at);
            let member = &self.bytes[at..member_end];
            at = member_end;
            if self.bytes.get(at) == Some(&b'-')
                && at + 1 < self.bytes.len()
                && self.bytes[at + 1] != b']'
            {
                let end = literal_end(self.bytes, at + 1);
                if member.len() == 1 && end - (at + 1) == 1 {
                    members.push(Member::Range(member[0], self.bytes[at + 1]));
                } else {
                    members.push(Member::Character(member.to_vec()));
                    members.push(Member::Character(self.bytes[at + 1..end].to_vec()));
                }
                at = end;
                continue;
            }
            members.push(Member::Character(member.to_vec()));
        }
    }

    /// Recognise `[:class:]`, `[.collate.]` and `[=equivalence=]`.
    fn bracket_member(&self, at: usize, members: &mut Vec<Member>) -> Option<usize> {
        let delimiter = *self.bytes.get(at + 1)?;
        if !matches!(delimiter, b':' | b'.' | b'=') {
            return None;
        }
        let mut close = at + 2;
        while close + 1 < self.bytes.len() {
            if self.bytes[close] == delimiter && self.bytes[close + 1] == b']' {
                break;
            }
            close += 1;
        }
        if close + 1 >= self.bytes.len() {
            return None;
        }
        let body = BString::from(&self.bytes[at + 2..close]);
        if body.is_empty() {
            return None;
        }
        members.push(if delimiter == b':' {
            Member::Class(body)
        } else {
            Member::Collating(body)
        });
        Some(close + 2)
    }
}

/// One literal character, treating a malformed multibyte lead as one byte.
fn literal_end(bytes: &[u8], at: usize) -> usize {
    let width = utf8_width(bytes, at);
    (at + width).min(bytes.len())
}

fn utf8_width(bytes: &[u8], at: usize) -> usize {
    let Some(&lead) = bytes.get(at) else {
        return 1;
    };
    let width = match lead {
        0x00..=0x7f => 1,
        0xc2..=0xdf => 2,
        0xe0..=0xef => 3,
        0xf0..=0xf4 => 4,
        _ => return 1,
    };
    if bytes
        .get(at + 1..at + width)
        .is_some_and(|tail| tail.iter().all(|byte| (0x80..0xc0).contains(byte)))
    {
        width
    } else {
        1
    }
}

/// Lowercase one character for a case-folded comparison.
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
    (at + width).min(bytes.len())
}

// ---- matching --------------------------------------------------------

struct Matcher<'a> {
    locale: &'a nsh_platform::Locale,
    subject: &'a [u8],
    groups: Vec<Option<(usize, usize)>>,
    steps: &'a mut u64,
    depth: u32,
    ignore_case: bool,
}

type Continue<'a> = dyn FnMut(&mut Matcher<'_>, usize) -> bool + 'a;

impl Matcher<'_> {
    /// Charge one step, take one frame, and explore the expression.
    ///
    /// Every recursion in this matcher passes through here, so the two
    /// bounds are checked in one place and cannot be reached around by
    /// nesting groups, alternations or repetitions.
    fn matches(&mut self, expr: &Expr, pos: usize, next: &mut Continue<'_>) -> bool {
        *self.steps += 1;
        if *self.steps > STEP_BUDGET || self.depth >= MAX_DEPTH {
            return false;
        }
        self.depth += 1;
        let matched = self.explore(expr, pos, next);
        self.depth -= 1;
        matched
    }

    fn explore(&mut self, expr: &Expr, pos: usize, next: &mut Continue<'_>) -> bool {
        match expr {
            Expr::Empty => next(self, pos),
            Expr::Literal(bytes) => match self.subject.get(pos..pos + bytes.len()) {
                Some(found) if self.same_text(found, bytes) => next(self, pos + bytes.len()),
                _ => false,
            },
            Expr::AnyCharacter | Expr::Bracket(_) => match self.single(expr, pos) {
                Some(end) => next(self, end),
                None => false,
            },
            Expr::Start => pos == 0 && next(self, pos),
            Expr::End => pos == self.subject.len() && next(self, pos),
            Expr::Sequence(parts) => self.match_sequence(parts, pos, next),
            Expr::Alternation(branches) => {
                for branch in branches {
                    if self.matches(branch, pos, next) {
                        return true;
                    }
                }
                false
            }
            Expr::Repeat { body, min, max } => self.match_repeat(body, *min, *max, 0, pos, next),
            Expr::Group { index, body } => self.match_group(*index, body, pos, next),
        }
    }

    fn match_sequence(&mut self, parts: &[Expr], pos: usize, next: &mut Continue<'_>) -> bool {
        match parts.split_first() {
            None => next(self, pos),
            Some((head, tail)) => self.matches(head, pos, &mut |state, at| {
                state.match_sequence(tail, at, next)
            }),
        }
    }

    fn match_group(
        &mut self,
        index: usize,
        body: &Expr,
        pos: usize,
        next: &mut Continue<'_>,
    ) -> bool {
        let saved = self.groups[index];
        let matched = self.matches(body, pos, &mut |state, at| {
            let previous = state.groups[index];
            state.groups[index] = Some((pos, at));
            if next(state, at) {
                return true;
            }
            state.groups[index] = previous;
            false
        });
        if !matched {
            self.groups[index] = saved;
        }
        matched
    }

    fn match_repeat(
        &mut self,
        body: &Expr,
        min: u32,
        max: Option<u32>,
        count: u32,
        pos: usize,
        next: &mut Continue<'_>,
    ) -> bool {
        if count == 0
            && let Some(outcome) = self.repeat_single(body, min, max, pos, next)
        {
            return outcome;
        }
        if count < min {
            return self.matches(body, pos, &mut |state, at| {
                state.match_repeat(body, min, max, count + 1, at, next)
            });
        }
        if max.is_none_or(|max| count < max)
            && self.matches(body, pos, &mut |state, at| {
                at != pos && state.match_repeat(body, min, max, count + 1, at, next)
            })
        {
            return true;
        }
        next(self, pos)
    }

    /// Repeat a single-character atom without recursing once per repetition,
    /// which is what keeps `.*` over a long subject off the call stack.
    fn repeat_single(
        &mut self,
        body: &Expr,
        min: u32,
        max: Option<u32>,
        pos: usize,
        next: &mut Continue<'_>,
    ) -> Option<bool> {
        if !matches!(
            body,
            Expr::AnyCharacter | Expr::Bracket(_) | Expr::Literal(_)
        ) {
            return None;
        }
        let mut offsets = vec![pos];
        let mut at = pos;
        while max.is_none_or(|max| (offsets.len() as u32) <= max) {
            let Some(end) = self.single(body, at) else {
                break;
            };
            at = end;
            offsets.push(at);
        }
        let lowest = min as usize;
        if offsets.len() <= lowest {
            return Some(false);
        }
        for &end in offsets[lowest..].iter().rev() {
            if next(self, end) {
                return Some(true);
            }
        }
        Some(false)
    }

    /// Match one single-character atom, returning where it ends.
    fn single(&self, expr: &Expr, pos: usize) -> Option<usize> {
        if pos >= self.subject.len() {
            return None;
        }
        let end = character_end(self.locale, self.subject, pos);
        match expr {
            Expr::AnyCharacter => Some(end),
            Expr::Literal(bytes) => self
                .subject
                .get(pos..pos + bytes.len())
                .is_some_and(|found| self.same_text(found, bytes))
                .then_some(pos + bytes.len()),
            Expr::Bracket(bracket) => self.bracket_matches(bracket, pos, end).then_some(end),
            _ => None,
        }
    }

    fn bracket_matches(&self, bracket: &Bracket, pos: usize, end: usize) -> bool {
        let character = &self.subject[pos..end];
        let mut found = false;
        for member in &bracket.members {
            found |= match member {
                Member::Character(bytes) => self.same_text(bytes, character),
                Member::Range(low, high) => {
                    character.len() == 1
                        && self
                            .folded_forms(character)
                            .iter()
                            .any(|form| (*low..=*high).contains(&form[0]))
                }
                Member::Class(name) => {
                    self.locale
                        .wide_class_matches(name, &self.subject[pos..], end - pos)
                        == Some(true)
                        || (self.ignore_case
                            && self.folded_forms(character).iter().any(|form| {
                                self.locale.wide_class_matches(name, form, form.len()) == Some(true)
                            }))
                }
                Member::Collating(body) => self.collating_matches(body, character),
            };
            if found {
                break;
            }
        }
        found != bracket.negated
    }

    /// Compare two characters, folding case when `nocasematch` is on.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn same_text(&self, left: &[u8], right: &[u8]) -> bool {
        left == right || (self.ignore_case && fold_case(left) == fold_case(right))
    }

    /// The forms one character takes when case is folded: itself, and both
    /// ASCII cases when `nocasematch` is on.
    fn folded_forms(&self, character: &[u8]) -> Vec<Vec<u8>> {
        let mut forms = vec![character.to_vec()];
        if self.ignore_case {
            for folded in [
                character.to_ascii_lowercase(),
                character.to_ascii_uppercase(),
            ] {
                if !forms.contains(&folded) {
                    forms.push(folded);
                }
            }
        }
        forms
    }

    fn collating_matches(&self, body: &BString, character: &[u8]) -> bool {
        let mut expression = Vec::with_capacity(body.len() + 6);
        expression.extend_from_slice(b"[[=");
        expression.extend_from_slice(body);
        expression.extend_from_slice(b"=]]");
        self.locale
            .collating_bracket_matches(&expression, character)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn compile(pattern: &[u8]) -> Regex {
        Regex::compile(pattern, &vec![false; pattern.len()], false).expect("pattern compiles")
    }

    fn find(pattern: &[u8], subject: &[u8]) -> Option<Vec<Option<(usize, usize)>>> {
        let locale = nsh_platform::Locale::c().expect("the C locale exists");
        compile(pattern)
            .search(&locale, subject)
            .map(|captures| captures.groups)
    }

    #[test]
    fn a_match_is_unanchored_at_both_ends() {
        assert!(find(b"a", b"bar").is_some());
        assert!(find(b"X", b"bar").is_none());
    }

    #[test]
    fn groups_report_their_spans() {
        let groups = find(b"([a-z]+)([0-9]+)", b"foo123").expect("the pattern matches");
        assert_eq!(groups[0], Some((0, 6)));
        assert_eq!(groups[1], Some((0, 3)));
        assert_eq!(groups[2], Some((3, 6)));
    }

    #[test]
    fn alternation_takes_the_longest_match() {
        let groups = find(b"a|ab", b"ab").expect("the pattern matches");
        assert_eq!(groups[0], Some((0, 2)));
    }

    #[test]
    fn nested_empty_groups_are_retained() {
        let groups = find(b"([a-z]+)(()z)", b"zz").expect("the pattern matches");
        assert_eq!(groups[1], Some((0, 1)));
        assert_eq!(groups[3], Some((1, 1)));
    }

    #[test]
    fn quoted_bytes_lose_their_syntax() {
        let regex = Regex::compile(b"a|b", &[false, true, false], false).expect("pattern compiles");
        let locale = nsh_platform::Locale::c().expect("the C locale exists");
        assert!(regex.search(&locale, b"a|b").is_some());
        assert!(regex.search(&locale, b"a").is_none());
    }

    #[test]
    fn malformed_expressions_are_rejected() {
        for pattern in [b"*".as_slice(), b"{", b")a(", b"[abc", b"a{2,1}"] {
            let quoted = vec![false; pattern.len()];
            assert!(Regex::compile(pattern, &quoted, false).is_err());
        }
    }

    #[test]
    fn intervals_bound_repetition() {
        assert!(find(b"^a{2,3}$", b"aa").is_some());
        assert!(find(b"^a{2,3}$", b"a").is_none());
        assert!(find(b"^a{2,3}$", b"aaaa").is_none());
    }

    // [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
    #[test]
    fn a_bracket_expression_ignores_shell_quoting() {
        let pattern = b"[a-z]";
        let quoted = vec![false, true, true, true, false];
        let regex = Regex::compile(pattern, &quoted, false).expect("pattern compiles");
        let locale = nsh_platform::Locale::c().expect("the C locale exists");
        assert!(regex.search(&locale, b"b").is_some());
        assert!(regex.search(&locale, b"-").is_none());
    }

    #[test]
    fn anchors_and_classes_apply() {
        assert!(find(b"^[[:digit:]]+$", b"1234").is_some());
        assert!(find(b"^[[:digit:]]+$", b"12a4").is_none());
        assert!(find(b"^[^0-9]+$", b"abc").is_some());
    }

    #[test]
    fn a_long_subject_does_not_exhaust_the_stack() {
        let subject = vec![b'x'; 200_000];
        assert!(find(b"^x*$", &subject).is_some());
    }

    /// A quantified *group* cannot be repeated without recursion, so this
    /// is the shape that reaches the stack. The bound turns it into a
    /// shorter match rather than a crash.
    // [spec:nsh:req:compat.bash.safe-core/test]
    #[test]
    fn a_quantified_group_is_depth_bounded() {
        let subject = vec![b'a'; 200_000];
        let groups = find(b"(a)+", &subject).expect("a truncated match is still a match");
        let (start, end) = groups[0].expect("the whole match has a span");
        assert_eq!(start, 0);
        assert!(end > 0 && end < subject.len());
    }

    /// One budget for the whole search: an attempt at every one of `n`
    /// start offsets must not buy `n` budgets.
    // [spec:nsh:req:compat.bash.safe-core/test]
    #[test]
    fn one_budget_covers_every_start_offset() {
        let subject = vec![b'a'; 20_000];
        assert!(find(b"(a*)*b", &subject).is_none());
    }
}
