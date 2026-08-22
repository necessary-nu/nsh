//! Arithmetic expansion as a typed Rust lexer and recursive-descent parser.
//!
//! Rules: `docs/spec/port/src/arith_yacc.md` and
//! `docs/spec/port/src/arith_yylex.md`.
//!
//! The original split a two-token window across four mutable globals and a
//! `yylex` function. None of that is part of arithmetic semantics. A lexer
//! now owns its byte offset, tokens carry their values, and a parser owns its
//! lookahead. One shell evaluation cannot overwrite another's state.

use bstr::{BStr, BString, ByteSlice};

use crate::context::Shell;
use crate::error::Error;
use crate::options::{Dialect, ShellOption};
use crate::variables::arrays::{self, ArraySelector};
use crate::variables::{
    CallbackPolicy, VariableAttributes, lookup_bytes, lookup_integer_bytes, set_integer_bytes,
};

/// How deep one variable's value may be re-read as an expression.
///
/// Bash evaluates a name's *value* as an expression, so `a=b; b=a` is a
/// cycle rather than a number. The limit turns that into a diagnostic.
const MAX_NAME_DEPTH: u32 = 32;

/// The value carried by an arithmetic token.
///
/// This is an enum rather than the C `union yystype`: a number cannot be
/// mistaken for a variable-name pointer, and names borrow the input bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Token<'a> {
    End,
    Bad,
    Number(i64),
    Variable(Name<'a>),
    Assign(Option<BinaryOperator>),
    LogicalOr,
    LogicalAnd,
    Not,
    BitNot,
    Binary(BinaryOperator),
    Power,
    Increment,
    Decrement,
    Comma,
    LParen,
    RParen,
    Question,
    Colon,
}

/// A name an expression can read or assign.
///
/// The subscript travels with the base rather than as separate bracket
/// tokens, because `a[i]` is one lvalue: the expression `a[i]++` has to
/// read and write the same element, and a parser that had already split
/// the brackets apart could not say so.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct Name<'a> {
    base: &'a BStr,
    subscript: Option<&'a BStr>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum BinaryOperator {
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
    Power,
}

impl BinaryOperator {
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
            // `**` is parsed by its own right-associative level and never
            // reaches the precedence climb; the arm exists for `**=`.
            Self::Power => 0,
        }
    }

    // [spec:dash:sem:arith-yacc.higher-prec-fn]
    const fn binding_power(self) -> u8 {
        8 - self.precedence()
    }
}

struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
    locale: nsh_platform::Locale,
    bash: bool,
}

impl<'a> Lexer<'a> {
    fn new(input: &'a BStr, locale: nsh_platform::Locale, bash: bool) -> Self {
        Self {
            input: input.as_ref(),
            pos: 0,
            locale,
            bash,
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

    // [spec:dash:sem:arith-yacc.yylex-fn]
    // [spec:dash:sem:arith-yylex.yylex-fn]
    // [spec:dash:sem:expand.yylex-fn]
    fn next(&mut self) -> Token<'a> {
        loop {
            match self.peek(0) {
                Some(b' ' | b'\t' | b'\n') => self.pos += 1,
                // Bash removes quoting before it evaluates, so `a['k']`
                // and `i = '3'` name the same things as their bare forms.
                Some(b'\'' | b'"') if self.bash => self.pos += 1,
                _ => break,
            }
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
            let base = self.input[start..self.pos].as_bstr();
            let subscript = if self.bash { self.subscript() } else { None };
            return Token::Variable(Name { base, subscript });
        }

        self.pos += 1;
        match byte {
            b'=' => {
                if self.take_if(b'=') {
                    Token::Binary(BinaryOperator::Equal)
                } else {
                    Token::Assign(None)
                }
            }
            b'>' => {
                if self.take_if(b'=') {
                    Token::Binary(BinaryOperator::GreaterEqual)
                } else if self.take_if(b'>') {
                    if self.take_if(b'=') {
                        Token::Assign(Some(BinaryOperator::ShiftRight))
                    } else {
                        Token::Binary(BinaryOperator::ShiftRight)
                    }
                } else {
                    Token::Binary(BinaryOperator::Greater)
                }
            }
            b'<' => {
                if self.take_if(b'=') {
                    Token::Binary(BinaryOperator::LessEqual)
                } else if self.take_if(b'<') {
                    if self.take_if(b'=') {
                        Token::Assign(Some(BinaryOperator::ShiftLeft))
                    } else {
                        Token::Binary(BinaryOperator::ShiftLeft)
                    }
                } else {
                    Token::Binary(BinaryOperator::Less)
                }
            }
            b'|' => {
                if self.take_if(b'|') {
                    Token::LogicalOr
                } else if self.take_if(b'=') {
                    Token::Assign(Some(BinaryOperator::BitOr))
                } else {
                    Token::Binary(BinaryOperator::BitOr)
                }
            }
            b'&' => {
                if self.take_if(b'&') {
                    Token::LogicalAnd
                } else if self.take_if(b'=') {
                    Token::Assign(Some(BinaryOperator::BitAnd))
                } else {
                    Token::Binary(BinaryOperator::BitAnd)
                }
            }
            b'!' => {
                if self.take_if(b'=') {
                    Token::Binary(BinaryOperator::NotEqual)
                } else {
                    Token::Not
                }
            }
            b',' if self.bash => Token::Comma,
            b'(' => Token::LParen,
            b')' => Token::RParen,
            b'~' => Token::BitNot,
            b'?' => Token::Question,
            b':' => Token::Colon,
            b'*' if self.bash && self.take_if(b'*') => {
                if self.take_if(b'=') {
                    Token::Assign(Some(BinaryOperator::Power))
                } else {
                    Token::Power
                }
            }
            b'*' => self.binary_or_assign(BinaryOperator::Multiply),
            b'/' => self.binary_or_assign(BinaryOperator::Divide),
            b'%' => self.binary_or_assign(BinaryOperator::Remainder),
            b'+' if self.bash && self.take_if(b'+') => Token::Increment,
            b'-' if self.bash && self.take_if(b'-') => Token::Decrement,
            b'+' => self.binary_or_assign(BinaryOperator::Add),
            b'-' => self.binary_or_assign(BinaryOperator::Subtract),
            b'^' => self.binary_or_assign(BinaryOperator::BitXor),
            _ => Token::Bad,
        }
    }

    fn binary_or_assign(&mut self, op: BinaryOperator) -> Token<'a> {
        if self.take_if(b'=') {
            Token::Assign(Some(op))
        } else {
            Token::Binary(op)
        }
    }

