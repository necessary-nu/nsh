//! Structural shell words.
//!
//! A parsed word is shell syntax, not a byte string with private opcodes.
//! Literal bytes remain byte-preserving, while quoting and every expansion
//! form have their own variants. Command substitutions live at the position
//! where they occur instead of in a second, parallel list.

use bstr::{BStr, BString, ByteSlice};

use crate::nodes::Node;

/// A word after lexical parsing and before expansion.
// [spec:nsh:def:idiom.word-ir]
#[derive(Clone, Default, PartialEq, Eq)]
pub(crate) struct ParsedWord {
    parts: Vec<WordPart>,
    spelling: BString,
}

/// One structural part of a parsed shell word.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum WordPart {
    /// Bytes that have no additional quoting protection.
    Literal(BString),
    /// One byte protected by a shell escape.
    Escaped(u8),
    /// A complete locale multibyte character, with its quoting protection.
    Multibyte { bytes: BString, escaped: bool },
    /// A quoting boundary.
    Quote(QuoteBoundary),
    /// A parameter expansion.
    Parameter(ParameterExpansion),
    /// A command substitution embedded at its lexical position.
    Command(Option<Box<Node>>),
    /// An arithmetic expansion.
    Arithmetic(Box<ParsedWord>),
}

/// Typed events emitted by the lexer while it constructs a nested word.
///
/// Start/end events are enum variants rather than byte values, so every
/// possible input byte remains ordinary shell data.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum WordToken {
    Literal(u8),
    Escaped(u8),
    Multibyte {
        bytes: BString,
        escaped: bool,
    },
    Quote(QuoteBoundary),
    ParameterStart {
        name: BString,
        operation: ParameterOperation,
        colon: bool,
        indirect: bool,
        /// The bytes that made an invalid expansion invalid, which the name
        /// cannot hold and the operand starts after.
        // [spec:nsh:req:idiom.printable-ast]
        invalid_prefix: BString,
    },
    ParameterEnd,
    Command(Option<Node>),
    ArithmeticStart,
    ArithmeticEnd,
}

/// One sliceable top-level word unit used by Bash-only array syntax.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum WordUnit {
    Literal(u8),
    Part(WordPart),
}

/// Whether a quoting region opens or closes at this position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuoteBoundary {
    Open(QuoteKind),
    Close,
}

/// Which quote opened a run.
///
/// The run's bytes do not say: `'a'` and `"a"` protect the same byte and
/// differ only in what else they would have protected. Printing has to put
/// back the one the source used, so the parser records it.
// [spec:nsh:req:idiom.printable-ast]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuoteKind {
    /// `'...'`
    Single,
    /// `"..."`
    Double,
    /// `$'...'`, whose escapes the lexer has already decoded.
    DollarSingle,
    /// `$"..."`, Bash's locale-translated run.
    DollarDouble,
}

/// A parameter expansion and its optional word operand.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ParameterExpansion {
    pub(crate) name: BString,
    pub(crate) operation: ParameterOperation,
    pub(crate) colon: bool,
    /// Bash's `${!name}`: the named variable holds the name to expand.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) indirect: bool,
    pub(crate) operand: Option<Box<ParsedWord>>,
    /// The bytes that made an invalid expansion invalid.
    ///
    /// `${(M)x}` fails on the `(`, which is neither a name nor part of the
    /// operand after it, so nothing else in the expansion holds it. Printing
    /// without it spells a different failure.
    // [spec:nsh:req:idiom.printable-ast]
    pub(crate) invalid_prefix: BString,
}

/// The operation selected by a parameter expansion.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ParameterOperation {
    Value,
    Default,
    Alternate,
    Error,
    Assign,
    RemoveSmallestSuffix,
    RemoveLargestSuffix,
    RemoveSmallestPrefix,
    RemoveLargestPrefix,
    Length,
    /// Bash's `${name:offset:length}`.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    Substring,
    /// Bash's `${name/pattern/replacement}`.
    SubstituteFirst,
    /// Bash's `${name//pattern/replacement}`.
    SubstituteAll,
    /// Bash's `${name^pattern}`.
    UpperFirst,
    /// Bash's `${name^^pattern}`.
    UpperAll,
    /// Bash's `${name,pattern}`.
    LowerFirst,
    /// Bash's `${name,,pattern}`.
    LowerAll,
    /// Bash's `${name@operator}`.
    Transform,
    Invalid,
}

