//! Shell parser derived from `src/parser.c` / `src/parser.h`.
//! Rules: `docs/spec/port/src/parser.md`.
//!
//! Parsing uses structural syntax nodes and ordinary Rust control flow.
//! Helpers such as `checkend`, `parseredir`, `parsesub`, `parsebackq`, and
//! `parsearith` operate on the current word-lexer state directly.

use core::mem;

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::expand::{ExpansionMode, expand_argument};
use crate::input::{
    PromptKind, push_string_input, read_input_unit, read_input_unit_or_alias_end, set_input_string,
    unread_input_unit, unread_input_units,
};
use crate::nodes::{
    BinaryCommand, CaseClause, CaseCommand, CompoundCommand, DescriptorRedirection,
    DescriptorRedirectionOperator, DescriptorTarget, FileRedirection, FileRedirectionOperator,
    ForCommand, FunctionDefinition, HereDocument, HereString, IfCommand, NegatedCommand, Node,
    NodeText, Pipeline, Redirection, RedirectionDescriptor, SimpleCommand, SourceLine,
    SourceTokens, TimedCommand, WordNode,
};
use crate::syntax::{InputUnit, SyntaxClass, SyntaxContext, is_in_name, is_name};
use crate::word::{ParameterOperation, ParsedWord, WordToken};

/// `MB_LEN_MAX` from `<limits.h>` (16 on the platforms dash targets).
const MAX_MULTIBYTE_LENGTH: usize = 16;

// [spec:posix:def:grammar.token-symbols]
// [spec:nsh:req:idiom.lexer-tokens]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum TokenKind {
    Eof,
    Blank,
    Newline,
    Semicolon,
    Background,
    AndIf,
    OrIf,
    Pipe,
    LeftParen,
    RightParen,
    EndCase,
    FallThrough,
    Redirection,
    Word,
    Bang,
    Case,
    Do,
    Done,
    Elif,
    Else,
    Esac,
    Fi,
    For,
    If,
    In,
    Select,
    Then,
    Time,
    Until,
    While,
    LeftBrace,
    RightBrace,
    DoubleParen,
}

impl TokenKind {
    fn ends_list(self) -> bool {
        matches!(
            self,
            Self::Eof
                | Self::RightParen
                | Self::EndCase
                | Self::FallThrough
                | Self::Do
                | Self::Done
                | Self::Elif
                | Self::Else
                | Self::Esac
                | Self::Fi
                | Self::Then
                | Self::RightBrace
        )
    }

    fn can_be_case_pattern(self) -> bool {
        matches!(
            self,
            Self::Word
                | Self::Bang
                | Self::Case
                | Self::Do
                | Self::Done
                | Self::Elif
                | Self::Else
                | Self::Esac
                | Self::Fi
                | Self::For
                | Self::If
                | Self::In
                | Self::Then
                | Self::Until
                | Self::While
                | Self::LeftBrace
                | Self::RightBrace
                | Self::DoubleParen
        )
    }

    fn description(self) -> &'static [u8] {
        match self {
            Self::Eof => b"end of file",
            Self::Blank => b"blank",
            Self::Newline => b"newline",
            Self::Semicolon => b"\";\"",
            Self::Background => b"\"&\"",
            Self::AndIf => b"\"&&\"",
            Self::OrIf => b"\"||\"",
            Self::Pipe => b"\"|\"",
            Self::LeftParen => b"\"(\"",
            Self::RightParen => b"\")\"",
            Self::EndCase => b"\";;\"",
            Self::FallThrough => b"\";&\"",
            Self::Redirection => b"redirection",
            Self::Word => b"word",
            Self::Bang => b"\"!\"",
            Self::Case => b"\"case\"",
            Self::Do => b"\"do\"",
            Self::Done => b"\"done\"",
            Self::Elif => b"\"elif\"",
            Self::Else => b"\"else\"",
            Self::Esac => b"\"esac\"",
            Self::Fi => b"\"fi\"",
            Self::For => b"\"for\"",
            Self::If => b"\"if\"",
            Self::In => b"\"in\"",
            Self::Select => b"\"select\"",
            Self::Time => b"\"time\"",
            Self::Then => b"\"then\"",
            Self::Until => b"\"until\"",
            Self::While => b"\"while\"",
            Self::LeftBrace => b"\"{\"",
            Self::RightBrace => b"\"}\"",
            Self::DoubleParen => b"\"((\"",
        }
    }
}

/// Where in a word a `[` opens a subscript rather than ending the name
/// before it.
///
/// Bash's lexer asks this question twice, and the two answers differ.
/// At the start of a simple command a bracket opens one only after a
/// name, so `a[1 + 1]=x` is one word while `argv.py a[1 + 2]=` is three.
/// Inside a compound assignment's parentheses the bracket has to be the
/// element's very first byte instead, which is why `a=(x[1 2]=v)` still
/// splits at the blank and Bash reads `x[1` and `2]=v`.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) enum SubscriptPosition {
    /// Nowhere: a bracket is an ordinary byte of an ordinary word.
    #[default]
    None,
    /// After the name of an assignment word.
    AfterName,
    /// As the first byte of a compound assignment's element.
    ElementStart,
}

/// Parser context for one token read. Each property names the grammar
/// distinction it enables; no caller constructs or decodes an integer mask.
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct TokenContext {
    aliases: bool,
    reserved_words: bool,
    skip_newlines: bool,
    check_here_document_end: bool,
    regex_operand: bool,
    /// Whether an assignment word may begin here, which is where Bash's
    /// lexer reads a balanced `[...]` after a name as part of the word.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    assignment_position: bool,
    /// Whether the word being read is one element of a compound array
    /// assignment, where a leading `[...]` is the element's subscript.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    compound_element: bool,
}

