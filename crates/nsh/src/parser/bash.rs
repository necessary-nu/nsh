//! Bash-only productions in the existing recursive-descent parser.

// [spec:nsh:req:idiom.operation-modes]
use core::mem;

use bstr::{BStr, BString, ByteSlice as _};

use super::{
    InputUnit, ListMode, SyntaxClass, SyntaxContext, TokenContext, TokenKind, TokenMark, WordLexer,
    command, consume_newline_without_prompt, expected_token_error, finalize, is_valid_name, list,
    parse_here_documents, read_input_unit, read_token, read_unit_skipping_line_continuations,
    set_input_string, syntax_error, syntax_stack, unread_input_unit,
};
use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::nodes::SourceLine;
use crate::nodes::{
    BashArithmeticCommand, BashArithmeticFor, BashArrayAssignment, BashArrayElement,
    BashArrayValue, BashAssignmentOperator, BashConditional, BashConditionalExpr, BashFunction,
    BashFunctionStyle, BashNode, BashProcessDirection, BashProcessSubstitution,
    FileRedirectionOperator, Node, NodeText, SourceTokens, WordNode,
};
use crate::options::{BashShopt, Dialect};
use crate::syntax::{is_in_name, is_name};
use crate::word::{ParameterOperation, ParsedWord, WordToken, WordUnit};

#[derive(Clone, Copy, Eq, PartialEq)]
enum Quote {
    None,
    Single,
    Double,
}

struct ConditionalWord {
    arg: WordNode,
    quoted: bool,
}

pub(super) fn active(shell: &Shell) -> bool {
    shell.input.parse_dialect() == Dialect::Bash
}

pub(super) fn command_prefix(
    shell: &mut Shell,
    token: TokenKind,
    line: SourceLine,
) -> Result<Option<Node>, Error> {
    if !active(shell) {
        return Ok(None);
    }
    if token == TokenKind::DoubleParen {
        return arithmetic_command(shell, line).map(Some);
    }
    if token != TokenKind::Word || shell.input.last_token_quoted {
        return Ok(None);
    }
    if shell.input.word_text() == BStr::new(b"[[") {
        conditional(shell, line).map(Some)
    } else if shell.input.word_text() == BStr::new(b"function") {
        function(shell, line).map(Some)
    } else {
        Ok(None)
    }
}

pub(super) fn arithmetic_command(shell: &mut Shell, line: SourceLine) -> Result<Node, Error> {
    let expression = arithmetic_text(shell)?;
    Ok(Node::Bash(BashNode::ArithmeticCommand(
        BashArithmeticCommand {
            tokens: SourceTokens::none(),
            line,
            expression,
        },
    )))
}

