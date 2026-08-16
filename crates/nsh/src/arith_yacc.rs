//! Literal port of `src/arith_yacc.c` / `src/arith_yacc.h`.
//! Rules: `docs/spec/port/src/arith_yacc.md`.
//!
//! Arithmetic expansion.  Despite the name there is no yacc grammar: a
//! hand-written recursive-descent parser that consumes tokens from
//! `yylex` (`arith_yylex.rs`) and evaluates as it goes.
//!
//! The token codes below are order-sensitive: compound assignments sit at
//! a fixed offset (11) from the binary operators they derive from, and
//! `prec[]` is indexed by `op - ARITH_BINOP_MIN`.

use core::ptr;
use std::ffi::CStr;

use libc::{c_char, c_int};

use crate::error::Error;
use crate::var::{lookupvarint, setvarint};

pub use libc::intmax_t;

// ---------------------------------------------------------------------
// src/arith_yacc.h
// ---------------------------------------------------------------------

pub const ARITH_ASS: c_int = 1;

pub const ARITH_OR: c_int = 2;
pub const ARITH_AND: c_int = 3;
pub const ARITH_BAD: c_int = 4;
pub const ARITH_NUM: c_int = 5;
pub const ARITH_VAR: c_int = 6;
pub const ARITH_NOT: c_int = 7;

pub const ARITH_BINOP_MIN: c_int = 8;
pub const ARITH_LE: c_int = 8;
pub const ARITH_GE: c_int = 9;
pub const ARITH_LT: c_int = 10;
pub const ARITH_GT: c_int = 11;
pub const ARITH_EQ: c_int = 12;
pub const ARITH_REM: c_int = 13;
pub const ARITH_BAND: c_int = 14;
pub const ARITH_LSHIFT: c_int = 15;
pub const ARITH_RSHIFT: c_int = 16;
pub const ARITH_MUL: c_int = 17;
pub const ARITH_ADD: c_int = 18;
pub const ARITH_BOR: c_int = 19;
pub const ARITH_SUB: c_int = 20;
pub const ARITH_BXOR: c_int = 21;
pub const ARITH_DIV: c_int = 22;
pub const ARITH_NE: c_int = 23;
pub const ARITH_BINOP_MAX: c_int = 24;

pub const ARITH_ASS_MIN: c_int = 24;
pub const ARITH_REMASS: c_int = 24;
pub const ARITH_BANDASS: c_int = 25;
pub const ARITH_LSHIFTASS: c_int = 26;
pub const ARITH_RSHIFTASS: c_int = 27;
pub const ARITH_MULASS: c_int = 28;
pub const ARITH_ADDASS: c_int = 29;
pub const ARITH_BORASS: c_int = 30;
pub const ARITH_SUBASS: c_int = 31;
pub const ARITH_BXORASS: c_int = 32;
pub const ARITH_DIVASS: c_int = 33;
pub const ARITH_ASS_MAX: c_int = 34;

pub const ARITH_LPAREN: c_int = 34;
pub const ARITH_RPAREN: c_int = 35;
pub const ARITH_BNOT: c_int = 36;
pub const ARITH_QMARK: c_int = 37;
pub const ARITH_COLON: c_int = 38;

// [spec:dash:def:arith-yacc.yystype]
#[repr(C)]
#[derive(Clone, Copy)]
pub union yystype {
    pub val: intmax_t,
    pub name: *mut c_char,
}

/* `int yylex(void)` — the tokeniser prototype used by this file; the
 * implementation lives in `arith_yylex.rs`. */
// [spec:dash:def:arith-yacc.yylex-fn]
// [spec:dash:sem:arith-yacc.yylex-fn]
pub use crate::arith_yylex::yylex;

// ---------------------------------------------------------------------
// src/arith_yacc.c
// ---------------------------------------------------------------------

/* #if ARITH_BOR + 11 != ARITH_BORASS || ARITH_ASS + 11 != ARITH_EQ
 * #error Arithmetic tokens are out of order.
 * #endif
 */
const _: () = assert!(ARITH_BOR + 11 == ARITH_BORASS && ARITH_ASS + 11 == ARITH_EQ);

static mut arith_startbuf: *const c_char = ptr::null();

pub static mut arith_buf: *const c_char = ptr::null();
pub static mut yylval: yystype = yystype { val: 0 };

static mut last_token: c_int = 0;

/*
 * #define ARITH_PRECEDENCE(op, prec) [op - ARITH_BINOP_MIN] = prec
 *
 * MUL/DIV/REM 0, ADD/SUB 1, shifts 2, relational 3, equality 4,
 * BAND 5, BXOR 6, BOR 7 — lower binds tighter.
 */
