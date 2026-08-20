//! Bash-only productions in the existing recursive-descent parser.

// [spec:nsh:req:idiom.operation-modes]
use core::mem;

use bstr::{BStr, BString, ByteSlice as _};

use super::{
    ListMode, TokenContext, TokenKind, WordLexer, command, consume_newline_without_prompt,
    expected_token_error, finalize, is_valid_name, list, parse_here_documents, read_input_unit,
    read_token, read_unit_skipping_line_continuations, set_input_string, syntax_error,
    unread_input_unit,
};
use crate::context::Shell;
use crate::error::Error;
use crate::nodes::{
    BashArithmeticCommand, BashArithmeticFor, BashArrayAssignment, BashArrayElement,
    BashArrayValue, BashAssignmentOperator, BashConditional, BashConditionalExpr, BashFunction,
    BashFunctionStyle, BashNode, BashProcessDirection, BashProcessSubstitution,
    FileRedirectionOperator, Node, NodeText, WordNode,
};
use crate::options::Dialect;
use crate::word::{ParsedWord, QuoteBoundary, WordPart, WordToken, WordUnit};

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
    line: i32,
) -> Result<Option<Node>, Error> {
    if !active(shell) {
        return Ok(None);
    }
    if token == TokenKind::DoubleParen {
        return arithmetic_command(shell).map(Some);
    }
    if token != TokenKind::Word || shell.input.last_token_quoted {
        return Ok(None);
    }
    if shell.input.word_text() == BStr::new(b"[[") {
        conditional(shell).map(Some)
    } else if shell.input.word_text() == BStr::new(b"function") {
        function(shell, line).map(Some)
    } else {
        Ok(None)
    }
}

pub(super) fn arithmetic_command(shell: &mut Shell) -> Result<Node, Error> {
    let expression = arithmetic_text(shell)?;
    Ok(Node::Bash(BashNode::ArithmeticCommand(
        BashArithmeticCommand { expression },
    )))
}

pub(super) fn arithmetic_for(shell: &mut Shell, line: i32) -> Result<Node, Error> {
    let text = arithmetic_text(shell)?;
    let [init, test, update] = for_clauses(shell, text.as_bstr())?;

    let separator = read_token(shell, TokenContext::RESERVED_WORDS)?.kind;
    let do_token = if separator == TokenKind::Do {
        TokenKind::Do
    } else if separator == TokenKind::Semicolon {
        read_token(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?.kind
    } else if separator == TokenKind::Newline {
        shell.input.token_pushed_back = true;
        read_token(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?.kind
    } else {
        return Err(expected_token_error(shell, Some(TokenKind::Do)));
    };
    if do_token != TokenKind::Do {
        return Err(expected_token_error(shell, Some(TokenKind::Do)));
    }

    let body = list(shell, ListMode::Compound)?
        .into_node()
        .ok_or_else(|| expected_token_error(shell, None))?;
    Ok(Node::Bash(BashNode::ArithmeticFor(BashArithmeticFor {
        line,
        init,
        test,
        update,
        body: Box::new(body),
    })))
}

pub(super) fn conditional(shell: &mut Shell) -> Result<Node, Error> {
    let first = read_token(shell, TokenContext::NONE)?;
    let expression = if closes_conditional(shell, first.kind, first.quoted) {
        BashConditionalExpr::Empty
    } else {
        shell.input.token_pushed_back = true;
        let expression = conditional_or(shell)?;
        let close = read_token(shell, TokenContext::NONE)?;
        if !closes_conditional(shell, close.kind, close.quoted) {
            return Err(syntax_error(shell, b"expected ']]'"));
        }
        expression
    };

    Ok(Node::Bash(BashNode::Conditional(BashConditional {
        expression,
    })))
}

// [spec:nsh:req:idiom.structural-ast]
pub(super) fn function(shell: &mut Shell, line: i32) -> Result<Node, Error> {
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
        .position(|unit| matches!(unit, WordUnit::Literal(b'[')))
    else {
        return Err(arg);
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
        name: NodeText::from(name.as_bstr()),
        subscript: Some(arg_part(&arg, open + 1, close)),
        operator,
        value: BashArrayValue::Word(arg_part(&arg, value_start, units.len())),
    };
    Ok(Node::Bash(BashNode::ArrayAssignment(assignment)))
}

pub(super) fn declaration_context(args: &[Node]) -> bool {
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
        let token = read_token(shell, TokenContext::SKIP_NEWLINES)?;
        if token.kind == TokenKind::RightParen {
            break;
        }
        if token.kind != TokenKind::Word {
            return Err(syntax_error(shell, b"invalid compound array assignment"));
        }
        let arg = take_word(shell, token.quoted).arg;
        elements.push(array_element(arg));
    }
    assignment.value = BashArrayValue::Compound(elements);
    target.push(Node::Bash(BashNode::ArrayAssignment(assignment)));
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
        let token = read_token(shell, TokenContext::NONE)?.kind;
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
        let token = read_token(shell, TokenContext::NONE)?.kind;
        if token != TokenKind::AndIf {
            shell.input.token_pushed_back = true;
            return Ok(left);
        }
        left = BashConditionalExpr::And(Box::new(left), Box::new(conditional_primary(shell)?));
    }
}

