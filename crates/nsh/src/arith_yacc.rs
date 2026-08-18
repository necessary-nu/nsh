//! Arithmetic expansion as a typed Rust lexer and recursive-descent parser.
//!
//! Rules: `docs/spec/port/src/arith_yacc.md` and
//! `docs/spec/port/src/arith_yylex.md`.
//!
//! The original split a two-token window across four mutable globals and a
//! `yylex` function. None of that is part of arithmetic semantics. A lexer
//! now owns its byte offset, tokens carry their values, and a parser owns its
//! lookahead. One shell evaluation cannot overwrite another's state.

use bstr::{BStr, ByteSlice};

use crate::context::Shell;
use crate::error::Error;
use crate::var::{lookupvarint_bytes, setvarint_bytes};


// [spec:dash:def:arith-yacc.yystype]
/// The value carried by an arithmetic token.
///
/// This is an enum rather than the C `union yystype`: a number cannot be
/// mistaken for a variable-name pointer, and names borrow the input bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Token<'a> {
    End,
    Bad,
    Number(i64),
    Variable(&'a BStr),
    Assign(Option<BinOp>),
    LogicalOr,
    LogicalAnd,
    Not,
    BitNot,
    Binary(BinOp),
    LParen,
    RParen,
    Question,
    Colon,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BinOp {
    LessEqual,
    GreaterEqual,
    Less,
    Greater,
    Equal,
    Remainder,
    BitAnd,
    ShiftLeft,
    ShiftRight,
    Multiply,
    Add,
    BitOr,
    Subtract,
    BitXor,
    Divide,
    NotEqual,
}

impl BinOp {
    // [spec:dash:def:arith-yacc.arith-prec-fn]
    // [spec:dash:sem:arith-yacc.arith-prec-fn]
    const fn precedence(self) -> u8 {
        match self {
            Self::Multiply | Self::Divide | Self::Remainder => 0,
            Self::Add | Self::Subtract => 1,
            Self::ShiftLeft | Self::ShiftRight => 2,
            Self::LessEqual | Self::GreaterEqual | Self::Less | Self::Greater => 3,
            Self::Equal | Self::NotEqual => 4,
            Self::BitAnd => 5,
            Self::BitXor => 6,
            Self::BitOr => 7,
        }
    }

    // [spec:dash:def:arith-yacc.higher-prec-fn]
    // [spec:dash:sem:arith-yacc.higher-prec-fn]
    const fn binding_power(self) -> u8 {
        8 - self.precedence()
    }
}

struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
    locale: nsh_platform::Locale,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a BStr, locale: nsh_platform::Locale) -> Self {
        Self {
            input: input.as_ref(),
            pos: 0,
            locale,
        }
    }

    fn peek(&self, offset: usize) -> Option<u8> {
        self.input.get(self.pos + offset).copied()
    }

    fn take_if(&mut self, byte: u8) -> bool {
        if self.peek(0) == Some(byte) {
            self.pos += 1;
            true
        } else {
            false
        }
    }

    // [spec:dash:def:arith-yacc.yylex-fn]
    // [spec:dash:sem:arith-yacc.yylex-fn]
    // [spec:dash:def:arith-yylex.yylex-fn]
    // [spec:dash:sem:arith-yylex.yylex-fn]
    // [spec:dash:def:expand.yylex-fn]
    // [spec:dash:sem:expand.yylex-fn]
    fn next(&mut self) -> Token<'a> {
        while matches!(self.peek(0), Some(b' ' | b'\t' | b'\n')) {
            self.pos += 1;
        }

        let Some(byte) = self.peek(0) else {
            return Token::End;
        };

        if byte.is_ascii_digit() {
            return self.number();
        }
        /* The C switch lists ASCII letters and `_` explicitly for the first
         * byte, then uses locale-sensitive `is_in_name` only for the tail. */
        if byte.is_ascii_alphabetic() || byte == b'_' {
            let start = self.pos;
            self.pos += 1;
            while self
                .peek(0)
                .is_some_and(|b| self.locale.is_alphanumeric(b) || b == b'_')
            {
                self.pos += 1;
            }
            return Token::Variable(self.input[start..self.pos].as_bstr());
        }

        self.pos += 1;
        match byte {
            b'=' => {
                if self.take_if(b'=') {
                    Token::Binary(BinOp::Equal)
                } else {
                    Token::Assign(None)
                }
            }
            b'>' => {
                if self.take_if(b'=') {
                    Token::Binary(BinOp::GreaterEqual)
                } else if self.take_if(b'>') {
                    if self.take_if(b'=') {
                        Token::Assign(Some(BinOp::ShiftRight))
                    } else {
                        Token::Binary(BinOp::ShiftRight)
                    }
                } else {
                    Token::Binary(BinOp::Greater)
                }
            }
            b'<' => {
                if self.take_if(b'=') {
                    Token::Binary(BinOp::LessEqual)
                } else if self.take_if(b'<') {
                    if self.take_if(b'=') {
                        Token::Assign(Some(BinOp::ShiftLeft))
                    } else {
                        Token::Binary(BinOp::ShiftLeft)
                    }
                } else {
                    Token::Binary(BinOp::Less)
                }
            }
            b'|' => {
                if self.take_if(b'|') {
                    Token::LogicalOr
                } else if self.take_if(b'=') {
                    Token::Assign(Some(BinOp::BitOr))
                } else {
                    Token::Binary(BinOp::BitOr)
                }
            }
            b'&' => {
                if self.take_if(b'&') {
                    Token::LogicalAnd
                } else if self.take_if(b'=') {
                    Token::Assign(Some(BinOp::BitAnd))
                } else {
                    Token::Binary(BinOp::BitAnd)
                }
            }
            b'!' => {
                if self.take_if(b'=') {
                    Token::Binary(BinOp::NotEqual)
                } else {
                    Token::Not
                }
            }
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'~' => Token::BitNot,
            b'?' => Token::Question,
            b':' => Token::Colon,
            b'*' => self.binary_or_assign(BinOp::Multiply),
            b'/' => self.binary_or_assign(BinOp::Divide),
            b'%' => self.binary_or_assign(BinOp::Remainder),
            b'+' => self.binary_or_assign(BinOp::Add),
            b'-' => self.binary_or_assign(BinOp::Subtract),
            b'^' => self.binary_or_assign(BinOp::BitXor),
            _ => Token::Bad,
        }
    }

    fn binary_or_assign(&mut self, op: BinOp) -> Token<'a> {
        if self.take_if(b'=') {
            Token::Assign(Some(op))
        } else {
            Token::Binary(op)
        }
    }

    fn number(&mut self) -> Token<'a> {
        let start = self.pos;
        let (base, digits) = if self.peek(0) == Some(b'0') {
            match (self.peek(1), self.peek(2).and_then(digit_value)) {
                (Some(b'x' | b'X'), Some(d)) if d < 16 => (16, start + 2),
                (Some(b'b' | b'B'), Some(d)) if d < 2 => (2, start + 2),
                _ => (8, start),
            }
        } else {
            (10, start)
        };

        self.pos = digits;
        let mut value = 0i64;
        while let Some(digit) = self.peek(0).and_then(digit_value) {
            if digit >= base {
                break;
            }
            value = value
                .saturating_mul(base as i64)
                .saturating_add(digit as i64);
            self.pos += 1;
        }
        Token::Number(value)
    }
}