static prec: [c_char; (ARITH_BINOP_MAX - ARITH_BINOP_MIN) as usize] = [
    3, /* ARITH_LE */
    3, /* ARITH_GE */
    3, /* ARITH_LT */
    3, /* ARITH_GT */
    4, /* ARITH_EQ */
    0, /* ARITH_REM */
    5, /* ARITH_BAND */
    2, /* ARITH_LSHIFT */
    2, /* ARITH_RSHIFT */
    0, /* ARITH_MUL */
    1, /* ARITH_ADD */
    7, /* ARITH_BOR */
    1, /* ARITH_SUB */
    6, /* ARITH_BXOR */
    0, /* ARITH_DIV */
    4, /* ARITH_NE */
];

pub const ARITH_MAX_PREC: c_int = 8;

// [spec:dash:def:arith-yacc.yyerror-fn]
// [spec:dash:sem:arith-yacc.yyerror-fn]
//
// The C's `yyerror` does not return, because `sh_error` longjmps out of it.
// Here it *builds* the error instead: the diagnostic is still written at
// this point, in these bytes, but the jump is the caller's `?`. Every
// caller in this file spells it `return Err(yyerror(..))`.
unsafe fn yyerror(sh: &mut crate::context::Shell, s: *const c_char) -> Error {
    let mut message = b"arithmetic expression: ".to_vec();
    message.extend_from_slice(CStr::from_ptr(s).to_bytes());
    message.extend_from_slice(b": \"");
    message.extend_from_slice(CStr::from_ptr(arith_startbuf).to_bytes());
    message.push(b'"');
    sh.sh_error_value(&message)
}

// [spec:dash:def:arith-yacc.arith-prec-fn]
// [spec:dash:sem:arith-yacc.arith-prec-fn]
unsafe fn arith_prec(op: c_int) -> c_int {
    prec[(op - ARITH_BINOP_MIN) as usize] as c_int
}

// [spec:dash:def:arith-yacc.higher-prec-fn]
// [spec:dash:sem:arith-yacc.higher-prec-fn]
unsafe fn higher_prec(op1: c_int, op2: c_int) -> c_int {
    (arith_prec(op1) < arith_prec(op2)) as c_int
}

// [spec:dash:def:arith-yacc.do-binop-fn]
// [spec:dash:sem:arith-yacc.do-binop-fn]
//
// Signed overflow and out-of-range shift counts are undefined in C; the
// wrapping forms below are the closest match to what the platforms dash
// targets actually do.
unsafe fn do_binop(sh: &mut crate::context::Shell, op: c_int, a: intmax_t, b: intmax_t) -> Result<intmax_t, Error> {
    Ok(match op {
        ARITH_MUL => a.wrapping_mul(b),
        ARITH_ADD => a.wrapping_add(b),
        ARITH_SUB => a.wrapping_sub(b),
        ARITH_LSHIFT => a.wrapping_shl(b as u32),
        ARITH_RSHIFT => a.wrapping_shr(b as u32),
        ARITH_LT => (a < b) as intmax_t,
        ARITH_LE => (a <= b) as intmax_t,
        ARITH_GT => (a > b) as intmax_t,
        ARITH_GE => (a >= b) as intmax_t,
        ARITH_EQ => (a == b) as intmax_t,
        ARITH_NE => (a != b) as intmax_t,
        ARITH_BAND => a & b,
        ARITH_BXOR => a ^ b,
        ARITH_BOR => a | b,
        /* default, ARITH_REM, ARITH_DIV */
        _ => {
            if b == 0 || (a == intmax_t::MIN && b == -1) {
                return Err(yyerror(sh, c"division error".as_ptr()));
            }
            if op == ARITH_REM { a % b } else { a / b }
        }
    })
}