impl TokenContext {
    pub(crate) const NONE: Self = Self {
        aliases: false,
        reserved_words: false,
        skip_newlines: false,
        check_here_document_end: false,
        regex_operand: false,
        assignment_position: false,
        compound_element: false,
    };
    /// Read the next word as the operand of Bash's `=~`.
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    const REGEX_OPERAND: Self = Self {
        regex_operand: true,
        ..Self::NONE
    };
    const ALIASES: Self = Self {
        aliases: true,
        assignment_position: true,
        ..Self::NONE
    };
    const RESERVED_WORDS: Self = Self {
        reserved_words: true,
        ..Self::NONE
    };
    const SKIP_NEWLINES: Self = Self {
        skip_newlines: true,
        ..Self::NONE
    };
    /// One element of `name=( ... )`, whose parentheses a newline may
    /// sit inside.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    const COMPOUND_ELEMENT: Self = Self {
        skip_newlines: true,
        compound_element: true,
        ..Self::NONE
    };
    const HERE_DOCUMENT_END: Self = Self {
        check_here_document_end: true,
        ..Self::NONE
    };
    const COMMAND_START: Self = Self {
        aliases: true,
        reserved_words: true,
        assignment_position: true,
        ..Self::NONE
    };
    const COMMAND_START_AFTER_NEWLINES: Self = Self {
        aliases: true,
        reserved_words: true,
        skip_newlines: true,
        assignment_position: true,
        ..Self::NONE
    };
    const RESERVED_WORDS_AFTER_NEWLINES: Self = Self {
        reserved_words: true,
        skip_newlines: true,
        ..Self::NONE
    };

    const fn with(self, other: Self) -> Self {
        Self {
            aliases: self.aliases || other.aliases,
            reserved_words: self.reserved_words || other.reserved_words,
            skip_newlines: self.skip_newlines || other.skip_newlines,
            check_here_document_end: self.check_here_document_end || other.check_here_document_end,
            regex_operand: self.regex_operand || other.regex_operand,
            assignment_position: self.assignment_position || other.assignment_position,
            compound_element: self.compound_element || other.compound_element,
        }
    }
}

// [spec:posix:def:grammar.reserved-word-tokens]
// [spec:posix:def:token.reserved-words]
// [spec:posix:def:token.reserved-words-optional]
// [spec:posix:req:token.reserved-word-time]
// [spec:posix:def:token.reserved-words-trailing-colon]
static RESERVED_WORDS: [(&[u8], TokenKind); 18] = [
    (b"!", TokenKind::Bang),
    (b"case", TokenKind::Case),
    (b"do", TokenKind::Do),
    (b"done", TokenKind::Done),
    (b"elif", TokenKind::Elif),
    (b"else", TokenKind::Else),
    (b"esac", TokenKind::Esac),
    (b"fi", TokenKind::Fi),
    (b"for", TokenKind::For),
    (b"if", TokenKind::If),
    (b"in", TokenKind::In),
    /* Bash's, not POSIX's: a POSIX script may name a command `select`, so
     * recognising it there would change what that script means. */
    (b"select", TokenKind::Select),
    (b"then", TokenKind::Then),
    /* POSIX's own -- XCU 2.4 reserves `time` -- so it is a keyword in
     * both dialects rather than a Bash extension. */
    // [spec:posix:req:token.reserved-word-time]
    (b"time", TokenKind::Time),
    (b"until", TokenKind::Until),
    (b"while", TokenKind::While),
    (b"{", TokenKind::LeftBrace),
    (b"}", TokenKind::RightBrace),
];

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct ParameterSyntax {
    braced: bool,
    operation: ParameterOperation,
    colon: bool,
}

impl ParameterSyntax {
    const fn unbraced() -> Self {
        Self {
            braced: false,
            operation: ParameterOperation::Value,
            colon: false,
        }
    }

    const fn braced() -> Self {
        Self {
            braced: true,
            operation: ParameterOperation::Invalid,
            colon: false,
        }
    }

    // [spec:posix:syn:param.positional-multi-digit-braces]
    const fn accepts_multiple_name_digits(self) -> bool {
        self.braced
            && matches!(
                self.operation,
                ParameterOperation::Invalid | ParameterOperation::Length
            )
    }

    const fn accepts_array_subscript(self) -> bool {
        self.braced && matches!(self.operation, ParameterOperation::Invalid)
    }

    /// `${#a[@]}` carries a subscript too, so the length operator has to
    /// be admitted here even though it cannot introduce one itself.
    const fn accepts_subscript_operand(self) -> bool {
        self.braced
            && matches!(
                self.operation,
                ParameterOperation::Invalid | ParameterOperation::Length
            )
    }

    const fn has_operand(self) -> bool {
        !matches!(self.operation, ParameterOperation::Value)
    }
}

/// Outcome of parsing one top-level input unit.
pub enum ParseResult {
    /// The input source reached end of file.
    Eof,
    /// a tree, or `None` where the C returned `NULL` for a blank line
    Tree(Option<Node>),
}

impl ParseResult {
    /// The tree, for the callers of `list()` that cannot see `NEOF`.
    fn into_node(self) -> Option<Node> {
        match self {
            ParseResult::Tree(node) => node,
            ParseResult::Eof => None,
        }
    }
}

fn required_compound_node(
    shell: &mut Shell,
    result: ParseResult,
    expected_at_eof: TokenKind,
) -> Result<Node, Error> {
    result.into_node().ok_or_else(|| {
        let expected = (shell.input.last_token == TokenKind::Eof).then_some(expected_at_eof);
        expected_token_error(shell, expected)
    })
}

/// `readtoken1`'s `eofmark` argument.
///
/// The C passes a `char *` that is overloaded three ways: NULL means "read a
/// word", `FAKEEOFMARK` (`(char *)1`) means "read a word but behave as if a
/// here-document were in progress", and anything else is the delimiter of a
/// real here-document. Only the third carries bytes, so only the third owns
/// any.
#[derive(Clone, Copy)]
enum EofMark<'a> {
    /// C: `NULL`
    None,
    /// C: `FAKEEOFMARK` — what `expandstr` passes
    Fake,
    /// C: the here-document's delimiter, after `rmescapes`
    Word(&'a BStr),
}

impl<'a> EofMark<'a> {
    /// `eofmark == NULL`
    fn is_none(self) -> bool {
        matches!(self, EofMark::None)
    }

