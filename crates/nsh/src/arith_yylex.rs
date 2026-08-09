//! Literal port of `src/arith_yylex.c`.
//! Rules: `docs/spec/port/src/arith_yylex.md`.
//!
//! The arithmetic tokeniser.  It reads from the global cursor
//! `arith_buf`, advances it past the token consumed, returns the token
//! code and leaves any associated value in the global `yylval`.
//!
//! Operators are recognised by the shared trick `value += TOKEN - char`,
//! which folds to the token value because `value` still holds the
//! character; the arithmetic is kept here so the correspondence with the
//! C is visible.

use core::ptr;

use libc::{c_char, c_int};

use crate::arith_yacc::{
    arith_buf, intmax_t, yylval, ARITH_ADD, ARITH_AND, ARITH_ASS, ARITH_BAD, ARITH_BAND,
    ARITH_BNOT, ARITH_BOR, ARITH_BXOR, ARITH_COLON, ARITH_DIV, ARITH_GE, ARITH_GT, ARITH_LE,
    ARITH_LPAREN, ARITH_LSHIFT, ARITH_LT, ARITH_MUL, ARITH_NE, ARITH_NOT, ARITH_NUM, ARITH_OR,
    ARITH_QMARK, ARITH_REM, ARITH_RPAREN, ARITH_RSHIFT, ARITH_SUB, ARITH_VAR, ARITH_BORASS,
    ARITH_EQ,
};
use crate::syntax::is_in_name;

/* #if ARITH_BOR + 11 != ARITH_BORASS || ARITH_ASS + 11 != ARITH_EQ
 * #error Arithmetic tokens are out of order.
 * #endif
 */
const _: () = assert!(ARITH_BOR + 11 == ARITH_BORASS && ARITH_ASS + 11 == ARITH_EQ);

/// The variable names `yylex` hands to the parser through `yylval.name`.
///
/// The C `stalloc`s each one and lets `expari`'s enclosing mark release
/// them all at once, and it needs *all* of them at once: `assignment`
/// copies `yylval` into a local, recurses — which calls `yylex` again and
/// overwrites `yylval` — and only then reads `val.name`, so `a = b = 1`
/// has two names live. That is a per-evaluation lifetime, not a
/// per-token one, and this is where it is now written down: `arith`
/// clears the list on entry and the `Vec` owns the bytes until the next
/// evaluation starts.
///
/// `arith` is already non-reentrant — it sets the `arith_buf` cursor from
/// its argument — so one list is enough.  Pushing to it can reallocate
/// the outer `Vec`, which moves the `Vec<u8>` headers but not the bytes
/// they point at, so a `yylval.name` handed out earlier stays valid.
static mut arith_names: Vec<Vec<u8>> = Vec::new();

/// `&mut arith_names`, without naming a reference to the `static mut`
/// twice at once.
#[inline]
pub unsafe fn arith_names_reset() {
    (*ptr::addr_of_mut!(arith_names)).clear();
}

/// The `goto` targets in the C: `checkeq` advances past the current
/// character first, `checkeqcur` does not (for the cases that have
/// already advanced), `break` out of the switch falls into the shared
/// `buf++` tail, and `out` positions the cursor as it stands.
enum Lbl {
    Checkeq,
    Checkeqcur,
    BreakSw,
    Out,
}

