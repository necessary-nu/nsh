//! Parsing for the reserved words whose bodies are more than a line.
//!
//! `for` and `select` share a shape exactly, and `time` prefixes a
//! pipeline rather than being one, so neither fits beside the flat
//! keyword dispatch in the parent. They live here to keep that dispatch
//! readable, and reach the parent's helpers the way any child module
//! does.

use bstr::{BStr, BString};

use super::{
    CaseClause, CaseCommand, Error, ForCommand, IfCommand, ListMode, Node, NodeText, ParsedWord,
    Shell, SourceLine, SourceTokens, Token, TokenContext, TokenKind, TokenMark, WordNode, command,
    expected_token_error, is_valid_name, list, mem, pipeline, read_token, required_compound_node,
    syntax_error,
};
use crate::word::WordToken;

/// How deeply one parse unit may nest, counting every construct that
/// spends stack on what is inside it.
///
/// Recursive descent spends stack per level and script text is untrusted,
/// so the depth is bounded rather than trusted. Unbounded, `(` repeated
/// often enough overflows the stack. Nothing has to run to reach it --
/// `sh -n` on a hostile file is enough -- and there is no unwind, so an
/// embedder gets a dead process where [`dec:nsh:shell-as-library`]
/// promised an `Err`.
///
/// One budget rather than one per construct, because the constructs
/// compose: `$( ${x:- $( ${x:- ... ` would otherwise multiply four
/// separate ceilings into a depth none of them names. Each level costs
/// what its own construct costs, so the budget is sized against the
/// dearest of them.
///
/// Set by measurement against the smallest stack the shell can plausibly
/// be asked to run on. In a release build a level costs 2,161 bytes
/// inside `$( )`, 1,744 in a compound command, 1,733 inside `[[ ]]`, 560
/// behind `time` and 304 in a nested expansion; 256 of the dearest is
/// 0.53 MiB and fits nearly four times over in the 2 MiB a spawned Rust
/// thread gets by default. A
/// debug build spends 14,928 bytes for a compound command, which is
/// 3.7 MiB at this depth -- comfortable on an 8 MiB main thread, and the
/// reason `bounded_recursion.rs` names its own stack size rather than
/// trusting the 2 MiB the test harness hands a thread.
///
/// It is the *nesting* depth, not the length of a list, and it is far
/// past what written scripts reach: generated `configure` scripts manage
/// a dozen or two.
///
/// The compound-command figures used to be far worse -- 5,200 and
/// 41,120 -- and what changed is where the nodes live. Moving
/// `if_command` and `case_command` out of `command`'s frame took the
/// first cut; boxing the fat `Node` variants took `Node` from 136 bytes
/// to 48 and took the rest, because roughly 26 node-sized slots are live
/// per level and every one of them shrank. dash spends about 160 bytes a
/// level for the same grammar, holding `union node *` where this held
/// the nodes themselves.
// [spec:nsh:req:idiom.bounded-recursion]
const MAX_NESTING_DEPTH: u32 = 256;

/// Enter one nesting level, refusing to go past [`MAX_NESTING_DEPTH`].
///
/// The count is decremented on the way out whether the level parsed or
/// not, so a refused parse leaves the budget as it found it.
// [spec:nsh:req:idiom.bounded-recursion]
pub(super) fn nested<T>(
    shell: &mut Shell,
    body: impl FnOnce(&mut Shell) -> Result<T, Error>,
) -> Result<T, Error> {
    if shell.input.nesting_depth >= MAX_NESTING_DEPTH {
        return Err(syntax_error(shell, b"too many nested commands"));
    }
    shell.input.nesting_depth += 1;
    let parsed = body(shell);
    shell.input.nesting_depth -= 1;
    parsed
}

/// Enter one command. Every route into a command goes through here, so
/// the count is the grammar's nesting depth exactly.
// [spec:nsh:req:idiom.bounded-recursion]
pub(super) fn nested_command(
    shell: &mut Shell,
    context: TokenContext,
) -> Result<Option<Node>, Error> {
    nested(shell, |shell| command(shell, context))
}

/// Charge a finished word's expansion nesting to the same budget.
///
/// `${x:-${x:-...}}` and `$(( $(( ... )) ))` cost no stack while the word
/// is being lexed -- the lexer is a loop over a flat event stream -- and
/// all of it when that stream is turned into a tree, which recurses once
/// per open expansion and drops the tree the same way. The events are
/// already in hand, so the depth is read off them rather than tracked
/// through the lexer's several exits, where an unpaired count would
/// refuse an ordinary word later in the same parse unit.
///
/// It is charged on top of the depth already spent, because a word is
/// read inside whatever commands enclose it.
// [spec:nsh:req:idiom.bounded-recursion]
pub(super) fn nested_expansions(shell: &mut Shell, tokens: &[WordToken]) -> Result<(), Error> {
    let word = crate::word::expansion_nesting(tokens);
    if shell.input.nesting_depth.saturating_add(word) > MAX_NESTING_DEPTH {
        return Err(syntax_error(shell, b"too many nested commands"));
    }
    Ok(())
}