    /// Capture the bracketed subscript that follows a name, if any.
    fn subscript(&mut self) -> Option<&'a BStr> {
        if self.peek(0) != Some(b'[') {
            return None;
        }
        let start = self.pos + 1;
        let mut depth = 1usize;
        let mut at = start;
        while let Some(byte) = self.input.get(at).copied() {
            match byte {
                b'[' => depth += 1,
                b']' => {
                    depth -= 1;
                    if depth == 0 {
                        self.pos = at + 1;
                        return Some(self.input[start..at].as_bstr());
                    }
                }
                _ => {}
            }
            at += 1;
        }
        None
    }

    /// Read `base#digits`, Bash's explicit-radix literal.
    fn radix_literal(&mut self, base: i64) -> Token<'a> {
        if !(2..=64).contains(&base) {
            return Token::Bad;
        }
        self.pos += 1;
        let start = self.pos;
        let mut value = 0i64;
        while let Some(digit) = self.peek(0).and_then(radix_digit) {
            if i64::from(digit) >= base {
                break;
            }
            value = value.saturating_mul(base).saturating_add(i64::from(digit));
            self.pos += 1;
        }
        if self.pos == start {
            return Token::Bad;
        }
        Token::Number(value)
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
        if self.bash && self.peek(0) == Some(b'#') && base == 10 {
            return self.radix_literal(value);
        }
        Token::Number(value)
    }
}