// [spec:dash:def:arith-yacc.primary-fn]
// [spec:dash:sem:arith-yacc.primary-fn]
unsafe fn primary(
    sh: &mut crate::context::Shell,
    token: c_int,
    val: *mut yystype,
    op: c_int,
    noeval: c_int,
) -> Result<intmax_t, Error> {
    let mut token = token;
    let mut op = op;

    loop {
        /* again: */
        match token {
            ARITH_LPAREN => {
                let result = assignment(sh, op, noeval)?;
                if last_token != ARITH_RPAREN {
                    return Err(yyerror(sh, c"expecting ')'".as_ptr()));
                }
                last_token = yylex();
                return Ok(result);
            }
            ARITH_NUM => {
                last_token = op;
                return Ok((*val).val);
            }
            ARITH_VAR => {
                last_token = op;
                return Ok(if noeval != 0 {
                    (*val).val
                } else {
                    lookupvarint(sh, (*val).name)?
                });
            }
            ARITH_ADD => {
                token = op;
                *val = yylval;
                op = yylex();
                continue; /* goto again */
            }
            ARITH_SUB => {
                *val = yylval;
                return Ok(primary(sh, op, val, yylex(), noeval)?.wrapping_neg());
            }
            ARITH_NOT => {
                *val = yylval;
                return Ok((primary(sh, op, val, yylex(), noeval)? == 0) as intmax_t);
            }
            ARITH_BNOT => {
                *val = yylval;
                return Ok(!primary(sh, op, val, yylex(), noeval)?);
            }
            _ => {
                return Err(yyerror(sh, c"expecting primary".as_ptr()));
            }
        }
    }
}

// [spec:dash:def:arith-yacc.binop2-fn]
// [spec:dash:sem:arith-yacc.binop2-fn]
/* The C names the third parameter `prec`, shadowing the file-scope `prec[]`
 * table; Rust forbids a parameter that shadows a static, so it is spelt
 * `prec_` here.  Nothing inside the function reads the table directly. */
unsafe fn binop2(
    sh: &mut crate::context::Shell,
    a: intmax_t,
    op: c_int,
    prec_: c_int,
    noeval: c_int,
) -> Result<intmax_t, Error> {
    let mut a = a;
    let mut op = op;

    loop {
        let mut val: yystype;
        let mut b: intmax_t;
        let mut op2: c_int;
        let token: c_int;

        token = yylex();
        val = yylval;

        b = primary(sh, token, &mut val, yylex(), noeval)?;

        op2 = last_token;
        if op2 >= ARITH_BINOP_MIN && op2 < ARITH_BINOP_MAX && higher_prec(op2, op) != 0 {
            b = binop2(sh, b, op2, arith_prec(op), noeval)?;
            op2 = last_token;
        }

        a = if noeval != 0 { b } else { do_binop(sh, op, a, b)? };

        if op2 < ARITH_BINOP_MIN || op2 >= ARITH_BINOP_MAX || arith_prec(op2) >= prec_ {
            return Ok(a);
        }

        op = op2;
    }
}

// [spec:dash:def:arith-yacc.binop-fn]
// [spec:dash:sem:arith-yacc.binop-fn]
unsafe fn binop(
    sh: &mut crate::context::Shell,
    token: c_int,
    val: *mut yystype,
    op: c_int,
    noeval: c_int,
) -> Result<intmax_t, Error> {
    let a: intmax_t = primary(sh, token, val, op, noeval)?;
    let op = last_token;

    if op < ARITH_BINOP_MIN || op >= ARITH_BINOP_MAX {
        return Ok(a);
    }

    binop2(sh, a, op, ARITH_MAX_PREC, noeval)
}

// [spec:dash:def:arith-yacc.and-fn]
// [spec:dash:sem:arith-yacc.and-fn]
unsafe fn and(
    sh: &mut crate::context::Shell,
    token: c_int,
    val: *mut yystype,
    op: c_int,
    noeval: c_int,
) -> Result<intmax_t, Error> {
    let a: intmax_t = binop(sh, token, val, op, noeval)?;
    let b: intmax_t;

    let op = last_token;
    if op != ARITH_AND {
        return Ok(a);
    }

    let token = yylex();
    *val = yylval;

    b = and(sh, token, val, yylex(), noeval | (a == 0) as c_int)?;

    Ok((a != 0 && b != 0) as intmax_t)
}

// [spec:dash:def:arith-yacc.or-fn]
// [spec:dash:sem:arith-yacc.or-fn]
unsafe fn or(
    sh: &mut crate::context::Shell,
    token: c_int,
    val: *mut yystype,
    op: c_int,
    noeval: c_int,
) -> Result<intmax_t, Error> {
    let a: intmax_t = and(sh, token, val, op, noeval)?;
    let b: intmax_t;

    let op = last_token;
    if op != ARITH_OR {
        return Ok(a);
    }

    let token = yylex();
    *val = yylval;

    b = or(sh, token, val, yylex(), noeval | (a != 0) as c_int)?;

    Ok((a != 0 || b != 0) as intmax_t)
}

