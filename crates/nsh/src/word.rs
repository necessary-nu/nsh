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
#[derive(Clone, Default, Eq)]
pub(crate) struct ParsedWord {
    parts: Vec<WordPart>,
    /// The parts flattened to bytes, for the grammar checks that want a
    /// name rather than a structure.
    ///
    /// Derived, and kept only because the parser asks for it once per
    /// token and wants to borrow it. It is a function of the parts, so
    /// it answers no question they do not.
    spelling: BString,
}

/// Two words are equal when they are the same program.
///
/// The spelling cache is derived from the parts, so comparing it as well
/// would ask the same question twice -- and a cache that ever drifted
/// would then decide equality, which is not a thing a cache may do.
// [spec:nsh:req:idiom.canonical-tree+1]
impl PartialEq for ParsedWord {
    fn eq(&self, other: &ParsedWord) -> bool {
        self.parts == other.parts
    }
}

/// One structural part of a parsed shell word.
///
/// What the program is, never how it was spelled. `echo 'a'`, `echo "a"`
/// and `echo \a` are one word here and differ only in the run the node
/// was read as, which is where [`dec:nsh:tokens-are-the-truth`] put the
/// spelling. Four ways to say "this byte, inert" were four shapes of one
/// program, and a representation that admits two forms of one program is
/// not a syntax tree of it.
// [spec:nsh:req:idiom.canonical-tree+1]
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum WordPart {
    /// A run of bytes, and whether the source made them inert.
    ///
    /// One flag covers every way a byte becomes data -- inside `'`,
    /// inside `"`, inside `$'...'`, behind a `\` -- because nothing the
    /// program does distinguishes them. An empty run is `''`: a word
    /// where no run at all is not a word.
    // [spec:nsh:req:idiom.canonical-tree+1]
    Text { bytes: BString, quoted: bool },
    /// A parameter expansion.
    Parameter(ParameterExpansion),
    /// A command substitution embedded at its lexical position.
    Command {
        command: Option<Box<Node>>,
        quoted: bool,
    },
    /// An arithmetic expansion.
    Arithmetic {
        expression: Box<ParsedWord>,
        quoted: bool,
    },
}

impl WordPart {
    /// Whether the source made this part inert.
    ///
    /// For a run of bytes that is whether they are data; for an expansion
    /// it is whether its result splits and globs. One question, because
    /// one thing put it there.
    // [spec:nsh:req:idiom.canonical-tree+1]
    pub(crate) const fn quoted(&self) -> bool {
        match self {
            WordPart::Text { quoted, .. }
            | WordPart::Command { quoted, .. }
            | WordPart::Arithmetic { quoted, .. } => *quoted,
            WordPart::Parameter(expansion) => expansion.quoted,
        }
    }
}

/// What the reader emits while it builds a word, before nesting.
///
/// Flat and byte-indexed on purpose: this is the reader's undo log, not a
/// second model of a word. Ten sites rewind it -- a `${` that turned out
/// not to open an expansion, a `$(` `(` that turned out to be arithmetic,
/// a `$'...'` cut short by a NUL, a name whose bytes are retyped as one
/// `ParameterStart` -- and a rewind is a truncation to a position, which
/// a tree of merged runs cannot offer. It carries the same vocabulary the
/// tree does, so neither admits two shapes of one program.
///
/// Start/end events are enum variants rather than byte values, so every
/// possible input byte remains ordinary shell data.
// [spec:nsh:req:idiom.canonical-tree+1]
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum WordToken {
    /// One byte as the source wrote it.
    Literal(u8),
    /// One byte the source made inert, however it did so.
    Inert(u8),
    /// One complete locale character, and whether it is inert.
    Character {
        bytes: BString,
        inert: bool,
    },
    /// A quote opening: a depth, not a kind. Which quote it was is the
    /// node's run.
    QuoteOpen,
    /// The quote closing again.
    QuoteClose,
    ParameterStart {
        name: BString,
        operation: ParameterOperation,
        colon: bool,
        indirect: bool,
    },
    ParameterEnd,
    Command(Option<Node>),
    ArithmeticStart,
    ArithmeticEnd,
}

/// One sliceable top-level word unit used by Bash-only array syntax.
///
/// A byte carries its run's inertness with it, because slicing a run in
/// half must not make either half ordinary.
// [spec:nsh:req:idiom.canonical-tree+1]
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum WordUnit {
    Literal { byte: u8, quoted: bool },
    Part(WordPart),
}