/// Digit values for `base#digits`, whose alphabet runs past base 36.
fn radix_digit(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some(u32::from(byte - b'0')),
        b'a'..=b'z' => Some(u32::from(byte - b'a') + 10),
        b'A'..=b'Z' => Some(u32::from(byte - b'A') + 36),
        b'@' => Some(62),
        b'_' => Some(63),
        _ => None,
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

struct Parser<'a, 'shell> {
    shell: &'shell mut Shell,
    input: &'a BStr,
    tokens: Vec<Token<'a>>,
    pos: usize,
    bash: bool,
    depth: u32,
}

impl<'a, 'shell> Parser<'a, 'shell> {
    // The C header's `arith_lex_reset()` was an empty macro in this build.
    // Local lexer state makes reset synonymous with constructing a parser.
    // [spec:dash:sem:expand.arith-lex-reset-fn]
    fn new(shell: &'shell mut Shell, input: &'a BStr, depth: u32) -> Self {
        let bash = shell.options.dialect() == Dialect::Bash;
        let mut lexer = Lexer::new(input, shell.locale.clone(), bash);
        let mut tokens = Vec::new();
        loop {
            let token = lexer.next();
            tokens.push(token);
            if token == Token::End {
                break;
            }
        }
        Self {
            shell,
            input,
            tokens,
            pos: 0,
            bash,
            depth,
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

    // [spec:dash:sem:arith-yacc.yyerror-fn]
    fn error(&mut self, message: &[u8]) -> Error {
        let mut text = b"arithmetic expression: ".to_vec();
        text.extend_from_slice(message);
        text.extend_from_slice(b": \"");
        text.extend_from_slice(self.input.as_ref());
        text.push(b'"');
        self.shell.diagnostics().shell_error(&text)
    }

    /// Bash's comma operator: evaluate each side, answer with the right.
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    fn comma(&mut self, evaluate: bool) -> Result<i64, Error> {
        let mut value = self.assignment(evaluate)?;
        while self.current() == Token::Comma {
            self.advance();
            value = self.assignment(evaluate)?;
        }
        Ok(value)
    }

    // [spec:dash:sem:arith-yacc.assignment-fn]
    fn assignment(&mut self, evaluate: bool) -> Result<i64, Error> {
        if let (Token::Variable(name), Token::Assign(op)) = (self.current(), self.peek(1)) {
            self.pos += 2;
            let result = self.assignment(evaluate)?;
            if !evaluate {
                return Ok(result);
            }
            let value = if let Some(op) = op {
                let current = self.read(name)?;
                self.apply(op, current, result)?
            } else {
                result
            };
            return self.write(name, value);
        }
        self.conditional(evaluate)
    }

    /// Read one name, or one array element.
    fn read(&mut self, name: Name<'a>) -> Result<i64, Error> {
        let Some(subscript) = name.subscript else {
            return self.scalar(name.base);
        };
        let subscript = unquote(subscript);
        let selector = arrays::resolve_selector(self.shell, name.base, subscript.as_bstr())?;
        let text = element_text(self.shell, name.base, &selector);
        self.value_of(text)
    }

    /// Read one plain name. Bash evaluates the stored text as an
    /// expression, which is what makes `x=1+2; $((x))` three.
    fn scalar(&mut self, name: &BStr) -> Result<i64, Error> {
        if !self.bash {
            return lookup_integer_bytes(self.shell, name);
        }
        match lookup_bytes(self.shell, name) {
            Some(value) => self.value_of(value),
            None if self.shell.options.enabled(ShellOption::Nounset) => {
                let mut message = name.to_vec();
                message.extend_from_slice(b": parameter not set");
                Err(self.shell.diagnostics().shell_error(&message))
            }
            None => Ok(0),
        }
    }

    fn value_of(&mut self, text: BString) -> Result<i64, Error> {
        if text.iter().all(u8::is_ascii_whitespace) {
            return Ok(0);
        }
        if self.depth >= MAX_NAME_DEPTH {
            return Err(self.error(b"expression recursion level exceeded"));
        }
        evaluate_at_depth(self.shell, text.as_bstr(), self.depth + 1)
    }

    /// Assign one name, or one array element.
    fn write(&mut self, name: Name<'a>, value: i64) -> Result<i64, Error> {
        if name.subscript.is_none() {
            return set_integer_bytes(
                self.shell,
                name.base,
                value,
                VariableAttributes::NONE,
                CallbackPolicy::Run,
            );
        }
        let subscript = unquote(name.subscript.unwrap_or_default());
        let selector = arrays::resolve_selector(self.shell, name.base, subscript.as_bstr())?;
        let text = value.to_string();
        arrays::assign_element(
            self.shell,
            name.base,
            &selector,
            BStr::new(text.as_bytes()),
            false,
        )?;
        Ok(value)
    }

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

    // [spec:dash:sem:arith-yacc.binop-fn]
    fn binary(&mut self, evaluate: bool) -> Result<i64, Error> {
        let left = self.power(evaluate)?;
        self.binary_rhs(left, 1, evaluate)
    }

    /// `**`, which is right-associative and binds tighter than a sign.
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    fn power(&mut self, evaluate: bool) -> Result<i64, Error> {
        let base = self.primary(evaluate)?;
        if self.current() != Token::Power {
            return Ok(base);
        }
        self.advance();
        let exponent = self.power(evaluate)?;
        if !evaluate {
            return Ok(exponent);
        }
        self.apply(BinaryOperator::Power, base, exponent)
    }

    // [spec:dash:sem:arith-yacc.binop2-fn]
    fn binary_rhs(&mut self, mut left: i64, min_power: u8, evaluate: bool) -> Result<i64, Error> {
        loop {
            let Token::Binary(op) = self.current() else {
                return Ok(left);
            };
            let power = op.binding_power();
            if power < min_power {
                return Ok(left);
            }
            self.advance();
            let mut right = self.power(evaluate)?;
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

    // [spec:dash:sem:arith-yacc.primary-fn]
    fn primary(&mut self, evaluate: bool) -> Result<i64, Error> {
        match self.advance() {
            Token::Number(value) => Ok(value),
            Token::Variable(name) => self.name_primary(name, evaluate),
            Token::Increment => self.step(1, evaluate),
            Token::Decrement => self.step(-1, evaluate),
            Token::LParen => {
                let value = self.comma(evaluate)?;
                if self.current() != Token::RParen {
                    return Err(self.error(b"expecting ')'"));
                }
                self.advance();
                Ok(value)
            }
            Token::Binary(BinaryOperator::Add) => self.power(evaluate),
            Token::Binary(BinaryOperator::Subtract) => Ok(self.power(evaluate)?.wrapping_neg()),
            Token::Not => Ok((self.power(evaluate)? == 0) as i64),
            Token::BitNot => Ok(!self.power(evaluate)?),
            _ => Err(self.error(b"expecting primary")),
        }
    }

    /// A name, with the postfix `++`/`--` that may follow it.
    fn name_primary(&mut self, name: Name<'a>, evaluate: bool) -> Result<i64, Error> {
        let value = if evaluate { self.read(name)? } else { 0 };
        let delta = match self.current() {
            Token::Increment => 1,
            Token::Decrement => -1,
            _ => return Ok(value),
        };
        self.advance();
        if evaluate {
            self.write(name, value.wrapping_add(delta))?;
        }
        Ok(value)
    }

    /// Prefix `++`/`--`, which answer with the value they stored.
    fn step(&mut self, delta: i64, evaluate: bool) -> Result<i64, Error> {
        let Token::Variable(name) = self.advance() else {
            return Err(self.error(b"expecting a name to increment"));
        };
        if !evaluate {
            return Ok(0);
        }
        let value = self.read(name)?.wrapping_add(delta);
        self.write(name, value)
    }

    // [spec:dash:sem:arith-yacc.do-binop-fn]
    // [spec:nsh:sem:idiom.specified-defects+1]
    fn apply(&mut self, op: BinaryOperator, left: i64, right: i64) -> Result<i64, Error> {
        Ok(match op {
            BinaryOperator::Multiply => left.wrapping_mul(right),
            BinaryOperator::Add => left.wrapping_add(right),
            BinaryOperator::Subtract => left.wrapping_sub(right),
            BinaryOperator::ShiftLeft => left.wrapping_shl(right as u32),
            BinaryOperator::ShiftRight => left.wrapping_shr(right as u32),
            BinaryOperator::Less => (left < right) as i64,
            BinaryOperator::LessEqual => (left <= right) as i64,
            BinaryOperator::Greater => (left > right) as i64,
            BinaryOperator::GreaterEqual => (left >= right) as i64,
            BinaryOperator::Equal => (left == right) as i64,
            BinaryOperator::NotEqual => (left != right) as i64,
            BinaryOperator::BitAnd => left & right,
            BinaryOperator::BitXor => left ^ right,
            BinaryOperator::BitOr => left | right,
            BinaryOperator::Power => {
                if right < 0 {
                    return Err(self.error(b"exponent less than 0"));
                }
                left.saturating_pow(right.min(i64::from(u32::MAX)) as u32)
            }
            BinaryOperator::Remainder | BinaryOperator::Divide => {
                if right == 0 || (left == i64::MIN && right == -1) {
                    return Err(self.error(b"division error"));
                }
                if op == BinaryOperator::Remainder {
                    left % right
                } else {
                    left / right
                }
            }
        })
    }
}

// [spec:dash:sem:arith-yacc.arith-fn]
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
pub fn evaluate(shell: &mut Shell, input: &BStr) -> Result<i64, Error> {
    evaluate_at_depth(shell, input, 0)
}

fn evaluate_at_depth(shell: &mut Shell, input: &BStr, depth: u32) -> Result<i64, Error> {
    let mut parser = Parser::new(shell, input, depth);
    // Bash's empty expression is zero rather than a diagnostic, which is
    // what makes `(( ))` false and `$(( ))` print nothing but a zero.
    if parser.bash && parser.current() == Token::End {
        return Ok(0);
    }
    let result = parser.comma(true)?;
    if parser.current() != Token::End {
        return Err(parser.error(b"expecting EOF"));
    }
    Ok(result)
}

/// Remove the quoting Bash removes before it reads a subscript, so that
/// `A['k']`, `A["k"]` and `A[k]` all name the key `k`.
fn unquote(subscript: &BStr) -> BString {
    let mut output = BString::default();
    let mut escaped = false;
    for &byte in subscript.iter() {
        if escaped {
            output.push(byte);
            escaped = false;
        } else if byte == b'\\' {
            escaped = true;
        } else if byte != b'\'' && byte != b'"' {
            output.push(byte);
        }
    }
    output
}

/// The stored text of one array element, empty when it is unset.
fn element_text(shell: &mut Shell, base: &BStr, selector: &ArraySelector) -> BString {
    let Some(stored) = crate::variables::value::variable_value(shell, base).cloned() else {
        return BString::default();
    };
    match selector {
        ArraySelector::Index(index) => stored
            .indexed(*index)
            .map(BStr::to_owned)
            .or_else(|| (*index == 0).then(|| stored.scalar_owned()).flatten())
            .unwrap_or_default(),
        ArraySelector::Key(key) => stored
            .associative(BStr::new(key.as_slice()))
            .map(BStr::to_owned)
            .unwrap_or_default(),
        ArraySelector::All | ArraySelector::Joined => arrays::elements(&stored)
            .first()
            .cloned()
            .unwrap_or_default(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::options::ShellOption;

    fn shell() -> Shell {
        Shell::new(crate::streams::Streams::INHERIT)
    }

    #[test]
    fn a_failed_evaluation_returns_its_diagnostic() {
        let mut shell = shell();
        let error = evaluate(&mut shell, BStr::new(b"1/0")).expect_err("1/0 must fail");
        assert_eq!(
            error.message().to_vec(),
            b"arithmetic expression: division error: \"1/0\"".to_vec()
        );
        assert_eq!(error.status().code(), 2);
    }

    #[test]
    fn a_trailing_token_returns_its_diagnostic() {
        let mut shell = shell();
        let error = evaluate(&mut shell, BStr::new(b"1 2")).expect_err("`1 2` must fail");
        assert_eq!(
            error.message().to_vec(),
            b"arithmetic expression: expecting EOF: \"1 2\"".to_vec()
        );
    }

    #[test]
    fn a_good_expression_still_evaluates() {
        let mut shell = shell();
        assert_eq!(evaluate(&mut shell, BStr::new(b"6*7")).unwrap(), 42);
    }

    #[test]
    fn base_prefixes_and_overflow_match_intmax() {
        let mut shell = shell();
        assert_eq!(
            evaluate(&mut shell, BStr::new(b"0b11 + 010 + 0x10")).unwrap(),
            27
        );
        assert_eq!(
            evaluate(&mut shell, BStr::new(b"9223372036854775808")).unwrap(),
            i64::MAX
        );
    }

    // [spec:nsh:sem:idiom.specified-defects+1/test]
    #[test]
    fn overflow_and_shift_semantics_are_defined() {
        let mut shell = shell();
        assert_eq!(
            evaluate(&mut shell, BStr::new(b"9223372036854775807 + 1")).unwrap(),
            i64::MIN
        );
        assert_eq!(evaluate(&mut shell, BStr::new(b"1 << 64")).unwrap(), 1);
        assert_eq!(
            evaluate(&mut shell, BStr::new(b"1 << -1")).unwrap(),
            i64::MIN
        );
    }

    #[test]
    fn short_circuit_skips_effects_but_parses_both_sides() {
        let mut shell = shell();
        assert_eq!(evaluate(&mut shell, BStr::new(b"0 && 1 / 0")).unwrap(), 0);
        assert_eq!(evaluate(&mut shell, BStr::new(b"1 || 1 / 0")).unwrap(), 1);
        assert_eq!(
            evaluate(&mut shell, BStr::new(b"1 ? 7 : 1 / 0")).unwrap(),
            7
        );
    }

    // [spec:nsh:def:idiom.shell-options]
    // [spec:posix:req:builtin.set.opt-u-nounset/test]
    #[test]
    fn nounset_rejects_evaluated_arithmetic_reads() {
        let mut shell = shell();
        shell.options.set(ShellOption::Nounset, true);

        let error = evaluate(&mut shell, BStr::new(b"undefined_name + 1"))
            .expect_err("an evaluated unset variable must fail under nounset");
        assert_eq!(
            error.message().to_vec(),
            b"undefined_name: parameter not set".to_vec()
        );
        assert_eq!(error.status().code(), 2);

        assert_eq!(
            evaluate(&mut shell, BStr::new(b"assigned_name = 7")).unwrap(),
            7
        );
        assert_eq!(
            evaluate(&mut shell, BStr::new(b"0 && skipped_name")).unwrap(),
            0
        );
    }
}
