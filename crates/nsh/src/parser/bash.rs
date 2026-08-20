//! Bash-only productions in the existing recursive-descent parser.

use core::ffi::c_int;
use core::mem;

use bstr::{BStr, BString};

use super::{
    CTLBACKQ, CTLESC, CTLMBCHAR, CTLQUOTEMARK, Rt1, TokenContext, TokenKind, command, finalize,
    goodname, list, nlnoprompt, parseheredoc, pgetc, pgetc_eatbnl, popfile, pungetc, readtoken,
    readtoken_with_flags, setinputstring, synerror, synexpect, wordtext, wordtext_node,
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
use crate::word::ParsedWord;

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

pub(super) fn active(sh: &Shell) -> bool {
    sh.input.parse_dialect() == Dialect::Bash
}

pub(super) fn command_prefix(
    sh: &mut Shell,
    token: TokenKind,
    line: c_int,
) -> Result<Option<Node>, Error> {
    if !active(sh) {
        return Ok(None);
    }
    if token == TokenKind::DoubleParen {
        return arithmetic_command(sh).map(Some);
    }
    if token != TokenKind::Word || sh.input.last_quoteflag {
        return Ok(None);
    }
    if wordtext(sh) == BStr::new(b"[[") {
        conditional(sh).map(Some)
    } else if wordtext(sh) == BStr::new(b"function") {
        function(sh, line).map(Some)
    } else {
        Ok(None)
    }
}

pub(super) fn arithmetic_command(sh: &mut Shell) -> Result<Node, Error> {
    let expression = arithmetic_text(sh)?;
    Ok(Node::Bash(BashNode::ArithmeticCommand(
        BashArithmeticCommand { expression },
    )))
}

pub(super) fn arithmetic_for(sh: &mut Shell, line: c_int) -> Result<Node, Error> {
    let text = arithmetic_text(sh)?;
    let [init, test, update] = for_clauses(sh, text.as_bstr())?;

    let separator = readtoken(sh, TokenContext::RESERVED_WORDS)?;
    let do_token = if separator == TokenKind::Do {
        TokenKind::Do
    } else if separator == TokenKind::Semicolon {
        readtoken(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)?
    } else if separator == TokenKind::Newline {
        sh.input.tokpushback = true;
        readtoken(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)?
    } else {
        return Err(synexpect(sh, Some(TokenKind::Do)));
    };
    if do_token != TokenKind::Do {
        return Err(synexpect(sh, Some(TokenKind::Do)));
    }

    let body = list(sh, 0)?
        .into_node()
        .ok_or_else(|| synexpect(sh, None))?;
    Ok(Node::Bash(BashNode::ArithmeticFor(BashArithmeticFor {
        line,
        init,
        test,
        update,
        body: Box::new(body),
    })))
}

pub(super) fn conditional(sh: &mut Shell) -> Result<Node, Error> {
    let first = readtoken_with_flags(sh, TokenContext::NONE)?;
    let expression = if closes_conditional(sh, first.kind, first.quoted) {
        BashConditionalExpr::Empty
    } else {
        sh.input.tokpushback = true;
        let expression = conditional_or(sh)?;
        let close = readtoken_with_flags(sh, TokenContext::NONE)?;
        if !closes_conditional(sh, close.kind, close.quoted) {
            return Err(synerror(sh, b"expected ']]'"));
        }
        expression
    };

    Ok(Node::Bash(BashNode::Conditional(BashConditional {
        expression,
    })))
}

// [spec:nsh:req:idiom.structural-ast]
pub(super) fn function(sh: &mut Shell, line: c_int) -> Result<Node, Error> {
    let name_token = readtoken_with_flags(sh, TokenContext::NONE)?;
    if name_token.kind != TokenKind::Word || wordtext(sh).is_empty() {
        return Err(synerror(sh, b"invalid Bash function name"));
    }
    let name = wordtext_node(sh);

    let next = readtoken(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)?;
    let style = if next == TokenKind::LeftParen {
        if readtoken(sh, TokenContext::NONE)? != TokenKind::RightParen {
            return Err(synexpect(sh, Some(TokenKind::RightParen)));
        }
        BashFunctionStyle::FunctionParens
    } else {
        sh.input.tokpushback = true;
        BashFunctionStyle::Function
    };
    let body = command(sh, TokenContext::COMMAND_START_AFTER_NEWLINES)?
        .ok_or_else(|| synexpect(sh, None))?;

    Ok(Node::Bash(BashNode::Function(BashFunction {
        line,
        name,
        style,
        body: Box::new(body),
    })))
}

pub(super) fn array_word(sh: &Shell, arg: WordNode) -> Result<Node, WordNode> {
    let encoded = arg.word.encode_legacy();
    let bytes = BStr::new(&encoded.bytes[..encoded.bytes.len() - 1]);
    let Some(open) = bytes.iter().position(|&byte| byte == b'[') else {
        return Err(arg);
    };
    if goodname(&sh.locale, BStr::new(&bytes[..open])) == 0 {
        return Err(arg);
    }
    let Some(close) = matching_bracket(bytes, open) else {
        return Err(arg);
    };
    let Some((operator, value_start)) = assignment_operator(bytes, close + 1) else {
        return Err(arg);
    };

    let assignment = BashArrayAssignment {
        name: node_text(&bytes[..open]),
        subscript: Some(arg_part(&arg, open + 1, close)),
        operator,
        value: BashArrayValue::Word(arg_part(&arg, value_start, bytes.len())),
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
    sh: &mut Shell,
    vars: &mut Vec<Node>,
    args: &mut Vec<Node>,
) -> Result<bool, Error> {
    let use_args = declaration_context(args) && compound_candidate(sh, args.last());
    let use_vars = args.is_empty() && compound_candidate(sh, vars.last());
    let target = if use_args {
        args
    } else if use_vars {
        vars
    } else {
        return Ok(false);
    };

    let previous = target.pop().expect("a compound candidate exists");
    let mut assignment =
        compound_prefix(sh, previous).expect("the candidate predicate and conversion agree");
    let mut elements = Vec::new();
    loop {
        let token = readtoken_with_flags(sh, TokenContext::SKIP_NEWLINES)?;
        if token.kind == TokenKind::RightParen {
            break;
        }
        if token.kind != TokenKind::Word {
            return Err(synerror(sh, b"invalid compound array assignment"));
        }
        let arg = take_word(sh, token.quoted).arg;
        elements.push(array_element(arg));
    }
    assignment.value = BashArrayValue::Compound(elements);
    target.push(Node::Bash(BashNode::ArrayAssignment(assignment)));
    Ok(true)
}

// [spec:nsh:req:idiom.lexer-tokens]
pub(super) fn process_substitutions(
    sh: &mut Shell,
    st: &mut Rt1<'_>,
    enabled: c_int,
) -> Result<(), Error> {
    loop {
        if enabled == 0 || !active(sh) || !st.eofmark.is_none() {
            return Ok(());
        }
        let direction = match st.input.byte() {
            Some(b'<') => BashProcessDirection::Input,
            Some(b'>') => BashProcessDirection::Output,
            _ => return Ok(()),
        };
        if !pgetc_eatbnl(sh)?.is(b'(') {
            pungetc(sh);
            return Ok(());
        }

        st.out.push(CTLBACKQ as u8);
        let parked = mem::take(&mut st.out);
        let slot = st.bqlist.len();
        st.bqlist.push(None);
        let saved_heredocs = mem::take(&mut sh.input.heredoclist);
        let completed_at = sh.input.completed_heredocs.len();
        let mut body = list(sh, 2)?.into_node();
        if readtoken(sh, TokenContext::NONE)? != TokenKind::RightParen {
            return Err(synexpect(sh, Some(TokenKind::RightParen)));
        }
        setinputstring(sh, BStr::new(b""));
        parseheredoc(sh)?;
        finalize::node(sh, &mut body, completed_at)?;
        sh.input.heredoclist = saved_heredocs;
        popfile(sh);

        st.bqlist[slot] = Some(Node::Bash(BashNode::ProcessSubstitution(
            BashProcessSubstitution {
                direction,
                body: body.map(Box::new),
            },
        )));
        st.out = parked;
        st.input = super::pgetc_top(sh, st.syn())?;
    }
}

pub(super) fn parameter_subscript(
    sh: &mut Shell,
    st: &mut Rt1<'_>,
    badsub: bool,
    subtype: c_int,
) -> Result<(), Error> {
    if badsub || subtype != 0 || !st.input.is(b'[') || !active(sh) {
        return Ok(());
    }
    let mut depth = 0usize;
    let mut quote = Quote::None;
    let mut escaped = false;

    loop {
        let input = st.input;
        let Some(byte) = input.byte() else {
            return Err(synerror(sh, b"unterminated array subscript"));
        };
        if input.is(b'\n') {
            nlnoprompt(sh);
        }
        st.out.push(byte);

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
                        st.input = pgetc_eatbnl(sh)?;
                        return Ok(());
                    }
                }
                Quote::None => {}
            }
        }
        st.input = pgetc_eatbnl(sh)?;
    }
}