    // [spec:dash:sem:parser.realeofmark-fn]
    fn real(self) -> Option<&'a BStr> {
        match self {
            EofMark::Word(w) => Some(w),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------

/// A here-document delimiter waiting for its body at a grammar newline.
pub struct PendingHereDocument {
    /// an expandable here-document uses double-quoted rather than
    /// single-quoted lexical rules
    pub expand: bool,
    /// string indicating end of input, with `rmescapes` already applied
    pub delimiter: BString,
    pub strip_tabs: bool,
}

/// A redirection operator whose required word operand has not been read yet.
// [spec:nsh:def:idiom.logical-descriptors]
pub(crate) enum PendingRedirection {
    File {
        operator: FileRedirectionOperator,
        descriptor: RedirectionDescriptor,
        /// Bash's `&>` / `&>>`, which carry the standard error along.
        // [spec:nsh:req:compat.bash.expansion-globbing]
        with_stderr: bool,
    },
    Descriptor {
        operator: DescriptorRedirectionOperator,
        descriptor: RedirectionDescriptor,
    },
    HereDocument {
        descriptor: RedirectionDescriptor,
    },
    /// Bash's `<<< word`. The operand is an ordinary word, so nothing is
    /// deferred to a grammar newline the way a here-document body is.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    HereString {
        descriptor: RedirectionDescriptor,
    },
}

/// One owned parse context. [`Rt1::synstack`] holds the contexts from the
/// base level through the current level, so the C's `next` link is the
/// preceding element and its `prev` link is spare `Vec` capacity left by a
/// pop. No cursor into the vector survives a push or pop.
pub struct SyntaxFrame {
    pub syntax: SyntaxContext,
    pub inner_double_quote: bool,
    pub variable_context_pushed: bool,
    pub double_quoted: bool,
    pub backquote: BackquoteContext,
    pub variable_depth: usize,
    pub parenthesis_depth: usize,
    pub double_quote_variable_depth: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) enum BackquoteContext {
    #[default]
    None,
    Modern,
    Legacy,
}

/// A token together with the word property the C parser carried in the
/// separate `quoteflag` global.
#[derive(Clone, Copy)]
pub(crate) struct Token {
    pub(crate) kind: TokenKind,
    quoted: bool,
}

impl Token {
    const fn plain(kind: TokenKind) -> Self {
        Self {
            kind,
            quoted: false,
        }
    }
}

// [spec:dash:sem:parser.issimplecmd-fn]
pub fn is_simple_command(node: Option<&Node>, name: &BStr) -> bool {
    match node {
        Some(Node::Command(command)) => command
            .arguments
            .first()
            .and_then(|argument| match argument {
                Node::Word(word) => Some(word.word.as_bstr()),
                _ => None,
            })
            .is_some_and(|word| word == name),
        _ => false,
    }
}

/// Parse one complete command unit from the current input source.
// [spec:dash:sem:parser.parsecmd-fn]
// [spec:posix:syn:grammar.program]
// [spec:posix:def:cmd.command-kinds]
// [spec:posix:req:cmd.no-size-limit]
// [spec:nsh:req:compat.bash.parse-boundary]
pub fn parse_command(shell: &mut Shell, interactive: bool) -> Result<ParseResult, Error> {
    let dialect = shell.options.dialect();
    shell.input.begin_parse(dialect);
    shell.input.token_pushed_back = false;
    shell.input.nesting_depth = 0;
    shell.input.pending_here_documents = Vec::new();
    shell.input.completed_here_documents = Vec::new();
    shell.input.prompt_before_read = interactive;
    if shell.input.prompt_before_read {
        select_prompt(shell, PromptKind::Primary)?;
    }
    shell.input.prompt_needed = false;
    let parsed = list(shell, ListMode::TopLevel);
    /* Sealed whether the parse succeeded or not: a rejected parse still
     * consumed bytes, and a log that dropped them would claim otherwise. */
    // [spec:nsh:def:idiom.token-stream]
    shell.input.tokens.seal();
    let mut result = parsed?;
    /* The unit's own bytes, which are more than its command's: the
     * newline that ended it belongs to no node, because trivia goes to
     * whatever follows it and at the end of a unit nothing does. */
    // [spec:nsh:req:idiom.printable-ast+2]
    if let ParseResult::Tree(Some(node)) = result {
        let unit = shell.input.tokens.whole();
        result = ParseResult::Tree(Some(node.with_tokens(unit)));
    }
    let bodies = core::mem::take(&mut shell.input.completed_here_documents);
    finalize::parse_result(shell, &mut result, bodies)?;
    Ok(result)
}

// [spec:dash:sem:parser.list-fn]
// [spec:posix:syn:grammar.separators]
// [spec:posix:def:cmd.list-definition]
// [spec:posix:def:cmd.compound-list-definition]
// [spec:posix:req:cmd.list-separator-semantics]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ListMode {
    TopLevel,
    Compound,
    StopAtTerminator,
}

/// Read a token without discarding whether a word contained quoting.
///
/// The context is an argument so one parse cannot change the next parse's
/// alias, reserved-word, newline, or here-document eligibility.
// [spec:posix:syn:grammar.token-context-dependent-distinction]
// [spec:posix:req:grammar.highest-numbered-rule-applies]
// [spec:posix:syn:grammar.command-name]
// [spec:posix:sem:token.categorization]
// [spec:posix:req:token.alias-substitution-conditions]
// [spec:posix:req:token.alias-reserved-word-unspecified]
// [spec:posix:req:token.alias-replacement]
// [spec:posix:req:token.reserved-word-recognition-contexts]
// [spec:dash:sem:parser.readtoken-fn]
pub(crate) fn read_token(shell: &mut Shell, mut context: TokenContext) -> Result<Token, Error> {
    let mut token: Token;

    loop {
        let replayed = shell.input.token_pushed_back;
        let frame = shell.input.current;
        let began = shell.input.tokens.position();
        token = read_next_token(shell, &context)?;
        // [spec:nsh:def:idiom.token-stream]
        shell
            .input
            .tokens
            .cut_token(token.kind, replayed, frame, began);

        /*
         * eat newlines
         */
        while context.skip_newlines && token.kind == TokenKind::Newline {
            parse_here_documents(shell)?;
            /* The alias bit is dropped with the rest: dash clears the
             * whole of `checkkwd` here, and the bit lived in it. */
            shell.input.clear_alias_boundary();
            let replayed = shell.input.token_pushed_back;
            let frame = shell.input.current;
            let began = shell.input.tokens.position();
            token = read_next_token(shell, &context)?;
            // [spec:nsh:def:idiom.token-stream]
            shell
                .input
                .tokens
                .cut_token(token.kind, replayed, frame, began);
        }

        /* `popstring` sets this while `xxreadtoken` runs. The bit belongs
         * to the input boundary now; this is the same hand-off point. */
        if shell.input.take_alias_boundary() {
            context.aliases = true;
        }

        if token.kind != TokenKind::Word || token.quoted {
            break;
        }

        /*
         * check for keywords
         */
        if context.reserved_words {
            if let Some(kind) = reserved_word(shell.input.word_text(), shell.options.dialect()) {
                token.kind = kind;
                shell.input.last_token = token.kind;
                break;
            }
        }

        if context.aliases
            && shell
                .options
                .alias_expansion_enabled(shell.input.parse_dialect())
        {
            /* Hoisted: the receiver cannot appear twice in one argument
             * list. A raw pointer ends its borrow at the `let`, and the
             * word it points at is the parser's own, not the alias's. */
            let name = shell.input.word_text().to_owned();
            if let Some(value) = shell.aliases.lookup(BStr::new(name.as_slice()), true) {
                // [spec:nsh:def:idiom.token-stream]
                shell
                    .input
                    .tokens
                    .retract_alias_name(BStr::new(name.as_slice()));
                if !value.is_empty() {
                    push_string_input(shell, BStr::new(value.as_slice()), Some(name));
                }
                continue;
            }
        }
        break;
    }
    Ok(token)
}

