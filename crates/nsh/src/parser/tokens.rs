//! The bytes a parse consumed, kept as tokens instead of dropped.
//!
//! The lexer already read every byte of the input; what it did not do was
//! keep them. Word text survived as far as `ParsedWord::from_tokens` and
//! nothing outside a word -- a keyword, an operator, a blank, a `#`
//! comment, a `\` line continuation, the newline between two commands --
//! was recorded at all. This module is where the reader's output lands so
//! that concatenating a parse's tokens reproduces what the parser read.
//!
//! Segmentation is driven by the parser, because a shell cannot be
//! tokenized ahead of one: a here-document body, a `${...}` operand and a
//! `=~` operand each have their own lexical rules and only the parser knows
//! which applies. The reader appends bytes; the parser says where the cuts
//! fall.

use bstr::{BStr, BString};

use crate::context::Shell;

/// What a run of consumed bytes was read as.
///
/// The distinctions are the reader's, not a second grammar: each variant
/// names a branch the lexer already took, so nothing here decides what a
/// byte means.
// [spec:nsh:def:idiom.token-stream]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SourceTokenKind {
    /// Unquoted spaces and tabs between two tokens.
    Blank,
    /// A `#` comment, up to but not including the newline that ends it.
    Comment,
    /// A `\` and the newline it cancels.
    LineContinuation,
    /// A newline the grammar sees.
    Newline,
    /// An operator, a reserved word, or any other non-word token.
    Operator,
    /// A word, with its quoting, expansions and nested commands.
    Word,
    /// A here-document body together with the delimiter line that ends it.
    HereDocument,
}

impl SourceTokenKind {
    /// Whether this run is between tokens rather than one of them.
    ///
    /// Trivia has no grammar position of its own, so it is claimed by the
    /// node that follows it rather than by the one it trails. That is what
    /// makes two consecutive nodes' runs meet instead of leaving the blank
    /// between them owned by nobody.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) const fn is_trivia(self) -> bool {
        matches!(
            self,
            SourceTokenKind::Blank
                | SourceTokenKind::Comment
                | SourceTokenKind::LineContinuation
                | SourceTokenKind::Newline
        )
    }
}

/// One run of consumed bytes, owned.
///
/// Owned rather than a span because there is no single buffer to span
/// into: `crate::input` supplies bytes from strings, files, terminals and
/// alias expansions interleaved, and a here-document body is read long
/// after the redirection that named it.
// [spec:nsh:def:idiom.token-stream]
#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct SourceToken {
    kind: SourceTokenKind,
    text: BString,
}

impl SourceToken {
    /// What the reader was doing when it consumed these bytes.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) const fn kind(&self) -> SourceTokenKind {
        self.kind
    }

    /// The bytes themselves, exactly as they were read.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn text(&self) -> &BStr {
        BStr::new(self.text.as_slice())
    }
}

/// Every byte one parse consumed, in order, cut into tokens.
///
/// Recording is bound to the input frame the parse started on. A frame
/// pushed underneath it -- the string `parsebackq` re-reads a legacy
/// backquote from, the empty string that closes a `$(...)` -- is a second
/// reading of bytes already recorded, so it is not recorded again.
// [spec:nsh:def:idiom.token-stream]
#[derive(Clone, Debug, Default)]
pub(crate) struct TokenLog {
    tokens: Vec<SourceToken>,
    /// Bytes read since the last cut. The parser has not yet said what
    /// token they belong to.
    pending: Vec<u8>,
    /// The input frame being recorded, while a parse is in progress.
    frame: Option<usize>,
    /// Where the token the reader last returned begins.
    ///
    /// The end of the log when that token consumed no bytes, which is
    /// what end of input is, and what a token the parser pushed back and
    /// was handed again is. Push-back cannot be undone by counting back
    /// one token, because those two cut nothing to count back over.
    returned: usize,
}