fn arithmetic_text(sh: &mut Shell) -> Result<NodeText, Error> {
    let mut out = BString::new(Vec::new());
    let mut depth = 0usize;
    let mut quote = Quote::None;
    let mut escaped = false;
    let mut pending = None;

    loop {
        let input = match pending.take() {
            Some(input) => input,
            None => pgetc(sh)?,
        };
        let Some(byte) = input.byte() else {
            return Err(synerror(sh, b"missing '))'"));
        };
        if input.is(b'\n') {
            nlnoprompt(sh);
        }

        if escaped {
            out.push(byte);
            escaped = false;
            continue;
        }
        match quote {
            Quote::Single => {
                out.push(byte);
                if input.is(b'\'') {
                    quote = Quote::None;
                }
            }
            Quote::Double => {
                out.push(byte);
                if input.is(b'\\') {
                    escaped = true;
                } else if input.is(b'"') {
                    quote = Quote::None;
                }
            }
            Quote::None if input.is(b'\\') => {
                out.push(byte);
                escaped = true;
            }
            Quote::None if input.is(b'\'') => {
                out.push(byte);
                quote = Quote::Single;
            }
            Quote::None if input.is(b'"') => {
                out.push(byte);
                quote = Quote::Double;
            }
            Quote::None if input.is(b'(') => {
                depth += 1;
                out.push(byte);
            }
            Quote::None if input.is(b')') && depth != 0 => {
                depth -= 1;
                out.push(byte);
            }
            Quote::None if input.is(b')') => {
                let next = pgetc(sh)?;
                if next.is(b')') {
                    break;
                }
                out.push(byte);
                pending = Some(next);
            }
            Quote::None => out.push(byte),
        }
    }
    Ok(node_text(&out))
}