// [spec:dash:sem:parser.nlprompt-fn]
fn prompt_after_newline(shell: &mut Shell) -> Result<(), Error> {
    crate::input::current_input_frame(&mut shell.input).line_number += 1;
    if shell.input.prompt_before_read {
        select_prompt(shell, PromptKind::Continuation)?;
    }
    Ok(())
}

// [spec:dash:sem:parser.nlnoprompt-fn]
fn consume_newline_without_prompt(shell: &mut Shell) {
    crate::input::current_input_frame(&mut shell.input).line_number += 1;
    shell.input.prompt_needed = shell.input.prompt_before_read;
}

/*
 * Read the next input token.
 * If the token is a word, we set backquotelist to the list of cmds in
 *	backquotes.  We set quoteflag to true if any part of the word was
 *	quoted.
 * If the token is TokenKind::Redirection, then we set redirnode to a structure containing
 *	the redirection.
 */

// [spec:dash:sem:parser.xxreadtoken-fn]
// [spec:posix:syn:grammar.token-classification]
// [spec:posix:req:token.input-lines]
// [spec:posix:syn:token.recognition-algorithm]
// [spec:posix:syn:token.delimit-at-end-of-input]
// [spec:posix:syn:token.operator-continue]
// [spec:posix:syn:token.operator-delimit]
// [spec:posix:syn:token.start-new-operator]
// [spec:posix:def:grammar.operator-tokens]
// [spec:posix:syn:token.unquoted-blank-delimits]
// [spec:posix:syn:token.comment]
// [spec:posix:syn:token.start-new-word]
fn read_next_token(shell: &mut Shell, context: &TokenContext) -> Result<Token, Error> {
    let check_here_document_end = context.check_here_document_end;
    let regex_operand = context.regex_operand;
    let mut input: InputUnit;

    if shell.input.token_pushed_back {
        shell.input.token_pushed_back = false;
        return Ok(Token {
            kind: shell.input.last_token,
            quoted: shell.input.last_token_quoted,
        });
    }
    shell.input.last_token_after_blank = false;
    if shell.input.prompt_needed {
        select_prompt(shell, PromptKind::Continuation)?;
    }
    loop {
        /* until token or start of word found */
        let before_skip = shell.input.tokens.pending_length();
        input = read_unit_skipping_line_continuations(shell)?;
        /* The continuations a token is reached through are read by the call
         * that reads its first byte, so the cut between them lands behind
         * the reader's position. Only when nothing was already pending is
         * the leading run known to be trivia and nothing else. */
        // [spec:nsh:def:idiom.token-stream]
        let leading = shell
            .input
            .tokens
            .pending_length()
            .saturating_sub(usize::from(input.byte().is_some()));
        if before_skip == 0 {
            shell
                .input
                .tokens
                .cut_head(leading, SourceTokenKind::LineContinuation);
        }
        if input.is(b' ') || input.is(b'\t') {
            shell.input.last_token_after_blank = true;
            // [spec:nsh:def:idiom.token-stream]
            shell.input.tokens.cut(SourceTokenKind::Blank);
            continue;
        } else if input.is(b'#') && !regex_operand {
            loop {
                input = read_input_unit(shell)?;
                if input.is(b'\n') || input == InputUnit::EndOfInput {
                    break;
                }
            }
            unread_input_unit(shell);
            // [spec:nsh:def:idiom.token-stream]
            shell.input.tokens.cut(SourceTokenKind::Comment);
            continue;
        } else if input.is(b'\n') {
            consume_newline_without_prompt(shell);
            shell.input.last_token = TokenKind::Newline;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::Newline));
        } else if input == InputUnit::EndOfInput {
            shell.input.last_token = TokenKind::Eof;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::Eof));
        } else if regex_operand && !matches!(input.byte(), Some(b'&' | b';' | b')')) {
            /* A regular-expression operand keeps `(`, `|` and the rest of
             * its own syntax; only the operators Bash still needs to see
             * fall through to the ordinary reader below. */
            let token = read_word_token(
                shell,
                input,
                SyntaxContext::Regex,
                EofMark::None,
                false,
                check_here_document_end,
                SubscriptPosition::None,
            )?;
            if token.kind != TokenKind::Blank {
                return Ok(token);
            }
        } else if input.is(b'&') {
            let after = read_unit_skipping_line_continuations(shell)?;
            if after.is(b'&') {
                shell.input.last_token = TokenKind::AndIf;
                shell.input.last_token_quoted = false;
                return Ok(Token::plain(TokenKind::AndIf));
            }
            /* Bash's `&>file` and `&>>file`. Read here rather than beside
             * the other redirections because the `&` reaches the operator
             * reader first, and without this arm it is the whole of the
             * command: `echo x &>f` runs `echo x` in the *background* and
             * opens `f` for a command with no words, which leaves the
             * output on the terminal, the file empty, and the echo racing
             * whatever the shell does next. POSIX means exactly that and
             * `/usr/bin/dash` does it, so the form belongs to the dialect
             * and not to the option: `bash --posix` still reads it as one
             * operator. */
            // [spec:nsh:req:compat.bash.expansion-globbing]
            if after.is(b'>') && bash::active(shell) {
                let operator = if read_unit_skipping_line_continuations(shell)?.is(b'>') {
                    FileRedirectionOperator::Append
                } else {
                    unread_input_unit(shell);
                    FileRedirectionOperator::Write
                };
                shell.input.pending_redirection = Some(PendingRedirection::File {
                    operator,
                    descriptor: RedirectionDescriptor::Fixed(LogicalDescriptor::STDOUT),
                    with_stderr: true,
                });
                shell.input.last_token = TokenKind::Redirection;
                shell.input.last_token_quoted = false;
                return Ok(Token::plain(TokenKind::Redirection));
            }
            unread_input_unit(shell);
            shell.input.last_token = TokenKind::Background;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::Background));
        } else if input.is(b'|') {
            if read_unit_skipping_line_continuations(shell)?.is(b'|') {
                shell.input.last_token = TokenKind::OrIf;
                shell.input.last_token_quoted = false;
                return Ok(Token::plain(TokenKind::OrIf));
            }
            unread_input_unit(shell);
            shell.input.last_token = TokenKind::Pipe;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::Pipe));
        } else if input.is(b';') {
            let next = read_unit_skipping_line_continuations(shell)?;
            if next.is(b';') {
                shell.input.last_token = TokenKind::EndCase;
                shell.input.last_token_quoted = false;
                return Ok(Token::plain(TokenKind::EndCase));
            } else if next.is(b'&') {
                shell.input.last_token = TokenKind::FallThrough;
                shell.input.last_token_quoted = false;
                return Ok(Token::plain(TokenKind::FallThrough));
            }
            unread_input_unit(shell);
            shell.input.last_token = TokenKind::Semicolon;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::Semicolon));
        } else if input.is(b'(') {
            if bash::active(shell) && read_unit_skipping_line_continuations(shell)?.is(b'(') {
                shell.input.last_token = TokenKind::DoubleParen;
                shell.input.last_token_quoted = false;
                return Ok(Token::plain(TokenKind::DoubleParen));
            }
            if bash::active(shell) {
                unread_input_unit(shell);
            }
            shell.input.last_token = TokenKind::LeftParen;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::LeftParen));
        } else if input.is(b')') {
            shell.input.last_token = TokenKind::RightParen;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::RightParen));
        }
        let token = read_word_token(
            shell,
            input,
            SyntaxContext::Base,
            EofMark::None,
            false,
            check_here_document_end,
            /* Where this context lets a `[` open a subscript. An
             * element's own bracket wins: a compound assignment's
             * contents are never also the start of a simple command. */
            // [spec:nsh:req:compat.bash.arrays-declarations]
            if context.compound_element {
                SubscriptPosition::ElementStart
            } else if context.assignment_position {
                SubscriptPosition::AfterName
            } else {
                SubscriptPosition::None
            },
        )?;
        if token.kind != TokenKind::Blank {
            return Ok(token);
        }
    }
}

