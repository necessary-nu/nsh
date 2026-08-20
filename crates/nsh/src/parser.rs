//! Shell parser derived from `src/parser.c` / `src/parser.h`.
//! Rules: `docs/spec/port/src/parser.md`.
//!
//! Parsing uses structural syntax nodes and ordinary Rust control flow.
//! Helpers such as `checkend`, `parseredir`, `parsesub`, `parsebackq`, and
//! `parsearith` operate on the current word-lexer state directly.

use core::mem;
use std::io::Write;

use bstr::{BStr, BString};
use core::ffi::{c_char, c_int, c_uint};

use crate::context::Shell;
use crate::error::Error;
use crate::expand::{EXP_QUOTED, expandarg, restore_handler_expandarg};
use crate::fd::LogicalDescriptor;
use crate::input::{
    pgetc, pgetc_eoa, popfile, pungetc, pungetn, pushstring, setinputstring, unwindfiles,
};
use crate::nodes::{
    BinaryCommand, CaseClause, CaseCommand, CompoundCommand, DescriptorRedirection,
    DescriptorRedirectionOperator, DescriptorTarget, FileRedirection, FileRedirectionOperator,
    ForCommand, FunctionDefinition, HereDocument, IfCommand, NegatedCommand, Node, NodeText,
    Pipeline, Redirection, SimpleCommand, WordNode,
};
use crate::syntax::{InputUnit, SyntaxClass, SyntaxContext, is_digit, is_in_name, is_name};
use crate::word::ParsedWord;

// ---------------------------------------------------------------------
// Local transcriptions of the C macros this file uses.
// ---------------------------------------------------------------------

/* `#define equal(s1, s2) (strcmp(s1, s2) == 0)` — src/mystring.h.  The
 * macro had one caller and `strcmp`'s ordering was never wanted there,
 * only its zero; `issimplecmd` now compares the two byte slices, and the
 * word's text already knows its own length. */

/* `USTPUTC` (src/memalloc.h:88) and `STADJUST` (:93) were the last two
 * `memalloc.h` macros this file expanded, and they are gone. Neither ever
 * touched the region allocator -- they were a store-and-advance and an
 * advance over a `BString`'s spare capacity -- which is why
 * [[delete-memalloc]] closed with them still here and recorded rehoming
 * them as bookkeeping.
 *
 * What deletes them is not a rehoming. Their two remaining callers,
 * `getmbc` and `dollarsq_escape`, took a raw cursor because the buffer
 * they wrote into belonged to someone else; both write into their own
 * scratch now, so the cursor is an offset and each macro is one indexed
 * statement written out with the C's name beside it. */

/// `MB_LEN_MAX` from `<limits.h>` (16 on the platforms dash targets).
const MB_LEN_MAX: usize = 16;

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
    EndBackquote,
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
    Then,
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
                | Self::EndBackquote
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
            Self::EndBackquote => b"\"`\"",
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
}

impl TokenContext {
    pub(crate) const NONE: Self = Self {
        aliases: false,
        reserved_words: false,
        skip_newlines: false,
        check_here_document_end: false,
    };
    const ALIASES: Self = Self {
        aliases: true,
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
        ..Self::NONE
    };
    const COMMAND_START_AFTER_NEWLINES: Self = Self {
        aliases: true,
        reserved_words: true,
        skip_newlines: true,
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
        }
    }
}

// [spec:posix:def:grammar.reserved-word-tokens]
// [spec:posix:def:token.reserved-words]
// [spec:posix:def:token.reserved-words-optional]
// [spec:posix:req:token.reserved-word-time]
// [spec:posix:def:token.reserved-words-trailing-colon]
static RESERVED_WORDS: [(&[u8], TokenKind); 16] = [
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
    (b"then", TokenKind::Then),
    (b"until", TokenKind::Until),
    (b"while", TokenKind::While),
    (b"{", TokenKind::LeftBrace),
    (b"}", TokenKind::RightBrace),
];

// ---------------------------------------------------------------------
// src/parser.h
// ---------------------------------------------------------------------

/* control characters in argument strings */
pub const CTL_FIRST: c_int = -127; /* first 'special' character */
pub const CTLESC: c_int = -127; /* escape next character */
pub const CTLVAR: c_int = -126; /* variable defn */
pub const CTLENDVAR: c_int = -125;
pub const CTLBACKQ: c_int = -124;
pub const CTLMBCHAR: c_int = -123;
pub const CTLARI: c_int = -122; /* arithmetic expression */
pub const CTLENDARI: c_int = -121;
pub const CTLQUOTEMARK: c_int = -120;
pub const CTL_LAST: c_int = -120; /* last 'special' character */

/* variable substitution byte (follows CTLVAR) */
pub const VSTYPE: c_int = 0x0f; /* type of variable substitution */
pub const VSNUL: c_int = 0x10; /* colon--treat the empty string as unset */
pub const VSBIT: c_int = 0x20; /* Ensure subtype is not zero */

/* values of VSTYPE field */
pub const VSNORMAL: c_int = 0x1; /* normal variable:  $var or ${var} */
pub const VSMINUS: c_int = 0x2; /* ${var-text} */
pub const VSPLUS: c_int = 0x3; /* ${var+text} */
pub const VSQUESTION: c_int = 0x4; /* ${var?message} */
pub const VSASSIGN: c_int = 0x5; /* ${var=text} */
pub const VSTRIMRIGHT: c_int = 0x6; /* ${var%pattern} */
pub const VSTRIMRIGHTMAX: c_int = 0x7; /* ${var%%pattern} */
pub const VSTRIMLEFT: c_int = 0x8; /* ${var#pattern} */
pub const VSTRIMLEFTMAX: c_int = 0x9; /* ${var##pattern} */
pub const VSLENGTH: c_int = 0xa; /* ${#var} */
/* VSLENGTH must come last. */

/// What the C returns from `list()` and `parsecmd`.
///
/// `union node *` is three things there at once: a tree, `NULL` for a blank
/// line, and the sentinel `#define NEOF ((union node *)&tokpushback)` for end
/// of file. Only `list(1)?` — that is, `parsecmd` — can produce `NEOF`,
/// because the `n1 = NEOF` assignment is guarded by `chknl == 0` and `chknl`
/// is only zero when `nlflag & 1`.
pub enum ParseResult {
    /// the C's `NEOF`
    Eof,
    /// a tree, or `None` where the C returned `NULL` for a blank line
    Tree(Option<Node>),
}

impl ParseResult {
    /// The tree, for the callers of `list()` that cannot see `NEOF`.
    fn into_node(self) -> Option<Node> {
        match self {
            ParseResult::Tree(n) => n,
            ParseResult::Eof => None,
        }
    }
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

    /// `#define realeofmark(m) ((m) && (m) != FAKEEOFMARK)` — src/parser.c
    // [spec:dash:def:parser.realeofmark-fn]
    // [spec:dash:sem:parser.realeofmark-fn]
    fn real(self) -> Option<&'a BStr> {
        match self {
            EofMark::Word(w) => Some(w),
            _ => None,
        }
    }
}

// ---------------------------------------------------------------------

// [spec:dash:def:parser.heredoc]
/// A here-document delimiter waiting for its body at a grammar newline.
pub struct heredoc {
    /// an expandable here-document uses double-quoted rather than
    /// single-quoted lexical rules
    pub expand: bool,
    /// string indicating end of input, with `rmescapes` already applied
    pub eofmark: BString,
    pub striptabs: c_int, /* if set, strip leading tabs */
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
}

// [spec:dash:def:parser.synstack]
/// One owned parse context. [`Rt1::synstack`] holds the contexts from the
/// base level through the current level, so the C's `next` link is the
/// preceding element and its `prev` link is spare `Vec` capacity left by a
/// pop. No cursor into the vector survives a push or pop.
pub struct synstack {
    pub syntax: SyntaxContext,
    pub innerdq: c_int,
    pub varpushed: c_int,
    pub dblquote: c_int,
    pub backq: c_int,      /* Inside back quotes (here-doc word only). */
    pub varnest: c_int,    /* levels of variables expansion */
    pub parenlevel: c_int, /* levels of parens in arithmetic */
    pub dqvarnest: c_int,  /* levels of variables expansion within double quotes */
}