/// A parameter expansion and its optional word operand.
///
/// What the expansion does, and not how it was written. An expansion the
/// shell refuses is `Invalid` and nothing more: the `!`, the `#` and the
/// byte that made it invalid were kept here so a rejected `${(M)x}` could
/// be written back as the failure the source spelled, and the word's run
/// is that now -- for every construct rather than for this one.
// [spec:nsh:req:idiom.canonical-tree+1]
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct ParameterExpansion {
    pub(crate) name: BString,
    pub(crate) operation: ParameterOperation,
    pub(crate) colon: bool,
    /// Bash's `${!name}`: the named variable holds the name to expand.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) indirect: bool,
    pub(crate) operand: Option<Box<ParsedWord>>,
    /// Whether the source quoted the expansion, so its result is one
    /// field rather than several.
    // [spec:nsh:req:idiom.canonical-tree+1]
    pub(crate) quoted: bool,
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
            vec![WordPart::Text {
                bytes: bytes.clone(),
                quoted: false,
            }]
        };
        Self {
            parts,
            spelling: bytes,
        }
    }

    /// Construct a quoted parameter expansion without legacy marker bytes.
    pub(crate) fn quoted_parameter(name: impl Into<BString>) -> Self {
        let mut word = Self {
            parts: vec![WordPart::Parameter(ParameterExpansion {
                name: name.into(),
                operation: ParameterOperation::Value,
                colon: false,
                indirect: false,
                operand: None,
                quoted: true,
            })],
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
                WordPart::Text {
                    bytes,
                    quoted: false,
                } => bytes.as_slice(),
                WordPart::Text { quoted: true, .. }
                | WordPart::Parameter(_)
                | WordPart::Command { .. }
                | WordPart::Arithmetic { .. } => return false,
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
        push_text(&mut self.parts, &[byte], false);
        self.spelling.push(byte);
    }

    pub(crate) fn parts(&self) -> &[WordPart] {
        &self.parts
    }

    /// Split the word into sliceable units.
    ///
    /// An empty run has no bytes to spread out and stays one unit, so
    /// that slicing cannot silently drop the `''` it stands for.
    // [spec:nsh:req:idiom.canonical-tree+1]
    pub(crate) fn units(&self) -> Vec<WordUnit> {
        let mut units = Vec::new();
        for part in &self.parts {
            match part {
                WordPart::Text { bytes, quoted } if !bytes.is_empty() => {
                    units.extend(bytes.iter().map(|byte| WordUnit::Literal {
                        byte: *byte,
                        quoted: *quoted,
                    }));
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
                WordUnit::Literal { byte, quoted } => push_text(&mut parts, &[*byte], *quoted),
                WordUnit::Part(WordPart::Text { bytes, quoted }) => {
                    push_text(&mut parts, bytes, *quoted);
                }
                WordUnit::Part(part) => parts.push(part.clone()),
            }
        }
        finish(parts)
    }

    /// Render a compact shell spelling for diagnostics and job display.
    pub(crate) fn render(&self, output: &mut BString) {
        for part in &self.parts {
            match part {
                WordPart::Text { bytes, .. } => {
                    for &byte in bytes.iter() {
                        if matches!(byte, b'\'' | b'\\' | b'"' | b'$') {
                            output.push(b'\\');
                        }
                        output.push(byte);
                    }
                }
                WordPart::Command { .. } => output.extend_from_slice(b"$(...)"),
                WordPart::Arithmetic { expression, .. } => {
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

/// How deep [`TokenDecoder::word_until`] will recurse over these events.
///
/// The lexer is a loop, so nothing is spent while a word is being read;
/// the whole cost arrives here, one frame per open expansion, and again
/// when the resulting tree is dropped. The parser reads the figure off
/// the events before building the tree, so hostile nesting is refused
/// with the stack still intact. The walk mirrors `word_until` exactly,
/// including its stopping at an end marker the word's own level did not
/// open.
// [spec:nsh:req:idiom.bounded-recursion]
pub(crate) fn expansion_nesting(tokens: &[WordToken]) -> u32 {
    let mut depth = 0u32;
    let mut deepest = 0u32;
    for token in tokens {
        match token {
            WordToken::ParameterStart { operation, .. }
                if *operation != ParameterOperation::Value =>
            {
                depth += 1;
                deepest = deepest.max(depth);
            }
            WordToken::ArithmeticStart => {
                depth += 1;
                deepest = deepest.max(depth);
            }
            WordToken::ParameterEnd | WordToken::ArithmeticEnd => {
                if depth == 0 {
                    break;
                }
                depth -= 1;
            }
            _ => {}
        }
    }
    deepest
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
    /// Un-flatten the lexer's events into the canonical parts.
    ///
    /// Quoting is a depth here rather than a pair of parts: what a run's
    /// inertness is, the tree records; which quote opened it, the node's
    /// run does. A quote that closes over nothing written leaves an empty
    /// inert run behind, because `''` is a word and nothing is not.
    // [spec:nsh:req:idiom.canonical-tree+1]
    fn word_until(&mut self, boundary: TokenBoundary) -> ParsedWord {
        let mut parts = Vec::new();
        let mut depth = 0usize;
        let mut opened: Vec<(usize, usize)> = Vec::new();
        while self.at < self.tokens.len() {
            let token = &self.tokens[self.at];
            self.at += 1;
            match token {
                WordToken::Literal(byte) => push_text(&mut parts, &[*byte], depth > 0),
                WordToken::Inert(byte) => push_text(&mut parts, &[*byte], true),
                WordToken::Character { bytes, inert } => {
                    push_text(&mut parts, bytes, *inert || depth > 0);
                }
                WordToken::QuoteOpen => {
                    depth += 1;
                    opened.push(written_so_far(&parts));
                }
                WordToken::QuoteClose => {
                    depth = depth.saturating_sub(1);
                    if opened.pop() == Some(written_so_far(&parts)) {
                        push_text(&mut parts, &[], true);
                    }
                }
                WordToken::ParameterStart {
                    name,
                    operation,
                    colon,
                    indirect,
                } => {
                    let operand = (*operation != ParameterOperation::Value)
                        .then(|| Box::new(self.word_until(TokenBoundary::Parameter)));
                    parts.push(WordPart::Parameter(ParameterExpansion {
                        name: name.clone(),
                        operation: *operation,
                        colon: *colon,
                        indirect: *indirect,
                        operand,
                        quoted: depth > 0,
                    }));
                }
                WordToken::Command(command) => {
                    parts.push(WordPart::Command {
                        command: command.clone().map(Box::new),
                        quoted: depth > 0,
                    });
                }
                WordToken::ArithmeticStart => {
                    let expression = Box::new(self.word_until(TokenBoundary::Arithmetic));
                    parts.push(WordPart::Arithmetic {
                        expression,
                        quoted: depth > 0,
                    });
                }
                WordToken::ParameterEnd if boundary == TokenBoundary::Parameter => break,
                WordToken::ArithmeticEnd if boundary == TokenBoundary::Arithmetic => break,
                WordToken::ParameterEnd | WordToken::ArithmeticEnd => break,
            }
        }
        finish(parts)
    }
}

/// How much of the word has been built, for telling `""` from `"a"`.
///
/// A quote that closes with this unchanged wrote nothing, and an empty
/// inert run is what stands for it. Counting parts alone is not enough:
/// a byte written into the run already there moves nothing else.
// [spec:nsh:req:idiom.canonical-tree+1]
fn written_so_far(parts: &[WordPart]) -> (usize, usize) {
    let last = match parts.last() {
        Some(WordPart::Text { bytes, .. }) => bytes.len(),
        _ => 0,
    };
    (parts.len(), last)
}

/// Append bytes to the run being built, or start a new one.
///
/// Runs of equal inertness merge, which is what makes the shape
/// canonical: `a'b'` and `'ab'` differ, `ab` written as two literal
/// pushes does not. An empty run merges into a neighbour of the same
/// inertness and survives beside one of the other, which is exactly when
/// `''` is a word of its own.
// [spec:nsh:req:idiom.canonical-tree+1]
fn push_text(parts: &mut Vec<WordPart>, bytes: &[u8], quoted: bool) {
    if let Some(WordPart::Text {
        bytes: run,
        quoted: run_quoted,
    }) = parts.last_mut()
        && *run_quoted == quoted
    {
        run.extend_from_slice(bytes);
        return;
    }
    parts.push(WordPart::Text {
        bytes: BString::from(bytes),
        quoted,
    });
}

fn finish(parts: Vec<WordPart>) -> ParsedWord {
    /* An empty run that nothing quoted is not a run. An empty run that
     * something did is `''`, which is a word. */
    // [spec:nsh:req:idiom.canonical-tree+1]
    let mut parts = parts;
    parts.retain(
        |part| !matches!(part, WordPart::Text { bytes, quoted: false } if bytes.is_empty()),
    );
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
                    WordPart::Text { bytes, .. } => output.extend_from_slice(bytes),
                    WordPart::Command { .. } => output.extend_from_slice(b"$(...)"),
                    WordPart::Arithmetic { expression, .. } => {
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
            WordToken::QuoteOpen,
            WordToken::ParameterStart {
                name: BString::from("x"),
                operation: ParameterOperation::Default,
                colon: true,
                indirect: false,
            },
            WordToken::Literal(b'y'),
            WordToken::ParameterEnd,
            WordToken::QuoteClose,
            WordToken::Command(None),
        ]);

        assert!(matches!(
            word.parts()[0],
            WordPart::Text { quoted: false, .. }
        ));
        let WordPart::Parameter(parameter) = &word.parts()[1] else {
            panic!("parameter part expected");
        };
        assert_eq!(parameter.operation, ParameterOperation::Default);
        assert!(parameter.colon);
        assert!(parameter.quoted, "the expansion was written inside quotes");
        assert_eq!(parameter.name, BString::from("x"));
        assert!(matches!(
            word.parts()[2],
            WordPart::Command {
                command: None,
                quoted: false
            }
        ));
    }

    /// One program, one shape. The three spellings differ in the run the
    /// word was read as and in nothing the tree records.
    // [spec:nsh:req:idiom.canonical-tree+1/test]
    #[test]
    fn one_inert_byte_has_one_shape() {
        let apostrophes = ParsedWord::from_tokens(vec![
            WordToken::QuoteOpen,
            WordToken::Literal(b'a'),
            WordToken::QuoteClose,
        ]);
        let quotes = ParsedWord::from_tokens(vec![
            WordToken::QuoteOpen,
            WordToken::Literal(b'a'),
            WordToken::QuoteClose,
        ]);
        let backslash = ParsedWord::from_tokens(vec![WordToken::Inert(b'a')]);
        assert!(apostrophes == quotes, "'a' and \"a\" are one program");
        assert!(quotes == backslash, "\"a\" and \\a are one program");
        assert!(matches!(
            apostrophes.parts(),
            [WordPart::Text { bytes, quoted: true }] if bytes == "a"
        ));
    }

    /// `''` is a word and nothing is not, so an inert run survives being
    /// empty where an ordinary one does not.
    // [spec:nsh:req:idiom.canonical-tree+1/test]
    #[test]
    fn an_empty_inert_run_is_a_word() {
        let empty = ParsedWord::from_tokens(vec![WordToken::QuoteOpen, WordToken::QuoteClose]);
        assert!(!empty.is_empty());
        assert!(matches!(
            empty.parts(),
            [WordPart::Text { bytes, quoted: true }] if bytes.is_empty()
        ));
        assert!(ParsedWord::from_tokens(Vec::new()).is_empty());
    }

    /// Runs of one inertness join, so a word has one shape however the
    /// lexer happened to cut it.
    // [spec:nsh:req:idiom.canonical-tree+1/test]
    #[test]
    fn runs_of_one_inertness_join() {
        let split = ParsedWord::from_tokens(vec![
            WordToken::QuoteOpen,
            WordToken::Literal(b'a'),
            WordToken::QuoteClose,
            WordToken::QuoteOpen,
            WordToken::Literal(b'b'),
            WordToken::QuoteClose,
        ]);
        let whole = ParsedWord::from_tokens(vec![
            WordToken::QuoteOpen,
            WordToken::Literal(b'a'),
            WordToken::Literal(b'b'),
            WordToken::QuoteClose,
        ]);
        assert!(split == whole, "one run however the lexer cut it");
        assert_eq!(split.parts().len(), 1);
    }

    #[test]
    fn top_level_units_slice_without_serializing() {
        let word = ParsedWord::from_tokens(vec![
            WordToken::Literal(b'a'),
            WordToken::Inert(b'*'),
            WordToken::ArithmeticStart,
            WordToken::Literal(b'1'),
            WordToken::Literal(b'+'),
            WordToken::Literal(b'2'),
            WordToken::ArithmeticEnd,
        ]);
        let units = word.units();
        let sliced = ParsedWord::from_units(&units[1..]);
        assert!(matches!(
            &sliced.parts()[0],
            WordPart::Text { bytes, quoted: true } if bytes == "*"
        ));
        assert!(matches!(sliced.parts()[1], WordPart::Arithmetic { .. }));
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
            WordToken::Inert(b'='),
            WordToken::Literal(b'b'),
        ]);
        let quoted_name = ParsedWord::from_tokens(vec![
            WordToken::QuoteOpen,
            WordToken::Literal(b'a'),
            WordToken::QuoteClose,
            WordToken::Literal(b'='),
            WordToken::Literal(b'b'),
        ]);

        assert!(assignment.is_assignment(&locale));
        assert!(!escaped_equal.is_assignment(&locale));
        assert!(!quoted_name.is_assignment(&locale));
    }
}