impl ParsedWord {
    pub(crate) const fn new() -> Self {
        Self {
            parts: Vec::new(),
            spelling: BString::new(Vec::new()),
        }
    }

    /// Construct a word made entirely of literal bytes.
    #[cfg(test)]
    pub(crate) fn literal(bytes: impl Into<BString>) -> Self {
        let bytes = bytes.into();
        let parts = if bytes.is_empty() {
            Vec::new()
        } else {
            vec![WordPart::Literal(bytes.clone())]
        };
        Self {
            parts,
            spelling: bytes,
        }
    }

    /// Construct a quoted parameter expansion without legacy marker bytes.
    pub(crate) fn quoted_parameter(name: impl Into<BString>) -> Self {
        let mut word = Self {
            parts: vec![
                WordPart::Quote(QuoteBoundary::Open(QuoteKind::Double)),
                WordPart::Parameter(ParameterExpansion {
                    name: name.into(),
                    operation: ParameterOperation::Value,
                    colon: false,
                    indirect: false,
                    operand: None,
                    invalid_prefix: BString::new(Vec::new()),
                }),
                WordPart::Quote(QuoteBoundary::Close),
            ],
            spelling: BString::new(Vec::new()),
        };
        word.render_spelling();
        word
    }

    /// Build the structural word represented by typed lexer events.
    pub(crate) fn from_tokens(tokens: Vec<WordToken>) -> Self {
        let mut decoder = TokenDecoder {
            tokens: &tokens,
            at: 0,
        };
        decoder.word_until(TokenBoundary::Word)
    }

    /// Marker-free bytes suitable for grammar checks on plain words.
    pub(crate) fn as_bstr(&self) -> &BStr {
        self.spelling.as_bstr()
    }

    /// Whether this word has the unquoted `name=value` shape recognized by
    /// the shell grammar.  The spelling cache deliberately omits quoting, so
    /// grammar classification must inspect the structural parts instead.
    // [spec:dash:sem:parser.isassignment-fn]
    pub(crate) fn is_assignment(&self, locale: &nsh_platform::Locale) -> bool {
        let mut name_is_empty = true;

        for part in &self.parts {
            let bytes = match part {
                WordPart::Literal(bytes) => bytes.as_slice(),
                WordPart::Multibyte {
                    bytes,
                    escaped: false,
                } => bytes.as_slice(),
                WordPart::Escaped(_)
                | WordPart::Multibyte { escaped: true, .. }
                | WordPart::Quote(_)
                | WordPart::Parameter(_)
                | WordPart::Command(_)
                | WordPart::Arithmetic(_) => return false,
            };

            for &byte in bytes {
                if byte == b'=' {
                    return !name_is_empty;
                }
                let is_name_byte = if name_is_empty {
                    crate::syntax::is_name(locale, byte)
                } else {
                    crate::syntax::is_in_name(locale, byte)
                };
                if !is_name_byte {
                    return false;
                }
                name_is_empty = false;
            }
        }

        false
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    /// Append one literal byte, keeping the spelling cache in step.
    pub(crate) fn push_literal_byte(&mut self, byte: u8) {
        match self.parts.last_mut() {
            Some(WordPart::Literal(bytes)) => bytes.push(byte),
            _ => self
                .parts
                .push(WordPart::Literal(BString::from(vec![byte]))),
        }
        self.spelling.push(byte);
    }

    pub(crate) fn parts(&self) -> &[WordPart] {
        &self.parts
    }

    pub(crate) fn units(&self) -> Vec<WordUnit> {
        let mut units = Vec::new();
        for part in &self.parts {
            match part {
                WordPart::Literal(bytes) => {
                    units.extend(bytes.iter().copied().map(WordUnit::Literal));
                }
                part => units.push(WordUnit::Part(part.clone())),
            }
        }
        units
    }

    pub(crate) fn from_units(units: &[WordUnit]) -> Self {
        let mut parts = Vec::new();
        for unit in units {
            match unit {
                WordUnit::Literal(byte) => push_literal(&mut parts, *byte),
                WordUnit::Part(part) => parts.push(part.clone()),
            }
        }
        finish(parts)
    }

    /// Render a compact shell spelling for diagnostics and job display.
    pub(crate) fn render(&self, output: &mut BString) {
        for part in &self.parts {
            match part {
                WordPart::Literal(bytes) | WordPart::Multibyte { bytes, .. } => {
                    for &byte in bytes.iter() {
                        if matches!(byte, b'\'' | b'\\' | b'"' | b'$') {
                            output.push(b'\\');
                        }
                        output.push(byte);
                    }
                }
                WordPart::Escaped(byte) => {
                    output.push(b'\\');
                    output.push(*byte);
                }
                WordPart::Quote(_) => output.push(b'"'),
                WordPart::Command(_) => output.extend_from_slice(b"$(...)"),
                WordPart::Arithmetic(expression) => {
                    output.extend_from_slice(b"$((");
                    expression.render(output);
                    output.extend_from_slice(b"))");
                }
                WordPart::Parameter(parameter) => parameter.render(output),
            }
        }
    }
}

impl ParameterOperation {
    pub(crate) fn operator(self) -> &'static [u8] {
        match self {
            Self::Value => b"",
            Self::Default => b"-",
            Self::Alternate => b"+",
            Self::Error => b"?",
            Self::Assign => b"=",
            Self::RemoveSmallestSuffix => b"%",
            Self::RemoveLargestSuffix => b"%%",
            Self::RemoveSmallestPrefix => b"#",
            Self::RemoveLargestPrefix => b"##",
            Self::Substring => b":",
            Self::SubstituteFirst => b"/",
            Self::SubstituteAll => b"//",
            Self::UpperFirst => b"^",
            Self::UpperAll => b"^^",
            Self::LowerFirst => b",",
            Self::LowerAll => b",,",
            Self::Transform => b"@",
            Self::Length | Self::Invalid => b"",
        }
    }
}