/// `if list; then list; [elif ...] [else list;] fi`.
///
/// Extracted from `command` for the stack rather than for tidiness: in a
/// debug build every branch of that dispatch keeps its locals in one
/// frame whether taken or not, and this one's clause vector was part of
/// why a nesting level cost 41 KiB. See [`MAX_NESTING_DEPTH`].
// [spec:posix:syn:grammar.if-clause]
pub(super) fn if_command(shell: &mut Shell, start: TokenMark) -> Result<Option<Node>, Error> {
    /* The C threads the elif chain through `elsepart` on the way down,
     * writing each new nif into its parent before parsing it.  An owned
     * tree cannot hand out that parent pointer, so the clauses are
     * collected in parse order and folded back up afterwards; the
     * sequence of `list(0)?` calls — and so of everything they read — is
     * unchanged. */
    let mut clauses: Vec<(TokenMark, Node, Node)> = Vec::new();
    let parsed = list(shell, ListMode::Compound)?;
    let test = required_compound_node(shell, parsed, TokenKind::Then)?;
    if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Then {
        return Err(expected_token_error(shell, Some(TokenKind::Then)));
    }
    let parsed = list(shell, ListMode::Compound)?;
    let then_branch = required_compound_node(shell, parsed, TokenKind::Fi)?;
    clauses.push((start, test, then_branch));
    /* Each `elif` opens a nested `if`, so each clause keeps the mark its
     * own reserved word sits at; they all end at the one `fi`. */
    // [spec:nsh:def:idiom.token-stream]
    loop {
        let elif_mark = super::tokens::mark(shell);
        if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Elif {
            break;
        }
        let parsed = list(shell, ListMode::Compound)?;
        let test = required_compound_node(shell, parsed, TokenKind::Then)?;
        if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Then {
            return Err(expected_token_error(shell, Some(TokenKind::Then)));
        }
        let parsed = list(shell, ListMode::Compound)?;
        let then_branch = required_compound_node(shell, parsed, TokenKind::Fi)?;
        clauses.push((elif_mark, test, then_branch));
    }
    let mut else_branch: Option<Node> = if shell.input.last_token == TokenKind::Else {
        list(shell, ListMode::Compound)?.into_node()
    } else {
        shell.input.token_pushed_back = true;
        None
    };
    for (mark, test, then_branch) in clauses.into_iter().rev() {
        else_branch = Some(Node::If(IfCommand {
            tokens: super::tokens::run(shell, mark),
            condition: Box::new(test),
            then_branch: Box::new(then_branch),
            else_branch: else_branch.map(Box::new),
        }));
    }
    Ok(else_branch)
}

/// `case word in [(] pattern [| pattern] ) list ;; ... esac`.
///
/// Extracted for the same reason as [`if_command`]: two vectors and a
/// nested token loop that every other branch of `command` was paying for.
// [spec:posix:syn:grammar.case-clause]
pub(super) fn case_command(shell: &mut Shell, line: i32) -> Result<Node, Error> {
    let word_mark = super::tokens::mark(shell);
    if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Word {
        return Err(expected_token_error(shell, Some(TokenKind::Word)));
    }
    let expr = Node::Word(WordNode {
        tokens: super::tokens::run(shell, word_mark),
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
        let clause_mark = super::tokens::mark(shell);
        let mut token = read_token(shell, TokenContext::RESERVED_WORDS_AFTER_NEWLINES)?.kind;
        if token == TokenKind::Esac {
            break;
        }
        let mut pattern_mark = clause_mark;
        if shell.input.last_token == TokenKind::LeftParen {
            pattern_mark = super::tokens::mark(shell);
            read_token(shell, TokenContext::NONE)?;
        }
        let mut pattern: Vec<Node> = Vec::new();
        loop {
            if !shell.input.last_token.can_be_case_pattern() {
                return Err(expected_token_error(shell, Some(TokenKind::Word)));
            }
            pattern.push(Node::Word(WordNode {
                tokens: super::tokens::run(shell, pattern_mark),
                word: mem::take(&mut shell.input.word),
            }));
            if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Pipe {
                break;
            }
            pattern_mark = super::tokens::mark(shell);
            read_token(shell, TokenContext::NONE)?;
        }
        if shell.input.last_token != TokenKind::RightParen {
            return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
        }
        let body = list(shell, ListMode::StopAtTerminator)?.into_node();
        token = read_token(shell, TokenContext::RESERVED_WORDS_AFTER_NEWLINES)?.kind;
        cases.push(CaseClause {
            tokens: super::tokens::run(shell, clause_mark),
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
    Ok(Node::Case(CaseCommand {
        tokens: SourceTokens::none(),
        line: SourceLine::new(line),
        word: Box::new(expr),
        clauses: cases,
    }))
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
    start: TokenMark,
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
        loop {
            let word_mark = super::tokens::mark(shell);
            if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Word {
                break;
            }
            words.push(Node::Word(WordNode {
                tokens: super::tokens::run(shell, word_mark),
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
        /* Syntax the grammar supplies rather than text the source wrote,
         * so it carries no run and a renderer has to spell it. */
        // [spec:nsh:req:idiom.printable-ast+2]
        words.push(Node::Word(WordNode {
            tokens: SourceTokens::none(),
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
        tokens: super::tokens::run(shell, start),
        line: SourceLine::new(line),
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
    /* `time` reaches `pipeline` without passing through a command, so
     * `time time time ...` recurses on a route `nested_command` never
     * sees. It is charged here instead. */
    // [spec:nsh:req:idiom.bounded-recursion]
    nested(shell, |shell| pipeline(shell, TokenContext::COMMAND_START))
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