// [spec:dash:def:arith-yacc.cond-fn]
// [spec:dash:sem:arith-yacc.cond-fn]
unsafe fn cond(
    sh: &mut crate::context::Shell,
    token: c_int,
    val: *mut yystype,
    op: c_int,
    noeval: c_int,
) -> Result<intmax_t, Error> {
    let a: intmax_t = or(sh, token, val, op, noeval)?;
    let b: intmax_t;
    let c: intmax_t;

    if last_token != ARITH_QMARK {
        return Ok(a);
    }

    b = assignment(sh, yylex(), noeval | (a == 0) as c_int)?;

    if last_token != ARITH_COLON {
        return Err(yyerror(sh, c"expecting ':'".as_ptr()));
    }

    let token = yylex();
    *val = yylval;

    c = cond(sh, token, val, yylex(), noeval | (a != 0) as c_int)?;

    Ok(if a != 0 { b } else { c })
}

// [spec:dash:def:arith-yacc.assignment-fn]
// [spec:dash:sem:arith-yacc.assignment-fn]
unsafe fn assignment(sh: &mut crate::context::Shell, var: c_int, noeval: c_int) -> Result<intmax_t, Error> {
    let mut val: yystype = yylval;
    let op: c_int = yylex();
    let result: intmax_t;

    if var != ARITH_VAR {
        return cond(sh, var, &mut val, op, noeval);
    }

    if op != ARITH_ASS && (op < ARITH_ASS_MIN || op >= ARITH_ASS_MAX) {
        return cond(sh, var, &mut val, op, noeval);
    }

    result = assignment(sh, yylex(), noeval)?;
    if noeval != 0 {
        return Ok(result);
    }

    /* The C reads the variable inside `setvarint`'s argument list. Both
     * take the shell now, so the read is hoisted to its own statement --
     * Rust evaluates arguments left to right, so it ran before the call
     * before and runs before it now. Do not re-inline it. */
    let value = if op == ARITH_ASS {
        result
    } else {
        {
            /* Hoisted for the borrow, not for the order: it is the third
             * argument and the two before it have no side effects, so
             * left-to-right evaluation is unchanged. See the note above. */
            let current = lookupvarint(sh, val.name)?;
            do_binop(sh, op - 11, current, result)?
        }
    };
    setvarint(sh, val.name, value, 0)
}

// [spec:dash:def:arith-yacc.arith-fn]
// [spec:dash:sem:arith-yacc.arith-fn]
pub unsafe fn arith(sh: &mut crate::context::Shell, s: *const c_char) -> Result<intmax_t, Error> {
    let result: intmax_t;

    arith_startbuf = s;
    arith_buf = arith_startbuf;
    /* The names `yylex` produces belong to this evaluation; the C's
     * `stalloc`s were released by `expari`'s mark, which is here. */
    crate::arith_yylex::arith_names_reset();

    result = assignment(sh, yylex(), 0)?;

    if last_token != 0 {
        return Err(yyerror(sh, c"expecting EOF".as_ptr()));
    }

    Ok(result)
}

#[cfg(test)]
mod tests {
    use super::*;

    /* `docs/errors-are-values.md` §5 is the list of things the differential
     * harness cannot see, and the first entry is the error value itself:
     * "the harness compares bytes on a stream, and the value never reaches
     * it". These are that test for the first module converted. The bytes
     * still go to stderr while they run, which is the point -- the
     * diagnostic is written where dash writes it *and* returned. */

    #[test]
    fn a_failed_evaluation_returns_its_diagnostic() {
        let _g = crate::testutil::lock();
        let expr = crate::testutil::CStr0::new("1/0");

        let mut owned = crate::context::Shell::new();

        let sh = &mut owned;

        let e = unsafe { arith(sh, expr.p()) }.expect_err("1/0 must fail");

        assert_eq!(
            e.message().to_vec(),
            b"arithmetic expression: division error: \"1/0\"".to_vec()
        );
        assert_eq!(e.status(), 2);
    }

    #[test]
    fn a_trailing_token_returns_its_diagnostic() {
        let _g = crate::testutil::lock();
        let expr = crate::testutil::CStr0::new("1 2");

        let mut owned = crate::context::Shell::new();

        let sh = &mut owned;

        let e = unsafe { arith(sh, expr.p()) }.expect_err("`1 2` must fail");

        assert_eq!(
            e.message().to_vec(),
            b"arithmetic expression: expecting EOF: \"1 2\"".to_vec()
        );
    }

    #[test]
    fn a_good_expression_still_evaluates() {
        let _g = crate::testutil::lock();
        let expr = crate::testutil::CStr0::new("6*7");
        let mut owned = crate::context::Shell::new();
        let sh = &mut owned;

        assert_eq!(unsafe { arith(sh, expr.p()) }.expect("6*7 evaluates"), 42);
    }
}