// [spec:dash:sem:parser.pgetc-eatbnl-fn]
// [spec:posix:req:quote.backslash-newline]
fn read_unit_skipping_line_continuations(shell: &mut Shell) -> Result<InputUnit, Error> {
    let mut input: InputUnit;

    loop {
        input = read_input_unit(shell)?;
        if !input.is(b'\\') {
            break;
        }
        if !read_input_unit(shell)?.is(b'\n') {
            unread_input_unit(shell);
            break;
        }

        prompt_after_newline(shell)?;
    }

    Ok(input)
}

// [spec:dash:sem:parser.pgetc-top-fn]
fn read_unit_for_syntax(shell: &mut Shell, stack: &SyntaxFrame) -> Result<InputUnit, Error> {
    if stack.syntax == SyntaxContext::SingleQuoted {
        read_input_unit(shell)
    } else {
        read_unit_skipping_line_continuations(shell)
    }
}

mod expansion;
mod grammar;
mod multibyte;
mod syntax_stack;
pub(crate) mod tokens;
mod word_lexer;

use expansion::{parse_command_substitution, parse_parameter_expansion};
pub(crate) use grammar::make_name_node;
use grammar::{command, list, parse_here_documents, pipeline};
use word_lexer::{WordLexer, read_word_token};

pub(crate) use multibyte::MultibyteMode;
pub(crate) use tokens::{SealedLog, SourceToken, SourceTokenKind, TokenLog, TokenMark};

/// Result of decoding the input unit at the current lexer position.
pub(crate) enum MultibyteInput {
    SingleByte,
    FieldBoundary,
    Character { bytes: BString, escaped: bool },
}

// [spec:dash:sem:parser.getmbc-fn]
/// The destination is a fixed scratch buffer sized to what this writes.
///
/// The C hands it a cursor into the stack block and it writes the
/// character's bytes *ahead* of that cursor while it is still deciding
/// whether they form one -- then either frames them and reports the
/// length, or reports 0 and leaves the bytes as scribble for the next
/// write to overwrite. That is a legitimate design, and the room it needs
/// is `MBSLOP`, which every caller had to know from a comment.
///
/// Written into scratch, the speculation is contained by construction:
/// the caller appends only the prefix this reports, and the scribble is
/// simply not copied out. Same bytes, same length, and the reservation
/// stops being a memory-safety contract.
/// Decode the character starting at `first` from bytes the input frame
/// is already holding, or answer `None` when it is not holding them all.
///
/// One thread-locale selection for the whole character, which is what
/// makes this worth having beside the incremental decoder. `None` is not
/// a failure: the buffer ran out mid-character, and only the incremental
/// loop can finish the job, because finishing it may mean blocking on a
/// read -- and must, for a terminal with one byte to give.
///
/// Every byte this consumes goes back through `read_input_unit_or_alias_end`
/// rather than being lifted out of the buffer, so the line count, the
/// token record and the alias bookkeeping see the same reads either way.
/// The buffer decides only *how many*, and answering that costs no read.
fn decode_buffered_character(
    shell: &mut Shell,
    first: u8,
    mode: MultibyteMode,
) -> Result<Option<MultibyteInput>, Error> {
    let mut window = [0_u8; MAX_MULTIBYTE_LENGTH];
    window[0] = first;
    let buffered = crate::input::buffered_line_bytes(&mut shell.input);
    if buffered.is_empty() {
        return Ok(None);
    }
    let taken = buffered.len().min(MAX_MULTIBYTE_LENGTH - 1);
    window[1..=taken].copy_from_slice(&buffered[..taken]);

    let width = match shell.locale.decode_prefix(&window[..=taken]) {
        // A byte the locale will not start a character with is a byte the
        // caller keeps, and the incremental loop reaches that verdict
        // without consuming anything either.
        nsh_platform::LocaleCharacter::Invalid => return Ok(Some(MultibyteInput::SingleByte)),
        nsh_platform::LocaleCharacter::Complete { width, .. } if width <= 1 => {
            return Ok(Some(MultibyteInput::SingleByte));
        }
        nsh_platform::LocaleCharacter::Complete { wide, width } => {
            if matches!(mode, MultibyteMode::FieldBoundary) && shell.locale.wide_is_blank(wide) {
                consume_buffered(shell, width - 1)?;
                return Ok(Some(MultibyteInput::FieldBoundary));
            }
            width
        }
        nsh_platform::LocaleCharacter::Incomplete => return Ok(None),
    };

    consume_buffered(shell, width - 1)?;
    Ok(Some(MultibyteInput::Character {
        bytes: BString::new(window[..width].to_vec()),
        escaped: matches!(mode, MultibyteMode::Escaped),
    }))
}

/// Take `count` units the buffer was just seen to be holding.
///
/// Each is a byte by construction -- `buffered_line_bytes` answers only
/// for a run the frame will hand over as bytes -- so a unit that is not
/// one means the invariant broke rather than that input ended, and the
/// walk stops rather than guessing.
fn consume_buffered(shell: &mut Shell, count: usize) -> Result<(), Error> {
    for _ in 0..count {
        let unit = read_input_unit_or_alias_end(shell)?;
        debug_assert!(unit.byte().is_some(), "a buffered unit was not a byte");
        if unit.byte().is_none() {
            break;
        }
    }
    Ok(())
}