impl ParameterExpansion {
    fn render(&self, output: &mut BString) {
        output.extend_from_slice(if self.operation == ParameterOperation::Length {
            b"${#"
        } else {
            b"${"
        });
        if self.indirect {
            output.push(b'!');
        }
        output.extend_from_slice(&self.name);
        if self.colon {
            output.push(b':');
        }
        output.extend_from_slice(self.operation.operator());
        if let Some(operand) = &self.operand {
            operand.render(output);
        }
        output.push(b'}');
    }
}

#[derive(Clone, Copy, Eq, PartialEq)]
enum TokenBoundary {
    Word,
    Parameter,
    Arithmetic,
}

struct TokenDecoder<'a> {
    tokens: &'a [WordToken],
    at: usize,
}

impl TokenDecoder<'_> {
    fn word_until(&mut self, boundary: TokenBoundary) -> ParsedWord {
        let mut parts = Vec::new();
        while self.at < self.tokens.len() {
            let token = &self.tokens[self.at];
            self.at += 1;
            match token {
                WordToken::Literal(byte) => push_literal(&mut parts, *byte),
                WordToken::Escaped(byte) => parts.push(WordPart::Escaped(*byte)),
                WordToken::Multibyte { bytes, escaped } => {
                    parts.push(WordPart::Multibyte {
                        bytes: bytes.clone(),
                        escaped: *escaped,
                    });
                }
                WordToken::Quote(quote) => parts.push(WordPart::Quote(*quote)),
                WordToken::ParameterStart {
                    name,
                    operation,
                    colon,
                    indirect,
                    invalid_prefix,
                } => {
                    let operand = (*operation != ParameterOperation::Value)
                        .then(|| Box::new(self.word_until(TokenBoundary::Parameter)));
                    parts.push(WordPart::Parameter(ParameterExpansion {
                        name: name.clone(),
                        operation: *operation,
                        colon: *colon,
                        indirect: *indirect,
                        operand,
                        invalid_prefix: invalid_prefix.clone(),
                    }));
                }
                WordToken::Command(command) => {
                    parts.push(WordPart::Command(command.clone().map(Box::new)));
                }
                WordToken::ArithmeticStart => parts.push(WordPart::Arithmetic(Box::new(
                    self.word_until(TokenBoundary::Arithmetic),
                ))),
                WordToken::ParameterEnd if boundary == TokenBoundary::Parameter => break,
                WordToken::ArithmeticEnd if boundary == TokenBoundary::Arithmetic => break,
                WordToken::ParameterEnd | WordToken::ArithmeticEnd => break,
            }
        }
        finish(parts)
    }
}

fn push_literal(parts: &mut Vec<WordPart>, byte: u8) {
    if let Some(WordPart::Literal(bytes)) = parts.last_mut() {
        bytes.push(byte);
    } else {
        parts.push(WordPart::Literal(BString::from(vec![byte])));
    }
}