// [spec:dash:def:arith-yylex.yylex-fn]
// [spec:dash:sem:arith-yylex.yylex-fn]
pub unsafe fn yylex() -> c_int {
    let mut value: c_int;
    let mut buf: *const c_char = arith_buf;

    'top: loop {
        value = *buf as c_int;
        let mut lbl: Lbl;

        if value == ' ' as c_int || value == '\t' as c_int || value == '\n' as c_int {
            buf = buf.offset(1);
            continue 'top;
        } else if value >= '0' as c_int && value <= '9' as c_int {
            yylval.val = strtoimax(buf, ptr::addr_of_mut!(arith_buf) as *mut *mut c_char, 0);
            return ARITH_NUM;
        } else if (value >= 'A' as c_int && value <= 'Z' as c_int)
            || value == '_' as c_int
            || (value >= 'a' as c_int && value <= 'z' as c_int)
        {
            let p: *const c_char = buf;
            loop {
                buf = buf.offset(1);
                if !is_in_name(*buf as c_int) {
                    break;
                }
            }
            let len = buf as usize - p as usize;
            let names = &mut *ptr::addr_of_mut!(arith_names);
            let mut name: Vec<u8> = Vec::with_capacity(len + 1);
            name.extend_from_slice(core::slice::from_raw_parts(p as *const u8, len));
            name.push(0);
            names.push(name);
            let last = names.len() - 1;
            yylval.name = names[last].as_mut_ptr() as *mut c_char;
            value = ARITH_VAR;
            lbl = Lbl::Out;
        } else if value == '=' as c_int {
            value += ARITH_ASS - '=' as c_int;
            lbl = Lbl::Checkeq;
        } else if value == '>' as c_int {
            buf = buf.offset(1);
            let c = *buf as c_int;
            if c == '=' as c_int {
                value += ARITH_GE - '>' as c_int;
                lbl = Lbl::BreakSw;
            } else if c == '>' as c_int {
                value += ARITH_RSHIFT - '>' as c_int;
                lbl = Lbl::Checkeq;
            } else {
                value += ARITH_GT - '>' as c_int;
                lbl = Lbl::Out;
            }
        } else if value == '<' as c_int {
            buf = buf.offset(1);
            let c = *buf as c_int;
            if c == '=' as c_int {
                value += ARITH_LE - '<' as c_int;
                lbl = Lbl::BreakSw;
            } else if c == '<' as c_int {
                value += ARITH_LSHIFT - '<' as c_int;
                lbl = Lbl::Checkeq;
            } else {
                value += ARITH_LT - '<' as c_int;
                lbl = Lbl::Out;
            }
        } else if value == '|' as c_int {
            buf = buf.offset(1);
            if *buf as c_int != '|' as c_int {
                value += ARITH_BOR - '|' as c_int;
                lbl = Lbl::Checkeqcur;
            } else {
                value += ARITH_OR - '|' as c_int;
                lbl = Lbl::BreakSw;
            }
        } else if value == '&' as c_int {
            buf = buf.offset(1);
            if *buf as c_int != '&' as c_int {
                value += ARITH_BAND - '&' as c_int;
                lbl = Lbl::Checkeqcur;
            } else {
                value += ARITH_AND - '&' as c_int;
                lbl = Lbl::BreakSw;
            }
        } else if value == '!' as c_int {
            buf = buf.offset(1);
            if *buf as c_int != '=' as c_int {
                value += ARITH_NOT - '!' as c_int;
                lbl = Lbl::Out;
            } else {
                value += ARITH_NE - '!' as c_int;
                lbl = Lbl::BreakSw;
            }
        } else if value == 0 {
            lbl = Lbl::Out;
        } else if value == '(' as c_int {
            value += ARITH_LPAREN - '(' as c_int;
            lbl = Lbl::BreakSw;
        } else if value == ')' as c_int {
            value += ARITH_RPAREN - ')' as c_int;
            lbl = Lbl::BreakSw;
        } else if value == '*' as c_int {
            value += ARITH_MUL - '*' as c_int;
            lbl = Lbl::Checkeq;
        } else if value == '/' as c_int {
            value += ARITH_DIV - '/' as c_int;
            lbl = Lbl::Checkeq;
        } else if value == '%' as c_int {
            value += ARITH_REM - '%' as c_int;
            lbl = Lbl::Checkeq;
        } else if value == '+' as c_int {
            value += ARITH_ADD - '+' as c_int;
            lbl = Lbl::Checkeq;
        } else if value == '-' as c_int {
            value += ARITH_SUB - '-' as c_int;
            lbl = Lbl::Checkeq;
        } else if value == '~' as c_int {
            value += ARITH_BNOT - '~' as c_int;
            lbl = Lbl::BreakSw;
        } else if value == '^' as c_int {
            value += ARITH_BXOR - '^' as c_int;
            lbl = Lbl::Checkeq;
        } else if value == '?' as c_int {
            value += ARITH_QMARK - '?' as c_int;
            lbl = Lbl::BreakSw;
        } else if value == ':' as c_int {
            value += ARITH_COLON - ':' as c_int;
            lbl = Lbl::BreakSw;
        } else {
            /* default: */
            return ARITH_BAD;
        }

        loop {
            match lbl {
                Lbl::Checkeq => {
                    /* checkeq: */
                    buf = buf.offset(1);
                    lbl = Lbl::Checkeqcur;
                }
                Lbl::Checkeqcur => {
                    /* checkeqcur: */
                    if *buf != '=' as c_char {
                        lbl = Lbl::Out;
                    } else {
                        value += 11;
                        lbl = Lbl::BreakSw;
                    }
                }
                Lbl::BreakSw => {
                    /* break out of the switch, then out of the for: buf++ */
                    buf = buf.offset(1);
                    lbl = Lbl::Out;
                }
                Lbl::Out => break,
            }
        }
        break 'top;
    }
    /* out: */
    arith_buf = buf;
    value
}

// ---------------------------------------------------------------------
// C library entry points the libc crate does not declare.
// ---------------------------------------------------------------------

/*
 * glibc >= 2.38 redirects `strtoimax`/`strtoumax` through `__isoc23_*`
 * for any translation unit with C23 strtol semantics enabled, which is
 * every unit in the dash build (`features.h` defaults
 * `__GLIBC_USE_C2X_STRTOL` to 1). Those variants also accept `0b`/`0B`
 * binary constants when the base is 0 or 2. `nm -D` on the reference
 * binary shows `__isoc23_strtoimax@GLIBC_2.38`, so the C really does
 * accept them — `$((0b11))` is 3 — and binding the plain symbol here
 * silently loses binary literals.
 */
unsafe extern "C" {
    #[link_name = "__isoc23_strtoimax"]
    fn strtoimax(nptr: *const c_char, endptr: *mut *mut c_char, base: c_int) -> intmax_t;
}