/// A token together with the word property the C parser carried in the
/// separate `quoteflag` global.
#[derive(Clone, Copy)]
struct Token {
    kind: TokenKind,
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

// [spec:dash:def:parser.isassignment-fn]
// [spec:dash:sem:parser.isassignment-fn]
pub fn isassignment(locale: &nsh_platform::Locale, text: &BStr) -> c_int {
    let end = endofname(locale, text);
    if end == 0 {
        return 0;
    }
    (text.get(end) == Some(&b'=')) as c_int
}

// [spec:dash:def:parser.issimplecmd-fn]
// [spec:dash:sem:parser.issimplecmd-fn]
pub fn issimplecmd(n: Option<&Node>, name: &BStr) -> c_int {
    match n {
        Some(Node::Command(command)) => command
            .arguments
            .first()
            .and_then(|argument| match argument {
                Node::Word(word) => Some(word.word.as_bstr()),
                _ => None,
            })
            .is_some_and(|word| word == name) as c_int,
        _ => 0,
    }
}

/// The last word read, as its shell-visible bytes.
///
/// Input storage retains the terminating NUL expected by the lexer;
/// `cstr_prefix` removes it without exposing a pointer.
fn wordtext(sh: &Shell) -> &BStr {
    sh.input.word.as_bstr()
}

/// The last word read, as a node's owned text.
///
/// The bytes are cloned into the syntax tree's owned storage.
fn wordtext_node(sh: &mut Shell) -> NodeText {
    let mut text = BString::from(sh.input.word.as_bstr());
    text.push(0);
    NodeText::new(text)
}

/*
 * Read and parse a command.  Returns NEOF on end of file.  (NULL is a
 * valid parse tree indicating a blank line.)
 */

// [spec:dash:def:parser.parsecmd-fn]
// [spec:dash:sem:parser.parsecmd-fn]
// [spec:posix:syn:grammar.program]
// [spec:posix:def:cmd.command-kinds]
// [spec:posix:syn:cmd.format-descriptions-informal]
// [spec:posix:req:cmd.no-size-limit]
// [spec:nsh:req:compat.bash.parse-boundary]
pub fn parsecmd(sh: &mut Shell, interact: c_int) -> Result<ParseResult, Error> {
    let dialect = sh.options.dialect();
    sh.input.begin_parse(dialect);
    sh.input.tokpushback = false;
    sh.input.heredoclist = Vec::new();
    sh.input.completed_heredocs = Vec::new();
    sh.input.doprompt = interact;
    if sh.input.doprompt != 0 {
        setprompt(sh, sh.input.doprompt);
    }
    sh.input.needprompt = 0;
    let mut result = list(sh, 1)?;
    let bodies = core::mem::take(&mut sh.input.completed_heredocs);
    finalize::parse_result(sh, &mut result, bodies)?;
    Ok(result)
}

// [spec:dash:def:parser.list-fn]
// [spec:dash:sem:parser.list-fn]
// [spec:posix:syn:grammar.separators]
// [spec:posix:def:cmd.list-definition]
// [spec:posix:def:cmd.compound-list-definition]
// [spec:posix:req:cmd.list-separator-semantics]
fn list(sh: &mut Shell, nlflag: c_int) -> Result<ParseResult, Error> {
    let mut nlflag = nlflag;
    let newline_context = if nlflag & 1 != 0 {
        TokenContext::NONE
    } else {
        TokenContext::SKIP_NEWLINES
    };
    let mut n1: Option<Node>;
    let mut tok: TokenKind;

    n1 = None;
    loop {
        tok = readtoken(sh, newline_context.with(TokenContext::COMMAND_START))?;
        match tok {
            TokenKind::Newline => {
                parseheredoc(sh)?;
                return Ok(ParseResult::Tree(n1));
            }

            TokenKind::Eof => {
                let eof = n1.is_none() && newline_context == TokenContext::NONE;
                /* out_eof: */
                parseheredoc(sh)?;
                sh.input.tokpushback = true;
                sh.input.lasttoken = TokenKind::Eof;
                return if eof {
                    Ok(ParseResult::Eof)
                } else {
                    Ok(ParseResult::Tree(n1))
                };
            }
            _ => {}
        }

        sh.input.tokpushback = true;
        if nlflag == 2 && tok.ends_list() {
            return Ok(ParseResult::Tree(n1));
        }
        nlflag |= 2;

        /* The line the backgrounded command starts on, captured before
         * anything consumes it. `command()?` and `pipeline()?` both take
         * their `savelinno` at this same point, so a wrapper built here
         * records the line its contents record. */
        let savelinno: c_int = crate::plinno!(sh);

        let mut next = andor(sh)?.ok_or_else(|| synexpect(sh, None))?;
        tok = readtoken(sh, TokenContext::NONE)?;
        if tok == TokenKind::Background {
            next = match next {
                Node::Pipeline(mut pipeline) => {
                    pipeline.background = true;
                    Node::Pipeline(pipeline)
                }
                Node::Redirect(wrapper) => Node::Background(wrapper),
                command => Node::Background(CompoundCommand {
                    line: savelinno,
                    command: Box::new(command),
                    redirections: Vec::new(),
                }),
            };
        }
        if let Some(left) = n1.take() {
            n1 = Some(Node::Sequence(BinaryCommand {
                left: Box::new(left),
                right: Box::new(next),
            }));
        } else {
            n1 = Some(next);
        }
        match tok {
            TokenKind::Eof => {
                parseheredoc(sh)?;
                sh.input.tokpushback = true;
                sh.input.lasttoken = TokenKind::Eof;
                return Ok(ParseResult::Tree(n1));
            }
            TokenKind::Newline => {
                sh.input.tokpushback = true;
            }
            TokenKind::Background | TokenKind::Semicolon => {}
            _ => {
                if newline_context == TokenContext::NONE {
                    return Err(synexpect(sh, None));
                }
                sh.input.tokpushback = true;
                return Ok(ParseResult::Tree(n1));
            }
        }
    }
}

// [spec:dash:def:parser.andor-fn]
// [spec:dash:sem:parser.andor-fn]
// [spec:posix:syn:grammar.list-and-or]
// [spec:posix:def:cmd.and-or-list-definition]
// [spec:posix:req:cmd.and-or-precedence]
// [spec:posix:syn:cmd.and-list-format]
// [spec:posix:syn:cmd.or-list-format]
fn andor(sh: &mut Shell) -> Result<Option<Node>, Error> {
    let mut n1: Option<Node>;

    n1 = pipeline(sh, TokenContext::NONE)?;
    loop {
        let operator: fn(BinaryCommand) -> Node = match readtoken(sh, TokenContext::NONE)? {
            TokenKind::AndIf => Node::And,
            TokenKind::OrIf => Node::Or,
            _ => {
                sh.input.tokpushback = true;
                return Ok(n1);
            }
        };
        let left = n1.take().ok_or_else(|| synexpect(sh, None))?;
        let right = pipeline(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)?
            .ok_or_else(|| synexpect(sh, None))?;
        n1 = Some(operator(BinaryCommand {
            left: Box::new(left),
            right: Box::new(right),
        }));
    }
}

// [spec:dash:def:parser.pipeline-fn]
// [spec:dash:sem:parser.pipeline-fn]
// [spec:posix:syn:grammar.pipeline]
// [spec:posix:def:cmd.pipeline-definition]
// [spec:posix:syn:cmd.pipeline-format]
// [spec:posix:req:cmd.pipeline-bang-subshell-separation]
fn pipeline(sh: &mut Shell, context: TokenContext) -> Result<Option<Node>, Error> {
    let mut n1: Option<Node>;
    let mut negate: c_int;

    negate = 0;
    /* TRACE(("pipeline: entered\n")); */
    let command_context = if readtoken(sh, context)? == TokenKind::Bang {
        negate = (negate == 0) as c_int;
        TokenContext::COMMAND_START
    } else {
        sh.input.tokpushback = true;
        TokenContext::NONE
    };
    n1 = command(sh, command_context)?;
    if readtoken(sh, TokenContext::NONE)? == TokenKind::Pipe {
        /* Every `stalloc(sizeof(struct nodelist))` the C does here is one
         * `Vec` slot; the list is built front to back either way, and
         * `command()?` cannot return NULL without having raised first. */
        let mut cmdlist: Vec<Node> = vec![n1.take().ok_or_else(|| synexpect(sh, None))?];
        loop {
            cmdlist.push(
                command(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)?
                    .ok_or_else(|| synexpect(sh, None))?,
            );
            if readtoken(sh, TokenContext::NONE)? != TokenKind::Pipe {
                break;
            }
        }
        n1 = Some(Node::Pipeline(Pipeline {
            background: false,
            commands: cmdlist,
        }));
    }
    sh.input.tokpushback = true;
    if negate != 0 {
        let command = n1.ok_or_else(|| synexpect(sh, None))?;
        Ok(Some(Node::Not(NegatedCommand {
            command: Box::new(command),
        })))
    } else {
        Ok(n1)
    }
}

// [spec:dash:def:parser.command-fn]
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
fn command(sh: &mut Shell, context: TokenContext) -> Result<Option<Node>, Error> {
    let mut n1: Option<Node>;
    let closing_token: Option<TokenKind>;
    let savelinno: c_int;

    savelinno = crate::plinno!(sh);

    let tok = readtoken(sh, context)?;
    if let Some(bash_node) = bash::command_prefix(sh, tok, savelinno)? {
        n1 = Some(bash_node);
        closing_token = None;
    } else if tok == TokenKind::If {
        /* The C threads the elif chain through `elsepart` on the way down,
         * writing each new nif into its parent before parsing it.  An owned
         * tree cannot hand out that parent pointer, so the clauses are
         * collected in parse order and folded back up afterwards; the
         * sequence of `list(0)?` calls — and so of everything they read — is
         * unchanged. */
        let mut clauses: Vec<(Node, Node)> = Vec::new();
        let test = list(sh, 0)?
            .into_node()
            .ok_or_else(|| synexpect(sh, None))?;
        if readtoken(sh, TokenContext::NONE)? != TokenKind::Then {
            return Err(synexpect(sh, Some(TokenKind::Then)));
        }
        let ifpart = list(sh, 0)?
            .into_node()
            .ok_or_else(|| synexpect(sh, None))?;
        clauses.push((test, ifpart));
        while readtoken(sh, TokenContext::NONE)? == TokenKind::Elif {
            let test = list(sh, 0)?
                .into_node()
                .ok_or_else(|| synexpect(sh, None))?;
            if readtoken(sh, TokenContext::NONE)? != TokenKind::Then {
                return Err(synexpect(sh, Some(TokenKind::Then)));
            }
            let ifpart = list(sh, 0)?
                .into_node()
                .ok_or_else(|| synexpect(sh, None))?;
            clauses.push((test, ifpart));
        }
        let mut elsepart: Option<Node> = if sh.input.lasttoken == TokenKind::Else {
            list(sh, 0)?.into_node()
        } else {
            sh.input.tokpushback = true;
            None
        };
        for (test, ifpart) in clauses.into_iter().rev() {
            elsepart = Some(Node::If(IfCommand {
                condition: Box::new(test),
                then_branch: Box::new(ifpart),
                else_branch: elsepart.map(Box::new),
            }));
        }
        n1 = elsepart;
        closing_token = Some(TokenKind::Fi);
    } else if tok == TokenKind::While || tok == TokenKind::Until {
        let got: TokenKind;
        let constructor: fn(BinaryCommand) -> Node = if sh.input.lasttoken == TokenKind::While {
            Node::While
        } else {
            Node::Until
        };
        let ch1 = list(sh, 0)?
            .into_node()
            .ok_or_else(|| synexpect(sh, None))?;
        got = readtoken(sh, TokenContext::NONE)?;
        if got != TokenKind::Do {
            return Err(synexpect(sh, Some(TokenKind::Do)));
        }
        let ch2 = list(sh, 0)?
            .into_node()
            .ok_or_else(|| synexpect(sh, None))?;
        n1 = Some(constructor(BinaryCommand {
            left: Box::new(ch1),
            right: Box::new(ch2),
        }));
        closing_token = Some(TokenKind::Done);
    } else if tok == TokenKind::For {
        let var_token = readtoken_with_flags(sh, TokenContext::NONE)?;
        if var_token.kind == TokenKind::DoubleParen {
            n1 = Some(bash::arithmetic_for(sh, savelinno)?);
        } else {
            if var_token.kind != TokenKind::Word
                || var_token.quoted
                || goodname(&sh.locale, wordtext(sh)) == 0
            {
                return Err(synerror(sh, b"Bad for loop variable"));
            }
            /* the C stores `wordtext` into the node here, before any further
             * token read can overwrite it */
            let var = wordtext_node(sh);
            let mut args: Vec<Node> = Vec::new();
            if readtoken(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)? == TokenKind::In {
                while readtoken(sh, TokenContext::NONE)? == TokenKind::Word {
                    args.push(Node::Word(WordNode {
                        word: mem::take(&mut sh.input.word),
                    }));
                }
                if sh.input.lasttoken != TokenKind::Newline
                    && sh.input.lasttoken != TokenKind::Semicolon
                {
                    return Err(synexpect(sh, None));
                }
            } else {
                /* The implicit `"$@"` of a `for` with no `in`. `dolatstr` is
                 * seven bytes ending in the NUL that a word's text keeps, so
                 * the value is the static and not what a C reader makes of
                 * it. */
                let dolatstr: [u8; 7] = crate::mystring::dolatstr.map(|c| c as u8);
                args.push(Node::Word(WordNode {
                    word: ParsedWord::from_legacy(BString::from(&dolatstr[..]), Vec::new()),
                }));
                /*
                 * Newline or semicolon here is optional (but note
                 * that the original Bourne shell only allowed NL).
                 */
                if sh.input.lasttoken != TokenKind::Semicolon {
                    sh.input.tokpushback = true;
                }
            }
            if readtoken(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)? != TokenKind::Do {
                return Err(synexpect(sh, Some(TokenKind::Do)));
            }
            let body = list(sh, 0)?
                .into_node()
                .ok_or_else(|| synexpect(sh, None))?;
            n1 = Some(Node::For(ForCommand {
                line: savelinno,
                words: args,
                body: Box::new(body),
                variable: var,
            }));
        }
        closing_token = Some(TokenKind::Done);
    } else if tok == TokenKind::Case {
        if readtoken(sh, TokenContext::NONE)? != TokenKind::Word {
            return Err(synexpect(sh, Some(TokenKind::Word)));
        }
        let expr = Node::Word(WordNode {
            word: mem::take(&mut sh.input.word),
        });
        if readtoken(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)? != TokenKind::In {
            return Err(synexpect(sh, Some(TokenKind::In)));
        }
        let mut cases: Vec<CaseClause> = Vec::new();
        loop {
            // [spec:posix:syn:grammar.case-clause]
            // Rule 4 applies here, before an optional `(`, and nowhere in
            // the pattern loop below: words after `(` or `|` stay patterns
            // even when their spelling is otherwise a reserved word.
            let mut token = readtoken(sh, TokenContext::RESERVED_WORDS_AFTER_NEWLINES)?;
            if token == TokenKind::Esac {
                break;
            }
            if sh.input.lasttoken == TokenKind::LeftParen {
                readtoken(sh, TokenContext::NONE)?;
            }
            let mut pattern: Vec<Node> = Vec::new();
            loop {
                if !sh.input.lasttoken.can_be_case_pattern() {
                    return Err(synexpect(sh, Some(TokenKind::Word)));
                }
                pattern.push(Node::Word(WordNode {
                    word: mem::take(&mut sh.input.word),
                }));
                if readtoken(sh, TokenContext::NONE)? != TokenKind::Pipe {
                    break;
                }
                readtoken(sh, TokenContext::NONE)?;
            }
            if sh.input.lasttoken != TokenKind::RightParen {
                return Err(synexpect(sh, Some(TokenKind::RightParen)));
            }
            let body = list(sh, 2)?.into_node();
            token = readtoken(sh, TokenContext::RESERVED_WORDS_AFTER_NEWLINES)?;
            cases.push(CaseClause {
                patterns: pattern,
                body: body.map(Box::new),
                fallthrough: token == TokenKind::FallThrough,
            });

            if token == TokenKind::Esac {
                break;
            }
            if token != TokenKind::EndCase && token != TokenKind::FallThrough {
                return Err(synexpect(sh, Some(TokenKind::EndCase)));
            }
        }
        n1 = Some(Node::Case(CaseCommand {
            line: savelinno,
            word: Box::new(expr),
            clauses: cases,
        }));
        closing_token = None;
    } else if tok == TokenKind::LeftParen {
        let inner = list(sh, 0)?
            .into_node()
            .ok_or_else(|| synexpect(sh, None))?;
        n1 = Some(Node::Subshell(CompoundCommand {
            line: savelinno,
            command: Box::new(inner),
            redirections: Vec::new(),
        }));
        closing_token = Some(TokenKind::RightParen);
    } else if tok == TokenKind::LeftBrace {
        n1 = list(sh, 0)?.into_node();
        closing_token = Some(TokenKind::RightBrace);
    } else if tok == TokenKind::Word || tok == TokenKind::Redirection {
        sh.input.tokpushback = true;
        return simplecmd(sh);
    } else {
        return Err(synexpect(sh, None));
        /* NOTREACHED */
    }

    if let Some(closing_token) = closing_token {
        if readtoken(sh, TokenContext::NONE)? != closing_token {
            return Err(synexpect(sh, Some(closing_token)));
        }
    }

    /* Now check for redirection which may follow command */
    let mut redir: Vec<Redirection> = Vec::new();
    let mut redirection_context = TokenContext::COMMAND_START;
    while readtoken(sh, redirection_context)? == TokenKind::Redirection {
        redirection_context = TokenContext::NONE;
        /* The C copies `redirnode` into a local *before* `parsefname`,
         * because the token read inside it can set the global again.
         * Taking ownership of it here is the same guarantee. */
        let pending = core::mem::take(&mut sh.input.redirnode)
            .ok_or_else(|| synerror(sh, b"missing redirection operator state"))?;
        redir.push(parsefname(sh, pending)?);
    }
    sh.input.tokpushback = true;
    if !redir.is_empty() {
        n1 = Some(match n1.take() {
            Some(Node::Subshell(mut wrapper)) => {
                wrapper.redirections = redir;
                Node::Subshell(wrapper)
            }
            Some(command) => Node::Redirect(CompoundCommand {
                line: savelinno,
                command: Box::new(command),
                redirections: redir,
            }),
            None => return Err(synexpect(sh, None)),
        });
    }

    Ok(n1)
}

// [spec:dash:def:parser.simplecmd-fn]
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
fn simplecmd(sh: &mut Shell) -> Result<Option<Node>, Error> {
    let mut args: Vec<Node> = Vec::new();
    let mut vars: Vec<Node> = Vec::new();
    let mut redir: Vec<Redirection> = Vec::new();
    let mut word_context: TokenContext;
    let savelinno: c_int;

    word_context = TokenContext::ALIASES;
    savelinno = crate::plinno!(sh);
    loop {
        let tok = readtoken(sh, word_context)?;
        if tok == TokenKind::Word {
            let ordinary_assignment = isassignment(&sh.locale, wordtext(sh)) != 0;
            let mut n = Node::Word(WordNode {
                word: mem::take(&mut sh.input.word),
            });
            if bash::active(sh)
                && (word_context != TokenContext::NONE || bash::declaration_context(&args))
            {
                n = match n {
                    Node::Word(word) => match bash::array_word(sh, word) {
                        Ok(array) => array,
                        Err(word) => Node::Word(word),
                    },
                    _ => unreachable!("a freshly parsed word is an argument node"),
                };
            }
            let bash_assignment =
                matches!(n, Node::Bash(crate::nodes::BashNode::ArrayAssignment(_)));
            if word_context != TokenContext::NONE && (ordinary_assignment || bash_assignment) {
                vars.push(n);
            } else {
                args.push(n);
                word_context = TokenContext::NONE;
            }
        } else if tok == TokenKind::Redirection {
            let pending = core::mem::take(&mut sh.input.redirnode)
                .ok_or_else(|| synerror(sh, b"missing redirection operator state"))?;
            redir.push(parsefname(sh, pending)?);
        } else {
            if tok == TokenKind::LeftParen
                && bash::active(sh)
                && bash::compound_array(sh, &mut vars, &mut args)?
            {
                continue;
            }
            /* The C's `app == &args->narg.next` says the argument list holds
             * exactly one word, which is the name being defined. */
            if tok == TokenKind::LeftParen && args.len() == 1 && vars.is_empty() && redir.is_empty()
            {
                /* We have a function */
                if readtoken(sh, TokenContext::NONE)? != TokenKind::RightParen {
                    return Err(synexpect(sh, Some(TokenKind::RightParen)));
                }
                /* the word becomes the function's name; the C keeps the same
                 * `char *` when it relabels the node */
                let Some(Node::Word(word)) = args.pop() else {
                    return Err(synerror(sh, b"Bad function name"));
                };
                let bcmd = crate::exec::builtin(sh, word.word.as_bstr());
                if goodname(&sh.locale, word.word.as_bstr()) == 0
                    || bcmd.is_some_and(|cmd| (cmd.flags & crate::builtins::BUILTIN_SPECIAL) != 0)
                {
                    return Err(synerror(sh, b"Bad function name"));
                }
                /* The C relabels its argument union arm as a function node
                 * in place. Moving the parsed name into a dedicated function
                 * variant states the result without an invalid intermediate. */
                let linno = crate::plinno!(sh);
                let body = command(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)?
                    .ok_or_else(|| synexpect(sh, None))?;
                return Ok(Some(Node::Function(FunctionDefinition {
                    line: linno,
                    name: {
                        let mut name = BString::from(word.word.as_bstr());
                        name.push(0);
                        NodeText::new(name)
                    },
                    body: Box::new(body),
                })));
            }
            sh.input.tokpushback = true;
            break;
        }
    }
    /* out: */
    Ok(Some(Node::Command(SimpleCommand {
        line: savelinno,
        assignments: vars,
        arguments: args,
        redirections: redir,
    })))
}

// [spec:dash:def:parser.makename-fn]
// [spec:dash:sem:parser.makename-fn]
pub(crate) fn makename(sh: &mut Shell) -> Node {
    Node::Word(WordNode {
        word: mem::take(&mut sh.input.word),
    })
}

// [spec:dash:def:parser.parsefname-fn]
// [spec:dash:sem:parser.parsefname-fn]
// [spec:posix:req:redir.here-doc-quoted-delimiter]
// [spec:posix:req:redir.here-doc-unquoted-delimiter]
// [spec:posix:req:grammar.here-doc-redirection]
//
// The C reads the redirection node out of the `redirnode` global; here the
// caller has already taken ownership of it, because the `readtoken` below can
// set that global again before this function is done with it.
fn parsefname(sh: &mut Shell, pending: PendingRedirection) -> Result<Redirection, Error> {
    let is_here_document = matches!(pending, PendingRedirection::HereDocument { .. });
    let token = readtoken_with_flags(
        sh,
        if is_here_document {
            TokenContext::HERE_DOCUMENT_END
        } else {
            TokenContext::NONE
        },
    )?;
    if token.kind != TokenKind::Word {
        return Err(synexpect(sh, None));
    }
    let redirection = match pending {
        PendingRedirection::HereDocument { descriptor } => {
            let mut here: heredoc = core::mem::take(&mut sh.input.heredoc)
                .ok_or_else(|| synerror(sh, b"missing here-document delimiter state"))?;
            let expand = !token.quoted;
            here.eofmark = BString::from(sh.input.word.as_bstr());
            here.expand = expand;
            sh.input.heredoclist.push(here);
            Redirection::HereDocument(HereDocument {
                descriptor,
                expand,
                body: WordNode {
                    word: ParsedWord::new(),
                },
            })
        }
        PendingRedirection::Descriptor {
            operator,
            descriptor,
        } => {
            let text = crate::mystring::cstr_prefix(wordtext(sh));
            let target = if text.len() == 1 && is_digit(text[0] as c_int) {
                DescriptorTarget::Number(
                    LogicalDescriptor::from_digit(text[0])
                        .expect("an ASCII digit names a logical descriptor"),
                )
            } else if text == BStr::new(b"-") {
                DescriptorTarget::Close
            } else {
                DescriptorTarget::Word(WordNode {
                    word: mem::take(&mut sh.input.word),
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
                word: mem::take(&mut sh.input.word),
            },
        }),
    };
    Ok(redirection)
}

/*
 * Input any here documents.
 */

// [spec:dash:def:parser.parseheredoc-fn]
// [spec:dash:sem:parser.parseheredoc-fn]
// [spec:posix:req:redir.here-doc-line-continuation]
// [spec:posix:req:redir.here-doc-backslash]
// [spec:posix:req:redir.here-doc-multiple]
// [spec:posix:req:redir.here-doc-ps2]
// [spec:posix:req:token.here-document-mode]
fn parseheredoc(sh: &mut Shell) -> Result<(), Error> {
    let list: Vec<heredoc> = core::mem::take(&mut sh.input.heredoclist);

    for here in list {
        if sh.input.needprompt != 0 {
            setprompt(sh, 2);
        }
        let mark = EofMark::Word(BStr::new(&here.eofmark));
        /* The C reads the first character inside the argument list. The
         * receiver is passed there too, so the read is its own statement:
         * evaluation order is unchanged, the first character is still
         * read before `readtoken1` runs. */
        if !here.expand {
            let firstc = pgetc(sh)?;
            readtoken1(
                sh,
                firstc,
                SyntaxContext::SingleQuoted,
                mark,
                here.striptabs,
                false,
            )?;
        } else {
            let firstc = pgetc_eatbnl(sh)?;
            readtoken1(
                sh,
                firstc,
                SyntaxContext::DoubleQuoted,
                mark,
                here.striptabs,
                false,
            )?;
        }
        let body = WordNode {
            word: mem::take(&mut sh.input.word),
        };
        sh.input.completed_heredocs.push(body);
    }
    Ok(())
}

// [spec:dash:def:parser.readtoken-fn]
// [spec:dash:sem:parser.readtoken-fn]
pub(crate) fn readtoken(sh: &mut Shell, context: TokenContext) -> Result<TokenKind, Error> {
    Ok(readtoken_with_flags(sh, context)?.kind)
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
fn readtoken_with_flags(sh: &mut Shell, mut context: TokenContext) -> Result<Token, Error> {
    let mut token: Token;

    loop {
        token = xxreadtoken(sh, context.check_here_document_end)?;

        /*
         * eat newlines
         */
        if context.skip_newlines {
            while token.kind == TokenKind::Newline {
                parseheredoc(sh)?;
                /* The alias bit is dropped with the rest: dash clears the
                 * whole of `checkkwd` here, and the bit lived in it. */
                crate::input::clear_alias_boundary(sh);
                token = xxreadtoken(sh, context.check_here_document_end)?;
            }
        }

        /* `popstring` sets this while `xxreadtoken` runs. The bit belongs
         * to the input boundary now; this is the same hand-off point. */
        if crate::input::take_alias_boundary(sh) {
            context.aliases = true;
        }

        if token.kind != TokenKind::Word || token.quoted {
            break;
        }

        /*
         * check for keywords
         */
        if context.reserved_words {
            if let Some(kind) = findkwd(wordtext(sh)) {
                token.kind = kind;
                sh.input.lasttoken = token.kind;
                break;
            }
        }

        if context.aliases && sh.options.alias_expansion_enabled(sh.input.parse_dialect()) {
            /* Hoisted: the receiver cannot appear twice in one argument
             * list. A raw pointer ends its borrow at the `let`, and the
             * word it points at is the parser's own, not the alias's. */
            let name = wordtext(sh).to_owned();
            if let Some(value) = crate::alias::lookup_alias(sh, BStr::new(name.as_slice()), true) {
                if !value.is_empty() {
                    pushstring(sh, BStr::new(value.as_slice()), Some(name));
                }
                continue;
            }
        }
        break;
    }
    Ok(token)
}

// [spec:dash:def:parser.nlprompt-fn]
// [spec:dash:sem:parser.nlprompt-fn]
fn nlprompt(sh: &mut Shell) {
    crate::plinno!(sh) += 1;
    if sh.input.doprompt != 0 {
        setprompt(sh, 2);
    }
}

// [spec:dash:def:parser.nlnoprompt-fn]
// [spec:dash:sem:parser.nlnoprompt-fn]
fn nlnoprompt(sh: &mut Shell) {
    crate::plinno!(sh) += 1;
    sh.input.needprompt = sh.input.doprompt;
}

/*
 * Read the next input token.
 * If the token is a word, we set backquotelist to the list of cmds in
 *	backquotes.  We set quoteflag to true if any part of the word was
 *	quoted.
 * If the token is TokenKind::Redirection, then we set redirnode to a structure containing
 *	the redirection.
 */

// [spec:dash:def:parser.xxreadtoken-fn]
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
fn xxreadtoken(sh: &mut Shell, check_here_document_end: bool) -> Result<Token, Error> {
    let mut input: InputUnit;

    if sh.input.tokpushback {
        sh.input.tokpushback = false;
        return Ok(Token {
            kind: sh.input.lasttoken,
            quoted: sh.input.last_quoteflag,
        });
    }
    if sh.input.needprompt != 0 {
        setprompt(sh, 2);
    }
    loop {
        /* until token or start of word found */
        let tok: Token;

        input = pgetc_eatbnl(sh)?;
        if input.is(b' ') || input.is(b'\t') {
            continue;
        } else if input.is(b'#') {
            loop {
                input = pgetc(sh)?;
                if input.is(b'\n') || input == InputUnit::EndOfInput {
                    break;
                }
            }
            pungetc(sh);
            continue;
        } else if input.is(b'\n') {
            nlnoprompt(sh);
            sh.input.lasttoken = TokenKind::Newline;
            sh.input.last_quoteflag = false;
            return Ok(Token::plain(TokenKind::Newline));
        } else if input == InputUnit::EndOfInput {
            sh.input.lasttoken = TokenKind::Eof;
            sh.input.last_quoteflag = false;
            return Ok(Token::plain(TokenKind::Eof));
        } else if input.is(b'&') {
            if pgetc_eatbnl(sh)?.is(b'&') {
                sh.input.lasttoken = TokenKind::AndIf;
                sh.input.last_quoteflag = false;
                return Ok(Token::plain(TokenKind::AndIf));
            }
            pungetc(sh);
            sh.input.lasttoken = TokenKind::Background;
            sh.input.last_quoteflag = false;
            return Ok(Token::plain(TokenKind::Background));
        } else if input.is(b'|') {
            if pgetc_eatbnl(sh)?.is(b'|') {
                sh.input.lasttoken = TokenKind::OrIf;
                sh.input.last_quoteflag = false;
                return Ok(Token::plain(TokenKind::OrIf));
            }
            pungetc(sh);
            sh.input.lasttoken = TokenKind::Pipe;
            sh.input.last_quoteflag = false;
            return Ok(Token::plain(TokenKind::Pipe));
        } else if input.is(b';') {
            let next = pgetc_eatbnl(sh)?;
            if next.is(b';') {
                sh.input.lasttoken = TokenKind::EndCase;
                sh.input.last_quoteflag = false;
                return Ok(Token::plain(TokenKind::EndCase));
            } else if next.is(b'&') {
                sh.input.lasttoken = TokenKind::FallThrough;
                sh.input.last_quoteflag = false;
                return Ok(Token::plain(TokenKind::FallThrough));
            }
            pungetc(sh);
            sh.input.lasttoken = TokenKind::Semicolon;
            sh.input.last_quoteflag = false;
            return Ok(Token::plain(TokenKind::Semicolon));
        } else if input.is(b'(') {
            if bash::active(sh) && pgetc_eatbnl(sh)?.is(b'(') {
                sh.input.lasttoken = TokenKind::DoubleParen;
                sh.input.last_quoteflag = false;
                return Ok(Token::plain(TokenKind::DoubleParen));
            }
            if bash::active(sh) {
                pungetc(sh);
            }
            sh.input.lasttoken = TokenKind::LeftParen;
            sh.input.last_quoteflag = false;
            return Ok(Token::plain(TokenKind::LeftParen));
        } else if input.is(b')') {
            sh.input.lasttoken = TokenKind::RightParen;
            sh.input.last_quoteflag = false;
            return Ok(Token::plain(TokenKind::RightParen));
        }
        tok = readtoken1(
            sh,
            input,
            SyntaxContext::Base,
            EofMark::None,
            0,
            check_here_document_end,
        )?;
        if tok.kind != TokenKind::Blank {
            return Ok(tok);
        }
    }
}

// [spec:dash:def:parser.pgetc-eatbnl-fn]
// [spec:dash:sem:parser.pgetc-eatbnl-fn]
// [spec:posix:req:quote.backslash-newline]
fn pgetc_eatbnl(sh: &mut Shell) -> Result<InputUnit, Error> {
    let mut input: InputUnit;

    loop {
        input = pgetc(sh)?;
        if !input.is(b'\\') {
            break;
        }
        if !pgetc(sh)?.is(b'\n') {
            pungetc(sh);
            break;
        }

        nlprompt(sh);
    }

    Ok(input)
}

// [spec:dash:def:parser.pgetc-top-fn]
// [spec:dash:sem:parser.pgetc-top-fn]
fn pgetc_top(sh: &mut Shell, stack: &synstack) -> Result<InputUnit, Error> {
    if stack.syntax == SyntaxContext::SingleQuoted {
        pgetc(sh)
    } else {
        pgetc_eatbnl(sh)
    }
}

mod synstack_ops;

// [spec:dash:def:parser.getmbc-fn]
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
pub fn getmbc(
    sh: &mut Shell,
    input: InputUnit,
    out: &mut [u8; MBSLOP],
    mode: c_int,
) -> Result<c_uint, Error> {
    let Some(mut byte) = input.byte() else {
        return Ok(0);
    };
    /* The C's `out` cursor, and its `start`, as the offsets they were. */
    let mut o: usize = 0;
    let mut decoder = sh.locale.decoder();
    let mut ml: c_uint = 0;
    let mut wc: i32 = 0;
    let mut complete = false;
    let mbc: usize;

    if byte.is_ascii() {
        return Ok(0);
    }

    mbc = if (mode & 3) < 2 {
        2 + (mode == 1) as usize
    } else {
        0
    };
    out[mbc + ml as usize] = byte;
    loop {
        /* `mbrtowc` is asked for exactly one byte, and the slice it is
         * given starts at the byte just written -- so the pointer is
         * bounded by the indexing rather than by the caller's promise. */
        let decoded = decoder.push(out[mbc + ml as usize]);
        ml += 1;
        match decoded {
            nsh_platform::LocaleDecode::Incomplete => {}
            nsh_platform::LocaleDecode::Complete(wide) => {
                wc = wide;
                complete = true;
                break;
            }
            nsh_platform::LocaleDecode::Invalid => break,
        }
        if ml as usize >= MB_LEN_MAX {
            break;
        }
        let next = pgetc_eoa(sh)?;
        let Some(next_byte) = next.byte() else {
            break;
        };
        byte = next_byte;
        out[mbc + ml as usize] = byte;
    }

    if complete && ml > 1 {
        if mode == 4 && sh.locale.wide_is_blank(wc) {
            return Ok(1);
        }

        /* The last two `memalloc.h` macros this file expanded were here.
         * Over an offset they are one statement each, so they are written
         * out with the C's name beside them and the macros are deleted --
         * the bookkeeping [[delete-memalloc]] left recorded. */
        if (mode & 3) < 2 {
            /* USTPUTC(CTLMBCHAR, out) */
            out[o] = CTLMBCHAR as u8;
            o += 1;
            if mode == 1 {
                /* USTPUTC(CTLESC, out) */
                out[o] = CTLESC as u8;
                o += 1;
            }
            /* USTPUTC(ml, out) */
            out[o] = ml as u8;
            o += 1;
        }
        /* STADJUST(ml, out) — step over the bytes written ahead of the
         * cursor, which are the character itself. */
        o += ml as usize;
        if (mode & 3) < 2 {
            /* USTPUTC(ml, out) */
            out[o] = ml as u8;
            o += 1;
            /* USTPUTC(CTLMBCHAR, out) */
            out[o] = CTLMBCHAR as u8;
            o += 1;
        }

        return Ok(o as c_uint);
    }

    if ml > 1 {
        pungetn(sh, ml as c_int - 1);
    }

    Ok(0)
}

/// The most `getmbc` or `conv_escape` can write past the cursor.
///
/// `readtoken1` spells it `CHECKSTRSPACE(MAX(MB_LEN_MAX, 16) + 7, out)` and
/// then lets both of them write through a bare `char *`. Reserving is what
/// makes those writes land in a `Vec`'s spare capacity rather than past the
/// end of it, so the number has to stay the C's.
pub const MBSLOP: usize = (if MB_LEN_MAX > 16 { MB_LEN_MAX } else { 16 }) + 7;

/// `getmbc` appending to a growable string.
///
/// The C hands it a cursor into the stack block; the byte count it returns is
/// how far that cursor moved, which here is what the caller commits. It can
/// also return 0 or 1 having scribbled on the block past the cursor, and the
/// C leaves that scribble for the next write to overwrite — so does this,
/// because the bytes stay uncommitted.
fn getmbc_at(
    sh: &mut Shell,
    out: &mut BString,
    input: InputUnit,
    mode: c_int,
) -> Result<c_uint, Error> {
    let mut scratch: [u8; MBSLOP] = [0; MBSLOP];
    let ml = getmbc(sh, input, &mut scratch, mode)?;
    /* The append *is* the commit the callers used to make with `set_len`,
     * so it happens here once instead of at each of the three of them. */
    out.extend_from_slice(&scratch[..ml as usize]);
    Ok(ml)
}

// [spec:dash:def:parser.dollarsq-escape-fn]
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
fn dollarsq_escape(sh: &mut Shell, dest: &mut BString) -> Result<(), Error> {
    /* The C writes into the stack block through a cursor and commits the
     * prefix. `conv_escape` takes a fixed scratch buffer now -- it writes
     * above the length it reports, which is why the buffer has to be
     * `CONV_ESCAPE_SLOP` and not the four bytes the C's `CHECKSTRSPACE`
     * names -- so the write lands there and the committed prefix is
     * appended. Same bytes, same length, and the reservation that used to
     * be a memory-safety contract is gone with the raw cursor. */
    let mut scratch: [u8; crate::escape::CONV_ESCAPE_SLOP] = [0; crate::escape::CONV_ESCAPE_SLOP];
    let mut o: usize = 0;
    /* 10 = length of UXXXXXXXX + NUL */
    let mut text = [0u8; 10];
    let mut len: usize;
    let mut at: usize;

    len = 0;
    while len < text.len() - 1 {
        let input = pgetc(sh)?;
        let Some(byte) = input.byte() else {
            break;
        };

        text[len] = byte;
        len += 1;

        if byte == b'\'' {
            break;
        }
    }
    text[len] = 0;

    at = 0;
    if text[at] != b'c' {
        let ret: c_uint;

        ret = crate::escape::conv_escape(&text[at..], &mut scratch, true);
        at += (ret >> 4) as usize;
        o += (ret & 15) as usize;
    } else {
        at += 1;
        if text[at] != 0 {
            let conv_ch: c_int;
            let c: c_int;

            c = text[at] as c_int;
            at += 1;

            at += (((c ^ text[at] as c_int) | (c ^ '\\' as c_int)) == 0) as usize;

            conv_ch = (c & !((c & 0x40) >> 1) & 0x7f) ^ 0x40;
            /* USTPUTC(conv_ch, out) */
            scratch[o] = conv_ch as u8;
            o += 1;
        }
    }

    pungetn(sh, len.saturating_sub(at) as c_int);
    dest.extend_from_slice(&scratch[..o]);
    Ok(())
}

/*
 * If eofmark is NULL, read a word or a redirection symbol.  If eofmark
 * is not NULL, read a here document.  In the latter case, eofmark is the
 * word which marks the end of the document and striptabs is true if
 * leading tabs should be stripped from the document.  The argument firstc
 * is the first character of the input token or document.
 *
 * The word lexer delegates here-document checks, redirections,
 * substitutions, backquotes, and arithmetic to focused helpers that borrow
 * the current lexer state.
 */

/// The locals of `readtoken1` that its internal subroutines share.
struct Rt1<'a> {
    /// Owned parse contexts, base first and current last. Popping retains the
    /// allocation, matching the C's reuse of its most recently popped level.
    synstack: Vec<synstack>,
    check_here_document_end: bool,
    printesc: bool,
    bqlist: Vec<Option<Node>>,
    dollar_single_quoted: bool,
    input: InputUnit,
    quoted: bool,
    /// The word being built. The C's `out` is a `char *` cursor into the
    /// stack block and `stackblock()` is the base; here the base is the
    /// buffer and the cursor is its length, so `STADJUST` is `truncate`
    /// or `push`. Nothing writes into this buffer's spare capacity any
    /// more -- the two routines that did, `getmbc` and `dollarsq_escape`,
    /// have their own scratch and hand back bytes to append.
    out: BString,
    eofmark: EofMark<'a>,
    striptabs: c_int,
}

impl Rt1<'_> {
    #[inline]
    fn syn(&self) -> &synstack {
        self.synstack.last().unwrap()
    }

    #[inline]
    fn syn_mut(&mut self) -> &mut synstack {
        self.synstack.last_mut().unwrap()
    }

    fn record_quote_boundary(&mut self, toggle_nested_double_quote: bool) {
        if toggle_nested_double_quote && self.syn().varnest != 0 {
            self.syn_mut().innerdq ^= 1;
        }
        if self.eofmark.is_none() {
            self.out.push(CTLQUOTEMARK as u8);
        }
    }
}

// [spec:dash:def:parser.readtoken1-fn]
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
fn readtoken1(
    sh: &mut Shell,
    first_input: InputUnit,
    syntax: SyntaxContext,
    eofmark: EofMark<'_>,
    striptabs: c_int,
    check_here_document_end: bool,
) -> Result<Token, Error> {
    let mut st = Rt1 {
        synstack: vec![synstack {
            syntax,
            innerdq: 0,
            varpushed: 0,
            dblquote: (syntax == SyntaxContext::DoubleQuoted) as c_int,
            backq: 0,
            varnest: 0,
            parenlevel: 0,
            dqvarnest: 0,
        }],
        check_here_document_end,
        printesc: syntax == SyntaxContext::SingleQuoted,
        bqlist: Vec::new(),
        dollar_single_quoted: false,
        input: first_input,
        quoted: false,
        out: BString::new(Vec::new()),
        eofmark,
        striptabs,
    };
    let len: usize;

    'word: loop {
        /* for each line, until end of word */
        checkend(sh, &mut st)?;
        /* Until end of line or end of word */
        loop {
            let fieldsplitting: c_int;
            let mut ml: c_uint;

            fieldsplitting = if st.syn().syntax == SyntaxContext::Base
                && (st.syn().varnest | st.syn().backq) == 0
            {
                4
            } else {
                0
            };
            bash::process_substitutions(sh, &mut st, fieldsplitting)?;
            /* The C's CHECKSTRSPACE, which permits max(MB_LEN_MAX, 23)
             * calls to USTPUTC, has no counterpart here: `getmbc`
             * writes into its own scratch and `getmbc_at` appends
             * what it reports, so there is no room for this frame to
             * make on its behalf. */
            ml = getmbc_at(
                sh,
                &mut st.out,
                st.input,
                fieldsplitting | (if st.printesc { 2 } else { 0 }),
            )?;
            if ml == 1 {
                if st.out.is_empty() {
                    return Ok(Token::plain(TokenKind::Blank));
                }
                st.input = pgetc(sh)?;
                break 'word;
            }
            if ml != 0 {
                st.input = pgetc_top(sh, st.syn())?;
                continue;
            }

            let class = st.syn().syntax.classify(st.input);

            match class {
                SyntaxClass::Newline => {
                    if fieldsplitting != 0 {
                        break 'word;
                    }
                    st.out.push(st.input.expect_byte());
                    nlprompt(sh);
                    st.input = pgetc_top(sh, st.syn())?;
                    continue 'word;
                }
                SyntaxClass::Word => st.out.push(st.input.expect_byte()),
                SyntaxClass::Control => {
                    if st.dollar_single_quoted && st.input.is(b'\\') {
                        dollarsq_escape(sh, &mut st.out)?;
                    } else {
                        if (st.eofmark.is_none() as c_int | st.syn().dblquote | st.syn().varnest)
                            != 0
                        {
                            st.out.push(CTLESC as u8);
                        }
                        st.out.push(st.input.expect_byte());
                    }
                }
                SyntaxClass::Backslash => {
                    st.input = pgetc(sh)?;
                    if st.input == InputUnit::EndOfInput {
                        st.out.push(CTLESC as u8);
                        st.out.push(b'\\');
                        pungetc(sh);
                    } else {
                        if (st.syn().dblquote | st.syn().backq) != 0
                            && !st.input.is(b'\\')
                            && !st.input.is(b'`')
                            && !st.input.is(b'$')
                            && (!st.input.is(b'"')
                                || (!st.eofmark.is_none() && st.syn().varnest == 0))
                            && (!st.input.is(b'}') || st.syn().varnest == 0)
                        {
                            st.out.push(CTLESC as u8);
                            st.out.push(b'\\');
                        }
                        st.quoted = true;

                        ml = getmbc_at(sh, &mut st.out, st.input, 1)?;
                        if ml == 0 {
                            st.out.push(CTLESC as u8);
                            st.out.push(st.input.expect_byte());
                        }
                    }
                }
                SyntaxClass::SingleQuote => {
                    st.syn_mut().syntax = SyntaxContext::SingleQuoted;
                    st.record_quote_boundary(false);
                }
                SyntaxClass::DoubleQuote => {
                    st.syn_mut().syntax = SyntaxContext::DoubleQuoted;
                    st.syn_mut().dblquote = 1;
                    st.record_quote_boundary(true);
                }
                SyntaxClass::EndQuote => {
                    if !st.eofmark.is_none() && st.syn().varnest == 0 {
                        st.out.push(st.input.expect_byte());
                    } else {
                        if st.syn().dqvarnest == 0 {
                            if st.dollar_single_quoted {
                                let end = st
                                    .out
                                    .iter()
                                    .position(|&byte| byte == 0)
                                    .unwrap_or(st.out.len());
                                st.out.truncate(end);
                                st.dollar_single_quoted = false;
                            }

                            st.syn_mut().syntax = SyntaxContext::Base;
                            st.syn_mut().dblquote = 0;
                        }

                        st.quoted = true;
                        st.record_quote_boundary(st.input.is(b'"'));
                    }
                }
                SyntaxClass::Variable => parsesub(sh, &mut st)?,
                SyntaxClass::EndVariable => {
                    if st.syn().innerdq == 0 && st.syn().varnest > 0 {
                        st.syn_mut().varnest -= 1;
                        if st.syn().varnest == 0 && st.syn().varpushed != 0 {
                            synstack_ops::pop(&mut st.synstack);
                        } else if st.syn().dqvarnest > 0 {
                            st.syn_mut().dqvarnest -= 1;
                        }
                        if !st.check_here_document_end {
                            st.input = InputUnit::Byte(CTLENDVAR as u8);
                        }
                    }
                    st.out.push(st.input.expect_byte());
                }
                SyntaxClass::LeftParen => {
                    st.syn_mut().parenlevel += 1;
                    st.out.push(st.input.expect_byte());
                }
                SyntaxClass::RightParen => {
                    if st.syn().parenlevel > 0 {
                        st.syn_mut().parenlevel -= 1;
                    } else if pgetc_eatbnl(sh)?.is(b')') {
                        synstack_ops::pop(&mut st.synstack);
                        if st.check_here_document_end {
                            st.out.push(st.input.expect_byte());
                        } else {
                            st.input = InputUnit::Byte(CTLENDARI as u8);
                        }
                    } else {
                        pungetc(sh);
                    }
                    st.out.push(st.input.expect_byte());
                }
                SyntaxClass::Backquote => {
                    if st.syn().backq == 2 {
                        synstack_ops::pop(&mut st.synstack);
                        st.printesc = false;
                        st.out.push(st.input.expect_byte());
                    } else {
                        st.out.push(b'`');
                        parsebackq(sh, &mut st, 1)?;
                    }
                }
                SyntaxClass::EndOfInput | SyntaxClass::EndOfAlias => break 'word,
                SyntaxClass::WordSeparator => {
                    if st.input.is(b')') && st.syn().backq == 1 {
                        synstack_ops::pop(&mut st.synstack);
                        st.printesc = false;
                        st.out.push(st.input.expect_byte());
                    } else if fieldsplitting != 0 {
                        break 'word;
                    } else {
                        st.out.push(st.input.expect_byte());
                    }
                }
            }

            st.input = pgetc_top(sh, st.syn())?;
        }
    }
    /* endword: */
    if st.syn().syntax == SyntaxContext::Arithmetic {
        return Err(synerror(sh, b"Missing '))'"));
    }
    if (st.syn().syntax != SyntaxContext::Base && st.eofmark.is_none()) || st.syn().backq != 0 {
        return Err(synerror(sh, b"Unterminated quoted string"));
    }
    if st.syn().varnest != 0 {
        /* { */
        return Err(synerror(sh, b"Missing '}'"));
    }
    st.out.push(b'\0');
    len = st.out.len();
    if st.eofmark.is_none() {
        if (st.input.is(b'>') || st.input.is(b'<'))
            && !st.quoted
            && len <= 2
            && (st.out[0] == 0 || is_digit(st.out[0] as i8 as c_int))
        {
            parseredir(sh, &mut st)?;
            sh.input.lasttoken = TokenKind::Redirection;
            sh.input.last_quoteflag = false;
            return Ok(Token::plain(TokenKind::Redirection));
        } else {
            pungetc(sh);
        }
    }
    sh.input.last_quoteflag = st.quoted;
    /* `grabstackblock(len)` reserved the bytes the C had been writing into
     * scratch space, which is what made `wordtext` outlive the next token.
     * Moving the buffer out is the same guarantee. */
    // [spec:nsh:def:idiom.word-ir]
    sh.input.word = ParsedWord::from_legacy(mem::take(&mut st.out), mem::take(&mut st.bqlist));
    sh.input.lasttoken = TokenKind::Word;
    Ok(Token {
        kind: TokenKind::Word,
        quoted: st.quoted,
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
fn checkend(sh: &mut Shell, st: &mut Rt1<'_>) -> Result<(), Error> {
    if let Some(mark) = st.eofmark.real() {
        let mut i: usize;
        let mut more_heredoc = false;

        if st.striptabs != 0 {
            while st.input.is(b'\t') {
                st.input = pgetc(sh)?;
            }
        }

        let mut consumed = Vec::new();
        i = 0;
        loop {
            if let Some(byte) = st.input.byte() {
                consumed.push(byte);
            }
            if i == mark.len() {
                break;
            }
            if !st.input.is(mark[i]) {
                more_heredoc = true;
                break;
            }

            st.input = pgetc(sh)?;
            i += 1;
        }

        if !more_heredoc {
            if st.input.is(b'\n') || st.input == InputUnit::EndOfInput {
                st.input = InputUnit::EndOfInput;
                nlnoprompt(sh);
            } else {
                more_heredoc = true;
            }
        }

        if more_heredoc {
            if let Some((&first, rest)) = consumed.split_first() {
                st.input = InputUnit::Byte(first);
                if !rest.is_empty() {
                    pushstring(sh, BStr::new(rest), None);
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
fn parseredir(sh: &mut Shell, st: &mut Rt1<'_>) -> Result<(), Error> {
    enum ParsedRedirection {
        File(FileRedirectionOperator),
        Descriptor(DescriptorRedirectionOperator),
        HereDocument,
    }

    let fdc: c_char = st.out[0] as c_char;
    /* The C carves one `struct nfile` and then decides what it is by
     * assigning `np->type`, re-allocating only because `nhere` is smaller.
     * The arm has to be chosen up front here, so the type and the fd are
     * worked out first and the node built at the end. */
    let mut descriptor: LogicalDescriptor;
    let redirection: ParsedRedirection;

    if st.input.is(b'>') {
        descriptor = LogicalDescriptor::STDOUT;
        st.input = pgetc_eatbnl(sh)?;
        if st.input.is(b'>') {
            redirection = ParsedRedirection::File(FileRedirectionOperator::Append);
        } else if st.input.is(b'|') {
            redirection = ParsedRedirection::File(FileRedirectionOperator::Clobber);
        } else if st.input.is(b'&') {
            redirection = ParsedRedirection::Descriptor(DescriptorRedirectionOperator::Output);
        } else {
            redirection = ParsedRedirection::File(FileRedirectionOperator::Write);
            pungetc(sh);
        }
    } else {
        /* c == '<' */
        descriptor = LogicalDescriptor::STDIN;
        st.input = pgetc_eatbnl(sh)?;
        if st.input.is(b'<') {
            let mut here = heredoc {
                expand: false,
                eofmark: BString::new(Vec::new()),
                striptabs: 0,
            };
            redirection = ParsedRedirection::HereDocument;
            st.input = pgetc_eatbnl(sh)?;
            if st.input.is(b'-') {
                here.striptabs = 1;
            } else {
                pungetc(sh);
            }
            sh.input.heredoc = Some(here);
        } else if st.input.is(b'&') {
            redirection = ParsedRedirection::Descriptor(DescriptorRedirectionOperator::Input);
        } else if st.input.is(b'>') {
            redirection = ParsedRedirection::File(FileRedirectionOperator::ReadWrite);
        } else {
            redirection = ParsedRedirection::File(FileRedirectionOperator::Read);
            pungetc(sh);
        }
    }
    if fdc != '\0' as c_char {
        descriptor = LogicalDescriptor::from_digit(fdc as u8)
            .expect("the lexer accepts only a descriptor digit before redirection");
    }
    sh.input.redirnode = Some(match redirection {
        ParsedRedirection::Descriptor(operator) => PendingRedirection::Descriptor {
            operator,
            descriptor,
        },
        ParsedRedirection::HereDocument => PendingRedirection::HereDocument { descriptor },
        ParsedRedirection::File(operator) => PendingRedirection::File {
            operator,
            descriptor,
        },
    });
    Ok(())
}

/*
 * Parse a substitution.  At this point, we have read the dollar sign
 * and nothing else.
 */
fn parsesub(sh: &mut Shell, st: &mut Rt1<'_>) -> Result<(), Error> {
    let mut newsyn = st.syn().syntax;
    static types: [u8; 6] = *b"}-+?=\0";
    let mut subtype: c_int;

    st.out.push('$' as u8);

    st.input = pgetc_eatbnl(sh)?;
    if st.input.is(b'(') {
        /* $(command) or $((arith)) */
        st.out.push(st.input.expect_byte());
        if pgetc_eatbnl(sh)?.is(b'(') {
            parsearith(sh, st)?;
        } else {
            pungetc(sh);
            parsebackq(sh, st, 0)?;
        }
    } else if st.input.is(b'\'') && newsyn.classify(InputUnit::Byte(b'&')) != SyntaxClass::Word {
        st.out.pop();
        st.dollar_single_quoted = true;
        st.syn_mut().syntax = SyntaxContext::SingleQuoted;
        st.record_quote_boundary(false);
        return Ok(());
    } else if st.input.is(b'{')
        || st.input.begins_name(&sh.locale)
        || st.input.is_special_parameter()
    {
        let typeloc: usize = st.out.len();
        let mut badsub = false;

        /* `STADJUST(chkeofmark == 0, out)` steps over the byte the CTLVAR
         * subtype lands in below; nothing reads it in between, so the
         * placeholder value is not observable. */
        st.out
            .resize(typeloc + (!st.check_here_document_end) as usize, 0);
        subtype = VSNORMAL;
        if st.input.is(b'{') {
            if st.check_here_document_end {
                st.out.push('{' as u8);
            }
            st.input = pgetc_eatbnl(sh)?;
            subtype = 0;
        }
        'varname: loop {
            if st.input.begins_name(&sh.locale) {
                loop {
                    st.out.push(st.input.expect_byte());
                    st.input = pgetc_eatbnl(sh)?;
                    if !st.input.continues_name(&sh.locale) {
                        break;
                    }
                }
            } else if st.input.is_digit() {
                loop {
                    st.out.push(st.input.expect_byte());
                    st.input = pgetc_eatbnl(sh)?;
                    if !((subtype <= 0 || subtype >= VSLENGTH) && st.input.is_digit()) {
                        break;
                    }
                }
            } else if !st.input.is(b'}') {
                let mut cc = st.input;

                st.input = pgetc_eatbnl(sh)?;

                if subtype == 0 && cc.is(b'#') {
                    subtype = VSLENGTH;

                    if st.input.is(b'_')
                        || st
                            .input
                            .byte()
                            .is_some_and(|byte| sh.locale.is_alphanumeric(byte))
                    {
                        if st.check_here_document_end {
                            st.out.push('#' as u8);
                        }
                        continue 'varname;
                    }

                    cc = st.input;
                    st.input = pgetc_eatbnl(sh)?;
                    if cc.is(b'}') || !st.input.is(b'}') {
                        pungetc(sh);
                        subtype = 0;
                        st.input = cc;
                        cc = InputUnit::Byte(b'#');
                    } else if st.check_here_document_end {
                        st.out.push('#' as u8);
                    }
                }

                if !cc.is_special_parameter() {
                    if subtype == VSLENGTH {
                        subtype = 0;
                    }
                    badsub = true;
                    break 'varname;
                }

                st.out.push(cc.expect_byte());
            } else {
                badsub = true;
                break 'varname;
            }
            break 'varname;
        }

        bash::parameter_subscript(sh, st, badsub, subtype)?;

        if badsub {
            /* badsub: */
            pungetc(sh);
        } else if subtype == 0 {
            let cc = st.input;

            if st.check_here_document_end {
                st.out.push(st.input.expect_byte());
            }

            if st.input.is(b'%') || st.input.is(b'#') {
                subtype = if st.input.is(b'#') {
                    VSTRIMLEFT
                } else {
                    VSTRIMRIGHT
                };
                st.input = pgetc_eatbnl(sh)?;
                if st.input == cc {
                    if st.check_here_document_end {
                        st.out.push(st.input.expect_byte());
                    }
                    subtype += 1;
                } else {
                    pungetc(sh);
                }

                newsyn = SyntaxContext::Base;
            } else {
                if st.input.is(b':') {
                    subtype = VSNUL;
                    st.input = pgetc_eatbnl(sh)?;
                    if st.check_here_document_end {
                        st.out.push(st.input.expect_byte());
                    }
                    /*FALLTHROUGH*/
                }
                /* default: */
                /* The search runs over the whole array, terminator
                 * included, because that is what `strchr` does: a NUL
                 * `c` lands on index 5 rather than missing. */
                if let Some(at) = types.iter().position(|&byte| st.input.is(byte)) {
                    subtype |= at as c_int + VSNORMAL;
                }
            }
        } else {
            if subtype == VSLENGTH && !st.input.is(b'}') {
                subtype = 0;
            }
            /* badsub: */
            pungetc(sh);
        }

        if newsyn == SyntaxContext::Arithmetic {
            newsyn = SyntaxContext::DoubleQuoted;
        }

        if (newsyn != st.syn().syntax || st.syn().innerdq != 0) && subtype != VSNORMAL {
            synstack_ops::push(&mut st.synstack, newsyn);

            st.syn_mut().varpushed += 1;
            st.syn_mut().dblquote = (newsyn != SyntaxContext::Base) as c_int;
        }

        if subtype != VSNORMAL {
            st.syn_mut().varnest += 1;
            if st.syn().dblquote != 0 {
                st.syn_mut().dqvarnest += 1;
            }
        }
        if !st.check_here_document_end {
            st.out[typeloc - 1] = CTLVAR as u8;
            st.out[typeloc] = (subtype | VSBIT) as u8;
            st.out.push(b'=');
        }
    } else {
        pungetc(sh);
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
fn parsebackq(sh: &mut Shell, st: &mut Rt1<'_>, oldstyle: c_int) -> Result<(), Error> {
    let mut saveprompt: c_int = 0;
    let saveheredoclist: Vec<heredoc>;
    let nlpp: usize;
    let mut n: Option<Node>;
    let completed_at: usize;
    let mut ml: c_uint;
    /* `grabstackstr(pout)` had to reserve the backquote's text because
     * `list(2)?` builds on the same stack; owning it says the same thing, and
     * it has to outlive the `popfile` below because `setinputstring` reads
     * through the pointer rather than copying. */
    let mut pstr: BString = BString::new(Vec::new());
    let str: BString;

    if st.check_here_document_end {
        synstack_ops::push(&mut st.synstack, SyntaxContext::Base);
        st.syn_mut().backq = oldstyle + 1;
        st.printesc = true;
        if oldstyle != 0 {
            sh.input.tokpushback = false;
        }
        return Ok(());
    }
    /* `STADJUST(oldstyle - 1, out)` drops the '(' of `$(` and leaves the '`'
     * of a backquote in place; either way the last byte becomes CTLBACKQ. */
    if oldstyle == 0 {
        st.out.pop();
    }
    let last = st.out.len() - 1;
    st.out[last] = CTLBACKQ as u8;
    /* The word so far is parked while `list(2)?` runs, which is what
     * `grabstackblock(savelen)` bought the C. */
    str = mem::take(&mut st.out);
    if oldstyle != 0 {
        /* We must read until the closing backquote, giving special
        treatment to some slashes, and then push the string and
        reread it as input, interpreting it normally.  */
        let mut done = false;
        let mut input: InputUnit;

        while !done {
            if sh.input.needprompt != 0 {
                setprompt(sh, 2);
            }
            input = pgetc_eatbnl(sh)?;
            if input.is(b'`') {
                done = true;
            } else if input.is(b'\\') {
                input = pgetc(sh)?;
                if !input.is(b'\\')
                    && !input.is(b'`')
                    && !input.is(b'$')
                    && (st.syn().dblquote == 0 || !input.is(b'"'))
                {
                    pstr.push(b'\\');
                }
                ml = getmbc_at(sh, &mut pstr, input, 2)?;
                if ml != 0 {
                    continue;
                }
            } else if input == InputUnit::EndOfInput {
                return Err(synerror(sh, b"EOF in backquote substitution"));
            } else if input.is(b'\n') {
                nlnoprompt(sh);
            }
            pstr.push(input.expect_byte());
        }
        /* `pout[-1] = '\0'` — over the closing backquote the loop just
         * wrote, which is why the buffer is never empty here. */
        let last = pstr.len() - 1;
        pstr[last] = 0;
        setinputstring(sh, crate::mystring::cstr_prefix(&pstr));
    }
    /* The C walks to the tail of `bqlist` and appends an empty cell, then
     * fills its `n` after the recursive parse.  Reserving the slot first is
     * the same order; nothing else can append to this list while `list(2)?`
     * runs, because `bqlist` is a local of *this* `readtoken1`. */
    nlpp = st.bqlist.len();
    st.bqlist.push(None);

    saveheredoclist = core::mem::take(&mut sh.input.heredoclist);
    completed_at = sh.input.completed_heredocs.len();

    if oldstyle != 0 {
        saveprompt = sh.input.doprompt;
        sh.input.doprompt = 0;
    }

    n = list(sh, 2)?.into_node();

    if oldstyle != 0 {
        sh.input.doprompt = saveprompt;
    } else {
        if readtoken(sh, TokenContext::NONE)? != TokenKind::RightParen {
            return Err(synexpect(sh, Some(TokenKind::RightParen)));
        }
        setinputstring(sh, BStr::new(b""));
    }

    parseheredoc(sh)?;
    finalize::node(sh, &mut n, completed_at)?;
    sh.input.heredoclist = saveheredoclist;

    st.bqlist[nlpp] = n;
    /* Start reading from old file again. */
    popfile(sh);

    st.out = str;

    if oldstyle != 0 {
        /* Ignore any pushed back tokens left from the backquote
         * parsing.
         */
        sh.input.tokpushback = false;
    }
    Ok(())
}

/*
 * Parse an arithmetic expansion (indicate start of one and set state)
 */
/* parsearith: */
// [spec:posix:syn:expand.arith-format]
fn parsearith(sh: &mut Shell, st: &mut Rt1<'_>) -> Result<(), Error> {
    synstack_ops::push(&mut st.synstack, SyntaxContext::Arithmetic);
    st.syn_mut().dblquote = 1;
    if st.check_here_document_end {
        st.out.push(st.input.expect_byte());
    } else {
        /* `STADJUST(-1); out[-1] = CTLARI` — drop the second '(' of `$((`
         * and relabel the '$'. */
        st.out.pop();
        let last = st.out.len() - 1;
        st.out[last] = CTLARI as u8;
    }
    Ok(())
}

/*
 * Return of a legal variable name (a letter or underscore followed by zero or
 * more letters, underscores, and digits).
 */

// [spec:dash:def:parser.endofname-fn]
// [spec:dash:sem:parser.endofname-fn]
pub fn endofname(locale: &nsh_platform::Locale, name: &BStr) -> usize {
    let name = crate::mystring::cstr_prefix(name);
    let Some(&first) = name.first() else {
        return 0;
    };
    if !is_name(locale, first as c_char as c_int) {
        return 0;
    }
    1 + name[1..]
        .iter()
        .position(|&byte| !is_in_name(locale, byte as c_char as c_int))
        .unwrap_or(name.len() - 1)
}

/*
 * Called when an unexpected token is read during the parse.  The argument
 * is the token that is expected, or -1 if more than one type of token can
 * occur at this point.
 */

// [spec:dash:def:parser.synexpect-fn]
// [spec:dash:sem:parser.synexpect-fn]
fn synexpect(sh: &mut Shell, expected: Option<TokenKind>) -> Error {
    let mut message = Vec::new();

    message.extend_from_slice(sh.input.lasttoken.description());
    message.extend_from_slice(b" unexpected");
    if let Some(expected) = expected {
        message.extend_from_slice(b" (expecting ");
        message.extend_from_slice(expected.description());
        message.push(b')');
    }
    message.truncate(63);
    synerror(sh, &message)
}

// [spec:dash:def:parser.synerror-fn]
// [spec:dash:sem:parser.synerror-fn]
fn synerror(sh: &mut Shell, msg: &[u8]) -> Error {
    sh.eval.errlinno = crate::plinno!(sh);
    let mut message = b"Syntax error: ".to_vec();
    message.extend_from_slice(msg);
    sh.sh_error_value(&message)
}

// [spec:dash:def:parser.setprompt-fn]
// [spec:dash:sem:parser.setprompt-fn]
#[inline(never)]
fn setprompt(sh: &mut Shell, which: c_int) {
    let show: c_int;

    sh.input.needprompt = 0;
    sh.input.whichprompt = which;

    /* #ifdef SMALL: show = 1 */
    show = (!crate::histedit::editing_active(sh)) as c_int;
    if show != 0 && crate::input::cur_pf(sh).nleft == 0 {
        /* `pushstackmark(&smark, stackblocksize())` bounded the prompt
         * `expandstr` had left in the region for `out2str` to read.  The
         * expansion buffer is owned, so there is nothing to bound. */
        let prompt = getprompt(sh);
        let _ = sh.io.stderr().write_all(&prompt);
    }
}

// [spec:dash:def:parser.expandstr-fn]
// [spec:dash:sem:parser.expandstr-fn]
pub fn expandstr(sh: &mut Shell, ps: &BStr) -> Result<BString, Error> {
    let file_stop: usize;
    let saveheredoclist: Vec<heredoc>;
    let mut result: BString;
    let saveprompt: c_int;

    file_stop = crate::input::cur_mark(sh);

    /* XXX Fix (char *) cast. */
    let ps = crate::mystring::cstr_prefix(ps);
    setinputstring(sh, ps);

    saveheredoclist = core::mem::take(&mut sh.input.heredoclist);
    saveprompt = sh.input.doprompt;
    sh.input.doprompt = 0;
    sh.input.needprompt = 0;
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
    result = BString::from(ps);
    /* Parse and expand inside one fallible operation so a failure leaves
     * the seeded result unchanged. */
    let caught = (|| -> Result<(), crate::error::Error> {
        let result = &mut result;
        let firstc = pgetc_eatbnl(sh)?;
        readtoken1(
            sh,
            firstc,
            SyntaxContext::DoubleQuoted,
            EofMark::Fake,
            0,
            false,
        )?;

        let n = Node::Word(WordNode {
            word: mem::take(&mut sh.input.word),
        });

        expandarg(sh, &n, None, EXP_QUOTED)?;
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
        *result = BString::from(crate::expand::expansion_result(sh));
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
    let caught = restore_handler_expandarg(sh, caught);

    sh.input.doprompt = saveprompt;
    unwindfiles(sh, file_stop);
    sh.input.heredoclist = saveheredoclist;

    match caught {
        Some(e) if e.is_interrupt() => Err(e),
        other => {
            /* A bad `PS1`/`PS4` is reported and the unexpanded prompt is
             * used; the status it took is still the shell's, so the catch
             * writes it. */
            if let Some(e) = &other {
                sh.status = e.status();
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

// [spec:dash:def:parser.getprompt-fn]
// [spec:dash:sem:parser.getprompt-fn]
// [spec:posix:req:param.ps1]
// [spec:posix:req:param.ps1-two-pass]
// [spec:posix:req:param.ps2]
pub fn getprompt(sh: &mut Shell) -> BString {
    let prompt = match sh.input.whichprompt {
        1 => {
            let prompt = crate::var::ps1val(sh);
            sh.histedit
                .expand_prompt_exclamation_marks(BStr::new(prompt.as_slice()))
        }
        2 => crate::var::ps2val(sh),
        /* default: falls into case 0 outside DEBUG builds.  The C returns
         * `nullstr`, whose *address* is load-bearing at other sites (see
         * `mystring::nullstr`) but not at this one: both readers here take
         * its bytes and there are none, so the empty value is exact. */
        _ => {
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
     * `Shell::detached()`, and the reason recorded for it -- a
     * `fn(*mut c_void) -> *const c_char` the line editor calls through a
     * function pointer, with nowhere to put a `&mut Shell` -- stopped
     * being true when the editor moved to its native Rust API: nothing
     * calls this through a pointer any more. `linedit::shell_prompt`
     * calls it, and `setprompt` calls it, and both are ordinary Rust
     * frames that can carry a receiver. So this is threading after all,
     * not the handle `docs/api-design.md` §5.1 keeps for the signal
     * handler -- which is still the one shape that cannot take a
     * parameter, because a handler has no frame to thread through. */
    match expandstr(sh, BStr::new(prompt.as_slice())) {
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
// [spec:dash:def:parser.findkwd-fn]
// [spec:dash:sem:parser.findkwd-fn]
pub fn findkwd(s: &BStr) -> Option<TokenKind> {
    let key = crate::mystring::cstr_prefix(s);
    RESERVED_WORDS
        .binary_search_by(|(word, _)| word.cmp(&key.as_ref()))
        .ok()
        .map(|index| RESERVED_WORDS[index].1)
}

// ---------------------------------------------------------------------
// src/parser.h inline functions
// ---------------------------------------------------------------------

// [spec:dash:def:parser.goodname-fn]
// [spec:dash:sem:parser.goodname-fn]
pub fn goodname(locale: &nsh_platform::Locale, name: &BStr) -> c_int {
    let name = crate::mystring::cstr_prefix(name);
    (endofname(locale, name) == name.len()) as c_int
}

// [spec:dash:def:parser.parser-eof-fn]
// [spec:dash:sem:parser.parser-eof-fn]
pub fn parser_eof(sh: &Shell) -> bool {
    sh.input.tokpushback && sh.input.lasttoken == TokenKind::Eof
}

mod bash;

mod finalize;

#[cfg(test)]
mod bash_mode_tests;

#[cfg(test)]
mod bash_ast_tests;

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:idiom.immutable-ast/test]
    #[test]
    fn parse_result_owns_here_document_bodies() {
        let mut sh = Shell::builder().build().unwrap();
        crate::input::setinputstring(&mut sh, BStr::new(b"cat <<A <<B\none\nA\ntwo\nB\n"));
        let tree = match parsecmd(&mut sh, 0).unwrap() {
            ParseResult::Tree(Some(tree)) => tree,
            ParseResult::Tree(None) => panic!("expected a command, found a blank parse unit"),
            ParseResult::Eof => panic!("expected a command, found EOF"),
        };
        let Node::Command(command) = tree else {
            panic!("expected a simple command");
        };
        let bodies: Vec<&BStr> = command
            .redirections
            .iter()
            .map(|redirection| match redirection {
                Redirection::HereDocument(document) => document.body.word.as_bstr(),
                _ => panic!("expected a here-document"),
            })
            .collect();

        assert_eq!(bodies, [BStr::new(b"one\n"), BStr::new(b"two\n")]);
    }

    // [spec:dash:sem:parser.findkwd-fn/test]
    #[test]
    fn findkwd_preserves_the_sorted_table_contract() {
        let mut previous: Option<&[u8]> = None;

        for &(bytes, kind) in &RESERVED_WORDS {
            if let Some(previous) = previous {
                assert!(previous < bytes, "reserved words must be strictly sorted");
            }
            previous = Some(bytes);

            assert_eq!(findkwd(BStr::new(bytes)), Some(kind));

            let mut longer = bytes.to_vec();
            longer.push(b'x');
            assert_eq!(findkwd(BStr::new(&longer)), None);
        }

        for missing in [b"".as_slice(), b"cas", b"integer", b"zebra"] {
            assert_eq!(findkwd(BStr::new(missing)), None);
        }

        assert_eq!(findkwd(BStr::new(&[0xff_u8])), None);
    }
}
