//! Structural shell words.
//!
//! A parsed word is shell syntax, not a byte string with private opcodes.
//! Literal bytes remain byte-preserving, while quoting and every expansion
//! form have their own variants. Command substitutions live at the position
//! where they occur instead of in a second, parallel list.

use bstr::{BStr, BString, ByteSlice};

use crate::nodes::Node;

const LEGACY_ESCAPE: u8 = (-127_i8) as u8;
const LEGACY_PARAMETER: u8 = (-126_i8) as u8;
const LEGACY_END_PARAMETER: u8 = (-125_i8) as u8;
const LEGACY_COMMAND: u8 = (-124_i8) as u8;
const LEGACY_MULTIBYTE: u8 = (-123_i8) as u8;
const LEGACY_ARITHMETIC: u8 = (-122_i8) as u8;
const LEGACY_END_ARITHMETIC: u8 = (-121_i8) as u8;
const LEGACY_QUOTE: u8 = (-120_i8) as u8;

const LEGACY_KIND_MASK: u8 = 0x0f;
const LEGACY_COLON: u8 = 0x10;
const LEGACY_PRESENT: u8 = 0x20;

/// A word after lexical parsing and before expansion.
// [spec:nsh:def:idiom.word-ir]
#[derive(Clone, Default)]
pub(crate) struct ParsedWord {
    parts: Vec<WordPart>,
    spelling: BString,
}

/// One structural part of a parsed shell word.
#[derive(Clone)]
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

/// Whether a quoting region opens or closes at this position.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QuoteBoundary {
    Open,
    Close,
}

/// A parameter expansion and its optional word operand.
#[derive(Clone)]
pub(crate) struct ParameterExpansion {
    pub(crate) name: BString,
    pub(crate) operation: ParameterOperation,
    pub(crate) colon: bool,
    pub(crate) operand: Option<Box<ParsedWord>>,
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
    Invalid,
}

/// Temporary input for the legacy expansion engine.
///
/// This is deliberately owned and short-lived: the syntax tree never stores
/// either the control-byte stream or a parallel substitution list.
pub(crate) struct EncodedWord {
    pub(crate) bytes: BString,
    pub(crate) substitutions: Vec<Option<Node>>,
}

impl ParsedWord {
    pub(crate) const fn new() -> Self {
        Self {
            parts: Vec::new(),
            spelling: BString::new(Vec::new()),
        }
    }