fn digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u32),
        b'a'..=b'z' => Some((byte - b'a' + 10) as u32),
        b'A'..=b'Z' => Some((byte - b'A' + 10) as u32),
        _ => None,
    }
}

struct Parser<'a, 'sh> {
    sh: &'sh mut Shell,
    input: &'a BStr,
    tokens: Vec<Token<'a>>,
    pos: usize,
}

impl<'a, 'sh> Parser<'a, 'sh> {
    // The C header's `arith_lex_reset()` was an empty macro in this build.
    // Local lexer state makes reset synonymous with constructing a parser.
    // [spec:dash:def:expand.arith-lex-reset-fn]
    // [spec:dash:sem:expand.arith-lex-reset-fn]
    fn new(sh: &'sh mut Shell, input: &'a BStr) -> Self {
        let mut lexer = Lexer::new(input, sh.locale.clone());
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next();
            tokens.push(token);
            if token == Token::End {
                break;
            }
        }
        Self {
            sh,
            input,
            tokens,
            pos: 0,
        }
    }

    fn current(&self) -> Token<'a> {
        self.tokens[self.pos]
    }

    fn peek(&self, offset: usize) -> Token<'a> {
        self.tokens
            .get(self.pos + offset)
            .copied()
            .unwrap_or(Token::End)
    }

    fn advance(&mut self) -> Token<'a> {
        let token = self.current();
        if token != Token::End {
            self.pos += 1;
        }
        token
    }

    // [spec:dash:def:arith-yacc.yyerror-fn]
    // [spec:dash:sem:arith-yacc.yyerror-fn]
    fn error(&mut self, message: &[u8]) -> Error {
        let mut text = b"arithmetic expression: ".to_vec();
        text.extend_from_slice(message);
        text.extend_from_slice(b": \"");
        text.extend_from_slice(self.input.as_ref());
        text.push(b'"');
        self.sh.sh_error_value(&text)
    }

    // [spec:dash:def:arith-yacc.assignment-fn]
    // [spec:dash:sem:arith-yacc.assignment-fn]
    fn assignment(&mut self, evaluate: bool) -> Result<i64, Error> {
        if let (Token::Variable(name), Token::Assign(op)) = (self.current(), self.peek(1)) {
            self.pos += 2;
            let result = self.assignment(evaluate)?;
            if !evaluate {
                return Ok(result);
            }
            let value = if let Some(op) = op {
                let current = lookupvarint_bytes(self.sh, name)?;
                self.apply(op, current, result)?
            } else {
                result
            };
            return setvarint_bytes(self.sh, name, value, 0);
        }
        self.conditional(evaluate)
    }

    // [spec:dash:def:arith-yacc.cond-fn]
    // [spec:dash:sem:arith-yacc.cond-fn]
    fn conditional(&mut self, evaluate: bool) -> Result<i64, Error> {
        let condition = self.logical_or(evaluate)?;
        if self.current() != Token::Question {
            return Ok(condition);
        }
        self.advance();
        let then_value = self.assignment(evaluate && condition != 0)?;
        if self.current() != Token::Colon {
            return Err(self.error(b"expecting ':'"));
        }
        self.advance();
        let else_value = self.conditional(evaluate && condition == 0)?;
        Ok(if condition != 0 {
            then_value
        } else {
            else_value
        })
    }

    // [spec:dash:def:arith-yacc.or-fn]
    // [spec:dash:sem:arith-yacc.or-fn]
    fn logical_or(&mut self, evaluate: bool) -> Result<i64, Error> {
        let left = self.logical_and(evaluate)?;
        if self.current() != Token::LogicalOr {
            return Ok(left);
        }
        self.advance();
        let right = self.logical_or(evaluate && left == 0)?;
        Ok((left != 0 || right != 0) as i64)
    }

    // [spec:dash:def:arith-yacc.and-fn]
    // [spec:dash:sem:arith-yacc.and-fn]
    fn logical_and(&mut self, evaluate: bool) -> Result<i64, Error> {
        let left = self.binary(evaluate)?;
        if self.current() != Token::LogicalAnd {
            return Ok(left);
        }
        self.advance();
        let right = self.logical_and(evaluate && left != 0)?;
        Ok((left != 0 && right != 0) as i64)
    }

    // [spec:dash:def:arith-yacc.binop-fn]
    // [spec:dash:sem:arith-yacc.binop-fn]
    fn binary(&mut self, evaluate: bool) -> Result<i64, Error> {
        let left = self.primary(evaluate)?;
        self.binary_rhs(left, 1, evaluate)
    }

    // [spec:dash:def:arith-yacc.binop2-fn]
    // [spec:dash:sem:arith-yacc.binop2-fn]
    fn binary_rhs(
        &mut self,
        mut left: i64,
        min_power: u8,
        evaluate: bool,
    ) -> Result<i64, Error> {
        loop {
            let Token::Binary(op) = self.current() else {
                return Ok(left);
            };
            let power = op.binding_power();
            if power < min_power {
                return Ok(left);
            }
            self.advance();
            let mut right = self.primary(evaluate)?;
            if let Token::Binary(next) = self.current() {
                if next.binding_power() > power {
                    right = self.binary_rhs(right, power + 1, evaluate)?;
                }
            }
            left = if evaluate {
                self.apply(op, left, right)?
            } else {
                right
            };
        }
    }

    // [spec:dash:def:arith-yacc.primary-fn]
    // [spec:dash:sem:arith-yacc.primary-fn]
    fn primary(&mut self, evaluate: bool) -> Result<i64, Error> {
        match self.advance() {
            Token::Number(value) => Ok(value),
            Token::Variable(name) => {
                if evaluate {
                    lookupvarint_bytes(self.sh, name)
                } else {
                    Ok(0)
                }
            }
            Token::LParen => {
                let value = self.assignment(evaluate)?;
                if self.current() != Token::RParen {
                    return Err(self.error(b"expecting ')'"));
                }
                self.advance();
                Ok(value)
            }
            Token::Binary(BinOp::Add) => self.primary(evaluate),
            Token::Binary(BinOp::Subtract) => {
                Ok(self.primary(evaluate)?.wrapping_neg())
            }
            Token::Not => Ok((self.primary(evaluate)? == 0) as i64),
            Token::BitNot => Ok(!self.primary(evaluate)?),
            _ => Err(self.error(b"expecting primary")),
        }
    }

    // [spec:dash:def:arith-yacc.do-binop-fn]
    // [spec:dash:sem:arith-yacc.do-binop-fn]
    fn apply(
        &mut self,
        op: BinOp,
        left: i64,
        right: i64,
    ) -> Result<i64, Error> {
        Ok(match op {
            BinOp::Multiply => left.wrapping_mul(right),
            BinOp::Add => left.wrapping_add(right),
            BinOp::Subtract => left.wrapping_sub(right),
            BinOp::ShiftLeft => left.wrapping_shl(right as u32),
            BinOp::ShiftRight => left.wrapping_shr(right as u32),
            BinOp::Less => (left < right) as i64,
            BinOp::LessEqual => (left <= right) as i64,
            BinOp::Greater => (left > right) as i64,
            BinOp::GreaterEqual => (left >= right) as i64,
            BinOp::Equal => (left == right) as i64,
            BinOp::NotEqual => (left != right) as i64,
            BinOp::BitAnd => left & right,
            BinOp::BitXor => left ^ right,
            BinOp::BitOr => left | right,
            BinOp::Remainder | BinOp::Divide => {
                if right == 0 || (left == i64::MIN && right == -1) {
                    return Err(self.error(b"division error"));
                }
                if op == BinOp::Remainder {
                    left % right
                } else {
                    left / right
                }
            }
        })
    }
}

