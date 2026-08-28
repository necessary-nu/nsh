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

#[cfg(test)]
use bstr::BStr;
use bstr::BString;

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
    ///
    /// Nothing outside the property test reads a retained token yet: the
    /// tree gains its token field in `carry-tokens-into-the-tree` and the
    /// renderer emits them in `print-by-emitting-tokens`. Retaining them
    /// and draining them are separate changes, and this is the first.
    // [spec:nsh:def:idiom.token-stream]
    #[cfg(test)]
    pub(crate) const fn kind(&self) -> SourceTokenKind {
        self.kind
    }

    /// The bytes themselves, exactly as they were read.
    // [spec:nsh:def:idiom.token-stream]
    #[cfg(test)]
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
}

impl TokenLog {
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) const fn new() -> Self {
        Self {
            tokens: Vec::new(),
            pending: Vec::new(),
            frame: None,
        }
    }

    /// Start recording a parse reading from `frame`, discarding the last.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn begin(&mut self, frame: usize) {
        self.tokens.clear();
        self.pending.clear();
        self.frame = Some(frame);
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
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn cut_token(&mut self, kind: crate::parser::TokenKind) {
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

    /// The tokens of the parse that just ran.
    // [spec:nsh:def:idiom.token-stream]
    #[cfg(test)]
    pub(crate) fn tokens(&self) -> &[SourceToken] {
        &self.tokens
    }
}