    /// Construct a word made entirely of literal bytes.
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
                WordPart::Quote(QuoteBoundary::Open),
                WordPart::Parameter(ParameterExpansion {
                    name: name.into(),
                    operation: ParameterOperation::Value,
                    colon: false,
                    operand: None,
                }),
                WordPart::Quote(QuoteBoundary::Close),
            ],
            spelling: BString::new(Vec::new()),
        };
        word.render_spelling();
        word
    }

    /// Decode one sliced legacy word while preserving its substitutions.
    pub(crate) fn from_legacy_fragment(bytes: &[u8], substitutions: Vec<Option<Node>>) -> Self {
        let mut decoder = Decoder {
            bytes,
            at: 0,
            substitutions: substitutions.into_iter(),
        };
        let word = decoder.word_until(None);
        debug_assert!(decoder.substitutions.next().is_none());
        word
    }

    /// Marker-free bytes suitable for grammar checks on plain words.
    pub(crate) fn as_bstr(&self) -> &BStr {
        self.spelling.as_bstr()
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.parts.is_empty()
    }

    pub(crate) fn parts(&self) -> &[WordPart] {
        &self.parts
    }

    /// Serialize for the old expansion implementation while it is being
    /// replaced. The result never enters the syntax tree.
    pub(crate) fn encode_legacy(&self) -> EncodedWord {
        let mut encoded = EncodedWord {
            bytes: BString::new(Vec::new()),
            substitutions: Vec::new(),
        };
        self.encode_into(&mut encoded);
        encoded
    }

    fn encode_into(&self, encoded: &mut EncodedWord) {
        for part in &self.parts {
            match part {
                WordPart::Literal(bytes) => encoded.bytes.extend_from_slice(bytes),
                WordPart::Escaped(byte) => {
                    encoded.bytes.push(LEGACY_ESCAPE);
                    encoded.bytes.push(*byte);
                }
                WordPart::Multibyte { bytes, escaped } => {
                    encoded.bytes.push(LEGACY_MULTIBYTE);
                    if *escaped {
                        encoded.bytes.push(LEGACY_ESCAPE);
                    }
                    encoded.bytes.push(bytes.len() as u8);
                    encoded.bytes.extend_from_slice(bytes);
                    encoded.bytes.push(bytes.len() as u8);
                    encoded.bytes.push(LEGACY_MULTIBYTE);
                }
                WordPart::Quote(_) => encoded.bytes.push(LEGACY_QUOTE),
                WordPart::Parameter(parameter) => {
                    encoded.bytes.push(LEGACY_PARAMETER);
                    let flags = parameter.operation.legacy_kind()
                        | if parameter.colon { LEGACY_COLON } else { 0 }
                        | LEGACY_PRESENT;
                    encoded.bytes.push(flags);
                    encoded.bytes.extend_from_slice(&parameter.name);
                    encoded.bytes.push(b'=');
                    if let Some(operand) = &parameter.operand {
                        operand.encode_into(encoded);
                    }
                    if parameter.operation != ParameterOperation::Value {
                        encoded.bytes.push(LEGACY_END_PARAMETER);
                    }
                }
                WordPart::Command(command) => {
                    encoded.bytes.push(LEGACY_COMMAND);
                    encoded.substitutions.push(command.as_deref().cloned());
                }
                WordPart::Arithmetic(expression) => {
                    encoded.bytes.push(LEGACY_ARITHMETIC);
                    expression.encode_into(encoded);
                    encoded.bytes.push(LEGACY_END_ARITHMETIC);
                }
            }
        }
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
    fn from_legacy(kind: u8) -> Self {
        match kind {
            1 => Self::Value,
            2 => Self::Default,
            3 => Self::Alternate,
            4 => Self::Error,
            5 => Self::Assign,
            6 => Self::RemoveSmallestSuffix,
            7 => Self::RemoveLargestSuffix,
            8 => Self::RemoveSmallestPrefix,
            9 => Self::RemoveLargestPrefix,
            10 => Self::Length,
            _ => Self::Invalid,
        }
    }

    fn legacy_kind(self) -> u8 {
        match self {
            Self::Invalid => 0,
            Self::Value => 1,
            Self::Default => 2,
            Self::Alternate => 3,
            Self::Error => 4,
            Self::Assign => 5,
            Self::RemoveSmallestSuffix => 6,
            Self::RemoveLargestSuffix => 7,
            Self::RemoveSmallestPrefix => 8,
            Self::RemoveLargestPrefix => 9,
            Self::Length => 10,
        }
    }

    fn operator(self) -> &'static [u8] {
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

struct Decoder<'a, I> {
    bytes: &'a [u8],
    at: usize,
    substitutions: I,
}

