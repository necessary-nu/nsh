//! Parsing for the reserved words whose bodies are more than a line.
//!
//! `for` and `select` share a shape exactly, and `time` prefixes a
//! pipeline rather than being one, so neither fits beside the flat
//! keyword dispatch in the parent. They live here to keep that dispatch
//! readable, and reach the parent's helpers the way any child module
//! does.

use bstr::{BStr, BString};

use super::{
    Error, ForCommand, ListMode, Node, NodeText, ParsedWord, Shell, Token, TokenContext, TokenKind,
    WordNode, command, expected_token_error, is_valid_name, list, mem, pipeline, read_token,
    required_compound_node, syntax_error,
};

/// How deeply one parse unit may nest commands.
///
/// Recursive descent spends stack per level and script text is untrusted,
/// so the depth is bounded rather than trusted. Unbounded, `(` repeated
/// often enough overflows the stack: about 1,700 times in a release build
/// on an 8 MiB stack, about 200 in a debug one. Nothing has to run to
/// reach it -- `sh -n` on a hostile file is enough -- and there is no
/// unwind, so an embedder gets a dead process where
/// [`dec:nsh:shell-as-library`] promised an `Err`.
///
/// The number is set by the worst configuration that is actually built,
/// with margin, rather than by the language: a debug build costs roughly
/// 40 KiB per level and overflows an 8 MiB stack at about 200, so 100
/// leaves a factor of two there. A release build costs about 4.8 KiB, so
/// the same 100 levels is half a mebibyte and fits four times over in the
/// 2 MiB a spawned Rust thread gets by default. It is the *nesting*
/// depth, not the length of a list, and it is far past what written
/// scripts reach -- generated `configure` scripts manage a dozen or two.
///
/// That per-level cost is itself the defect. dash spends about 160 bytes
/// a level and Bash keeps its parser stack on the heap entirely, so both
/// tolerate depths this cannot. Most of ours is `command`'s frame holding
/// the locals of every keyword branch at once, taken or not. Filed on
/// `bound-recursive-evaluation`; this bound is correct whatever that
/// number becomes, and can rise when it falls.
// [spec:nsh:req:idiom.bounded-recursion]
const MAX_COMMAND_DEPTH: u32 = 100;

/// Enter one command, refusing to nest past [`MAX_COMMAND_DEPTH`].
///
/// Every route into a command goes through here, so the count is the
/// nesting depth exactly, and it is decremented on the way out whether
/// the command parsed or not.
// [spec:nsh:req:idiom.bounded-recursion]
pub(super) fn nested_command(
    shell: &mut Shell,
    context: TokenContext,
) -> Result<Option<Node>, Error> {
    if shell.input.command_depth >= MAX_COMMAND_DEPTH {
        return Err(syntax_error(shell, b"too many nested commands"));
    }
    shell.input.command_depth += 1;
    let parsed = command(shell, context);
    shell.input.command_depth -= 1;
    parsed
}

/// The shape `for` and `select` share: a name, an optional `in` list, and
/// a `do ... done` body.
///
/// `for`'s arithmetic form is not here -- different syntax all the way
/// down, and its own closing token -- so this is the word-list form, which
/// is the whole of `select`.
// [spec:posix:syn:grammar.for-clause]
pub(super) fn iteration_command(
    shell: &mut Shell,
    line: i32,
    var_token: Token,
) -> Result<ForCommand, Error> {
    if var_token.kind != TokenKind::Word
        || var_token.quoted
        || !is_valid_name(&shell.locale, shell.input.word_text())
    {
        return Err(syntax_error(shell, b"Bad for loop variable"));
    }
    /* the C stores `wordtext` into the node here, before any further
     * token read can overwrite it */
    let variable = NodeText::from(shell.input.word_text());
    let mut words: Vec<Node> = Vec::new();
    if read_token(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?.kind == TokenKind::In {
        while read_token(shell, TokenContext::NONE)?.kind == TokenKind::Word {
            words.push(Node::Word(WordNode {
                word: mem::take(&mut shell.input.word),
            }));
        }
        if shell.input.last_token != TokenKind::Newline
            && shell.input.last_token != TokenKind::Semicolon
        {
            return Err(expected_token_error(shell, None));
        }
    } else {
        /* The implicit `"$@"` of a `for` with no `in` is syntax,
         * so construct the structural word directly. */
        words.push(Node::Word(WordNode {
            word: ParsedWord::quoted_parameter(BString::from(b"@".as_slice())),
        }));
        /*
         * Newline or semicolon here is optional (but note
         * that the original Bourne shell only allowed NL).
         */
        if shell.input.last_token != TokenKind::Semicolon {
            shell.input.token_pushed_back = true;
        }
    }
    if read_token(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?.kind != TokenKind::Do {
        return Err(expected_token_error(shell, Some(TokenKind::Do)));
    }
    let parsed = list(shell, ListMode::Compound)?;
    let body = required_compound_node(shell, parsed, TokenKind::Done)?;
    Ok(ForCommand {
        line,
        words,
        body: Box::new(body),
        variable,
    })
}

/// The pipeline `time` prefixes, if there is one.
///
/// A bare `time` is a whole command in Bash and reports zeros, so the
/// end of the command is not an error here the way it is in front of any
/// other prefix.
pub(super) fn timed_pipeline(shell: &mut Shell) -> Result<Option<Node>, Error> {
    let next = read_token(shell, TokenContext::COMMAND_START)?.kind;
    shell.input.token_pushed_back = true;
    if matches!(
        next,
        TokenKind::Eof | TokenKind::Newline | TokenKind::Semicolon | TokenKind::Background
    ) || next.ends_list()
    {
        return Ok(None);
    }
    pipeline(shell, TokenContext::COMMAND_START)
}

/// The optional `-p` after `time`, which asks for the POSIX report format.
///
/// It is a word rather than an option the built-in parses, because `time`
/// is a reserved word and there is no built-in to parse it.
pub(super) fn timed_posix_format(shell: &mut Shell) -> Result<bool, Error> {
    if read_token(shell, TokenContext::NONE)?.kind == TokenKind::Word
        && shell.input.word_text() == BStr::new(b"-p")
    {
        return Ok(true);
    }
    shell.input.token_pushed_back = true;
    Ok(false)
}