impl TokenLog {
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) const fn new() -> Self {
        Self {
            tokens: Vec::new(),
            pending: Vec::new(),
            frame: None,
            returned: 0,
        }
    }

    /// Start recording a parse reading from `frame`, discarding the last.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn begin(&mut self, frame: usize) {
        self.tokens.clear();
        self.pending.clear();
        self.frame = Some(frame);
        self.returned = 0;
    }

    /// Stop recording, keeping whatever the parse got through.
    ///
    /// Bytes still pending belong to a token the parser never finished,
    /// which happens when a parse ends in a syntax error. They are kept as
    /// a word rather than dropped, because dropping them would make the
    /// log claim the parser read less than it did.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn seal(&mut self) {
        self.cut(SourceTokenKind::Word);
        self.frame = None;
    }

    /// Record one byte the reader handed to the parser.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn record(&mut self, frame: usize, byte: u8) {
        if self.frame == Some(frame) {
            self.pending.push(byte);
        }
    }

    /// Give back `count` bytes the parser pushed onto the input again.
    ///
    /// The reader hands a byte over before the parser knows whether it
    /// wanted it, so a push-back has to reach back across a cut that has
    /// already happened: `;` is read, then the byte after it, and only
    /// then is the token known to be `;` alone.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn unrecord(&mut self, frame: usize, count: usize) {
        if self.frame != Some(frame) {
            return;
        }
        let mut remaining = count;
        while remaining > 0 {
            if self.pending.is_empty() {
                let Some(last) = self.tokens.pop() else {
                    debug_assert!(false, "push-back of a byte that was never read");
                    return;
                };
                self.pending = last.text.into();
                continue;
            }
            let keep = self.pending.len().saturating_sub(remaining);
            remaining -= self.pending.len() - keep;
            self.pending.truncate(keep);
        }
    }

    /// Drop the token holding an alias name the input has replaced.
    ///
    /// The expansion is pushed onto the input and read like any other
    /// text, so keeping the name too would record the same command twice:
    /// once as written and once as substituted. Which of the two the
    /// tokens are is settled -- they are the expansion, and reproducing
    /// what was typed is a further decision that has not been taken.
    ///
    /// Declined unless the last token really is that name, because a token
    /// the parser pushed back and read again consumed no bytes and left an
    /// unrelated token last.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn retract_alias_name(&mut self, name: &BStr) {
        let matches = self.pending.is_empty()
            && self
                .tokens
                .last()
                .is_some_and(|last| last.kind == SourceTokenKind::Word && last.text == name);
        if matches {
            self.tokens.pop();
        }
    }

    /// Close the pending bytes as one token.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn cut(&mut self, kind: SourceTokenKind) {
        let length = self.pending.len();
        self.cut_head(length, kind);
    }

    /// Close the first `length` pending bytes as one token.
    ///
    /// The trivia a token is reached through is read by the same call that
    /// reads the token's first byte, so the cut between them lands behind
    /// the reader's position rather than at it.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn cut_head(&mut self, length: usize, kind: SourceTokenKind) {
        if length == 0 {
            return;
        }
        debug_assert!(length <= self.pending.len());
        let rest = self.pending.split_off(length.min(self.pending.len()));
        let text = BString::from(core::mem::replace(&mut self.pending, rest));
        self.tokens.push(SourceToken { kind, text });
    }

    /// Close the pending bytes as the token the reader just returned.
    ///
    /// The kind is the reader's own answer relabelled, not a second
    /// reading of the bytes: everything that is not a word or a newline
    /// reached the parser as an operator or a reserved word.
    ///
    /// `replayed` says the reader handed back a token the parser had
    /// pushed on again rather than reading one. It consumed no bytes and
    /// is already in the log, so it does not move where the last token
    /// begins; a fresh read that consumed no bytes -- end of input --
    /// does, because there is no token of it to point at.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn cut_token(&mut self, kind: crate::parser::TokenKind, replayed: bool) {
        if !replayed {
            self.returned = self.tokens.len();
        }
        self.cut(match kind {
            crate::parser::TokenKind::Word => SourceTokenKind::Word,
            crate::parser::TokenKind::Newline => SourceTokenKind::Newline,
            _ => SourceTokenKind::Operator,
        });
    }

    /// How many bytes have been read since the last cut.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn pending_length(&self) -> usize {
        self.pending.len()
    }

    /// The tokens of the parse that just ran, all of them.
    ///
    /// A node is given the run it was parsed from and nothing shipping
    /// wants the whole log, so the callers are the property tests that
    /// check the log against the input frame's own cursor.
    // [spec:nsh:def:idiom.token-stream]
    #[cfg(test)]
    pub(crate) fn tokens(&self) -> &[SourceToken] {
        &self.tokens
    }

    /// Close the pending bytes as one token and hand back that token.
    ///
    /// A here-document's body is the one thing read outside the order the
    /// grammar reads in -- at the newline that ends the redirection's
    /// line, and not where the redirection was -- so it is the one node
    /// whose run is the token just cut rather than everything since a
    /// mark.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn cut_run(&mut self, kind: SourceTokenKind) -> crate::nodes::SourceTokens {
        self.cut(kind);
        crate::nodes::SourceTokens::new(self.tokens.last().map_or(&[], core::slice::from_ref))
    }
}

/// A position in a [`TokenLog`], taken before a node is parsed.
///
/// Opaque because the only thing a caller may do with one is hand it back
/// to [`run`]; arithmetic on it would be a second opinion about where a
/// node begins.
// [spec:nsh:def:idiom.token-stream]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TokenMark(usize);

/// Where the node about to be parsed begins.
///
/// A token the parser read as lookahead and pushed back will be handed to
/// that node, so the mark sits behind it -- at where the reader's last
/// token begins, which is not one token back from the end: end of input
/// cuts nothing to count back over, and a token pushed back twice is
/// already in the log. It sits behind the trivia in front of it too,
/// because a blank belongs to what follows it, which is what makes two
/// consecutive nodes' runs meet rather than leave it owned by nobody.
// [spec:nsh:def:idiom.token-stream]
pub(crate) fn mark(shell: &Shell) -> TokenMark {
    let log = &shell.input.tokens;
    let mut at = if shell.input.token_pushed_back {
        log.returned.min(log.tokens.len())
    } else {
        log.tokens.len()
    };
    while at > 0 && log.tokens[at - 1].kind().is_trivia() {
        at -= 1;
    }
    TokenMark(at)
}

/// The run of tokens the node just parsed was read from.
///
/// The end is [`mark`] again: whatever the parser has pushed back belongs
/// to what is parsed next, and so does the trivia in front of it.
// [spec:nsh:def:idiom.token-stream]
pub(crate) fn run(shell: &Shell, start: TokenMark) -> crate::nodes::SourceTokens {
    let TokenMark(start) = start;
    let TokenMark(end) = mark(shell);
    crate::nodes::SourceTokens::new(&shell.input.tokens.tokens[start.min(end)..end])
}
