//! The recursive descent behind `[[ ]]`, which is a grammar of its own.
//!
//! `[[ ]]` is not a command with words. It has two precedence levels, a
//! negation, parenthesised groups, unary and binary operators, and it reads
//! its operands under token rules the surrounding grammar does not use --
//! newlines are skipped wherever an operand or a connective may begin and
//! nowhere else, and the right operand of `==` reads extended globs whether
//! or not `extglob` is on.
//!
//! TWO THINGS IT USES AND DOES NOT OWN. `take_word` and the
//! `ConditionalWord` it returns are also how `compound_array` reads one
//! element of `name=( ... )`, so they stay in the file that has both
//! callers and are taken from `super` here. Nothing crosses the other way:
//! the four operator predicates below have no caller outside this module,
//! and `conditional` is reached only from `command_prefix`.

use core::mem;

use bstr::BStr;

use super::{ConditionalWord, take_word};
use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::nodes::{
    BashConditional, BashConditionalExpr, BashNode, FileRedirectionOperator, Node, NodeText,
    SourceLine, SourceTokens,
};
use crate::parser::{
    PendingRedirection, TokenContext, TokenKind, expected_token_error, keywords, line_reached,
    read_token, syntax_error, tokens,
};

/// Bash enables extended globs inside `[[ ]]` regardless of `shopt`, but
/// only for the pattern operand of `==` and `!=`. Everywhere else in the
/// conditional the option decides, which is what leaves `[[ !(word) ]]`
/// a negated group until `extglob` is on. The flag is saved and restored
/// rather than cleared, because a conditional can nest inside a command
/// substitution that appears in such a pattern.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(super) fn conditional(shell: &mut Shell) -> Result<Node, Error> {
    let enclosing = mem::replace(&mut shell.input.parsing_conditional, false);
    let parsed = conditional_expression(shell);
    shell.input.parsing_conditional = enclosing;
    parsed
}

/// A parsed conditional expression and the line its node records.
///
/// `[[ ]]` records neither the line it opens on nor the line it closes
/// on, but the line the parser had reached when the *top* node of the
/// expression was built -- so the line belongs to the expression rather
/// than to the construct, and is carried back out with it. A test holds
/// its last operand's line, a group holds its `)`, an `&&` or `||` holds
/// the line of whatever follows its right operand, and a `!` builds no
/// node of its own and so holds its operand's.
// [spec:nsh:req:compat.bash.traps-introspection]
struct ConditionalExpression {
    expression: BashConditionalExpr,
    line: SourceLine,
}

fn conditional_expression(shell: &mut Shell) -> Result<Node, Error> {
    let first = read_token(shell, TokenContext::SKIP_NEWLINES)?;
    // `[[ ]]` has nothing to be true or false about, and Bash rejects it
    // while parsing rather than answering with a status.
    if closes_conditional(shell, first.kind, first.quoted) {
        return Err(syntax_error(shell, b"expected a conditional expression"));
    }
    shell.input.token_pushed_back = true;
    let parsed = conditional_or(shell)?;
    let close = read_token(shell, TokenContext::NONE)?;
    if !closes_conditional(shell, close.kind, close.quoted) {
        return Err(syntax_error(shell, b"expected ']]'"));
    }

    Ok(Node::Bash(BashNode::Conditional(Box::new(
        BashConditional {
            tokens: SourceTokens::none(),
            line: parsed.line,
            expression: parsed.expression,
        },
    ))))
}

/// One precedence level of `[[ ]]`'s `&&` and `||`.
///
/// A newline before either connective continues the expression, so a long
/// condition can be written over several lines. The line such a node
/// records is the one the parser reaches after the whole right operand,
/// because Bash builds the node only once it has looked past that operand
/// for the next connective -- which is why `[[ 1 = 2\n]]` records line 1
/// and `[[ 1 = 1 && 1 = 2\n]]` records line 2. A level that joined nothing
/// builds no node, and hands its operand's own line straight back.
// [spec:nsh:req:compat.bash.traps-introspection]
fn conditional_chain(
    shell: &mut Shell,
    connective: TokenKind,
    operand: fn(&mut Shell) -> Result<ConditionalExpression, Error>,
    join: fn(Box<BashConditionalExpr>, Box<BashConditionalExpr>) -> BashConditionalExpr,
) -> Result<ConditionalExpression, Error> {
    let mut parsed = operand(shell)?;
    let mut joined = false;
    loop {
        let token = read_token(shell, TokenContext::SKIP_NEWLINES)?.kind;
        if token != connective {
            shell.input.token_pushed_back = true;
            if joined {
                parsed.line = line_reached(shell);
            }
            return Ok(parsed);
        }
        let right = operand(shell)?;
        parsed.expression = join(Box::new(parsed.expression), Box::new(right.expression));
        joined = true;
    }
}