// [spec:dash:def:arith-yacc.arith-fn]
// [spec:dash:sem:arith-yacc.arith-fn]
// [spec:dash:def:expand.arith-fn]
// [spec:dash:sem:expand.arith-fn]
// [spec:posix:req:expand.arith-evaluation]
// [spec:posix:req:expand.arith-variable-changes]
// [spec:posix:req:expand.arith-variable-reference]
// [spec:posix:req:expand.arith-extensions]
// [spec:posix:req:expand.arith-invalid-expression]
// [spec:posix:req:xcurel.iso-c-concepts]
// [spec:posix:req:xcurel.arithmetic-precision]
// [spec:posix:req:xcurel.arithmetic-variable-initialization]
// [spec:posix:req:xcurel.arithmetic-operators]
// [spec:posix:req:xcurel.arithmetic-expression-evaluation]
pub fn arith(sh: &mut Shell, input: &BStr) -> Result<i64, Error> {
    let mut parser = Parser::new(sh, input);
    let result = parser.assignment(true)?;
    if parser.current() != Token::End {
        return Err(parser.error(b"expecting EOF"));
    }
    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn shell() -> Shell {
        Shell::new(crate::streams::Streams::INHERIT)
    }

    #[test]
    fn a_failed_evaluation_returns_its_diagnostic() {
        let mut sh = shell();
        let e = arith(&mut sh, BStr::new(b"1/0")).expect_err("1/0 must fail");
        assert_eq!(
            e.message().to_vec(),
            b"arithmetic expression: division error: \"1/0\"".to_vec()
        );
        assert_eq!(e.status(), 2);
    }

    #[test]
    fn a_trailing_token_returns_its_diagnostic() {
        let mut sh = shell();
        let e = arith(&mut sh, BStr::new(b"1 2")).expect_err("`1 2` must fail");
        assert_eq!(
            e.message().to_vec(),
            b"arithmetic expression: expecting EOF: \"1 2\"".to_vec()
        );
    }

    #[test]
    fn a_good_expression_still_evaluates() {
        let mut sh = shell();
        assert_eq!(arith(&mut sh, BStr::new(b"6*7")).unwrap(), 42);
    }

    #[test]
    fn base_prefixes_and_overflow_match_intmax() {
        let mut sh = shell();
        assert_eq!(arith(&mut sh, BStr::new(b"0b11 + 010 + 0x10")).unwrap(), 27);
        assert_eq!(
            arith(&mut sh, BStr::new(b"9223372036854775808")).unwrap(),
            i64::MAX
        );
    }

    #[test]
    fn short_circuit_skips_effects_but_parses_both_sides() {
        let mut sh = shell();
        assert_eq!(arith(&mut sh, BStr::new(b"0 && 1 / 0")).unwrap(), 0);
        assert_eq!(arith(&mut sh, BStr::new(b"1 || 1 / 0")).unwrap(), 1);
        assert_eq!(arith(&mut sh, BStr::new(b"1 ? 7 : 1 / 0")).unwrap(), 7);
    }

    // [spec:posix:req:builtin.set.opt-u-nounset/test]
    #[test]
    fn nounset_rejects_evaluated_arithmetic_reads() {
        let mut sh = shell();
        sh.options.set_flag(crate::options::uflag, 1);

        let error = arith(&mut sh, BStr::new(b"undefined_name + 1"))
            .expect_err("an evaluated unset variable must fail under nounset");
        assert_eq!(
            error.message().to_vec(),
            b"undefined_name: parameter not set".to_vec()
        );
        assert_eq!(error.status(), 2);

        assert_eq!(arith(&mut sh, BStr::new(b"assigned_name = 7")).unwrap(), 7);
        assert_eq!(arith(&mut sh, BStr::new(b"0 && skipped_name")).unwrap(), 0);
    }
}