fn finish(parts: Vec<WordPart>) -> ParsedWord {
    let mut word = ParsedWord {
        parts,
        spelling: BString::new(Vec::new()),
    };
    word.render_spelling();
    word
}

impl ParsedWord {
    fn render_spelling(&mut self) {
        fn append(word: &ParsedWord, output: &mut BString) {
            for part in &word.parts {
                match part {
                    WordPart::Literal(bytes) | WordPart::Multibyte { bytes, .. } => {
                        output.extend_from_slice(bytes)
                    }
                    WordPart::Escaped(byte) => output.push(*byte),
                    WordPart::Quote(_) => {}
                    WordPart::Command(_) => output.extend_from_slice(b"$(...)"),
                    WordPart::Arithmetic(expression) => {
                        output.extend_from_slice(b"$((");
                        append(expression, output);
                        output.extend_from_slice(b"))");
                    }
                    WordPart::Parameter(parameter) => {
                        output.extend_from_slice(b"${");
                        if parameter.operation == ParameterOperation::Length {
                            output.push(b'#');
                        }
                        if parameter.indirect {
                            output.push(b'!');
                        }
                        output.extend_from_slice(&parameter.name);
                        if parameter.colon {
                            output.push(b':');
                        }
                        output.extend_from_slice(parameter.operation.operator());
                        if let Some(operand) = &parameter.operand {
                            append(operand, output);
                        }
                        output.push(b'}');
                    }
                }
            }
        }

        let mut spelling = BString::new(Vec::new());
        append(self, &mut spelling);
        self.spelling = spelling;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // [spec:nsh:def:idiom.word-ir/test]
    fn typed_tokens_build_nested_word_parts() {
        let word = ParsedWord::from_tokens(vec![
            WordToken::Literal(b'a'),
            WordToken::Quote(QuoteBoundary::Open(QuoteKind::Double)),
            WordToken::ParameterStart {
                name: BString::from("x"),
                operation: ParameterOperation::Default,
                colon: true,
                indirect: false,
                invalid_prefix: BString::new(Vec::new()),
            },
            WordToken::Literal(b'y'),
            WordToken::ParameterEnd,
            WordToken::Quote(QuoteBoundary::Close),
            WordToken::Command(None),
        ]);

        assert!(matches!(word.parts()[0], WordPart::Literal(_)));
        assert!(matches!(
            word.parts()[1],
            WordPart::Quote(QuoteBoundary::Open(QuoteKind::Double))
        ));
        let WordPart::Parameter(parameter) = &word.parts()[2] else {
            panic!("parameter part expected");
        };
        assert_eq!(parameter.operation, ParameterOperation::Default);
        assert!(parameter.colon);
        assert_eq!(parameter.name, BString::from("x"));
        assert!(matches!(word.parts()[4], WordPart::Command(None)));
    }

    #[test]
    fn top_level_units_slice_without_serializing() {
        let word = ParsedWord::from_tokens(vec![
            WordToken::Literal(b'a'),
            WordToken::Escaped(b'*'),
            WordToken::ArithmeticStart,
            WordToken::Literal(b'1'),
            WordToken::Literal(b'+'),
            WordToken::Literal(b'2'),
            WordToken::ArithmeticEnd,
        ]);
        let units = word.units();
        let sliced = ParsedWord::from_units(&units[1..]);
        assert!(matches!(sliced.parts()[0], WordPart::Escaped(b'*')));
        assert!(matches!(sliced.parts()[1], WordPart::Arithmetic(_)));
    }

    #[test]
    fn assignment_recognition_uses_structure() {
        let locale = nsh_platform::Locale::c().unwrap();
        let assignment = ParsedWord::from_tokens(vec![
            WordToken::Literal(b'a'),
            WordToken::Literal(b'='),
            WordToken::Literal(b'b'),
        ]);
        let escaped_equal = ParsedWord::from_tokens(vec![
            WordToken::Literal(b'a'),
            WordToken::Escaped(b'='),
            WordToken::Literal(b'b'),
        ]);
        let quoted_name = ParsedWord::from_tokens(vec![
            WordToken::Quote(QuoteBoundary::Open(QuoteKind::Double)),
            WordToken::Literal(b'a'),
            WordToken::Quote(QuoteBoundary::Close),
            WordToken::Literal(b'='),
            WordToken::Literal(b'b'),
        ]);

        assert!(assignment.is_assignment(&locale));
        assert!(!escaped_equal.is_assignment(&locale));
        assert!(!quoted_name.is_assignment(&locale));
    }
}