fn conditional_primary(shell: &mut Shell) -> Result<BashConditionalExpr, Error> {
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

    let first = take_word(shell, token.quoted);
    if !first.quoted && first.arg.word.as_bstr() == BStr::new(b"!") {
        return Ok(BashConditionalExpr::Not(Box::new(conditional_primary(
            shell,
        )?)));
    }
    if !first.quoted && unary_operator(first.arg.word.as_bstr()) {
        let operand_token = read_token(shell, TokenContext::NONE)?;
        if operand_token.kind != TokenKind::Word
            || closes_conditional(shell, operand_token.kind, operand_token.quoted)
        {
            return Err(syntax_error(shell, b"expected unary-test operand"));
        }
        return Ok(BashConditionalExpr::Unary {
            operator: NodeText::from(first.arg.word.as_bstr()),
            operand: take_word(shell, operand_token.quoted).arg,
        });
    }

    let operator_token = read_token(shell, TokenContext::NONE)?;
    let operator = if operator_token.kind == super::TokenKind::Redirection {
        let redirection = shell.input.pending_redirection.take();
        match redirection.as_ref() {
            Some(super::PendingRedirection::File { operator, .. })
                if *operator == FileRedirectionOperator::Read =>
            {
                Some(NodeText::from(b"<".as_slice()))
            }
            Some(super::PendingRedirection::File { operator, .. })
                if *operator == FileRedirectionOperator::Write =>
            {
                Some(NodeText::from(b">".as_slice()))
            }
            _ => None,
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

    let right_token = read_token(shell, TokenContext::NONE)?;
    if right_token.kind != TokenKind::Word
        || closes_conditional(shell, right_token.kind, right_token.quoted)
    {
        return Err(syntax_error(shell, b"expected binary-test operand"));
    }
    Ok(BashConditionalExpr::Binary {
        left: first.arg,
        operator,
        right: take_word(shell, right_token.quoted).arg,
    })
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

fn take_word(shell: &mut Shell, quoted: bool) -> ConditionalWord {
    ConditionalWord {
        arg: WordNode {
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
                name: NodeText::from(name.as_bstr()),
                subscript: None,
                operator,
                value: BashArrayValue::Word(WordNode {
                    word: ParsedWord::new(),
                }),
            })
        }
        Node::Bash(BashNode::ArrayAssignment(assignment)) => Some(assignment),
        _ => None,
    }
}

fn plain_prefix(arg: &WordNode) -> Option<(BString, BashAssignmentOperator)> {
    let units = arg.word.units();
    let (name_end, operator) = if matches!(
        units.as_slice(),
        [.., WordUnit::Literal(b'+'), WordUnit::Literal(b'=')]
    ) {
        (units.len() - 2, BashAssignmentOperator::Append)
    } else if matches!(units.last(), Some(WordUnit::Literal(b'='))) {
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
    if matches!(units.first(), Some(WordUnit::Literal(b'['))) {
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
    let mut quoted = false;
    while index < units.len() {
        match &units[index] {
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Open)) => {
                quoted = true;
                index += 1;
            }
            WordUnit::Part(WordPart::Quote(QuoteBoundary::Close)) => {
                quoted = false;
                index += 1;
            }
            WordUnit::Literal(b'[') if !quoted => {
                depth += 1;
                index += 1;
            }
            WordUnit::Literal(b']') if !quoted => {
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
        Some([WordUnit::Literal(b'+'), WordUnit::Literal(b'=')])
    ) {
        Some((BashAssignmentOperator::Append, start + 2))
    } else if matches!(units.get(start), Some(WordUnit::Literal(b'='))) {
        Some((BashAssignmentOperator::Set, start + 1))
    } else {
        None
    }
}

fn arg_part(arg: &WordNode, start: usize, end: usize) -> WordNode {
    // [spec:nsh:def:idiom.word-ir]
    let units = arg.word.units();
    WordNode {
        word: ParsedWord::from_units(units.get(start..end).unwrap_or_default()),
    }
}

fn literal_bytes(units: &[WordUnit]) -> Option<BString> {
    units
        .iter()
        .map(|unit| match unit {
            WordUnit::Literal(byte) => Some(*byte),
            WordUnit::Part(_) => None,
        })
        .collect::<Option<Vec<_>>>()
        .map(BString::from)
}