fn for_clauses(sh: &mut Shell, text: &BStr) -> Result<[NodeText; 3], Error> {
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
        return Err(synerror(sh, b"arithmetic for requires three expressions"));
    }
    Ok([
        node_text(&text[..separators[0]]),
        node_text(&text[separators[0] + 1..separators[1]]),
        node_text(&text[separators[1] + 1..]),
    ])
}

fn conditional_or(sh: &mut Shell) -> Result<BashConditionalExpr, Error> {
    let mut left = conditional_and(sh)?;
    loop {
        let token = readtoken(sh, TokenContext::NONE)?;
        if token != TokenKind::OrIf {
            sh.input.tokpushback = true;
            return Ok(left);
        }
        left = BashConditionalExpr::Or(Box::new(left), Box::new(conditional_and(sh)?));
    }
}

fn conditional_and(sh: &mut Shell) -> Result<BashConditionalExpr, Error> {
    let mut left = conditional_primary(sh)?;
    loop {
        let token = readtoken(sh, TokenContext::NONE)?;
        if token != TokenKind::AndIf {
            sh.input.tokpushback = true;
            return Ok(left);
        }
        left = BashConditionalExpr::And(Box::new(left), Box::new(conditional_primary(sh)?));
    }
}

fn conditional_primary(sh: &mut Shell) -> Result<BashConditionalExpr, Error> {
    let token = readtoken_with_flags(sh, TokenContext::NONE)?;
    if token.kind == TokenKind::LeftParen {
        let expression = conditional_or(sh)?;
        if readtoken(sh, TokenContext::NONE)? != TokenKind::RightParen {
            return Err(synexpect(sh, Some(TokenKind::RightParen)));
        }
        return Ok(BashConditionalExpr::Group(Box::new(expression)));
    }
    if token.kind != TokenKind::Word || closes_conditional(sh, token.kind, token.quoted) {
        return Err(synerror(sh, b"expected conditional expression"));
    }

    let first = take_word(sh, token.quoted);
    if !first.quoted && first.arg.word.as_bstr() == BStr::new(b"!") {
        return Ok(BashConditionalExpr::Not(Box::new(conditional_primary(sh)?)));
    }
    if !first.quoted && unary_operator(first.arg.word.as_bstr()) {
        let operand_token = readtoken_with_flags(sh, TokenContext::NONE)?;
        if operand_token.kind != TokenKind::Word
            || closes_conditional(sh, operand_token.kind, operand_token.quoted)
        {
            return Err(synerror(sh, b"expected unary-test operand"));
        }
        return Ok(BashConditionalExpr::Unary {
            operator: node_text(first.arg.word.as_bstr()),
            operand: take_word(sh, operand_token.quoted).arg,
        });
    }

    let operator_token = readtoken_with_flags(sh, TokenContext::NONE)?;
    let operator = if operator_token.kind == super::TokenKind::Redirection {
        let redirection = sh.input.redirnode.take();
        match redirection.as_ref() {
            Some(super::PendingRedirection::File { operator, .. })
                if *operator == FileRedirectionOperator::Read =>
            {
                Some(node_text(b"<"))
            }
            Some(super::PendingRedirection::File { operator, .. })
                if *operator == FileRedirectionOperator::Write =>
            {
                Some(node_text(b">"))
            }
            _ => None,
        }
    } else if operator_token.kind == TokenKind::Word
        && !operator_token.quoted
        && binary_operator(wordtext(sh))
    {
        let operator = wordtext_node(sh);
        Some(operator)
    } else {
        None
    };
    let Some(operator) = operator else {
        sh.input.tokpushback = true;
        return Ok(BashConditionalExpr::Word(first.arg));
    };

    let right_token = readtoken_with_flags(sh, TokenContext::NONE)?;
    if right_token.kind != TokenKind::Word
        || closes_conditional(sh, right_token.kind, right_token.quoted)
    {
        return Err(synerror(sh, b"expected binary-test operand"));
    }
    Ok(BashConditionalExpr::Binary {
        left: first.arg,
        operator,
        right: take_word(sh, right_token.quoted).arg,
    })
}