fn conditional_or(shell: &mut Shell) -> Result<ConditionalExpression, Error> {
    conditional_chain(
        shell,
        TokenKind::OrIf,
        conditional_and,
        BashConditionalExpr::Or,
    )
}

fn conditional_and(shell: &mut Shell) -> Result<ConditionalExpression, Error> {
    conditional_chain(
        shell,
        TokenKind::AndIf,
        conditional_primary,
        BashConditionalExpr::And,
    )
}

fn conditional_primary(shell: &mut Shell) -> Result<ConditionalExpression, Error> {
    let first_mark = tokens::mark(shell);
    /* Bash skips newlines wherever a conditional may begin an operand or
     * an operator, which is every position this read is reached from:
     * after `[[`, after `&&` or `||`, after `(` and after `!`. The two
     * positions it does not skip are the operand of a unary or a binary
     * operator, and those are read below with the newline left in place
     * so that they refuse it exactly as Bash does. */
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    let token = read_token(shell, TokenContext::SKIP_NEWLINES)?;
    if token.kind == TokenKind::LeftParen {
        /* `[[ ( ( ( ... ) ) ) ]]` is its own recursive descent, reached
         * from a command rather than through one, so it is charged the
         * same nesting budget the grammar around it spends. */
        // [spec:nsh:req:idiom.bounded-recursion]
        let expression = keywords::nested(shell, conditional_or)?.expression;
        if read_token(shell, TokenContext::NONE)?.kind != TokenKind::RightParen {
            return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
        }
        return Ok(ConditionalExpression {
            expression: BashConditionalExpr::Group(Box::new(expression)),
            line: line_reached(shell),
        });
    }
    if token.kind != TokenKind::Word || closes_conditional(shell, token.kind, token.quoted) {
        return Err(syntax_error(shell, b"expected conditional expression"));
    }

    let first = take_word(shell, token.quoted, first_mark);
    if !first.quoted && first.arg.word.as_bstr() == BStr::new(b"!") {
        // [spec:nsh:req:idiom.bounded-recursion]
        let negated = keywords::nested(shell, conditional_primary)?;
        return Ok(ConditionalExpression {
            expression: BashConditionalExpr::Not(Box::new(negated.expression)),
            line: negated.line,
        });
    }
    if !first.quoted && unary_operator(first.arg.word.as_bstr()) {
        let operand_mark = tokens::mark(shell);
        let operand_token = read_token(shell, TokenContext::NONE)?;
        if operand_token.kind != TokenKind::Word
            || closes_conditional(shell, operand_token.kind, operand_token.quoted)
        {
            return Err(syntax_error(shell, b"expected unary-test operand"));
        }
        return Ok(ConditionalExpression {
            expression: BashConditionalExpr::Unary {
                operator: NodeText::from(first.arg.word.as_bstr()),
                operand: take_word(shell, operand_token.quoted, operand_mark).arg,
            },
            line: line_reached(shell),
        });
    }
    conditional_test(shell, first)
}

