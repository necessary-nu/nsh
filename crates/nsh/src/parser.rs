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
    NodeText, Pipeline, Redirection, SimpleCommand, TimedCommand, WordNode,
};
use crate::syntax::{InputUnit, SyntaxClass, SyntaxContext, is_in_name, is_name};
use crate::word::{ParameterOperation, ParsedWord, QuoteBoundary, WordToken};

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
}

impl TokenContext {
    pub(crate) const NONE: Self = Self {
        aliases: false,
        reserved_words: false,
        skip_newlines: false,
        check_here_document_end: false,
        regex_operand: false,
        assignment_position: false,
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
        descriptor: LogicalDescriptor,
    },
    Descriptor {
        operator: DescriptorRedirectionOperator,
        descriptor: LogicalDescriptor,
    },
    HereDocument {
        descriptor: LogicalDescriptor,
    },
    /// Bash's `<<< word`. The operand is an ordinary word, so nothing is
    /// deferred to a grammar newline the way a here-document body is.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    HereString {
        descriptor: LogicalDescriptor,
    },
}

/// One owned parse context. [`Rt1::synstack`] holds the contexts from the
/// base level through the current level, so the C's `next` link is the
/// preceding element and its `prev` link is spare `Vec` capacity left by a
/// pop. No cursor into the vector survives a push or pop.
pub struct SyntaxFrame {
    pub syntax: SyntaxContext,
    /// Bash's `$[expression]` ends at `]` rather than at `))`.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub bracketed: bool,
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
    shell.input.command_depth = 0;
    shell.input.pending_here_documents = Vec::new();
    shell.input.completed_here_documents = Vec::new();
    shell.input.prompt_before_read = interactive;
    if shell.input.prompt_before_read {
        select_prompt(shell, PromptKind::Primary)?;
    }
    shell.input.prompt_needed = false;
    let mut result = list(shell, ListMode::TopLevel)?;
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

fn list(shell: &mut Shell, mode: ListMode) -> Result<ParseResult, Error> {
    let mut stop_at_terminator = mode == ListMode::StopAtTerminator;
    let newline_context = if mode == ListMode::TopLevel {
        TokenContext::NONE
    } else {
        TokenContext::SKIP_NEWLINES
    };
    let mut parsed_command: Option<Node>;
    let mut token: TokenKind;

    parsed_command = None;
    loop {
        token = read_token(shell, newline_context.with(TokenContext::COMMAND_START))?.kind;
        match token {
            TokenKind::Newline => {
                parse_here_documents(shell)?;
                return Ok(ParseResult::Tree(parsed_command));
            }

            TokenKind::Eof => {
                let eof = parsed_command.is_none() && newline_context == TokenContext::NONE;
                /* out_eof: */
                parse_here_documents(shell)?;
                shell.input.token_pushed_back = true;
                shell.input.last_token = TokenKind::Eof;
                return if eof {
                    Ok(ParseResult::Eof)
                } else {
                    Ok(ParseResult::Tree(parsed_command))
                };
            }
            _ => {}
        }

        shell.input.token_pushed_back = true;
        if stop_at_terminator && token.ends_list() {
            return Ok(ParseResult::Tree(parsed_command));
        }
        // Top-level input has no enclosing grammar production whose
        // terminator it may return to.  A stray `do`, `}`, and similar
        // token is therefore a syntax error here; only compound lists
        // begin accepting terminators after their first command.
        if mode != ListMode::TopLevel {
            stop_at_terminator = true;
        }

        /* The line the backgrounded command starts on, captured before
         * anything consumes it. `command()?` and `pipeline()?` both take
         * their `savelinno` at this same point, so a wrapper built here
         * records the line its contents record. */
        let saved_line_number = crate::input::current_input_frame(&mut shell.input).line_number;

        let mut next = parse_and_or(shell)?.ok_or_else(|| expected_token_error(shell, None))?;
        token = read_token(shell, TokenContext::NONE)?.kind;
        if token == TokenKind::Background {
            next = match next {
                Node::Pipeline(mut pipeline) => {
                    pipeline.background = true;
                    Node::Pipeline(pipeline)
                }
                Node::Redirect(wrapper) => Node::Background(wrapper),
                command => Node::Background(CompoundCommand {
                    line: saved_line_number,
                    command: Box::new(command),
                    redirections: Vec::new(),
                }),
            };
        }
        if let Some(left) = parsed_command.take() {
            parsed_command = Some(Node::Sequence(BinaryCommand {
                left: Box::new(left),
                right: Box::new(next),
            }));
        } else {
            parsed_command = Some(next);
        }
        match token {
            TokenKind::Eof => {
                parse_here_documents(shell)?;
                shell.input.token_pushed_back = true;
                shell.input.last_token = TokenKind::Eof;
                return Ok(ParseResult::Tree(parsed_command));
            }
            TokenKind::Newline => {
                shell.input.token_pushed_back = true;
            }
            TokenKind::Background | TokenKind::Semicolon => {}
            _ => {
                if newline_context == TokenContext::NONE {
                    return Err(expected_token_error(shell, None));
                }
                shell.input.token_pushed_back = true;
                return Ok(ParseResult::Tree(parsed_command));
            }
        }
    }
}

// [spec:dash:sem:parser.andor-fn]
// [spec:posix:syn:grammar.list-and-or]
// [spec:posix:def:cmd.and-or-list-definition]
// [spec:posix:req:cmd.and-or-precedence]
// [spec:posix:syn:cmd.and-list-format]
// [spec:posix:syn:cmd.or-list-format]
fn parse_and_or(shell: &mut Shell) -> Result<Option<Node>, Error> {
    let mut parsed_command: Option<Node>;

    parsed_command = pipeline(shell, TokenContext::NONE)?;
    loop {
        let operator: fn(BinaryCommand) -> Node = match read_token(shell, TokenContext::NONE)?.kind
        {
            TokenKind::AndIf => Node::And,
            TokenKind::OrIf => Node::Or,
            _ => {
                shell.input.token_pushed_back = true;
                return Ok(parsed_command);
            }
        };
        let left = parsed_command
            .take()
            .ok_or_else(|| expected_token_error(shell, None))?;
        let right = pipeline(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?
            .ok_or_else(|| expected_token_error(shell, None))?;
        parsed_command = Some(operator(BinaryCommand {
            left: Box::new(left),
            right: Box::new(right),
        }));
    }
}

// [spec:dash:sem:parser.pipeline-fn]
// [spec:posix:syn:grammar.pipeline]
// [spec:posix:def:cmd.pipeline-definition]
// [spec:posix:syn:cmd.pipeline-format]
// [spec:posix:req:cmd.pipeline-bang-subshell-separation]
fn pipeline(shell: &mut Shell, context: TokenContext) -> Result<Option<Node>, Error> {
    let line = crate::input::current_input_frame(&mut shell.input).line_number;
    let first = read_token(shell, context)?.kind;
    /* `time` prefixes the whole pipeline, `!` included -- `time ! true`
     * times the negation and answers 1 -- so it is read before the `!`
     * and wraps whatever the rest of this function builds. A bare `time`
     * has no pipeline to time and reports zeros, which is why the command
     * is optional. */
    // [spec:posix:req:token.reserved-word-time]
    // [spec:nsh:req:compat.bash.select-time-grammar]
    if first == TokenKind::Time {
        let posix_format = keywords::timed_posix_format(shell)?;
        return Ok(Some(Node::Timed(TimedCommand {
            line,
            posix_format,
            command: keywords::timed_pipeline(shell)?.map(Box::new),
        })));
    }
    let mut parsed_command: Option<Node>;
    let mut negate = false;
    let command_context = if first == TokenKind::Bang {
        negate = true;
        TokenContext::COMMAND_START
    } else {
        shell.input.token_pushed_back = true;
        TokenContext::NONE
    };
    parsed_command = keywords::nested_command(shell, command_context)?;
    if read_token(shell, TokenContext::NONE)?.kind == TokenKind::Pipe {
        /* Every `stalloc(sizeof(struct nodelist))` the C does here is one
         * `Vec` slot; the list is built front to back either way, and
         * `command()?` cannot return NULL without having raised first. */
        let mut render_command_list: Vec<Node> = vec![
            parsed_command
                .take()
                .ok_or_else(|| expected_token_error(shell, None))?,
        ];
        loop {
            render_command_list.push(
                keywords::nested_command(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?
                    .ok_or_else(|| expected_token_error(shell, None))?,
            );
            if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Pipe {
                break;
            }
        }
        parsed_command = Some(Node::Pipeline(Pipeline {
            background: false,
            commands: render_command_list,
        }));
    }
    shell.input.token_pushed_back = true;
    if negate {
        let command = parsed_command.ok_or_else(|| expected_token_error(shell, None))?;
        Ok(Some(Node::Not(NegatedCommand {
            command: Box::new(command),
        })))
    } else {
        Ok(parsed_command)
    }
}

// [spec:dash:sem:parser.command-fn]
// [spec:posix:syn:grammar.command]
// [spec:posix:syn:grammar.subshell-and-compound-list]
// [spec:posix:syn:grammar.for-clause]
// [spec:posix:syn:grammar.for-name]
// [spec:posix:syn:grammar.third-word-of-for-and-case]
// [spec:posix:syn:grammar.case-statement-termination]
// [spec:posix:syn:grammar.if-clause]
// [spec:posix:syn:grammar.while-until-clause]
// [spec:posix:syn:grammar.brace-group-and-do-group]
// [spec:posix:def:cmd.compound-definition]
// [spec:posix:req:cmd.group-double-paren-ambiguity]
// [spec:posix:req:cmd.for-do-done-delimiters]
// [spec:posix:syn:cmd.for-format]
// [spec:posix:syn:cmd.case-clause-syntax]
// [spec:posix:syn:cmd.case-format]
// [spec:posix:syn:cmd.if-format]
// [spec:posix:syn:cmd.while-format]
// [spec:posix:syn:cmd.until-format]
// [spec:nsh:req:idiom.structural-ast]
fn command(shell: &mut Shell, context: TokenContext) -> Result<Option<Node>, Error> {
    let mut parsed_command: Option<Node>;
    let closing_token: Option<TokenKind>;
    let saved_line_number = crate::input::current_input_frame(&mut shell.input).line_number;

    let token = read_token(shell, context)?.kind;
    if let Some(bash_node) = bash::command_prefix(shell, token, saved_line_number)? {
        parsed_command = Some(bash_node);
        closing_token = None;
    } else if token == TokenKind::If {
        /* The C threads the elif chain through `elsepart` on the way down,
         * writing each new nif into its parent before parsing it.  An owned
         * tree cannot hand out that parent pointer, so the clauses are
         * collected in parse order and folded back up afterwards; the
         * sequence of `list(0)?` calls — and so of everything they read — is
         * unchanged. */
        let mut clauses: Vec<(Node, Node)> = Vec::new();
        let parsed = list(shell, ListMode::Compound)?;
        let test = required_compound_node(shell, parsed, TokenKind::Then)?;
        if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Then {
            return Err(expected_token_error(shell, Some(TokenKind::Then)));
        }
        let parsed = list(shell, ListMode::Compound)?;
        let then_branch = required_compound_node(shell, parsed, TokenKind::Fi)?;
        clauses.push((test, then_branch));
        while read_token(shell, TokenContext::NONE)?.kind == TokenKind::Elif {
            let parsed = list(shell, ListMode::Compound)?;
            let test = required_compound_node(shell, parsed, TokenKind::Then)?;
            if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Then {
                return Err(expected_token_error(shell, Some(TokenKind::Then)));
            }
            let parsed = list(shell, ListMode::Compound)?;
            let then_branch = required_compound_node(shell, parsed, TokenKind::Fi)?;
            clauses.push((test, then_branch));
        }
        let mut else_branch: Option<Node> = if shell.input.last_token == TokenKind::Else {
            list(shell, ListMode::Compound)?.into_node()
        } else {
            shell.input.token_pushed_back = true;
            None
        };
        for (test, then_branch) in clauses.into_iter().rev() {
            else_branch = Some(Node::If(IfCommand {
                condition: Box::new(test),
                then_branch: Box::new(then_branch),
                else_branch: else_branch.map(Box::new),
            }));
        }
        parsed_command = else_branch;
        closing_token = Some(TokenKind::Fi);
    } else if token == TokenKind::While || token == TokenKind::Until {
        let constructor: fn(BinaryCommand) -> Node = if shell.input.last_token == TokenKind::While {
            Node::While
        } else {
            Node::Until
        };
        let parsed = list(shell, ListMode::Compound)?;
        let left_command = required_compound_node(shell, parsed, TokenKind::Do)?;
        let got = read_token(shell, TokenContext::NONE)?.kind;
        if got != TokenKind::Do {
            return Err(expected_token_error(shell, Some(TokenKind::Do)));
        }
        let parsed = list(shell, ListMode::Compound)?;
        let right_command = required_compound_node(shell, parsed, TokenKind::Done)?;
        parsed_command = Some(constructor(BinaryCommand {
            left: Box::new(left_command),
            right: Box::new(right_command),
        }));
        closing_token = Some(TokenKind::Done);
    } else if token == TokenKind::For {
        let var_token = read_token(shell, TokenContext::NONE)?;
        // The arithmetic form takes its own closing token, because Bash
        // lets it end at `}` as well as at `done`.
        let mut arithmetic_form = false;
        if var_token.kind == TokenKind::DoubleParen {
            arithmetic_form = true;
            parsed_command = Some(bash::arithmetic_for(shell, saved_line_number)?);
        } else {
            parsed_command = Some(Node::For(keywords::iteration_command(
                shell,
                saved_line_number,
                var_token,
            )?));
        }
        closing_token = (!arithmetic_form).then_some(TokenKind::Done);
    } else if token == TokenKind::Select {
        /* `for`'s syntax exactly; the menu and the read are the
         * evaluator's, not the grammar's. */
        // [spec:nsh:req:compat.bash.select-time-grammar]
        let var_token = read_token(shell, TokenContext::NONE)?;
        parsed_command = Some(Node::Select(keywords::iteration_command(
            shell,
            saved_line_number,
            var_token,
        )?));
        closing_token = Some(TokenKind::Done);
    } else if token == TokenKind::Case {
        if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Word {
            return Err(expected_token_error(shell, Some(TokenKind::Word)));
        }
        let expr = Node::Word(WordNode {
            word: mem::take(&mut shell.input.word),
        });
        if read_token(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?.kind != TokenKind::In {
            return Err(expected_token_error(shell, Some(TokenKind::In)));
        }
        let mut cases: Vec<CaseClause> = Vec::new();
        loop {
            // [spec:posix:syn:grammar.case-clause]
            // Rule 4 applies here, before an optional `(`, and nowhere in
            // the pattern loop below: words after `(` or `|` stay patterns
            // even when their spelling is otherwise a reserved word.
            let mut token = read_token(shell, TokenContext::RESERVED_WORDS_AFTER_NEWLINES)?.kind;
            if token == TokenKind::Esac {
                break;
            }
            if shell.input.last_token == TokenKind::LeftParen {
                read_token(shell, TokenContext::NONE)?;
            }
            let mut pattern: Vec<Node> = Vec::new();
            loop {
                if !shell.input.last_token.can_be_case_pattern() {
                    return Err(expected_token_error(shell, Some(TokenKind::Word)));
                }
                pattern.push(Node::Word(WordNode {
                    word: mem::take(&mut shell.input.word),
                }));
                if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Pipe {
                    break;
                }
                read_token(shell, TokenContext::NONE)?;
            }
            if shell.input.last_token != TokenKind::RightParen {
                return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
            }
            let body = list(shell, ListMode::StopAtTerminator)?.into_node();
            token = read_token(shell, TokenContext::RESERVED_WORDS_AFTER_NEWLINES)?.kind;
            cases.push(CaseClause {
                patterns: pattern,
                body: body.map(Box::new),
                fallthrough: token == TokenKind::FallThrough,
            });

            if token == TokenKind::Esac {
                break;
            }
            if token != TokenKind::EndCase && token != TokenKind::FallThrough {
                return Err(expected_token_error(shell, Some(TokenKind::EndCase)));
            }
        }
        parsed_command = Some(Node::Case(CaseCommand {
            line: saved_line_number,
            word: Box::new(expr),
            clauses: cases,
        }));
        closing_token = None;
    } else if token == TokenKind::LeftParen {
        let parsed = list(shell, ListMode::Compound)?;
        let inner = required_compound_node(shell, parsed, TokenKind::RightParen)?;
        parsed_command = Some(Node::Subshell(CompoundCommand {
            line: saved_line_number,
            command: Box::new(inner),
            redirections: Vec::new(),
        }));
        closing_token = Some(TokenKind::RightParen);
    } else if token == TokenKind::LeftBrace {
        parsed_command = list(shell, ListMode::Compound)?.into_node();
        closing_token = Some(TokenKind::RightBrace);
    } else if token == TokenKind::Word || token == TokenKind::Redirection {
        shell.input.token_pushed_back = true;
        return parse_simple_command(shell);
    } else {
        return Err(expected_token_error(shell, None));
    }

    if let Some(closing_token) = closing_token {
        if read_token(shell, TokenContext::NONE)?.kind != closing_token {
            return Err(expected_token_error(shell, Some(closing_token)));
        }
    }

    /* Now check for redirection which may follow command */
    let mut redirections: Vec<Redirection> = Vec::new();
    let mut redirection_context = TokenContext::COMMAND_START;
    while read_token(shell, redirection_context)?.kind == TokenKind::Redirection {
        redirection_context = TokenContext::NONE;
        /* The C copies `redirnode` into a local *before* `parsefname`,
         * because the token read inside it can set the global again.
         * Taking ownership of it here is the same guarantee. */
        let pending = core::mem::take(&mut shell.input.pending_redirection)
            .ok_or_else(|| syntax_error(shell, b"missing redirection operator state"))?;
        redirections.push(parse_redirection_target(shell, pending)?);
    }
    shell.input.token_pushed_back = true;
    if !redirections.is_empty() {
        parsed_command = Some(match parsed_command.take() {
            Some(Node::Subshell(mut wrapper)) => {
                wrapper.redirections = redirections;
                Node::Subshell(wrapper)
            }
            Some(command) => Node::Redirect(CompoundCommand {
                line: saved_line_number,
                command: Box::new(command),
                redirections,
            }),
            None => return Err(expected_token_error(shell, None)),
        });
    }

    Ok(parsed_command)
}

// [spec:dash:sem:parser.simplecmd-fn]
// [spec:posix:req:redir.not-in-command-arguments]
// [spec:posix:syn:grammar.simple-command]
// [spec:posix:syn:grammar.assignment-first-word]
// [spec:posix:syn:grammar.assignment-word-recognition]
// [spec:posix:syn:grammar.function-definition]
// [spec:posix:syn:grammar.function-name]
// [spec:posix:req:grammar.function-body-no-expansion]
// [spec:posix:def:cmd.simple-definition]
// [spec:posix:def:cmd.function-definition-term]
// [spec:posix:syn:cmd.function-format]
// [spec:posix:req:cmd.function-name-requirements]
// [spec:posix:req:cmd.function-no-expansion-at-definition]
fn parse_simple_command(shell: &mut Shell) -> Result<Option<Node>, Error> {
    let mut args: Vec<Node> = Vec::new();
    let mut variables: Vec<Node> = Vec::new();
    let mut redirections: Vec<Redirection> = Vec::new();
    let mut word_context = TokenContext::ALIASES;
    let saved_line_number = crate::input::current_input_frame(&mut shell.input).line_number;
    loop {
        let token = read_token(shell, word_context)?.kind;
        if token == TokenKind::Word {
            let ordinary_assignment = shell.input.word.is_assignment(&shell.locale);
            let mut node = Node::Word(WordNode {
                word: mem::take(&mut shell.input.word),
            });
            if bash::active(shell)
                && (word_context != TokenContext::NONE || bash::declaration_context(&args))
            {
                node = match node {
                    Node::Word(word) => match bash::array_word(shell, word) {
                        Ok(array) => array,
                        Err(word) => Node::Word(word),
                    },
                    _ => unreachable!("a freshly parsed word is an argument node"),
                };
            }
            let bash_assignment =
                matches!(node, Node::Bash(crate::nodes::BashNode::ArrayAssignment(_)));
            if word_context != TokenContext::NONE && (ordinary_assignment || bash_assignment) {
                variables.push(node);
            } else {
                args.push(node);
                word_context = TokenContext::NONE;
            }
        } else if token == TokenKind::Redirection {
            let pending = core::mem::take(&mut shell.input.pending_redirection)
                .ok_or_else(|| syntax_error(shell, b"missing redirection operator state"))?;
            redirections.push(parse_redirection_target(shell, pending)?);
        } else {
            if token == TokenKind::LeftParen
                && bash::active(shell)
                && bash::compound_array(shell, &mut variables, &mut args)?
            {
                continue;
            }
            /* The C's `app == &args->narg.next` says the argument list holds
             * exactly one word, which is the name being defined. */
            if token == TokenKind::LeftParen
                && args.len() == 1
                && variables.is_empty()
                && redirections.is_empty()
            {
                /* We have a function */
                if read_token(shell, TokenContext::NONE)?.kind != TokenKind::RightParen {
                    return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
                }
                /* the word becomes the function's name; the C keeps the same
                 * `char *` when it relabels the node */
                let Some(Node::Word(word)) = args.pop() else {
                    return Err(syntax_error(shell, b"Bad function name"));
                };
                let builtin_spec = crate::execution::builtin(shell, word.word.as_bstr());
                if !bash::accepts_function_name(shell, word.word.as_bstr())
                    || builtin_spec.is_some_and(|cmd| cmd.attributes().is_special())
                {
                    return Err(syntax_error(shell, b"Bad function name"));
                }
                /* Move the parsed name into a dedicated function variant so
                 * the tree never passes through an invalid intermediate. */
                let line_number = crate::input::current_input_frame(&mut shell.input).line_number;
                let body =
                    keywords::nested_command(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?
                        .ok_or_else(|| expected_token_error(shell, None))?;
                return Ok(Some(Node::Function(FunctionDefinition {
                    line: line_number,
                    name: NodeText::new(BString::from(word.word.as_bstr())),
                    body: Box::new(body),
                })));
            }
            shell.input.token_pushed_back = true;
            break;
        }
    }
    /* out: */
    Ok(Some(Node::Command(SimpleCommand {
        line: saved_line_number,
        assignments: variables,
        arguments: args,
        redirections,
    })))
}

// [spec:dash:sem:parser.makename-fn]
pub(crate) fn make_name_node(shell: &mut Shell) -> Node {
    Node::Word(WordNode {
        word: mem::take(&mut shell.input.word),
    })
}

// [spec:dash:sem:parser.parsefname-fn]
// [spec:dash:sem:parser.fixredir-fn]
// [spec:posix:req:redir.here-doc-quoted-delimiter]
// [spec:posix:req:redir.here-doc-unquoted-delimiter]
// [spec:posix:req:grammar.here-doc-redirection]
//
// The C reads the redirection node out of the `redirnode` global; here the
// caller has already taken ownership of it, because the `readtoken` below can
// set that global again before this function is done with it.
fn parse_redirection_target(
    shell: &mut Shell,
    pending: PendingRedirection,
) -> Result<Redirection, Error> {
    let is_here_document = matches!(pending, PendingRedirection::HereDocument { .. });
    let token = read_token(
        shell,
        if is_here_document {
            TokenContext::HERE_DOCUMENT_END
        } else {
            TokenContext::NONE
        },
    )?;
    if token.kind != TokenKind::Word {
        return Err(expected_token_error(shell, None));
    }
    let redirection = match pending {
        PendingRedirection::HereDocument { descriptor } => {
            let mut here = core::mem::take(&mut shell.input.pending_here_document)
                .ok_or_else(|| syntax_error(shell, b"missing here-document delimiter state"))?;
            let expand = !token.quoted;
            here.delimiter = BString::from(shell.input.word.as_bstr());
            here.expand = expand;
            shell.input.pending_here_documents.push(here);
            Redirection::HereDocument(HereDocument {
                descriptor,
                expand,
                body: WordNode {
                    word: ParsedWord::new(),
                },
            })
        }
        PendingRedirection::HereString { descriptor } => Redirection::HereString(HereString {
            descriptor,
            word: WordNode {
                word: mem::take(&mut shell.input.word),
            },
        }),
        PendingRedirection::Descriptor {
            operator,
            descriptor,
        } => {
            let text = shell.input.word_text();
            /* "If word evaluates to one or more digits, the file
             * descriptor denoted by n shall be a duplicate" -- so `>&42`
             * duplicates onto slot 42, not only `>&2`. */
            // [spec:posix:req:redir.duplicate-output]
            let target = if let Some(number) = LogicalDescriptor::from_digits(text) {
                DescriptorTarget::Number(number)
            } else if text == BStr::new(b"-") {
                DescriptorTarget::Close
            } else {
                DescriptorTarget::Word(WordNode {
                    word: mem::take(&mut shell.input.word),
                })
            };
            Redirection::Descriptor(DescriptorRedirection {
                operator,
                descriptor,
                target,
            })
        }
        PendingRedirection::File {
            operator,
            descriptor,
        } => Redirection::File(FileRedirection {
            operator,
            descriptor,
            target: WordNode {
                word: mem::take(&mut shell.input.word),
            },
        }),
    };
    Ok(redirection)
}

/*
 * Input any here documents.
 */

// [spec:dash:sem:parser.parseheredoc-fn]
// [spec:posix:req:redir.here-doc-line-continuation]
// [spec:posix:req:redir.here-doc-backslash]
// [spec:posix:req:redir.here-doc-multiple]
// [spec:posix:req:redir.here-doc-ps2]
// [spec:posix:req:token.here-document-mode]
fn parse_here_documents(shell: &mut Shell) -> Result<(), Error> {
    let list: Vec<PendingHereDocument> = core::mem::take(&mut shell.input.pending_here_documents);

    for here in list {
        if shell.input.prompt_needed {
            select_prompt(shell, PromptKind::Continuation)?;
        }
        let mark = EofMark::Word(BStr::new(&here.delimiter));
        /* The C reads the first character inside the argument list. The
         * receiver is passed there too, so the read is its own statement:
         * evaluation order is unchanged, the first character is still
         * read before `readtoken1` runs. */
        if !here.expand {
            let firstc = read_input_unit(shell)?;
            read_word_token(
                shell,
                firstc,
                SyntaxContext::SingleQuoted,
                mark,
                here.strip_tabs,
                false,
                false,
            )?;
        } else {
            let firstc = read_unit_skipping_line_continuations(shell)?;
            read_word_token(
                shell,
                firstc,
                SyntaxContext::DoubleQuoted,
                mark,
                here.strip_tabs,
                false,
                false,
            )?;
        }
        let body = WordNode {
            word: mem::take(&mut shell.input.word),
        };
        shell.input.completed_here_documents.push(body);
    }
    Ok(())
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
        token = read_next_token(shell, &context)?;

        /*
         * eat newlines
         */
        if context.skip_newlines {
            while token.kind == TokenKind::Newline {
                parse_here_documents(shell)?;
                /* The alias bit is dropped with the rest: dash clears the
                 * whole of `checkkwd` here, and the bit lived in it. */
                shell.input.clear_alias_boundary();
                token = read_next_token(shell, &context)?;
            }
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
        input = read_unit_skipping_line_continuations(shell)?;
        if input.is(b' ') || input.is(b'\t') {
            shell.input.last_token_after_blank = true;
            continue;
        } else if input.is(b'#') && !regex_operand {
            loop {
                input = read_input_unit(shell)?;
                if input.is(b'\n') || input == InputUnit::EndOfInput {
                    break;
                }
            }
            unread_input_unit(shell);
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
                false,
            )?;
            if token.kind != TokenKind::Blank {
                return Ok(token);
            }
        } else if input.is(b'&') {
            if read_unit_skipping_line_continuations(shell)?.is(b'&') {
                shell.input.last_token = TokenKind::AndIf;
                shell.input.last_token_quoted = false;
                return Ok(Token::plain(TokenKind::AndIf));
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
            context.assignment_position,
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

mod multibyte;
mod syntax_stack;
mod word_lexer;

pub(crate) use multibyte::MultibyteMode;
use word_lexer::{ParenthesisOutcome, WordPosition, close_parenthesis};

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
pub(crate) fn read_multibyte_character(
    shell: &mut Shell,
    input: InputUnit,
    mode: MultibyteMode,
) -> Result<MultibyteInput, Error> {
    let Some(mut byte) = input.byte() else {
        return Ok(MultibyteInput::SingleByte);
    };
    let mut decoder = shell.locale.decoder();
    let mut bytes = BString::new(Vec::new());
    let mut wc: i32 = 0;
    let mut complete = false;
    let escaped = matches!(mode, MultibyteMode::Escaped);

    if byte.is_ascii() {
        return Ok(MultibyteInput::SingleByte);
    }

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
        let converted = crate::escape::parse_escape(&text, true);
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
    destination.extend(bytes.into_iter().map(WordToken::Escaped));
    Ok(())
}

/*
 * If eofmark is NULL, read a word or a redirection symbol.  If eofmark
 * is not NULL, read a here document.  In the latter case, eofmark is the
 * word which marks the end of the document and strip_tabs is true if
 * leading tabs should be stripped from the document.  The argument firstc
 * is the first character of the input token or document.
 *
 * The word lexer delegates here-document checks, redirections,
 * substitutions, backquotes, and arithmetic to focused helpers that borrow
 * the current lexer state.
 */

/// The locals of `readtoken1` that its internal subroutines share.
struct WordLexer<'a> {
    /// Owned parse contexts, base first and current last. Popping retains the
    /// allocation, matching the C's reuse of its most recently popped level.
    syntax_frames: Vec<SyntaxFrame>,
    /// The unquoted context a closing quote returns to.
    base_syntax: SyntaxContext,
    check_here_document_end: bool,
    preserve_escapes: bool,
    dollar_single_quoted: bool,
    input: InputUnit,
    quoted: bool,
    /// How many `X(` extended-glob groups are open in this word. While
    /// one is, `(`, `)`, `|`, and blanks are the pattern's own bytes.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    extglob_depth: usize,
    /// Whether this word could be an assignment: only at the start of a
    /// simple command, where Bash's lexer reads `name[` as the opening
    /// of a subscript rather than as the end of a name.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    assignment_position: bool,
    /// How many `[` of an assignment word's subscript are open. While
    /// one is, blanks and shell operators are the subscript's own bytes.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    subscript_depth: usize,
    /// Typed lexer events for the word being built.
    output: Vec<WordToken>,
    delimiter: EofMark<'a>,
    strip_tabs: bool,
}

// [spec:dash:sem:parser.readtoken1-fn]
// [spec:posix:req:shell.hashbang-unspecified]
// [spec:posix:sem:shell.tokenization-and-parsing]
// [spec:posix:def:quote.purpose]
// [spec:posix:req:quote.always-special-characters]
// [spec:posix:req:quote.conditionally-special-characters]
// [spec:posix:req:quote.future-special-characters]
// [spec:posix:def:quote.mechanisms]
// [spec:posix:req:quote.backslash-literal]
// [spec:posix:req:quote.single-quotes]
// [spec:posix:req:quote.double-quotes-literal]
// [spec:posix:req:quote.double-quotes-dollar-sign]
// [spec:posix:req:quote.double-quotes-command-substitution]
// [spec:posix:req:quote.double-quotes-substring-parameter-expansion]
// [spec:posix:req:quote.double-quotes-other-parameter-expansion]
// [spec:posix:req:quote.double-quotes-backquote]
// [spec:posix:req:quote.double-quotes-backquote-undefined]
// [spec:posix:req:quote.double-quotes-backslash]
// [spec:posix:req:quote.double-quotes-expansion-result]
// [spec:posix:req:quote.double-quotes-embedded-double-quote]
// [spec:posix:req:quote.dollar-single-quotes]
// [spec:posix:syn:token.quoting-characters]
// [spec:posix:syn:token.expansion-candidates]
// [spec:posix:syn:token.append-to-word]
// [spec:nsh:req:idiom.parser-control-flow]
fn read_word_token(
    shell: &mut Shell,
    first_input: InputUnit,
    syntax: SyntaxContext,
    delimiter: EofMark<'_>,
    strip_tabs: bool,
    check_here_document_end: bool,
    assignment_position: bool,
) -> Result<Token, Error> {
    let mut lexer = WordLexer {
        syntax_frames: vec![SyntaxFrame {
            syntax,
            bracketed: false,
            inner_double_quote: false,
            variable_context_pushed: false,
            double_quoted: syntax == SyntaxContext::DoubleQuoted,
            backquote: BackquoteContext::None,
            variable_depth: 0,
            parenthesis_depth: 0,
            double_quote_variable_depth: 0,
        }],
        base_syntax: if syntax == SyntaxContext::Regex {
            SyntaxContext::Regex
        } else {
            SyntaxContext::Base
        },
        check_here_document_end,
        preserve_escapes: syntax == SyntaxContext::SingleQuoted,
        dollar_single_quoted: false,
        input: first_input,
        quoted: false,
        extglob_depth: 0,
        assignment_position: assignment_position && bash::active(shell) && delimiter.is_none(),
        subscript_depth: 0,
        output: Vec::new(),
        delimiter,
        strip_tabs,
    };
    'word: loop {
        /* for each line, until end of word */
        finish_word_if_delimited(shell, &mut lexer)?;
        /* Until end of line or end of word */
        loop {
            let position = WordPosition::of(lexer.current_syntax());
            let field_splitting =
                position.field_splitting && lexer.extglob_depth == 0 && lexer.subscript_depth == 0;
            bash::process_substitutions(shell, &mut lexer, field_splitting)?;
            if bash::open_extended_glob(shell, &mut lexer)? {
                continue;
            }
            /* The C's CHECKSTRSPACE, which permits max(MB_LEN_MAX, 23)
             * calls to USTPUTC, has no counterpart here: `getmbc`
             * writes into its own scratch and `getmbc_at` appends
             * what it reports, so there is no room for this frame to
             * make on its behalf. */
            let multibyte_mode = MultibyteMode::for_word(field_splitting, lexer.preserve_escapes);
            match read_multibyte_character(shell, lexer.input, multibyte_mode)? {
                MultibyteInput::FieldBoundary => {
                    if lexer.output.is_empty() {
                        return Ok(Token::plain(TokenKind::Blank));
                    }
                    lexer.input = read_input_unit(shell)?;
                    break 'word;
                }
                MultibyteInput::Character { bytes, escaped } => {
                    lexer.push_multibyte(bytes, escaped);
                    lexer.input = read_unit_for_syntax(shell, lexer.current_syntax())?;
                    continue;
                }
                MultibyteInput::SingleByte => {}
            }

            let class = lexer.current_syntax().syntax.classify(lexer.input);

            match class {
                SyntaxClass::Newline => {
                    if field_splitting || position.regex_word {
                        break 'word;
                    }
                    lexer.push_literal(lexer.input.expect_byte());
                    prompt_after_newline(shell)?;
                    lexer.input = read_unit_for_syntax(shell, lexer.current_syntax())?;
                    continue 'word;
                }
                SyntaxClass::Word => {
                    bash::track_assignment_subscript(shell, &mut lexer);
                    if !bash::close_arithmetic_bracket(&mut lexer) {
                        lexer.push_literal(lexer.input.expect_byte());
                    }
                }
                SyntaxClass::Control => {
                    if lexer.dollar_single_quoted && lexer.input.is(b'\\') {
                        parse_dollar_single_quote_escape(shell, &mut lexer.output)?;
                    } else {
                        if lexer.delimiter.is_none()
                            || lexer.current_syntax().double_quoted
                            || lexer.current_syntax().variable_depth != 0
                        {
                            lexer.push_escaped(lexer.input.expect_byte());
                        } else {
                            lexer.push_literal(lexer.input.expect_byte());
                        }
                    }
                }
                SyntaxClass::Backslash => word_lexer::read_backslash(shell, &mut lexer)?,
                SyntaxClass::SingleQuote => {
                    lexer.current_syntax_mut().syntax = SyntaxContext::SingleQuoted;
                    lexer.record_quote_boundary(QuoteBoundary::Open, false);
                }
                SyntaxClass::DoubleQuote => {
                    lexer.current_syntax_mut().syntax = SyntaxContext::DoubleQuoted;
                    lexer.current_syntax_mut().double_quoted = true;
                    lexer.record_quote_boundary(QuoteBoundary::Open, true);
                }
                SyntaxClass::EndQuote => lexer.close_quote(),
                SyntaxClass::Variable => parse_parameter_expansion(shell, &mut lexer)?,
                SyntaxClass::EndVariable => lexer.close_parameter_expansion(),
                SyntaxClass::LeftParen => {
                    lexer.current_syntax_mut().parenthesis_depth += 1;
                    lexer.push_literal(lexer.input.expect_byte());
                }
                SyntaxClass::RightParen => match close_parenthesis(shell, &mut lexer, position)? {
                    ParenthesisOutcome::EndWord => break 'word,
                    ParenthesisOutcome::Advanced => continue,
                    ParenthesisOutcome::Consumed => {}
                },
                SyntaxClass::Backquote => {
                    if lexer.current_syntax().backquote == BackquoteContext::Legacy {
                        syntax_stack::pop(&mut lexer.syntax_frames);
                        lexer.preserve_escapes = false;
                        lexer.push_literal(lexer.input.expect_byte());
                    } else {
                        lexer.push_literal(b'`');
                        parse_command_substitution(shell, &mut lexer, true)?;
                    }
                }
                SyntaxClass::EndOfInput | SyntaxClass::EndOfAlias => break 'word,
                SyntaxClass::WordSeparator => {
                    if bash::inside_extended_glob(&mut lexer) {
                        lexer.push_literal(lexer.input.expect_byte());
                    } else if lexer.input.is(b')')
                        && lexer.current_syntax().backquote == BackquoteContext::Modern
                    {
                        syntax_stack::pop(&mut lexer.syntax_frames);
                        lexer.preserve_escapes = false;
                        lexer.push_literal(lexer.input.expect_byte());
                    } else if field_splitting || position.regex_boundary {
                        break 'word;
                    } else {
                        lexer.push_literal(lexer.input.expect_byte());
                    }
                }
            }

            lexer.input = read_unit_for_syntax(shell, lexer.current_syntax())?;
        }
    }
    finish_word_token(shell, &mut lexer)
}

/// Close one word: reject an unterminated construct, hand a bare
/// descriptor digit to the redirection parser, and otherwise publish the
/// structural word the lexer built.
// [spec:dash:sem:parser.readtoken1-fn]
fn finish_word_token(shell: &mut Shell, lexer: &mut WordLexer<'_>) -> Result<Token, Error> {
    if lexer.current_syntax().syntax == SyntaxContext::Arithmetic {
        return Err(syntax_error(shell, b"Missing '))'"));
    }
    if (!matches!(
        lexer.current_syntax().syntax,
        SyntaxContext::Base | SyntaxContext::Regex
    ) && lexer.delimiter.is_none())
        || lexer.current_syntax().backquote != BackquoteContext::None
    {
        return Err(syntax_error(shell, b"Unterminated quoted string"));
    }
    if lexer.current_syntax().variable_depth != 0 {
        /* { */
        return Err(syntax_error(shell, b"Missing '}'"));
    }
    /* An assignment word's subscript swallowed the blanks and operators
     * inside it, so one that never closed has swallowed the rest of the
     * input. Bash reports the same unterminated construct. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    if lexer.subscript_depth != 0 {
        return Err(syntax_error(shell, b"Missing ']'"));
    }
    /* IO_NUMBER is "a string consisting solely of digits", not one digit:
     * `exec 42>file` names slot 42. The outer `Option` is whether this is
     * an IO_NUMBER at all; the inner one is whether it carried a number,
     * since an operator with nothing before it takes its own default. A
     * digit run too large to name a slot is not an IO_NUMBER either, and
     * the standard says the token identifier is then TOKEN -- an ordinary
     * word, which is what falling through to the bottom of this function
     * produces. */
    // [spec:posix:syn:grammar.token-classification]
    // [spec:posix:syn:redir.format]
    let io_number: Option<Option<LogicalDescriptor>> = if lexer.output.is_empty() {
        Some(None)
    } else {
        lexer
            .output
            .iter()
            .map(|token| match token {
                WordToken::Literal(byte) if byte.is_ascii_digit() => Some(*byte),
                _ => None,
            })
            .collect::<Option<Vec<u8>>>()
            .and_then(|digits| LogicalDescriptor::from_digits(&digits))
            .map(Some)
    };
    if lexer.delimiter.is_none() {
        if let Some(explicit) =
            io_number.filter(|_| (lexer.input.is(b'>') || lexer.input.is(b'<')) && !lexer.quoted)
        {
            parse_redirection(shell, lexer, explicit)?;
            shell.input.last_token = TokenKind::Redirection;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::Redirection));
        }
        unread_input_unit(shell);
    }
    shell.input.last_token_quoted = lexer.quoted;
    /* `grabstackblock(len)` reserved the bytes the C had been writing into
     * scratch space, which is what made `wordtext` outlive the next token.
     * Moving the buffer out is the same guarantee. */
    // [spec:nsh:def:idiom.word-ir]
    shell.input.word = ParsedWord::from_tokens(mem::take(&mut lexer.output));
    shell.input.last_token = TokenKind::Word;
    Ok(Token {
        kind: TokenKind::Word,
        quoted: lexer.quoted,
    })
}
/* end of readtoken routine */

/*
 * Check to see whether we are at the end of the here document.  When this
 * is called, c is set to the first character of the next input line.  If
 * we are at the end of the here document, this routine records an explicit
 * end-of-input boundary.
 */

/* checkend: */
// [spec:posix:req:redir.here-doc-delimiter]
// [spec:posix:req:redir.here-doc-tab-strip]
fn finish_word_if_delimited(shell: &mut Shell, lexer: &mut WordLexer<'_>) -> Result<(), Error> {
    if let Some(mark) = lexer.delimiter.real() {
        let mut index: usize;
        let mut more_heredoc = false;

        if lexer.strip_tabs {
            while lexer.input.is(b'\t') {
                lexer.input = read_input_unit(shell)?;
            }
        }

        let mut consumed = Vec::new();
        index = 0;
        loop {
            if let Some(byte) = lexer.input.byte() {
                consumed.push(byte);
            }
            if index == mark.len() {
                break;
            }
            if !lexer.input.is(mark[index]) {
                more_heredoc = true;
                break;
            }

            lexer.input = read_input_unit(shell)?;
            index += 1;
        }

        if !more_heredoc {
            if lexer.input.is(b'\n') || lexer.input == InputUnit::EndOfInput {
                lexer.input = InputUnit::EndOfInput;
                consume_newline_without_prompt(shell);
            } else {
                more_heredoc = true;
            }
        }

        if more_heredoc {
            if let Some((&first, rest)) = consumed.split_first() {
                lexer.input = InputUnit::Byte(first);
                if !rest.is_empty() {
                    push_string_input(shell, BStr::new(rest), None);
                }
            }
        }
    }
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
    explicit: Option<LogicalDescriptor>,
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
    let mut descriptor: LogicalDescriptor;
    let redirection: ParsedRedirection;

    if lexer.input.is(b'>') {
        descriptor = LogicalDescriptor::STDOUT;
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
        descriptor = LogicalDescriptor::STDIN;
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
        },
    });
    Ok(())
}

// [spec:posix:req:expand.param-hash-requires-word]
fn parse_parameter_operator(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    bad_substitution: bool,
    parameter_syntax: &mut ParameterSyntax,
    nested_syntax: &mut SyntaxContext,
) -> Result<(), Error> {
    if bad_substitution {
        unread_input_unit(shell);
    } else if parameter_syntax.operation == ParameterOperation::Invalid {
        let current_unit = lexer.input;

        /* A here-document delimiter is recorded literally so the body can
         * be matched against it later, but the input can end in the
         * middle of one -- `<<${e` is the whole file -- and then there is
         * no byte to record. Ending is not an operator either, so this
         * falls through and the parse fails for the reason it really
         * failed: an unterminated construct, which is what every other
         * shell reports. Found by fuzzing; it panicked here. */
        if lexer.check_here_document_end
            && let Some(byte) = lexer.input.byte()
        {
            lexer.push_literal(byte);
        }

        if let Some(operation) = bash::parameter_operator(shell, lexer)? {
            parameter_syntax.operation = operation;
            *nested_syntax = SyntaxContext::Base;
        } else if lexer.input.is(b'%') || lexer.input.is(b'#') {
            let trim_prefix = lexer.input.is(b'#');
            parameter_syntax.operation = if trim_prefix {
                ParameterOperation::RemoveSmallestPrefix
            } else {
                ParameterOperation::RemoveSmallestSuffix
            };
            lexer.input = read_unit_skipping_line_continuations(shell)?;
            if lexer.input == current_unit {
                if lexer.check_here_document_end {
                    lexer.push_literal(lexer.input.expect_byte());
                }
                parameter_syntax.operation = if trim_prefix {
                    ParameterOperation::RemoveLargestPrefix
                } else {
                    ParameterOperation::RemoveLargestSuffix
                };
            } else {
                unread_input_unit(shell);
            }

            *nested_syntax = SyntaxContext::Base;
        } else {
            if lexer.input.is(b':') {
                parameter_syntax.colon = true;
                lexer.input = read_unit_skipping_line_continuations(shell)?;
                if lexer.check_here_document_end {
                    lexer.push_literal(lexer.input.expect_byte());
                }
            }
            parameter_syntax.operation = match lexer.input.byte() {
                Some(b'}') if !parameter_syntax.colon || !bash::active(shell) => {
                    ParameterOperation::Value
                }
                Some(b'-') => ParameterOperation::Default,
                Some(b'+') => ParameterOperation::Alternate,
                Some(b'?') => ParameterOperation::Error,
                Some(b'=') => ParameterOperation::Assign,
                _ if parameter_syntax.colon && bash::active(shell) => {
                    // `${name:offset:length}` reuses the colon that
                    // `${name:-word}` spends on its own operator, so the
                    // byte that decided against those forms belongs to
                    // the offset expression and is read again.
                    unread_input_unit(shell);
                    parameter_syntax.colon = false;
                    *nested_syntax = SyntaxContext::Base;
                    ParameterOperation::Substring
                }
                _ => ParameterOperation::Invalid,
            };
        }
    } else {
        if parameter_syntax.operation == ParameterOperation::Length && !lexer.input.is(b'}') {
            parameter_syntax.operation = ParameterOperation::Invalid;
        }
        unread_input_unit(shell);
    }
    Ok(())
}

/*
 * Parse a substitution.  At this point, we have read the dollar sign
 * and nothing else.
 */
// [spec:posix:syn:expand.param-format]
// [spec:posix:syn:expand.param-braces-optional]
// [spec:posix:syn:expand.param-unbraced-resolution]
fn parse_parameter_expansion(shell: &mut Shell, lexer: &mut WordLexer<'_>) -> Result<(), Error> {
    let mut nested_syntax = lexer.current_syntax().syntax;
    let substitution_start = lexer.output.len();

    lexer.push_literal(b'$');

    lexer.input = read_unit_skipping_line_continuations(shell)?;
    if lexer.input.is(b'(') {
        /* $(command) or $((arith)) */
        lexer.push_literal(lexer.input.expect_byte());
        if read_unit_skipping_line_continuations(shell)?.is(b'(') {
            parse_arithmetic_expansion(lexer)?;
        } else {
            unread_input_unit(shell);
            parse_command_substitution(shell, lexer, false)?;
        }
    } else if lexer.input.is(b'\'')
        && nested_syntax.classify(InputUnit::Byte(b'&')) != SyntaxClass::Word
    {
        lexer.output.truncate(substitution_start);
        lexer.dollar_single_quoted = true;
        lexer.current_syntax_mut().syntax = SyntaxContext::SingleQuoted;
        lexer.record_quote_boundary(QuoteBoundary::Open, false);
        return Ok(());
    } else if bash::locale_quote(shell, lexer, nested_syntax, substitution_start)
        || bash::arithmetic_bracket(shell, lexer, substitution_start)
    {
        return Ok(());
    } else if lexer.input.is(b'{')
        || lexer.input.begins_name(&shell.locale)
        || lexer.input.is_special_parameter()
    {
        let mut parameter_syntax = ParameterSyntax::unbraced();
        if lexer.input.is(b'{') {
            if lexer.check_here_document_end {
                lexer.push_literal(b'{');
            }
            lexer.input = read_unit_skipping_line_continuations(shell)?;
            parameter_syntax = ParameterSyntax::braced();
        }
        let indirection = bash::parameter_indirection(shell, lexer, parameter_syntax.braced)?;
        let indirect = indirection == bash::Indirection::Present;
        let mut bad_substitution = indirection == bash::Indirection::Invalid;
        let name_start = lexer.output.len();
        'assignment_name: loop {
            if bad_substitution {
                break 'assignment_name;
            }
            if lexer.input.begins_name(&shell.locale) {
                loop {
                    lexer.push_literal(lexer.input.expect_byte());
                    lexer.input = read_unit_skipping_line_continuations(shell)?;
                    if !lexer.input.continues_name(&shell.locale) {
                        break;
                    }
                }
            } else if lexer.input.is_digit() {
                loop {
                    lexer.push_literal(lexer.input.expect_byte());
                    lexer.input = read_unit_skipping_line_continuations(shell)?;
                    if !(parameter_syntax.accepts_multiple_name_digits() && lexer.input.is_digit())
                    {
                        break;
                    }
                }
            } else if !lexer.input.is(b'}') {
                let mut current_unit = lexer.input;

                lexer.input = read_unit_skipping_line_continuations(shell)?;

                if parameter_syntax.accepts_array_subscript() && current_unit.is(b'#') {
                    parameter_syntax.operation = ParameterOperation::Length;

                    if lexer.input.is(b'_')
                        || lexer
                            .input
                            .byte()
                            .is_some_and(|byte| shell.locale.is_alphanumeric(byte))
                    {
                        if lexer.check_here_document_end {
                            lexer.push_literal(b'#');
                        }
                        continue 'assignment_name;
                    }

                    current_unit = lexer.input;
                    lexer.input = read_unit_skipping_line_continuations(shell)?;
                    if current_unit.is(b'}') || !lexer.input.is(b'}') {
                        unread_input_unit(shell);
                        parameter_syntax.operation = ParameterOperation::Invalid;
                        lexer.input = current_unit;
                        current_unit = InputUnit::Byte(b'#');
                    } else if lexer.check_here_document_end {
                        lexer.push_literal(b'#');
                    }
                }

                if !current_unit.is_special_parameter() {
                    if parameter_syntax.operation == ParameterOperation::Length {
                        parameter_syntax.operation = ParameterOperation::Invalid;
                    }
                    bad_substitution = true;
                    break 'assignment_name;
                }

                lexer.push_literal(current_unit.expect_byte());
            } else {
                bad_substitution = true;
                break 'assignment_name;
            }
            break 'assignment_name;
        }

        bash::parameter_subscript(
            shell,
            lexer,
            bad_substitution,
            parameter_syntax.accepts_subscript_operand(),
        )?;
        bash::parameter_prefix_selector(shell, lexer, indirect && !bad_substitution)?;
        let name_end = lexer.output.len();

        parse_parameter_operator(
            shell,
            lexer,
            bad_substitution,
            &mut parameter_syntax,
            &mut nested_syntax,
        )?;

        if nested_syntax == SyntaxContext::Arithmetic {
            nested_syntax = SyntaxContext::DoubleQuoted;
        }

        if (nested_syntax != lexer.current_syntax().syntax
            || lexer.current_syntax().inner_double_quote)
            && parameter_syntax.has_operand()
        {
            syntax_stack::push(&mut lexer.syntax_frames, nested_syntax);

            lexer.current_syntax_mut().variable_context_pushed = true;
            lexer.current_syntax_mut().double_quoted = nested_syntax != SyntaxContext::Base;
        }

        if parameter_syntax.has_operand() {
            lexer.current_syntax_mut().variable_depth += 1;
            if lexer.current_syntax().double_quoted {
                lexer.current_syntax_mut().double_quote_variable_depth += 1;
            }
        }
        if !lexer.check_here_document_end {
            let name = lexer
                .literal_bytes(name_start..name_end)
                .unwrap_or_default();
            lexer.output.truncate(substitution_start);
            lexer.output.push(WordToken::ParameterStart {
                name,
                operation: parameter_syntax.operation,
                colon: parameter_syntax.colon,
                indirect,
            });
        }
    } else {
        unread_input_unit(shell);
    }

    Ok(())
}

/*
 * Called to parse command substitutions.  oldstyle is set if the command
 * is enclosed inside `...` rather than $(...).
 */

/* parsebackq: */
// [spec:posix:def:expand.cmdsub-forms]
// [spec:posix:req:expand.cmdsub-backquote-backslash]
// [spec:posix:req:expand.cmdsub-backquote-matching]
// [spec:posix:syn:expand.cmdsub-dollar-paren-extent]
// [spec:posix:req:expand.cmdsub-parsing]
// [spec:posix:req:expand.cmdsub-redirections-only]
// [spec:posix:req:expand.cmdsub-alias-substitution]
// [spec:posix:req:expand.cmdsub-terminating-paren]
// [spec:posix:req:expand.cmdsub-nesting]
// [spec:posix:req:expand.cmdsub-arith-ambiguity]
fn parse_command_substitution(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    legacy: bool,
) -> Result<(), Error> {
    let mut saved_prompt_enabled = false;
    /* `grabstackstr(pout)` had to reserve the backquote's text because
     * `list(2)?` builds on the same stack; owning it says the same thing, and
     * it has to outlive the `popfile` below because `setinputstring` reads
     * through the pointer rather than copying. */
    let mut substitution_text: BString = BString::new(Vec::new());

    if lexer.check_here_document_end {
        syntax_stack::push(&mut lexer.syntax_frames, SyntaxContext::Base);
        lexer.current_syntax_mut().backquote = if legacy {
            BackquoteContext::Legacy
        } else {
            BackquoteContext::Modern
        };
        lexer.preserve_escapes = true;
        if legacy {
            shell.input.token_pushed_back = false;
        }
        return Ok(());
    }
    let introducer_length = if legacy { 1 } else { 2 };
    lexer
        .output
        .truncate(lexer.output.len().saturating_sub(introducer_length));
    if legacy {
        /* We must read until the closing backquote, giving special
        treatment to some slashes, and then push the string and
        reread it as input, interpreting it normally.  */
        let mut input: InputUnit;

        loop {
            if shell.input.prompt_needed {
                select_prompt(shell, PromptKind::Continuation)?;
            }
            input = read_unit_skipping_line_continuations(shell)?;
            if input.is(b'`') {
                break;
            } else if input.is(b'\\') {
                input = read_input_unit(shell)?;
                if input.byte().is_none() {
                    return Err(syntax_error(shell, b"EOF in backquote substitution"));
                }
                if !input.is(b'\\')
                    && !input.is(b'`')
                    && !input.is(b'$')
                    && (!lexer.current_syntax().double_quoted || !input.is(b'"'))
                {
                    substitution_text.push(b'\\');
                }
                if let MultibyteInput::Character { bytes, .. } =
                    read_multibyte_character(shell, input, MultibyteMode::Literal)?
                {
                    substitution_text.extend_from_slice(&bytes);
                    continue;
                }
            } else if input == InputUnit::EndOfInput {
                return Err(syntax_error(shell, b"EOF in backquote substitution"));
            } else if input.is(b'\n') {
                consume_newline_without_prompt(shell);
            }
            substitution_text.push(input.expect_byte());
        }
    }
    let saved_here_documents = core::mem::take(&mut shell.input.pending_here_documents);
    let completed_at = shell.input.completed_here_documents.len();

    if legacy {
        saved_prompt_enabled = shell.input.prompt_before_read;
        shell.input.prompt_before_read = false;
    }

    let parsed = crate::resource::with_resources(shell, |shell, _resources| {
        if legacy {
            set_input_string(shell, BStr::new(&substitution_text));
        }
        let mut node = list(shell, ListMode::StopAtTerminator)?.into_node();

        if !legacy {
            if read_token(shell, TokenContext::NONE)?.kind != TokenKind::RightParen {
                return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
            }
            set_input_string(shell, BStr::new(b""));
        }

        parse_here_documents(shell)?;
        finalize::node(shell, &mut node, completed_at)?;
        Ok(node)
    });

    if legacy {
        shell.input.prompt_before_read = saved_prompt_enabled;
    }
    shell.input.pending_here_documents = saved_here_documents;
    if legacy {
        /* Ignore any pushed back tokens left from the backquote
         * parsing.
         */
        shell.input.token_pushed_back = false;
    }
    lexer.output.push(WordToken::Command(parsed?));
    Ok(())
}

/*
 * Parse an arithmetic expansion (indicate start of one and set state)
 */
/* parsearith: */
// [spec:posix:syn:expand.arith-format]
fn parse_arithmetic_expansion(lexer: &mut WordLexer<'_>) -> Result<(), Error> {
    syntax_stack::push(&mut lexer.syntax_frames, SyntaxContext::Arithmetic);
    lexer.current_syntax_mut().double_quoted = true;
    if lexer.check_here_document_end {
        lexer.push_literal(lexer.input.expect_byte());
    } else {
        lexer.output.truncate(lexer.output.len().saturating_sub(2));
        lexer.output.push(WordToken::ArithmeticStart);
    }
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
                false,
            )?;

            let node = Node::Word(WordNode {
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