pub(super) fn arithmetic_for(shell: &mut Shell, line: SourceLine) -> Result<Node, Error> {
    let text = arithmetic_text(shell)?;
    let [init, test, update] = for_clauses(shell, text.as_bstr())?;

    let separator = read_token(shell, TokenContext::RESERVED_WORDS)?.kind;
    let opener = if matches!(separator, TokenKind::Do | TokenKind::LeftBrace) {
        separator
    } else if separator == TokenKind::Semicolon {
        read_token(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?.kind
    } else if separator == TokenKind::Newline {
        shell.input.token_pushed_back = true;
        read_token(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?.kind
    } else {
        return Err(expected_token_error(shell, Some(TokenKind::Do)));
    };
    // Bash accepts a brace group as the loop body, which is the ksh
    // spelling this whole construct came from.
    let closer = match opener {
        TokenKind::Do => TokenKind::Done,
        TokenKind::LeftBrace => TokenKind::RightBrace,
        _ => return Err(expected_token_error(shell, Some(TokenKind::Do))),
    };

    let body = list(shell, ListMode::Compound)?
        .into_node()
        .ok_or_else(|| expected_token_error(shell, None))?;
    if read_token(shell, TokenContext::RESERVED_WORDS)?.kind != closer {
        return Err(expected_token_error(shell, Some(closer)));
    }
    Ok(Node::Bash(BashNode::ArithmeticFor(Box::new(
        BashArithmeticFor {
            tokens: SourceTokens::none(),
            line,
            init,
            test,
            update,
            body: Box::new(body),
        },
    ))))
}

/// Bash enables extended globs inside `[[ ]]` regardless of `shopt`, but
/// only for the pattern operand of `==` and `!=`. Everywhere else in the
/// conditional the option decides, which is what leaves `[[ !(word) ]]`
/// a negated group until `extglob` is on. The flag is saved and restored
/// rather than cleared, because a conditional can nest inside a command
/// substitution that appears in such a pattern.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(super) fn conditional(shell: &mut Shell, line: SourceLine) -> Result<Node, Error> {
    let enclosing = mem::replace(&mut shell.input.parsing_conditional, false);
    let parsed = conditional_expression(shell, line);
    shell.input.parsing_conditional = enclosing;
    parsed
}

fn conditional_expression(shell: &mut Shell, line: SourceLine) -> Result<Node, Error> {
    let first = read_token(shell, TokenContext::NONE)?;
    // `[[ ]]` has nothing to be true or false about, and Bash rejects it
    // while parsing rather than answering with a status.
    if closes_conditional(shell, first.kind, first.quoted) {
        return Err(syntax_error(shell, b"expected a conditional expression"));
    }
    shell.input.token_pushed_back = true;
    let expression = conditional_or(shell)?;
    let close = read_token(shell, TokenContext::NONE)?;
    if !closes_conditional(shell, close.kind, close.quoted) {
        return Err(syntax_error(shell, b"expected ']]'"));
    }

    Ok(Node::Bash(BashNode::Conditional(Box::new(
        BashConditional {
            tokens: SourceTokens::none(),
            line,
            expression,
        },
    ))))
}

/// Whether `name()` may introduce a function definition.
///
/// POSIX names a function with a `name`, and dash enforces exactly that.
/// Bash names one with any word the tokenizer already produced, which is
/// why `ble/array#push`, `py-repr` and `1+1` are ordinary function names
/// in Bash scripts, and why refusing them is a parse failure rather than
/// a missing feature. The `function name { ... }` form below is already
/// this permissive in both dialects.
// [spec:nsh:req:compat.bash.functions-scoping]
pub(super) fn accepts_function_name(shell: &mut Shell, name: &BStr) -> bool {
    if active(shell) {
        return !name.is_empty();
    }
    is_valid_name(&shell.locale, name)
}

// [spec:nsh:req:idiom.structural-ast]
pub(super) fn function(shell: &mut Shell, line: SourceLine) -> Result<Node, Error> {
    let name_token = read_token(shell, TokenContext::NONE)?;
    if name_token.kind != TokenKind::Word || shell.input.word_text().is_empty() {
        return Err(syntax_error(shell, b"invalid Bash function name"));
    }
    let name = NodeText::from(shell.input.word_text());

    let next = read_token(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?.kind;
    let style = if next == TokenKind::LeftParen {
        if read_token(shell, TokenContext::NONE)?.kind != TokenKind::RightParen {
            return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
        }
        BashFunctionStyle::FunctionParens
    } else {
        shell.input.token_pushed_back = true;
        BashFunctionStyle::Function
    };
    let body = command(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?
        .ok_or_else(|| expected_token_error(shell, None))?;

    Ok(Node::Bash(BashNode::Function(BashFunction {
        tokens: SourceTokens::none(),
        line,
        name,
        style,
        body: Box::new(body),
    })))
}

pub(super) fn array_word(shell: &Shell, arg: WordNode) -> Result<Node, WordNode> {
    let units = arg.word.units();
    let Some(open) = units
        .iter()
        .position(|unit| matches!(unit, WordUnit::Literal { byte: b'[', .. }))
    else {
        // `name+=value` and `name+=` are assignments too, but the POSIX
        // recogniser cannot see them: `+` is not a name byte, so the
        // whole word would be classified as an argument.
        return append_word(shell, arg);
    };
    let Some(name) = literal_bytes(&units[..open]) else {
        return Err(arg);
    };
    if !is_valid_name(&shell.locale, name.as_bstr()) {
        return Err(arg);
    }
    let Some(close) = matching_bracket(&units, open) else {
        return Err(arg);
    };
    let Some((operator, value_start)) = assignment_operator(&units, close + 1) else {
        return Err(arg);
    };

    let assignment = BashArrayAssignment {
        /* The reader cut one word here; the name, the subscript and the
         * value are this parser's reading of it, not the reader's cuts,
         * so the run belongs to the assignment and the parts carry none. */
        // [spec:nsh:def:idiom.token-stream]
        tokens: arg.tokens.clone(),
        name: NodeText::from(name.as_bstr()),
        subscript: Some(arg_part(&arg, open + 1, close)),
        operator,
        value: BashArrayValue::Word(arg_part(&arg, value_start, units.len())),
    };
    Ok(Node::Bash(BashNode::ArrayAssignment(Box::new(assignment))))
}

/// Recognise an unsubscripted `name+=...` append assignment.
fn append_word(shell: &Shell, arg: WordNode) -> Result<Node, WordNode> {
    let units = arg.word.units();
    let Some(plus) = units
        .iter()
        .position(|unit| matches!(unit, WordUnit::Literal { byte: b'+', .. }))
    else {
        return Err(arg);
    };
    if plus == 0
        || !matches!(
            units.get(plus + 1),
            Some(WordUnit::Literal { byte: b'=', .. })
        )
    {
        return Err(arg);
    }
    let Some(name) = literal_bytes(&units[..plus]) else {
        return Err(arg);
    };
    if !is_valid_name(&shell.locale, name.as_bstr()) {
        return Err(arg);
    }
    let value = arg_part(&arg, plus + 2, units.len());
    Ok(Node::Bash(BashNode::ArrayAssignment(Box::new(
        BashArrayAssignment {
            tokens: arg.tokens,
            name: NodeText::from(name.as_bstr()),
            subscript: None,
            operator: BashAssignmentOperator::Append,
            value: BashArrayValue::Word(value),
        },
    ))))
}

/// Whether this command's assignment-shaped operands are assignments.
///
/// Bash decides that while parsing, so an operand of a declaration
/// built-in reached through an expansion -- `cmd=typeset; $cmd x=$y` --
/// is an ordinary word and splits. POSIX mode keeps deciding it from
/// the built-in that ran.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn declaration_operands(shell: &Shell, args: &[Node]) -> bool {
    shell.options.dialect() != Dialect::Bash || declaration_context(args)
}

/// Whether a simple command's own word names a declaration built-in.
///
/// The name has to be written out: Bash decides this while parsing, so
/// `declare x=$y` expands its operand as an assignment while
/// `cmd=declare; $cmd x=$y` splits the operand like any other word.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(crate) fn declaration_context(args: &[Node]) -> bool {
    let Some(Node::Word(command)) = args.first() else {
        return false;
    };
    let command: &[u8] = command.word.as_bstr().as_ref();
    matches!(
        command,
        b"declare" | b"typeset" | b"local" | b"readonly" | b"export"
    )
}

pub(super) fn compound_array(
    shell: &mut Shell,
    variables: &mut Vec<Node>,
    args: &mut Vec<Node>,
) -> Result<bool, Error> {
    // Bash requires `name=` and `(` to be adjacent: `a= (1 2)` is a
    // syntax error, not a compound assignment followed by a subshell.
    if shell.input.last_token_after_blank {
        return Ok(false);
    }
    let use_args = declaration_context(args) && compound_candidate(shell, args.last());
    let use_vars = args.is_empty() && compound_candidate(shell, variables.last());
    let target = if use_args {
        args
    } else if use_vars {
        variables
    } else {
        return Ok(false);
    };

    let previous = target.pop().expect("a compound candidate exists");
    let mut assignment =
        compound_prefix(shell, previous).expect("the candidate predicate and conversion agree");
    let mut elements = Vec::new();
    loop {
        let element_mark = super::tokens::mark(shell);
        let token = read_token(shell, TokenContext::SKIP_NEWLINES)?;
        if token.kind == TokenKind::RightParen {
            break;
        }
        if token.kind != TokenKind::Word {
            return Err(syntax_error(shell, b"invalid compound array assignment"));
        }
        let arg = take_word(shell, token.quoted, element_mark).arg;
        elements.push(array_element(arg));
    }
    assignment.value = BashArrayValue::Compound(elements);
    target.push(Node::Bash(BashNode::ArrayAssignment(Box::new(assignment))));
    Ok(true)
}

// [spec:nsh:req:idiom.lexer-tokens]
pub(super) fn process_substitutions(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    enabled: bool,
) -> Result<(), Error> {
    loop {
        if !enabled || !active(shell) || !lexer.delimiter.is_none() {
            return Ok(());
        }
        let direction = match lexer.input.byte() {
            Some(b'<') => BashProcessDirection::Input,
            Some(b'>') => BashProcessDirection::Output,
            _ => return Ok(()),
        };
        if !read_unit_skipping_line_continuations(shell)?.is(b'(') {
            unread_input_unit(shell);
            return Ok(());
        }

        let saved_heredocs = mem::take(&mut shell.input.pending_here_documents);
        let completed_at = shell.input.completed_here_documents.len();
        let parsed = crate::resource::with_resources(shell, |shell, _resources| {
            let mut body = list(shell, ListMode::StopAtTerminator)?.into_node();
            if read_token(shell, TokenContext::NONE)?.kind != TokenKind::RightParen {
                return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
            }
            set_input_string(shell, BStr::new(b""));
            parse_here_documents(shell)?;
            finalize::node(shell, &mut body, completed_at)?;
            Ok(body)
        });
        shell.input.pending_here_documents = saved_heredocs;
        let body = parsed?;

        lexer.output.push(WordToken::Command(Some(Node::Bash(
            BashNode::ProcessSubstitution(BashProcessSubstitution {
                /* `<(list)` is inside a word, and the reader cuts words
                 * whole: the run that spells this is the enclosing
                 * word's, and cutting a second one for it would record
                 * the same bytes twice. */
                // [spec:nsh:def:idiom.token-stream]
                tokens: SourceTokens::none(),
                direction,
                body: body.map(Box::new),
            }),
        ))));
        lexer.input = super::read_unit_for_syntax(shell, lexer.current_syntax())?;
    }
}

pub(super) fn parameter_subscript(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    bad_substitution: bool,
    allowed: bool,
) -> Result<(), Error> {
    if bad_substitution || !allowed || !lexer.input.is(b'[') || !active(shell) {
        return Ok(());
    }
    let mut depth = 0usize;
    let mut quote = Quote::None;
    let mut escaped = false;

    loop {
        let input = lexer.input;
        let Some(byte) = input.byte() else {
            return Err(syntax_error(shell, b"unterminated array subscript"));
        };
        if input.is(b'\n') {
            consume_newline_without_prompt(shell);
        }
        lexer.push_literal(byte);

        if escaped {
            escaped = false;
        } else {
            match quote {
                Quote::Single if input.is(b'\'') => quote = Quote::None,
                Quote::Double if input.is(b'"') => quote = Quote::None,
                Quote::Single => {}
                Quote::Double if input.is(b'\\') => escaped = true,
                Quote::Double => {}
                Quote::None if input.is(b'\\') => escaped = true,
                Quote::None if input.is(b'\'') => quote = Quote::Single,
                Quote::None if input.is(b'"') => quote = Quote::Double,
                Quote::None if input.is(b'[') => depth += 1,
                Quote::None if input.is(b']') => {
                    depth -= 1;
                    if depth == 0 {
                        lexer.input = read_unit_skipping_line_continuations(shell)?;
                        return Ok(());
                    }
                }
                Quote::None => {}
            }
        }
        lexer.input = read_unit_skipping_line_continuations(shell)?;
    }
}

fn arithmetic_text(shell: &mut Shell) -> Result<NodeText, Error> {
    let mut output = BString::new(Vec::new());
    let mut depth = 0usize;
    let mut quote = Quote::None;
    let mut escaped = false;
    let mut pending = None;

    loop {
        let input = match pending.take() {
            Some(input) => input,
            None => read_input_unit(shell)?,
        };
        let Some(byte) = input.byte() else {
            return Err(syntax_error(shell, b"missing '))'"));
        };
        if input.is(b'\n') {
            consume_newline_without_prompt(shell);
        }

        if escaped {
            output.push(byte);
            escaped = false;
            continue;
        }
        match quote {
            Quote::Single => {
                output.push(byte);
                if input.is(b'\'') {
                    quote = Quote::None;
                }
            }
            Quote::Double => {
                output.push(byte);
                if input.is(b'\\') {
                    escaped = true;
                } else if input.is(b'"') {
                    quote = Quote::None;
                }
            }
            Quote::None if input.is(b'\\') => {
                output.push(byte);
                escaped = true;
            }
            Quote::None if input.is(b'\'') => {
                output.push(byte);
                quote = Quote::Single;
            }
            Quote::None if input.is(b'"') => {
                output.push(byte);
                quote = Quote::Double;
            }
            Quote::None if input.is(b'(') => {
                depth += 1;
                output.push(byte);
            }
            Quote::None if input.is(b')') && depth != 0 => {
                depth -= 1;
                output.push(byte);
            }
            Quote::None if input.is(b')') => {
                let next = read_input_unit(shell)?;
                if next.is(b')') {
                    break;
                }
                output.push(byte);
                pending = Some(next);
            }
            Quote::None => output.push(byte),
        }
    }
    /* Read byte at a time rather than through `read_token`, so nothing
     * has closed these bytes into a token and the run of the node about
     * to be built would stop after its `((`. */
    // [spec:nsh:req:idiom.printable-ast+2]
    shell.input.tokens.cut(super::SourceTokenKind::Operator);
    Ok(NodeText::from(output.as_slice()))
}

fn for_clauses(shell: &mut Shell, text: &BStr) -> Result<[NodeText; 3], Error> {
    let mut separators = Vec::new();
    let mut parens = 0usize;
    let mut brackets = 0usize;
    let mut quote = Quote::None;
    let mut escaped = false;

    for (index, &byte) in text.iter().enumerate() {
        if escaped {
            escaped = false;
            continue;
        }
        match quote {
            Quote::Single if byte == b'\'' => quote = Quote::None,
            Quote::Double if byte == b'"' => quote = Quote::None,
            Quote::Single => {}
            Quote::Double if byte == b'\\' => escaped = true,
            Quote::Double => {}
            Quote::None if byte == b'\\' => escaped = true,
            Quote::None if byte == b'\'' => quote = Quote::Single,
            Quote::None if byte == b'"' => quote = Quote::Double,
            Quote::None if byte == b'(' => parens += 1,
            Quote::None if byte == b')' => parens = parens.saturating_sub(1),
            Quote::None if byte == b'[' => brackets += 1,
            Quote::None if byte == b']' => brackets = brackets.saturating_sub(1),
            Quote::None if byte == b';' && parens == 0 && brackets == 0 => separators.push(index),
            Quote::None => {}
        }
    }
    if separators.len() != 2 {
        return Err(syntax_error(
            shell,
            b"arithmetic for requires three expressions",
        ));
    }
    Ok([
        NodeText::from(&text[..separators[0]]),
        NodeText::from(&text[separators[0] + 1..separators[1]]),
        NodeText::from(&text[separators[1] + 1..]),
    ])
}

fn conditional_or(shell: &mut Shell) -> Result<BashConditionalExpr, Error> {
    let mut left = conditional_and(shell)?;
    loop {
        // A newline inside `[[ ]]` continues the expression, so a long
        // condition can be written over several lines.
        let token = read_token(shell, TokenContext::SKIP_NEWLINES)?.kind;
        if token != TokenKind::OrIf {
            shell.input.token_pushed_back = true;
            return Ok(left);
        }
        left = BashConditionalExpr::Or(Box::new(left), Box::new(conditional_and(shell)?));
    }
}

fn conditional_and(shell: &mut Shell) -> Result<BashConditionalExpr, Error> {
    let mut left = conditional_primary(shell)?;
    loop {
        let token = read_token(shell, TokenContext::SKIP_NEWLINES)?.kind;
        if token != TokenKind::AndIf {
            shell.input.token_pushed_back = true;
            return Ok(left);
        }
        left = BashConditionalExpr::And(Box::new(left), Box::new(conditional_primary(shell)?));
    }
}

fn conditional_primary(shell: &mut Shell) -> Result<BashConditionalExpr, Error> {
    let first_mark = super::tokens::mark(shell);
    let token = read_token(shell, TokenContext::NONE)?;
    if token.kind == TokenKind::LeftParen {
        let expression = conditional_or(shell)?;
        if read_token(shell, TokenContext::NONE)?.kind != TokenKind::RightParen {
            return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
        }
        return Ok(BashConditionalExpr::Group(Box::new(expression)));
    }
    if token.kind != TokenKind::Word || closes_conditional(shell, token.kind, token.quoted) {
        return Err(syntax_error(shell, b"expected conditional expression"));
    }

    let first = take_word(shell, token.quoted, first_mark);
    if !first.quoted && first.arg.word.as_bstr() == BStr::new(b"!") {
        return Ok(BashConditionalExpr::Not(Box::new(conditional_primary(
            shell,
        )?)));
    }
    if !first.quoted && unary_operator(first.arg.word.as_bstr()) {
        let operand_mark = super::tokens::mark(shell);
        let operand_token = read_token(shell, TokenContext::NONE)?;
        if operand_token.kind != TokenKind::Word
            || closes_conditional(shell, operand_token.kind, operand_token.quoted)
        {
            return Err(syntax_error(shell, b"expected unary-test operand"));
        }
        return Ok(BashConditionalExpr::Unary {
            operator: NodeText::from(first.arg.word.as_bstr()),
            operand: take_word(shell, operand_token.quoted, operand_mark).arg,
        });
    }

    let operator_token = read_token(shell, TokenContext::NONE)?;
    let operator = if operator_token.kind == super::TokenKind::Redirection {
        let redirection = shell.input.pending_redirection.take();
        /* `[[ a < b ]]` compares strings, but `[[ a 3< b ]]` names a
         * descriptor, and a descriptor is not an operator here. */
        match redirection.as_ref() {
            Some(super::PendingRedirection::File {
                operator,
                descriptor,
            }) if *operator == FileRedirectionOperator::Read
                && descriptor.fixed() == Some(LogicalDescriptor::STDIN) =>
            {
                Some(NodeText::from(b"<".as_slice()))
            }
            Some(super::PendingRedirection::File {
                operator,
                descriptor,
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
        shell.input.token_pushed_back = true;
        return Ok(BashConditionalExpr::Word(first.arg));
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
    let right_mark = super::tokens::mark(shell);
    let right_token = read_token(shell, context);
    shell.input.parsing_conditional = enclosing;
    let right_token = right_token?;
    if right_token.kind != TokenKind::Word
        || closes_conditional(shell, right_token.kind, right_token.quoted)
    {
        return Err(syntax_error(shell, b"expected binary-test operand"));
    }
    Ok(BashConditionalExpr::Binary {
        left: first.arg,
        operator,
        right: take_word(shell, right_token.quoted, right_mark).arg,
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

fn take_word(shell: &mut Shell, quoted: bool, mark: TokenMark) -> ConditionalWord {
    ConditionalWord {
        arg: WordNode {
            tokens: super::tokens::run(shell, mark),
            word: mem::take(&mut shell.input.word),
        },
        quoted,
    }
}

fn compound_candidate(shell: &Shell, node: Option<&Node>) -> bool {
    match node {
        Some(Node::Word(arg)) => {
            plain_prefix(arg).is_some_and(|(name, _)| is_valid_name(&shell.locale, name.as_bstr()))
        }
        Some(Node::Bash(BashNode::ArrayAssignment(assignment))) => {
            matches!(&assignment.value, BashArrayValue::Word(value) if value.word.as_bstr().is_empty())
        }
        _ => false,
    }
}

fn compound_prefix(shell: &Shell, node: Node) -> Option<BashArrayAssignment> {
    match node {
        Node::Word(arg) => {
            let (name, operator) = plain_prefix(&arg)?;
            if !is_valid_name(&shell.locale, name.as_bstr()) {
                return None;
            }
            Some(BashArrayAssignment {
                tokens: arg.tokens.clone(),
                name: NodeText::from(name.as_bstr()),
                subscript: None,
                operator,
                value: BashArrayValue::Word(WordNode {
                    tokens: SourceTokens::none(),
                    word: ParsedWord::new(),
                }),
            })
        }
        Node::Bash(BashNode::ArrayAssignment(assignment)) => Some(*assignment),
        _ => None,
    }
}

fn plain_prefix(arg: &WordNode) -> Option<(BString, BashAssignmentOperator)> {
    let units = arg.word.units();
    let (name_end, operator) = if matches!(
        units.as_slice(),
        [
            ..,
            WordUnit::Literal { byte: b'+', .. },
            WordUnit::Literal { byte: b'=', .. }
        ]
    ) {
        (units.len() - 2, BashAssignmentOperator::Append)
    } else if matches!(units.last(), Some(WordUnit::Literal { byte: b'=', .. })) {
        (units.len() - 1, BashAssignmentOperator::Set)
    } else {
        return None;
    };
    if name_end == 0 {
        return None;
    }
    Some((literal_bytes(&units[..name_end])?, operator))
}

fn array_element(arg: WordNode) -> BashArrayElement {
    let units = arg.word.units();
    if matches!(units.first(), Some(WordUnit::Literal { byte: b'[', .. })) {
        if let Some(close) = matching_bracket(&units, 0) {
            if let Some((operator, value_start)) = assignment_operator(&units, close + 1) {
                return BashArrayElement {
                    subscript: Some(arg_part(&arg, 1, close)),
                    operator,
                    value: arg_part(&arg, value_start, units.len()),
                };
            }
        }
    }
    BashArrayElement {
        subscript: None,
        operator: BashAssignmentOperator::Set,
        value: arg,
    }
}

fn matching_bracket(units: &[WordUnit], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    while index < units.len() {
        match &units[index] {
            WordUnit::Literal {
                byte: b'[',
                quoted: false,
            } => {
                depth += 1;
                index += 1;
            }
            WordUnit::Literal {
                byte: b']',
                quoted: false,
            } => {
                depth = depth.checked_sub(1)?;
                if depth == 0 {
                    return Some(index);
                }
                index += 1;
            }
            _ => index += 1,
        }
    }
    None
}

fn assignment_operator(
    units: &[WordUnit],
    start: usize,
) -> Option<(BashAssignmentOperator, usize)> {
    if matches!(
        units.get(start..start + 2),
        Some([
            WordUnit::Literal { byte: b'+', .. },
            WordUnit::Literal { byte: b'=', .. }
        ])
    ) {
        Some((BashAssignmentOperator::Append, start + 2))
    } else if matches!(units.get(start), Some(WordUnit::Literal { byte: b'=', .. })) {
        Some((BashAssignmentOperator::Set, start + 1))
    } else {
        None
    }
}

fn arg_part(arg: &WordNode, start: usize, end: usize) -> WordNode {
    // [spec:nsh:def:idiom.word-ir]
    let units = arg.word.units();
    WordNode {
        tokens: SourceTokens::none(),
        word: ParsedWord::from_units(units.get(start..end).unwrap_or_default()),
    }
}

fn literal_bytes(units: &[WordUnit]) -> Option<BString> {
    units
        .iter()
        .map(|unit| match unit {
            /* A quoted byte is not a name byte: `"a"[0]=1` names
             * nothing, the way it did when quoting was a part of its
             * own and stopped this walk. */
            // [spec:nsh:req:idiom.canonical-tree+1]
            WordUnit::Literal {
                byte,
                quoted: false,
            } => Some(*byte),
            WordUnit::Literal { .. } | WordUnit::Part(_) => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(BString::from)
}

/// Recognise Bash's `${!name}` marker.
///
/// `${!}` is still the special parameter, so the marker is only taken
/// when a name can follow it.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn parameter_indirection(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    braced: bool,
) -> Result<Indirection, Error> {
    if !braced || !active(shell) || !lexer.input.is(b'!') {
        return Ok(Indirection::Absent);
    }
    let next = read_unit_skipping_line_continuations(shell)?;
    if next.begins_name(&shell.locale) || next.is_digit() || next.is(b'@') || next.is(b'*') {
        lexer.input = next;
        return Ok(Indirection::Present);
    }
    /* `${!}` is the special parameter and `${!-word}` applies an
     * operator to it, but `${!#word}` is neither: after `!` a `#` can
     * only be the whole target, so anything following it makes the
     * substitution one Bash refuses rather than a length operator. */
    // [spec:nsh:req:compat.bash.expansion-globbing]
    if next.is(b'#') {
        let following = read_unit_skipping_line_continuations(shell)?;
        unread_input_unit(shell);
        if !following.is(b'}') {
            lexer.input = next;
            return Ok(Indirection::Invalid);
        }
    }
    unread_input_unit(shell);
    Ok(Indirection::Absent)
}

/// What `${!` turned out to introduce.
// [spec:nsh:req:compat.bash.expansion-globbing]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Indirection {
    /// The `!` is the special parameter, not an indirection marker.
    Absent,
    /// An indirection whose target the name lexer reads next.
    Present,
    /// An indirection whose target cannot be a parameter at all.
    Invalid,
}

/// `${!prefix*}` and `${!prefix@}` name every variable whose name starts
/// with the prefix, so the trailing selector belongs to the name and not
/// to an operator. `${!name@Q}` does not: there the `@` introduces one.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn parameter_prefix_selector(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    indirect: bool,
) -> Result<(), Error> {
    if !indirect || !active(shell) {
        return Ok(());
    }
    if lexer.input.is(b'*') {
        lexer.push_literal(b'*');
        lexer.input = read_unit_skipping_line_continuations(shell)?;
        return Ok(());
    }
    if !lexer.input.is(b'@') {
        return Ok(());
    }
    let next = read_unit_skipping_line_continuations(shell)?;
    if next.is(b'}') {
        lexer.push_literal(b'@');
        lexer.input = next;
    } else {
        unread_input_unit(shell);
    }
    Ok(())
}

/// The Bash-only parameter operators, each of which has a doubled
/// spelling that widens what it applies to.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn parameter_operator(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
) -> Result<Option<ParameterOperation>, Error> {
    if !active(shell) {
        return Ok(None);
    }
    let (single, doubled) = match lexer.input.byte() {
        Some(b'/') => (
            ParameterOperation::SubstituteFirst,
            ParameterOperation::SubstituteAll,
        ),
        Some(b'^') => (ParameterOperation::UpperFirst, ParameterOperation::UpperAll),
        Some(b',') => (ParameterOperation::LowerFirst, ParameterOperation::LowerAll),
        Some(b'@') => return Ok(Some(ParameterOperation::Transform)),
        _ => return Ok(None),
    };
    let first = lexer.input;
    lexer.input = read_unit_skipping_line_continuations(shell)?;
    if lexer.input == first {
        if lexer.check_here_document_end {
            lexer.push_literal(lexer.input.expect_byte());
        }
        return Ok(Some(doubled));
    }
    unread_input_unit(shell);
    Ok(Some(single))
}

/// `$"…"` marks a string for locale translation. Without a message
/// catalogue the translation is the string itself, so the only lasting
/// effect is that the contents are double-quoted.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn locale_quote(
    shell: &Shell,
    lexer: &mut WordLexer<'_>,
    nested: SyntaxContext,
    substitution_start: usize,
) -> bool {
    if !active(shell)
        || !lexer.input.is(b'"')
        || nested.classify(InputUnit::Byte(b'&')) == SyntaxClass::Word
    {
        return false;
    }
    lexer.output.truncate(substitution_start);
    lexer.current_syntax_mut().syntax = SyntaxContext::DoubleQuoted;
    lexer.current_syntax_mut().double_quoted = true;
    lexer.record_quote_boundary(true, true);
    true
}

/// `$[expression]` is Bash's older spelling of `$((expression))`; it
/// evaluates the same way and only its terminator differs.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn arithmetic_bracket(
    shell: &Shell,
    lexer: &mut WordLexer<'_>,
    substitution_start: usize,
) -> bool {
    if !active(shell) || !lexer.input.is(b'[') || lexer.check_here_document_end {
        return false;
    }
    syntax_stack::push(&mut lexer.syntax_frames, SyntaxContext::ArithmeticBracket);
    lexer.current_syntax_mut().double_quoted = true;
    lexer.output.truncate(substitution_start);
    lexer.output.push(WordToken::ArithmeticStart);
    true
}

/// Track the `[...]` of an assignment word, where blanks and shell
/// operators are the subscript's own bytes.
///
/// Bash's lexer, at a position where an assignment word may begin,
/// consumes a balanced bracket pair after a name: `a[1 + 1]=x` is one
/// word and one assignment, where the ordinary rules would end the word
/// at the blank and leave three. The subscript is an arithmetic
/// expression, so what is inside it is data -- including `&`, `|` and
/// `;` -- and only the matching `]` ends it.
///
/// A word that is not in that position is untouched, which is what
/// keeps `argv.py a[1 + 2]=` three arguments.
// [spec:nsh:req:compat.bash.arrays-declarations]
pub(super) fn track_assignment_subscript(shell: &Shell, lexer: &mut WordLexer<'_>) {
    if !lexer.assignment_position || lexer.current_syntax().syntax != SyntaxContext::Base {
        return;
    }
    /* A bracket inside `${...}` belongs to the expansion, not to the
     * subscript around it: Bash skips a whole expansion while looking for
     * the matching `]`, so `a[${x:-]}]=1` subscripts on the expansion and
     * closes on the bracket after it, and `a[${ ]}` is still unterminated. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    if lexer.current_syntax().variable_depth != 0 {
        return;
    }
    if lexer.input.is(b'[') {
        if lexer.subscript_depth > 0 {
            lexer.subscript_depth += 1;
            return;
        }
        /* The bytes before the bracket have to spell a name; anything
         * else -- a glob, a quoted run, an expansion -- is an ordinary
         * word that happens to contain a bracket. */
        let name = lexer.literal_bytes(0..lexer.output.len());
        if name.is_some_and(|name| !name.is_empty() && is_valid_name(&shell.locale, name.as_bstr()))
        {
            lexer.subscript_depth = 1;
        }
        return;
    }
    if lexer.input.is(b']') && lexer.subscript_depth > 0 {
        lexer.subscript_depth -= 1;
    }
}

/// Scan one byte of a `$[…]` expression, and say whether it was consumed.
///
/// Bash reads `$[` with `parse_matched_pair`, which is a much smaller
/// scanner than this one: only the brackets nest, a parenthesis is one
/// of the expression's own bytes, and a quoted run is a nested pair
/// whose contents -- a `]` included -- are data. Sharing the arithmetic
/// context made `$[(]` unterminated, let `$[))` close, and let `$[']`
/// end an expression Bash reads to end of input.
///
/// The quotes stay in the expression's text, because Bash hands them to
/// the arithmetic evaluator too. All this decides is where the scan ends.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn scan_arithmetic_bracket(lexer: &mut WordLexer<'_>) -> bool {
    let run = match lexer.current_syntax().syntax {
        SyntaxContext::ArithmeticSingleQuoted => Some(b'\''),
        SyntaxContext::ArithmeticDoubleQuoted => Some(b'"'),
        SyntaxContext::ArithmeticBracket => None,
        _ => return false,
    };
    if let Some(quote) = run {
        if lexer.input.is(quote) {
            lexer.current_syntax_mut().syntax = SyntaxContext::ArithmeticBracket;
        }
        return false;
    }
    if lexer.input.is(b'\'') {
        lexer.current_syntax_mut().syntax = SyntaxContext::ArithmeticSingleQuoted;
        return false;
    }
    if lexer.input.is(b'"') {
        lexer.current_syntax_mut().syntax = SyntaxContext::ArithmeticDoubleQuoted;
        return false;
    }
    if lexer.input.is(b'[') {
        lexer.current_syntax_mut().parenthesis_depth += 1;
        return false;
    }
    if !lexer.input.is(b']') {
        return false;
    }
    if lexer.current_syntax().parenthesis_depth > 0 {
        lexer.current_syntax_mut().parenthesis_depth -= 1;
        return false;
    }
    syntax_stack::pop(&mut lexer.syntax_frames);
    lexer.output.push(WordToken::ArithmeticEnd);
    true
}

/// Open an `X(alternative|…)` extended-glob group.
///
/// `shopt -s extglob` has to be read while the word is being lexed,
/// because it decides whether `(` belongs to the word or ends it.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn open_extended_glob(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
) -> Result<bool, Error> {
    if !active(shell)
        || !(shell.options.shopt(BashShopt::ExtGlob) || shell.input.parsing_conditional)
        || lexer.current_syntax().syntax != SyntaxContext::Base
        || !matches!(lexer.input.byte(), Some(b'?' | b'*' | b'+' | b'@' | b'!'))
    {
        return Ok(false);
    }
    let operator = lexer.input;
    lexer.input = read_unit_skipping_line_continuations(shell)?;
    if !lexer.input.is(b'(') {
        unread_input_unit(shell);
        lexer.input = operator;
        return Ok(false);
    }
    lexer.push_literal(operator.expect_byte());
    lexer.push_literal(b'(');
    lexer.extglob_depth += 1;
    lexer.input = super::read_unit_for_syntax(shell, lexer.current_syntax())?;
    Ok(true)
}

/// Whether this word separator is inside an extended-glob group, and so
/// is one of the pattern's own bytes.
// [spec:nsh:req:compat.bash.expansion-globbing]
pub(super) fn inside_extended_glob(lexer: &mut WordLexer<'_>) -> bool {
    if lexer.extglob_depth == 0 {
        return false;
    }
    if lexer.input.is(b')') {
        lexer.extglob_depth -= 1;
    }
    true
}

/// The name in a `{name}` redirection prefix, if the word is one.
///
/// `{name}` names no descriptor and asks for one to be allocated, with the
/// number assigned to `name`. So `name` has to be somewhere a number can
/// go, which is what keeps the form from swallowing ordinary words: `{1a}`
/// and `{}` are words -- Bash runs `exec {1a}` as a command and reports it
/// not found -- and the braces have to be the whole word, so `echo {fd}x>f`
/// redirects and allocates nothing. A subscript is a place too, because
/// Bash accepts `{a[0]}`.
// [spec:nsh:req:compat.bash.parser-ast]
pub(super) fn allocated_descriptor(shell: &Shell, bytes: &[u8]) -> Option<NodeText> {
    if !active(shell) {
        return None;
    }
    let name = bytes.strip_prefix(b"{")?.strip_suffix(b"}")?;
    /* The `x` stands in for a subscript that is there, so the emptiness
     * test below reads the same whether or not one was written. */
    let (head, subscript) = match name.iter().position(|byte| *byte == b'[') {
        Some(open) if name.last() == Some(&b']') => {
            (&name[..open], &name[open + 1..name.len() - 1])
        }
        Some(_) => return None,
        None => (name, &b"x"[..]),
    };
    let assignable = !subscript.is_empty()
        && head
            .first()
            .is_some_and(|byte| is_name(&shell.locale, *byte))
        && head[1..]
            .iter()
            .all(|byte| is_in_name(&shell.locale, *byte));
    assignable.then(|| NodeText::new(BString::from(name)))
}
