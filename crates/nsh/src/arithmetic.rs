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

mod lexer;
use lexer::{BinaryOperator, Lexer, Name, Token};

/// How deep an expression may nest before the evaluator refuses it.
///
/// Three things spend this budget and they spend the same stack, so they
/// share one counter: a parenthesis; a prefix `+`, `-`, `!` or `~`; and a
/// name whose *value* is read back as an expression. Bash evaluates a
/// name's value that way, which makes `a=b; b=a` a cycle rather than a
/// number and `loop='i<=100&&(s+=i,i++,loop)'` a way to count to a
/// hundred.
///
/// Counting all three is what keeps the ceiling meaningful.
/// `[dec:nsh:safety-trumps-compatibility]` does not allow the
/// alternative: Bash bounds only the name recursion, so sixty
/// parentheses inside a self-referring name crash it, and a shell that
/// matched it there would crash too.
const MAX_NAME_DEPTH: u32 = 1024;

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
        /* Bash answers 1 and abandons the record for every arithmetic
         * failure -- a syntax error, a division by zero, a subscript that
         * will not evaluate. `set -u` on an unset name inside an
         * expression is not one of them and keeps the fatal boundary in
         * both dialects, which is why that raise is separate.
         *
         * The mark is set here because this is the only place that knows
         * the failure was arithmetic, and `errexit` does not end the
         * shell for one: the reference reads the next record after
         * `declare -i x=1+` with `set -e` live, and stops at the
         * read-only refusal that reaches `dialect_error` by every other
         * caller. The POSIX dialect builds no abandonment to mark. */
        // [spec:nsh:req:compat.bash.error-boundary]
        match self.shell.diagnostics().dialect_error(&text) {
            Error::Abandoned {
                line,
                message,
                from_assignment,
                ..
            } => Error::Abandoned {
                line,
                message,
                from_assignment,
                from_arithmetic: true,
            },
            fatal => fatal,
        }
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
        let selector = arrays::resolve_text_selector(self.shell, name.base, subscript)?;
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
        let selector = arrays::resolve_text_selector(
            self.shell,
            name.base,
            name.subscript.unwrap_or_default(),
        )?;
        let text = value.to_string();
        arrays::assign_element(
            self.shell,
            name.base,
            &selector,
            BStr::new(text.as_bytes()),
            false,
            arrays::ReadOnlyGuard::Enforce,
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
                if self.depth >= MAX_NAME_DEPTH {
                    return Err(self.error(b"expression recursion level exceeded"));
                }
                self.depth += 1;
                let value = self.comma(evaluate);
                self.depth -= 1;
                let value = value?;
                if self.current() != Token::RParen {
                    return Err(self.error(b"expecting ')'"));
                }
                self.advance();
                Ok(value)
            }
            Token::Binary(BinaryOperator::Add) => self.prefixed(evaluate),
            Token::Binary(BinaryOperator::Subtract) => Ok(self.prefixed(evaluate)?.wrapping_neg()),
            Token::Not => Ok((self.prefixed(evaluate)? == 0) as i64),
            Token::BitNot => Ok(!self.prefixed(evaluate)?),
            _ => Err(self.error(b"expecting primary")),
        }
    }

    /// The operand of a prefix `+`, `-`, `!` or `~`.
    ///
    /// Charged the budget a parenthesis is charged, because it spends the
    /// same stack: `$(( ---...1 ))` recurses once per sign, and untouched
    /// it overflows where `$(( (((...))) ))` is refused.
    // [spec:nsh:req:idiom.bounded-recursion]
    fn prefixed(&mut self, evaluate: bool) -> Result<i64, Error> {
        if self.depth >= MAX_NAME_DEPTH {
            return Err(self.error(b"expression recursion level exceeded"));
        }
        self.depth += 1;
        let value = self.power(evaluate);
        self.depth -= 1;
        value
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
        // `$(( a[-5] ))` is reported and then reads as zero, as an unset
        // element does.
        ArraySelector::Missing => BString::default(),
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