/// A word, and either the binary operator that follows it or nothing.
///
/// Split from `conditional_primary` because the operator is most of it:
/// two of `[[ ]]`'s operators arrive as redirections rather than as
/// words, and a bare word is a whole test of its own.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
fn conditional_test(
    shell: &mut Shell,
    first: ConditionalWord,
) -> Result<ConditionalExpression, Error> {
    let operator_token = read_token(shell, TokenContext::NONE)?;
    let operator = if operator_token.kind == TokenKind::Redirection {
        let redirection = shell.input.pending_redirection.take();
        /* `[[ a < b ]]` compares strings, but `[[ a 3< b ]]` names a
         * descriptor, and a descriptor is not an operator here. */
        match redirection.as_ref() {
            Some(PendingRedirection::File {
                operator,
                descriptor,
                with_stderr: false,
            }) if *operator == FileRedirectionOperator::Read
                && descriptor.fixed() == Some(LogicalDescriptor::STDIN) =>
            {
                Some(NodeText::from(b"<".as_slice()))
            }
            Some(PendingRedirection::File {
                operator,
                descriptor,
                with_stderr: false,
            }) if *operator == FileRedirectionOperator::Write
                && descriptor.fixed() == Some(LogicalDescriptor::STDOUT) =>
            {
                Some(NodeText::from(b">".as_slice()))
            }
            _ => return Err(syntax_error(shell, b"unexpected redirection in '[[ ]]'")),
        }
    } else if operator_token.kind == TokenKind::Word
        && !operator_token.quoted
        && binary_operator(shell.input.word_text())
    {
        let operator = NodeText::from(shell.input.word_text());
        Some(operator)
    } else {
        None
    };
    let Some(operator) = operator else {
        /* A bare word is a whole test -- `[[ $x ]]` is `[[ -n $x ]]` --
         * and Bash builds its node only after reading what follows, so a
         * line continuation before the `]]` moves the line it records.
         * That read is also the one place inside `[[ ]]` where Bash does
         * not skip a newline, so `[[ $x\n]]` is a syntax error there and
         * is refused here rather than accepted as an extension. */
        // [spec:nsh:req:compat.bash.conditionals-arithmetic]
        if operator_token.kind == TokenKind::Newline {
            return Err(syntax_error(shell, b"expected ']]'"));
        }
        shell.input.token_pushed_back = true;
        return Ok(ConditionalExpression {
            expression: BashConditionalExpr::Word(first.arg),
            line: line_reached(shell),
        });
    };

    let context = if operator.as_bstr() == BStr::new(b"=~") {
        TokenContext::REGEX_OPERAND
    } else {
        TokenContext::NONE
    };
    /* The operand of a pattern-matching operator is the one place a
     * conditional reads extended-glob syntax without the option. */
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    let enclosing = mem::replace(
        &mut shell.input.parsing_conditional,
        matches_a_pattern(operator.as_bstr()),
    );
    let right_mark = tokens::mark(shell);
    let right_token = read_token(shell, context);
    shell.input.parsing_conditional = enclosing;
    let right_token = right_token?;
    if right_token.kind != TokenKind::Word
        || closes_conditional(shell, right_token.kind, right_token.quoted)
    {
        return Err(syntax_error(shell, b"expected binary-test operand"));
    }
    let right = take_word(shell, right_token.quoted, right_mark).arg;
    Ok(ConditionalExpression {
        expression: BashConditionalExpr::Binary {
            left: first.arg,
            operator,
            right,
        },
        line: line_reached(shell),
    })
}

/// Whether this operator takes a pattern on its right, which is where a
/// conditional reads extended globs without the option.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
fn matches_a_pattern(operator: &BStr) -> bool {
    matches!(operator.as_ref() as &[u8], b"=" | b"==" | b"!=")
}

fn closes_conditional(shell: &Shell, kind: TokenKind, quoted: bool) -> bool {
    kind == TokenKind::Word && !quoted && shell.input.word_text() == BStr::new(b"]]")
}

fn unary_operator(operator: &BStr) -> bool {
    let operator: &[u8] = operator.as_ref();
    matches!(
        operator,
        b"-a"
            | b"-b"
            | b"-c"
            | b"-d"
            | b"-e"
            | b"-f"
            | b"-g"
            | b"-h"
            | b"-k"
            | b"-L"
            | b"-n"
            | b"-N"
            | b"-o"
            | b"-O"
            | b"-p"
            | b"-r"
            | b"-R"
            | b"-s"
            | b"-S"
            | b"-t"
            | b"-u"
            | b"-v"
            | b"-w"
            | b"-x"
            | b"-z"
            | b"-G"
    )
}

fn binary_operator(operator: &BStr) -> bool {
    let operator: &[u8] = operator.as_ref();
    matches!(
        operator,
        b"=" | b"=="
            | b"!="
            | b"=~"
            | b"-ef"
            | b"-nt"
            | b"-ot"
            | b"-eq"
            | b"-ne"
            | b"-lt"
            | b"-le"
            | b"-gt"
            | b"-ge"
    )
}
