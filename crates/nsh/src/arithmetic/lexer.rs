//! Arithmetic expression bytes, as tokens.
//!
//! Rules: `docs/spec/port/src/arith_yylex.md`.
//!
//! `yylex` scanned out of four mutable globals and returned its value in a
//! shared `union yystype`. Here a lexer owns its offset and a token owns
//! its value, so the scan is a self-contained pass: the whole input is
//! tokenised before the parser looks at any of it, and nothing the parser
//! does can move the cursor back.
//!
//! It is a separate file because the two halves overlap in almost nothing.
//! Everything dialect-dependent about *spelling* is here -- `**`, `++`,
//! `,`, `base#digits`, the quotes Bash has already removed by the time it
//! evaluates -- and none of it reaches the parser, which sees only the
//! token that came out. Nothing here reads a variable, evaluates anything
//! or reports anything: what the scan cannot make sense of becomes
//! [`Token::Bad`], and it is the parser that decides what to say about it.
//! What crosses back the other way is a [`Token`] and the two types one
//! carries -- the parser reads them and never scans a byte itself.
//!
//! A name is lexed together with its subscript rather than as three
//! tokens, because `a[i]` is one lvalue: `a[i]++` has to read and write
//! the same element, and a parser handed `a`, `[`, `i`, `]` could not say
//! so.

use bstr::{BStr, ByteSlice};

/// The value carried by an arithmetic token.
///
/// This is an enum rather than the C `union yystype`: a number cannot be
/// mistaken for a variable-name pointer, and names borrow the input bytes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum Token<'a> {
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
pub(super) struct Name<'a> {
    pub(super) base: &'a BStr,
    pub(super) subscript: Option<&'a BStr>,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum BinaryOperator {
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
    pub(super) const fn binding_power(self) -> u8 {
        8 - self.precedence()
    }
}

pub(super) struct Lexer<'a> {
    input: &'a [u8],
    pos: usize,
    locale: nsh_platform::Locale,
    bash: bool,
}

impl<'a> Lexer<'a> {
    pub(super) fn new(input: &'a BStr, locale: nsh_platform::Locale, bash: bool) -> Self {
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
    pub(super) fn next(&mut self) -> Token<'a> {
        loop {
            match self.peek(0) {
                Some(b' ' | b'\t' | b'\n') => self.pos += 1,
                /* Bash's arithmetic text arrives already expanded as if
                 * it were inside double quotes, so a double quote in it
                 * has been removed by then and never reaches an
                 * evaluator. A *single* quote has not: it is an ordinary
                 * byte to that expansion and an unknown token here, and
                 * `(( i = '3' ))` and `${a['1']}` are both arithmetic
                 * syntax errors in the reference. This once skipped both
                 * and made `'3'` mean 3. */
                // [spec:nsh:req:compat.bash.arrays-declarations]
                Some(b'"') if self.bash => self.pos += 1,
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