pub(crate) fn read_multibyte_character(
    shell: &mut Shell,
    input: InputUnit,
    mode: MultibyteMode,
) -> Result<MultibyteInput, Error> {
    let Some(mut byte) = input.byte() else {
        return Ok(MultibyteInput::SingleByte);
    };
    let escaped = matches!(mode, MultibyteMode::Escaped);

    if byte.is_ascii() {
        return Ok(MultibyteInput::SingleByte);
    }

    if let Some(decoded) = decode_buffered_character(shell, byte, mode)? {
        return Ok(decoded);
    }

    let mut decoder = shell.locale.decoder();
    let mut bytes = BString::new(Vec::new());
    let mut wc: i32 = 0;
    let mut complete = false;

    loop {
        bytes.push(byte);
        let decoded = decoder.push(byte);
        match decoded {
            nsh_platform::LocaleDecode::Incomplete => {}
            nsh_platform::LocaleDecode::Complete(wide) => {
                wc = wide;
                complete = true;
                break;
            }
            nsh_platform::LocaleDecode::Invalid => break,
        }
        if bytes.len() >= MAX_MULTIBYTE_LENGTH {
            break;
        }
        let next = read_input_unit_or_alias_end(shell)?;
        let Some(next_byte) = next.byte() else {
            break;
        };
        byte = next_byte;
    }

    if complete && bytes.len() > 1 {
        if matches!(mode, MultibyteMode::FieldBoundary) && shell.locale.wide_is_blank(wc) {
            return Ok(MultibyteInput::FieldBoundary);
        }
        return Ok(MultibyteInput::Character { bytes, escaped });
    }

    if bytes.len() > 1 {
        unread_input_units(shell, bytes.len() - 1);
    }

    Ok(MultibyteInput::SingleByte)
}

// [spec:dash:sem:parser.dollarsq-escape-fn]
// [spec:posix:def:quote.dollar-single-quotes-escapes]
// [spec:posix:def:quote.dollar-single-quotes-control-escape]
// [spec:posix:def:quote.dollar-single-quotes-hex-escape]
// [spec:posix:def:quote.dollar-single-quotes-octal-escape]
// [spec:posix:req:quote.dollar-single-quotes-undefined-escape]
// [spec:posix:syn:quote.dollar-single-quotes-escape-termination]
// [spec:posix:req:quote.dollar-single-quotes-processing-time]
// [spec:posix:req:quote.dollar-single-quotes-null-byte]
// [spec:posix:req:quote.dollar-single-quotes-octal-overflow]
// [spec:posix:req:quote.dollar-single-quotes-unencodable]
// [spec:posix:req:quote.dollar-single-quotes-quote-escape-not-terminator]
fn parse_dollar_single_quote_escape(
    shell: &mut Shell,
    destination: &mut Vec<WordToken>,
) -> Result<(), Error> {
    /* Longest accepted escape spelling is `UXXXXXXXX`. */
    let mut text = Vec::with_capacity(9);

    while text.len() < text.capacity() {
        let input = read_input_unit(shell)?;
        let Some(byte) = input.byte() else {
            break;
        };

        text.push(byte);

        if byte == b'\'' {
            break;
        }
    }
    let (bytes, consumed) = if text.first() != Some(&b'c') {
        let converted = crate::escape::parse_escape(&shell.locale, &text, true);
        (converted.bytes().to_vec(), converted.consumed)
    } else {
        let mut consumed = 1;
        let bytes = if let Some(&control_byte) = text.get(consumed) {
            consumed += 1;
            consumed += usize::from(control_byte == b'\\' && text.get(consumed) == Some(&b'\\'));
            vec![(control_byte & !((control_byte & 0x40) >> 1) & 0x7f) ^ 0x40]
        } else {
            Vec::new()
        };
        (bytes, consumed)
    };

    unread_input_units(shell, text.len().saturating_sub(consumed));
    /* A decoded byte is data, and the `$'...'` around it is what makes it
     * so: the run it lands in is inert because of the quote, not because
     * of the escape that spelled it. `$'\"'` and `$'"'` are one word
     * holding one byte, and the run each was read as is what tells them
     * apart. */
    // [spec:nsh:req:idiom.printable-ast+2]
    destination.extend(bytes.into_iter().map(WordToken::Literal));
    Ok(())
}

/*
 * Parse a redirection operator.  The variable "out" points to a string
 * specifying the fd to be redirected.  The variable "c" contains the
 * first character of the redirection operator.
 */

/* parseredir: */
// [spec:posix:syn:redir.format]
// [spec:posix:syn:redir.quoting-suppresses-recognition]
// [spec:posix:req:redir.location-format]
// [spec:posix:req:redir.max-fd-number]
// [spec:posix:syn:redir.output-format]
// [spec:posix:syn:redir.append-format]
// [spec:posix:def:redir.here-doc]
// [spec:posix:syn:redir.here-doc-format]
// [spec:posix:syn:grammar.io-redirect]
// [spec:posix:syn:grammar.io-file]
// [spec:posix:syn:grammar.io-here]
// [spec:posix:def:grammar.operator-tokens]
fn parse_redirection(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    explicit: Option<RedirectionDescriptor>,
) -> Result<(), Error> {
    enum ParsedRedirection {
        File(FileRedirectionOperator),
        Descriptor(DescriptorRedirectionOperator),
        HereDocument,
        HereString,
    }

    /* The C carves one `struct nfile` and then decides what it is by
     * assigning `np->type`, re-allocating only because `nhere` is smaller.
     * The arm has to be chosen up front here, so the type and the fd are
     * worked out first and the node built at the end. */
    let mut descriptor: RedirectionDescriptor;
    let redirection: ParsedRedirection;

    if lexer.input.is(b'>') {
        descriptor = RedirectionDescriptor::Fixed(LogicalDescriptor::STDOUT);
        lexer.input = read_unit_skipping_line_continuations(shell)?;
        if lexer.input.is(b'>') {
            redirection = ParsedRedirection::File(FileRedirectionOperator::Append);
        } else if lexer.input.is(b'|') {
            redirection = ParsedRedirection::File(FileRedirectionOperator::Clobber);
        } else if lexer.input.is(b'&') {
            redirection = ParsedRedirection::Descriptor(DescriptorRedirectionOperator::Output);
        } else {
            redirection = ParsedRedirection::File(FileRedirectionOperator::Write);
            unread_input_unit(shell);
        }
    } else {
        /* c == '<' */
        descriptor = RedirectionDescriptor::Fixed(LogicalDescriptor::STDIN);
        lexer.input = read_unit_skipping_line_continuations(shell)?;
        if lexer.input.is(b'<') {
            let mut here = PendingHereDocument {
                expand: false,
                delimiter: BString::new(Vec::new()),
                strip_tabs: false,
            };
            lexer.input = read_unit_skipping_line_continuations(shell)?;
            if lexer.input.is(b'<') && bash::active(shell) {
                /* `<<<` is a here-string; the third `<` is only an
                 * operator in Bash mode, so POSIX still reads `<< <word`
                 * as a here-document with a delimiter that starts `<`. */
                // [spec:nsh:req:compat.bash.expansion-globbing]
                redirection = ParsedRedirection::HereString;
            } else {
                redirection = ParsedRedirection::HereDocument;
                if lexer.input.is(b'-') {
                    here.strip_tabs = true;
                } else {
                    unread_input_unit(shell);
                }
                shell.input.pending_here_document = Some(here);
            }
        } else if lexer.input.is(b'&') {
            redirection = ParsedRedirection::Descriptor(DescriptorRedirectionOperator::Input);
        } else if lexer.input.is(b'>') {
            redirection = ParsedRedirection::File(FileRedirectionOperator::ReadWrite);
        } else {
            redirection = ParsedRedirection::File(FileRedirectionOperator::Read);
            unread_input_unit(shell);
        }
    }
    if let Some(explicit) = explicit {
        descriptor = explicit;
    }
    shell.input.pending_redirection = Some(match redirection {
        ParsedRedirection::Descriptor(operator) => PendingRedirection::Descriptor {
            operator,
            descriptor,
        },
        ParsedRedirection::HereDocument => PendingRedirection::HereDocument { descriptor },
        ParsedRedirection::HereString => PendingRedirection::HereString { descriptor },
        ParsedRedirection::File(operator) => PendingRedirection::File {
            operator,
            descriptor,
            with_stderr: false,
        },
    });
    Ok(())
}