impl<I> Decoder<'_, I>
where
    I: Iterator<Item = Option<Node>>,
{
    fn word_until(&mut self, stop: Option<u8>) -> ParsedWord {
        let mut parts = Vec::new();
        // Quote boundaries are local to each structural word. A parameter
        // operand begins its own word even when the containing expansion is
        // inside double quotes, so its first marker is still an opening
        // boundary rather than a close inherited from the parent word.
        let mut quoted = false;
        while self.at < self.bytes.len() {
            let byte = self.bytes[self.at];
            if Some(byte) == stop {
                self.at += 1;
                break;
            }
            self.at += 1;
            match byte {
                LEGACY_ESCAPE => {
                    if let Some(&escaped) = self.bytes.get(self.at) {
                        self.at += 1;
                        parts.push(WordPart::Escaped(escaped));
                    }
                }
                LEGACY_MULTIBYTE => self.decode_multibyte(&mut parts),
                LEGACY_QUOTE => {
                    let boundary = if quoted {
                        QuoteBoundary::Close
                    } else {
                        QuoteBoundary::Open
                    };
                    quoted = !quoted;
                    parts.push(WordPart::Quote(boundary));
                }
                LEGACY_PARAMETER => parts.push(WordPart::Parameter(self.decode_parameter())),
                LEGACY_COMMAND => parts.push(WordPart::Command(
                    self.substitutions.next().flatten().map(Box::new),
                )),
                LEGACY_ARITHMETIC => {
                    parts.push(WordPart::Arithmetic(Box::new(
                        self.word_until(Some(LEGACY_END_ARITHMETIC)),
                    )));
                }
                LEGACY_END_PARAMETER | LEGACY_END_ARITHMETIC => break,
                0 if self.at == self.bytes.len() => break,
                ordinary => Self::push_literal(&mut parts, ordinary),
            }
        }
        Self::finish(parts)
    }

    fn decode_parameter(&mut self) -> ParameterExpansion {
        let flags = self.bytes.get(self.at).copied().unwrap_or(LEGACY_PRESENT);
        self.at += usize::from(self.at < self.bytes.len());
        let operation = ParameterOperation::from_legacy(flags & LEGACY_KIND_MASK);
        let name_start = self.at;
        while self.at < self.bytes.len() && self.bytes[self.at] != b'=' {
            self.at += 1;
        }
        let name = BString::from(&self.bytes[name_start..self.at]);
        self.at += usize::from(self.at < self.bytes.len());
        let operand = if operation == ParameterOperation::Value {
            None
        } else {
            Some(Box::new(self.word_until(Some(LEGACY_END_PARAMETER))))
        };
        ParameterExpansion {
            name,
            operation,
            colon: flags & LEGACY_COLON != 0,
            operand,
        }
    }

    fn decode_multibyte(&mut self, parts: &mut Vec<WordPart>) {
        let escaped = self.bytes.get(self.at) == Some(&LEGACY_ESCAPE);
        self.at += usize::from(escaped);
        let length = self.bytes.get(self.at).copied().unwrap_or(0) as usize;
        self.at += usize::from(self.at < self.bytes.len());
        let end = self.at.saturating_add(length).min(self.bytes.len());
        let bytes = BString::from(&self.bytes[self.at..end]);
        self.at = end.saturating_add(2).min(self.bytes.len());
        parts.push(WordPart::Multibyte { bytes, escaped });
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
    fn legacy_transport_decodes_to_typed_parts() {
        let encoded = BString::from(vec![
            b'a',
            LEGACY_QUOTE,
            LEGACY_PARAMETER,
            LEGACY_PRESENT | 2 | LEGACY_COLON,
            b'x',
            b'=',
            b'y',
            LEGACY_END_PARAMETER,
            LEGACY_QUOTE,
            LEGACY_COMMAND,
        ]);
        let word = ParsedWord::from_legacy_fragment(&encoded, vec![None]);

        assert!(matches!(word.parts()[0], WordPart::Literal(_)));
        assert!(matches!(
            word.parts()[1],
            WordPart::Quote(QuoteBoundary::Open)
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
    fn compatibility_encoding_round_trips() {
        let original = BString::from(vec![
            LEGACY_QUOTE,
            b'a',
            LEGACY_ESCAPE,
            b'*',
            LEGACY_QUOTE,
            LEGACY_ARITHMETIC,
            b'1',
            b'+',
            b'2',
            LEGACY_END_ARITHMETIC,
        ]);
        let word = ParsedWord::from_legacy_fragment(&original, Vec::new());
        assert_eq!(word.encode_legacy().bytes, original);
    }
}