fn closes_conditional(sh: &Shell, kind: TokenKind, quoted: bool) -> bool {
    kind == TokenKind::Word && !quoted && wordtext(sh) == BStr::new(b"]]")
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

fn take_word(sh: &mut Shell, quoted: bool) -> ConditionalWord {
    ConditionalWord {
        arg: WordNode {
            word: mem::take(&mut sh.input.word),
        },
        quoted,
    }
}

fn compound_candidate(sh: &Shell, node: Option<&Node>) -> bool {
    match node {
        Some(Node::Word(arg)) => plain_prefix(arg).is_some_and(|(name_end, _)| {
            goodname(&sh.locale, BStr::new(&arg.word.as_bstr()[..name_end])) != 0
        }),
        Some(Node::Bash(BashNode::ArrayAssignment(assignment))) => {
            matches!(&assignment.value, BashArrayValue::Word(value) if value.word.as_bstr().is_empty())
        }
        _ => false,
    }
}

fn compound_prefix(sh: &Shell, node: Node) -> Option<BashArrayAssignment> {
    match node {
        Node::Word(arg) => {
            let (name_end, operator) = plain_prefix(&arg)?;
            if goodname(&sh.locale, BStr::new(&arg.word.as_bstr()[..name_end])) == 0 {
                return None;
            }
            Some(BashArrayAssignment {
                name: node_text(&arg.word.as_bstr()[..name_end]),
                subscript: None,
                operator,
                value: BashArrayValue::Word(arg_part(
                    &arg,
                    arg.word.as_bstr().len(),
                    arg.word.as_bstr().len(),
                )),
            })
        }
        Node::Bash(BashNode::ArrayAssignment(assignment)) => Some(assignment),
        _ => None,
    }
}

fn plain_prefix(arg: &WordNode) -> Option<(usize, BashAssignmentOperator)> {
    let bytes = arg.word.as_bstr();
    let (name_end, operator) = if bytes.ends_with(b"+=") {
        (bytes.len() - 2, BashAssignmentOperator::Append)
    } else if bytes.ends_with(b"=") {
        (bytes.len() - 1, BashAssignmentOperator::Set)
    } else {
        return None;
    };
    if name_end == 0 {
        return None;
    }
    Some((name_end, operator))
}

fn array_element(arg: WordNode) -> BashArrayElement {
    let encoded = arg.word.encode_legacy();
    let bytes = BStr::new(&encoded.bytes[..encoded.bytes.len() - 1]);
    if bytes.first() == Some(&b'[') {
        if let Some(close) = matching_bracket(bytes, 0) {
            if let Some((operator, value_start)) = assignment_operator(bytes, close + 1) {
                return BashArrayElement {
                    subscript: Some(arg_part(&arg, 1, close)),
                    operator,
                    value: arg_part(&arg, value_start, bytes.len()),
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

fn matching_bracket(bytes: &[u8], open: usize) -> Option<usize> {
    let mut depth = 0usize;
    let mut index = open;
    let mut quoted = false;
    while index < bytes.len() {
        match bytes[index] {
            byte if byte == CTLESC as u8 => index += 2,
            byte if byte == CTLQUOTEMARK as u8 => {
                quoted = !quoted;
                index += 1;
            }
            byte if byte == CTLMBCHAR as u8 => {
                index = multibyte_end(bytes, index);
            }
            b'[' if !quoted => {
                depth += 1;
                index += 1;
            }
            b']' if !quoted => {
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

fn assignment_operator(bytes: &[u8], start: usize) -> Option<(BashAssignmentOperator, usize)> {
    if bytes.get(start..start + 2) == Some(b"+=") {
        Some((BashAssignmentOperator::Append, start + 2))
    } else if bytes.get(start) == Some(&b'=') {
        Some((BashAssignmentOperator::Set, start + 1))
    } else {
        None
    }
}

fn arg_part(arg: &WordNode, start: usize, end: usize) -> WordNode {
    // [spec:nsh:def:idiom.word-ir]
    let encoded = arg.word.encode_legacy();
    let bytes = &encoded.bytes[..encoded.bytes.len() - 1];
    let first = backquote_count(&bytes[..start]);
    let count = backquote_count(&bytes[start..end]);
    WordNode {
        word: ParsedWord::from_legacy_fragment(
            &bytes[start..end],
            encoded
                .substitutions
                .get(first..first + count)
                .unwrap_or(&[])
                .to_vec(),
        ),
    }
}

fn backquote_count(bytes: &[u8]) -> usize {
    let mut count = 0usize;
    let mut index = 0usize;
    while index < bytes.len() {
        if bytes[index] == CTLESC as u8 {
            index += 2;
        } else if bytes[index] == CTLMBCHAR as u8 {
            index = multibyte_end(bytes, index);
        } else {
            count += usize::from(bytes[index] == CTLBACKQ as u8);
            index += 1;
        }
    }
    count
}

fn multibyte_end(bytes: &[u8], start: usize) -> usize {
    let length_at = start + 1 + usize::from(bytes.get(start + 1) == Some(&(CTLESC as u8)));
    let length = bytes.get(length_at).copied().unwrap_or(0) as usize;
    length_at.saturating_add(length).saturating_add(3)
}

fn node_text(bytes: &[u8]) -> NodeText {
    let mut text = BString::from(bytes);
    text.push(0);
    NodeText::new(text)
}