/*
 * Return of a legal variable name (a letter or underscore followed by zero or
 * more letters, underscores, and digits).
 */

// [spec:dash:sem:parser.endofname-fn]
pub fn name_end(locale: &nsh_platform::Locale, name: &BStr) -> usize {
    let Some(&first) = name.first() else {
        return 0;
    };
    if !is_name(locale, first) {
        return 0;
    }
    1 + name[1..]
        .iter()
        .position(|&byte| !is_in_name(locale, byte))
        .unwrap_or(name.len() - 1)
}

/*
 * Called when an unexpected token is read during the parse.  The argument
 * is the token that is expected, or -1 if more than one type of token can
 * occur at this point.
 */

// [spec:dash:sem:parser.synexpect-fn]
// [spec:nsh:req:idiom.no-artificial-limits]
fn expected_token_error(shell: &mut Shell, expected: Option<TokenKind>) -> Error {
    let mut message = Vec::new();

    message.extend_from_slice(shell.input.last_token.description());
    message.extend_from_slice(b" unexpected");
    if let Some(expected) = expected {
        message.extend_from_slice(b" (expecting ");
        message.extend_from_slice(expected.description());
        message.push(b')');
    }
    syntax_error(shell, &message)
}

/// The line the parser has read as far as.
///
/// Every node that records a position records this one, taken at the
/// moment that node is built, and which moment that is decides the
/// answer: a construct built at its closing token holds a later line than
/// one built at its opening word. See `record_command_line`.
// [spec:nsh:req:compat.bash.traps-introspection]
fn line_reached(shell: &mut Shell) -> SourceLine {
    SourceLine::new(crate::input::current_input_frame(&mut shell.input).line_number)
}

// [spec:dash:sem:parser.synerror-fn]
fn syntax_error(shell: &mut Shell, msg: &[u8]) -> Error {
    shell.evaluation.diagnostic_line =
        crate::input::current_input_frame(&mut shell.input).line_number;
    let mut message = b"Syntax error: ".to_vec();
    message.extend_from_slice(msg);
    shell.diagnostics().shell_error(&message)
}

// [spec:dash:sem:parser.setprompt-fn]
#[inline(never)]
fn select_prompt(shell: &mut Shell, prompt: PromptKind) -> Result<(), Error> {
    shell.input.prompt_needed = false;
    shell.input.prompt = Some(prompt);

    if !crate::editor::editing_active(shell)
        && crate::input::current_input_frame(&mut shell.input).line_remaining == 0
    {
        /* `pushstackmark(&smark, stackblocksize())` bounded the prompt
         * `expandstr` had left in the region for `out2str` to read.  The
         * expansion buffer is owned, so there is nothing to bound. */
        let prompt = render_prompt(shell);
        shell.write_output(crate::output::OutputDestination::Stderr, &prompt)?;
    }
    Ok(())
}

// [spec:dash:sem:parser.expandstr-fn]
// [spec:dash:sem:expand.restore-handler-expandarg-fn]
pub fn expand_string(shell: &mut Shell, source: &BStr) -> Result<BString, Error> {
    let saved_here_documents = core::mem::take(&mut shell.input.pending_here_documents);
    let saved_prompt_state = shell.input.prompt_before_read;
    /* A pushed-back token belongs to the source the caller was reading,
     * and the text below is a different one. `read_token` is reached
     * from here whenever the string holds a `$( )` or a backquote, and
     * it replays whatever was pushed back rather than reading the new
     * source: with a string-fed shell the outer parse has left `Eof`
     * there, so `-c 'a=(9 8 7); echo "${a[$(echo 1)]}"'` reported
     * `end of file unexpected (expecting ")")` and the backquote form
     * silently expanded to nothing. Both work from a file, where the
     * outer parse has not reached its end.
     *
     * NEITHER DIALECT KEEPS IT, and the reason it is not a dialect
     * question is that the state being replayed belongs to the *caller's*
     * source rather than to either shell's grammar. dash has the same
     * defect -- `dash -c "PS4='\$(echo P)+ '; set -x; echo hi"` reports
     * the syntax error and traces with the unexpanded text -- and
     * `[spec:posix:req:param.ps4]` leaves substitution in `PS4`
     * unspecified, so declining to expand would conform. Emitting a parse
     * diagnostic for text that parses does not: that is a stale token, not
     * a choice. Registered as `re_entered_prompt_substitution` in
     * `docs/divergences.md`. */
    // [spec:nsh:def:idiom.token-stream]
    let saved_pushed_back = shell.input.token_pushed_back;
    let saved_last_token = shell.input.last_token;
    /* `result = ps` — the C seeds the answer with the string it was given
     * and the failure path is what leaves the seed standing.
     *
     * The seed is a *copy* rather than the pointer, and that is a fix
     * rather than a transcription. `ps` points into a shell variable's
     * text, and the expansion about to run can reassign that variable —
     * `PS1='$(PS1=x; echo)'` reaches it — which reallocates the text and
     * leaves `ps` dangling for exactly the failure path that reads it.
     * The C read it anyway. Copying at the point the C takes the seed
     * keeps the C's sequence and removes the read-after-free. */
    let mut result = BString::from(source);
    let caught = crate::resource::with_resources(shell, |shell, _resources| {
        set_input_string(shell, source);
        shell.input.prompt_before_read = false;
        shell.input.prompt_needed = false;
        shell.input.token_pushed_back = false;
        /* Parse and expand inside one fallible operation so a failure leaves
         * the seeded result unchanged. */
        let caught = (|| -> Result<(), crate::error::Error> {
            let result = &mut result;
            let firstc = read_unit_skipping_line_continuations(shell)?;
            read_word_token(
                shell,
                firstc,
                SyntaxContext::DoubleQuoted,
                EofMark::Fake,
                false,
                false,
                SubscriptPosition::None,
            )?;

            /* Expanded and thrown away rather than kept in a tree, so
             * there is nothing for a run to be printed back from. */
            // [spec:nsh:req:idiom.printable-ast+2]
            let node = Node::Word(WordNode {
                tokens: SourceTokens::none(),
                word: mem::take(&mut shell.input.word),
            });

            expand_argument(shell, &node, None, ExpansionMode::QUOTED)?;
            /* The C reads the expansion back as `stackblock()`; the expansion
             * buffer is owned now, so the read is named.  The C's pointer was
             * live only until the next `stalloc`; this one is live until the
             * next expansion, which is a superset.
             *
             * Neither `?` above reaches this line: a prompt that fails to parse
             * or expand leaves `result` as the `ps` it was seeded with, and the
             * caller renders the prompt unexpanded.
             *
             * The copy is what the C's pointer bought with liveness instead.
             * It costs one allocation per prompt — drawn at the rate a human
             * presses return, or once per traced command — and it buys the
             * caller a value that no later expansion can pull out from under
             * it. `getprompt` handing back a borrow of the expansion buffer
             * would have to outlive the next `expandarg` to be useful, and it
             * cannot. */
            *result = BString::from(crate::expand::expansion_result(shell));
            Ok(())
        })()
        .err();

        /* A *diagnostic* is dropped, and that is dash: `expandstr` reports it
         * and hands back the string it was given, which is why a bad `PS1` or
         * `PS4` cannot abort a script (`docs/api-design.md` §3.3).
         *
         * An interrupt is not, and cannot be — the C re-raised it from
         * `restore_handler_expandarg` and there is nothing here that may
         * swallow a ^C. It is why this function returns a `Result` at all;
         * both callers are fallible and neither wanted one otherwise. */
        caught
    });

    shell.input.prompt_before_read = saved_prompt_state;
    shell.input.pending_here_documents = saved_here_documents;
    shell.input.token_pushed_back = saved_pushed_back;
    shell.input.last_token = saved_last_token;

    match caught {
        Some(e) if e.is_interrupt() => Err(e),
        other => {
            /* A bad `PS1`/`PS4` is reported and the unexpanded prompt is
             * used; the status it took is still the shell's, so the catch
             * writes it. */
            if let Some(e) = &other {
                shell.status = e.status();
            }
            drop(other);
            Ok(result)
        }
    }
}

/*
 * called by editline -- any expansions to the prompt
 *    should be added here.
 */

// [spec:dash:sem:parser.getprompt-fn]
// [spec:posix:req:param.ps1]
// [spec:posix:req:param.ps1-two-pass]
// [spec:posix:req:param.ps2]
pub fn render_prompt(shell: &mut Shell) -> BString {
    let prompt = match shell.input.prompt {
        Some(PromptKind::Primary) => {
            let prompt = crate::variables::primary_prompt_value(shell);
            shell
                .editor
                .expand_prompt_exclamation_marks(BStr::new(prompt.as_slice()))
        }
        Some(PromptKind::Continuation) => crate::variables::continuation_prompt_value(shell),
        /* An unknown prompt selector is empty. Both readers consume bytes,
         * so there is no distinct identity to preserve here. */
        None => {
            return BString::default();
        }
    };

    /* A callback: the line editor calls this through a function pointer
     * and there is nothing here to return a `Result` to. A diagnostic is
     * dropped, as everywhere `expandstr` is used; an interrupt is put
     * back for the next poll site, because dropping it would lose the
     * user's ^C. See `error::rearm_interrupt`. */
    /* The receiver arrives by parameter, and that is the whole of the
     * answer `move-state` owed this site. It was the last
     * `Shell::detached()`, and the reason recorded for it -- an opaque
     * callback the line editor invokes with nowhere to put a `&mut Shell`
     * -- stopped
     * being true when the editor moved to its native Rust API: nothing
     * calls this through a pointer any more. `editor::shell_prompt`
     * calls it, and `setprompt` calls it, and both are ordinary Rust
     * frames that can carry a receiver. So this is threading after all,
     * not the handle `docs/api-design.md` §5.1 keeps for the signal
     * handler -- which is still the one shape that cannot take a
     * parameter, because a handler has no frame to thread through. */
    match expand_string(shell, BStr::new(prompt.as_slice())) {
        Ok(expanded) => expanded,
        Err(e) => {
            crate::error::rearm_interrupt(e);
            /* The interrupted prompt is rendered unexpanded, which is the
             * same answer `expandstr`'s own failure path gives — and the
             * same read of `prompt`, taken here because this arm never
             * entered the copy `expandstr` makes. */
            prompt
        }
    }
}

/// Recognize a reserved word and return its semantic token directly.
///
/// The dialect decides one entry. `select` is Bash's, and a POSIX script
/// may legitimately name a command that -- reserving it there would change
/// what an existing script means. `time` is POSIX's own (XCU 2.4), so it
/// is a keyword either way.
// [spec:dash:sem:parser.findkwd-fn]
// [spec:posix:req:token.reserved-word-time]
pub fn reserved_word(s: &BStr, dialect: crate::options::Dialect) -> Option<TokenKind> {
    RESERVED_WORDS
        .binary_search_by(|(word, _)| word.cmp(&s.as_ref()))
        .ok()
        .map(|index| RESERVED_WORDS[index].1)
        .filter(|kind| *kind != TokenKind::Select || dialect == crate::options::Dialect::Bash)
}

// ---------------------------------------------------------------------
// src/parser.h inline functions
// ---------------------------------------------------------------------

// [spec:dash:sem:parser.goodname-fn]
pub fn is_valid_name(locale: &nsh_platform::Locale, name: &BStr) -> bool {
    name_end(locale, name) == name.len()
}

// [spec:dash:sem:parser.parser-eof-fn]
pub fn parser_eof(shell: &Shell) -> bool {
    shell.input.token_pushed_back && shell.input.last_token == TokenKind::Eof
}

pub(crate) mod bash;
mod keywords;

mod finalize;

#[cfg(test)]
mod bash_mode_tests;

#[cfg(test)]
mod bash_ast_tests;

#[cfg(test)]
mod tests;

#[cfg(test)]
mod token_stream_tests;

#[cfg(test)]
mod token_tree_tests;
