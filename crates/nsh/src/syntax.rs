//! Literal port of `src/syntax.c` / `src/syntax.h`.
//!
//! These two files are *generated* at build time by `src/mksyntax.c`
//! (`docs/spec/port/src/mksyntax.md`).  They are not checked-in C source, so
//! nothing here carries `[spec:dash:…]` annotations.
//!
//! The five tables below are byte-for-byte the output of running the real
//! `mksyntax` on this tree (with `CTL_FIRST`/`CTL_LAST` from `src/parser.h`),
//! and the test at the foot of this file asserts exactly that against the
//! `syntax.c` the reference build generated — which is what makes the claim
//! checkable rather than a comment.
//!
//! Each table has 257 entries covering the byte values `-129..=127` reached
//! through the `SYNBASE` offset: C indexes them as `basesyntax + SYNBASE`, so
//! a sign-extended `char` can be used directly and `PEOF` (`-129`) lands at
//! index 0.  Rust cannot form a pointer into the middle of an array and index
//! it negatively, so the offset is applied by the accessor functions
//! [`BASESYNTAX`], [`DQSYNTAX`], [`SQSYNTAX`], [`ARISYNTAX`] and [`is_type_at`]
//! (or by [`syntax_at`] for a table held in a variable, which is how
//! `parser.c` carries `synstack->syntax` around).

use libc::{c_char, c_int, c_uint};

// ---- syntax classes (numbered by position in mksyntax.c's synclass[]) ----

/// character is nothing special
pub const CWORD: c_char = 0;
/// newline character
pub const CNL: c_char = 1;
/// a backslash character
pub const CBACK: c_char = 2;
/// single quote
pub const CSQUOTE: c_char = 3;
/// double quote
pub const CDQUOTE: c_char = 4;
/// a terminating quote
pub const CENDQUOTE: c_char = 5;
/// backwards single quote
pub const CBQUOTE: c_char = 6;
/// a dollar sign
pub const CVAR: c_char = 7;
/// a '}' character
pub const CENDVAR: c_char = 8;
/// a left paren in arithmetic
pub const CLP: c_char = 9;
/// a right paren in arithmetic
pub const CRP: c_char = 10;
/// end of file
pub const CEOF: c_char = 11;
/// like CWORD, except it must be escaped
pub const CCTL: c_char = 12;
/// these terminate a word
pub const CSPCL: c_char = 13;

// ---- syntax classes for the is_ functions (bit flags) ----

/// a digit
pub const ISDIGIT: c_char = 0o1;
/// an upper case letter
pub const ISUPPER: c_char = 0o2;
/// a lower case letter
pub const ISLOWER: c_char = 0o4;
/// an underscore
pub const ISUNDER: c_char = 0o10;
/// the name of a special parameter
pub const ISSPECL: c_char = 0o20;

pub const SYNBASE: c_int = 129;
pub const PEOF: c_int = -129;

/// One syntax table: `const char name[257]` in the generated `syntax.c`.
pub type Syntax = [c_char; 257];

// ---- the offset accessors (the `BASESYNTAX` ... macros of syntax.h) ----

/// `tab[c]` where `tab` is one of the tables *after* the `SYNBASE` offset has
/// been applied, i.e. the C expression `(tab + SYNBASE)[c]`.
#[inline]
pub fn syntax_at(tab: &Syntax, c: c_int) -> c_char {
    tab[(c + SYNBASE) as usize]
}

/// `BASESYNTAX[c]`
#[inline]
pub fn BASESYNTAX(c: c_int) -> c_char {
    syntax_at(&basesyntax, c)
}

/// `DQSYNTAX[c]`
#[inline]
pub fn DQSYNTAX(c: c_int) -> c_char {
    syntax_at(&dqsyntax, c)
}

/// `SQSYNTAX[c]`
#[inline]
pub fn SQSYNTAX(c: c_int) -> c_char {
    syntax_at(&sqsyntax, c)
}

/// `ARISYNTAX[c]`
#[inline]
pub fn ARISYNTAX(c: c_int) -> c_char {
    syntax_at(&arisyntax, c)
}

/// `(is_type + SYNBASE)[c]`
#[inline]
pub fn is_type_at(c: c_int) -> c_char {
    syntax_at(&is_type, c)
}

// ---- the is_* macros of syntax.h (see mksyntax.c's macro[]) ----

/// `#define is_digit(c)\t((unsigned)((c) - '0') <= 9)`
#[inline]
pub fn is_digit(c: c_int) -> bool {
    (c.wrapping_sub(b'0' as c_int)) as c_uint <= 9
}

/// `#define is_alpha(c)\tisalpha((unsigned char)(c))`
#[inline]
pub fn is_alpha(c: c_int) -> bool {
    unsafe { libc::isalpha(c as u8 as c_int) != 0 }
}

/// `#define is_name(c)\t((c) == '_' || isalpha((unsigned char)(c)))`
#[inline]
pub fn is_name(c: c_int) -> bool {
    c == b'_' as c_int || unsafe { libc::isalpha(c as u8 as c_int) != 0 }
}

/// `#define is_in_name(c)\t((c) == '_' || isalnum((unsigned char)(c)))`
#[inline]
pub fn is_in_name(c: c_int) -> bool {
    c == b'_' as c_int || unsafe { libc::isalnum(c as u8 as c_int) != 0 }
}

/// `#define is_special(c)\t((is_type+SYNBASE)[(signed char)(c)] & (ISSPECL|ISDIGIT))`
#[inline]
pub fn is_special(c: c_int) -> c_int {
    (syntax_at(&is_type, c as c_char as c_int) as c_int) & ((ISSPECL | ISDIGIT) as c_int)
}

/// `#define digit_val(c)\t((c) - '0')`
#[inline]
pub fn digit_val(c: c_int) -> c_int {
    c - b'0' as c_int
}

// syntax table used when not in quotes
pub static basesyntax: Syntax = [
    CEOF,      CWORD,     CCTL,      CCTL,      CCTL,      CCTL,      CCTL,      CCTL,       // PEOF..=-122
    CCTL,      CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -121..=-114
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -113..=-106
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -105..=-98
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -97..=-90
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -89..=-82
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -81..=-74
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -73..=-66
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -65..=-58
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -57..=-50
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -49..=-42
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -41..=-34
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -33..=-26
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -25..=-18
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -17..=-10
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -9..=-2
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -1..=6
    CWORD,     CWORD,     CSPCL,     CNL,       CWORD,     CWORD,     CWORD,     CWORD,      // 7..=14
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 15..=22
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 23..=30
    CWORD,     CSPCL,     CWORD,     CDQUOTE,   CWORD,     CVAR,      CWORD,     CSPCL,      // 31..='&'
    CSQUOTE,   CSPCL,     CSPCL,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '\''..='.'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '/'..='6'
    CWORD,     CWORD,     CWORD,     CWORD,     CSPCL,     CSPCL,     CWORD,     CSPCL,      // '7'..='>'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '?'..='F'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'G'..='N'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'O'..='V'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CBACK,     CWORD,     CWORD,      // 'W'..='^'
    CWORD,     CBQUOTE,   CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '_'..='f'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'g'..='n'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'o'..='v'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CSPCL,     CENDVAR,   CWORD,      // 'w'..='~'
    CWORD,                                                                                   // 127..=127
];

// syntax table used when in double quotes
pub static dqsyntax: Syntax = [
    CEOF,      CWORD,     CCTL,      CCTL,      CCTL,      CCTL,      CCTL,      CCTL,       // PEOF..=-122
    CCTL,      CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -121..=-114
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -113..=-106
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -105..=-98
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -97..=-90
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -89..=-82
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -81..=-74
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -73..=-66
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -65..=-58
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -57..=-50
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -49..=-42
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -41..=-34
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -33..=-26
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -25..=-18
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -17..=-10
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -9..=-2
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -1..=6
    CWORD,     CWORD,     CWORD,     CNL,       CWORD,     CWORD,     CWORD,     CWORD,      // 7..=14
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 15..=22
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 23..=30
    CWORD,     CWORD,     CCTL,      CENDQUOTE, CWORD,     CVAR,      CWORD,     CWORD,      // 31..='&'
    CWORD,     CWORD,     CWORD,     CCTL,      CWORD,     CWORD,     CCTL,      CWORD,      // '\''..='.'
    CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '/'..='6'
    CWORD,     CWORD,     CWORD,     CCTL,      CWORD,     CWORD,     CCTL,      CWORD,      // '7'..='>'
    CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '?'..='F'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'G'..='N'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'O'..='V'
    CWORD,     CWORD,     CWORD,     CWORD,     CCTL,      CBACK,     CCTL,      CCTL,       // 'W'..='^'
    CWORD,     CBQUOTE,   CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '_'..='f'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'g'..='n'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'o'..='v'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CENDVAR,   CCTL,       // 'w'..='~'
    CWORD,                                                                                   // 127..=127
];

// syntax table used when in single quotes
pub static sqsyntax: Syntax = [
    CEOF,      CWORD,     CCTL,      CCTL,      CCTL,      CCTL,      CCTL,      CCTL,       // PEOF..=-122
    CCTL,      CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -121..=-114
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -113..=-106
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -105..=-98
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -97..=-90
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -89..=-82
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -81..=-74
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -73..=-66
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -65..=-58
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -57..=-50
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -49..=-42
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -41..=-34
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -33..=-26
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -25..=-18
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -17..=-10
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -9..=-2
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -1..=6
    CWORD,     CWORD,     CWORD,     CNL,       CWORD,     CWORD,     CWORD,     CWORD,      // 7..=14
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 15..=22
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 23..=30
    CWORD,     CWORD,     CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 31..='&'
    CENDQUOTE, CWORD,     CWORD,     CCTL,      CWORD,     CWORD,     CCTL,      CWORD,      // '\''..='.'
    CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '/'..='6'
    CWORD,     CWORD,     CWORD,     CCTL,      CWORD,     CWORD,     CCTL,      CWORD,      // '7'..='>'
    CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '?'..='F'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'G'..='N'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'O'..='V'
    CWORD,     CWORD,     CWORD,     CWORD,     CCTL,      CCTL,      CCTL,      CCTL,       // 'W'..='^'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '_'..='f'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'g'..='n'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'o'..='v'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CCTL,       // 'w'..='~'
    CWORD,                                                                                   // 127..=127
];

// syntax table used when in arithmetic
pub static arisyntax: Syntax = [
    CEOF,      CWORD,     CCTL,      CCTL,      CCTL,      CCTL,      CCTL,      CCTL,       // PEOF..=-122
    CCTL,      CCTL,      CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -121..=-114
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -113..=-106
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -105..=-98
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -97..=-90
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -89..=-82
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -81..=-74
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -73..=-66
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -65..=-58
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -57..=-50
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -49..=-42
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -41..=-34
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -33..=-26
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -25..=-18
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -17..=-10
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -9..=-2
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // -1..=6
    CWORD,     CWORD,     CWORD,     CNL,       CWORD,     CWORD,     CWORD,     CWORD,      // 7..=14
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 15..=22
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 23..=30
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CVAR,      CWORD,     CWORD,      // 31..='&'
    CWORD,     CLP,       CRP,       CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '\''..='.'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '/'..='6'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '7'..='>'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '?'..='F'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'G'..='N'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'O'..='V'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CBACK,     CWORD,     CWORD,      // 'W'..='^'
    CWORD,     CBQUOTE,   CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // '_'..='f'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'g'..='n'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,      // 'o'..='v'
    CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CWORD,     CENDVAR,   CWORD,      // 'w'..='~'
    CWORD,                                                                                   // 127..=127
];

// character classification table
pub static is_type: Syntax = [
    0,         0,         0,         0,         0,         0,         0,         0,          // PEOF..=-122
    0,         0,         0,         0,         0,         0,         0,         0,          // -121..=-114
    0,         0,         0,         0,         0,         0,         0,         0,          // -113..=-106
    0,         0,         0,         0,         0,         0,         0,         0,          // -105..=-98
    0,         0,         0,         0,         0,         0,         0,         0,          // -97..=-90
    0,         0,         0,         0,         0,         0,         0,         0,          // -89..=-82
    0,         0,         0,         0,         0,         0,         0,         0,          // -81..=-74
    0,         0,         0,         0,         0,         0,         0,         0,          // -73..=-66
    0,         0,         0,         0,         0,         0,         0,         0,          // -65..=-58
    0,         0,         0,         0,         0,         0,         0,         0,          // -57..=-50
    0,         0,         0,         0,         0,         0,         0,         0,          // -49..=-42
    0,         0,         0,         0,         0,         0,         0,         0,          // -41..=-34
    0,         0,         0,         0,         0,         0,         0,         0,          // -33..=-26
    0,         0,         0,         0,         0,         0,         0,         0,          // -25..=-18
    0,         0,         0,         0,         0,         0,         0,         0,          // -17..=-10
    0,         0,         0,         0,         0,         0,         0,         0,          // -9..=-2
    0,         0,         0,         0,         0,         0,         0,         0,          // -1..=6
    0,         0,         0,         0,         0,         0,         0,         0,          // 7..=14
    0,         0,         0,         0,         0,         0,         0,         0,          // 15..=22
    0,         0,         0,         0,         0,         0,         0,         0,          // 23..=30
    0,         0,         ISSPECL,   0,         ISSPECL,   ISSPECL,   0,         0,          // 31..='&'
    0,         0,         0,         ISSPECL,   0,         0,         ISSPECL,   0,          // '\''..='.'
    0,         ISDIGIT,   ISDIGIT,   ISDIGIT,   ISDIGIT,   ISDIGIT,   ISDIGIT,   ISDIGIT,    // '/'..='6'
    ISDIGIT,   ISDIGIT,   ISDIGIT,   0,         0,         0,         0,         0,          // '7'..='>'
    ISSPECL,   ISSPECL,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,    // '?'..='F'
    ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,    // 'G'..='N'
    ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,    // 'O'..='V'
    ISUPPER,   ISUPPER,   ISUPPER,   ISUPPER,   0,         0,         0,         0,          // 'W'..='^'
    ISUNDER,   0,         ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,    // '_'..='f'
    ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,    // 'g'..='n'
    ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,    // 'o'..='v'
    ISLOWER,   ISLOWER,   ISLOWER,   ISLOWER,   0,         0,         0,         0,          // 'w'..='~'
    0,                                                                                       // 127..=127
];

// ---------------------------------------------------------------------
// Provenance: the five tables against the C generator's own output.
//
// The module claims these are `mksyntax`'s output byte for byte. The
// reference build runs the real `mksyntax` and leaves `syntax.c` beside
// the binary the differential harness compares against, so the claim can
// be checked directly on the table the shell actually indexes rather than
// inferred from a second implementation of the generator.
//
// Where the reference build is absent -- a linked worktree, which shares
// no `tests/.build` with the checkout that built it -- there is nothing
// to compare against and the test says so and returns. `DASH_ROOT` points
// it at a checkout that has one.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// The C generator's output from the reference build, if built.
    fn reference(name: &str) -> Option<String> {
        let root = std::env::var("DASH_ROOT")
            .unwrap_or_else(|_| concat!(env!("CARGO_MANIFEST_DIR"), "/../..").to_string());
        std::fs::read_to_string(format!("{root}/tests/.build/ref/src/{name}")).ok()
    }

    /// The entries of `const char <name>[] = { … };`, in order.
    fn table_of(text: &str, name: &str) -> Vec<String> {
        let head = format!("const char {name}[] = {{");
        let start = text
            .find(&head)
            .unwrap_or_else(|| panic!("{name} not found in the generated syntax.c"))
            + head.len();
        let len = text[start..]
            .find("\n};")
            .unwrap_or_else(|| panic!("{name} is not terminated"));
        text[start..start + len]
            .split(',')
            .map(str::trim)
            .filter(|e| !e.is_empty())
            .map(str::to_string)
            .collect()
    }

    /// The generator emits class names, not numbers; `is_type` is filled
    /// with a literal `0` rather than a class, which is why that arm is
    /// here and not a missing case.
    fn value_of(sym: &str) -> c_char {
        match sym {
            "0" => 0,
            "CWORD" => CWORD,
            "CNL" => CNL,
            "CBACK" => CBACK,
            "CSQUOTE" => CSQUOTE,
            "CDQUOTE" => CDQUOTE,
            "CENDQUOTE" => CENDQUOTE,
            "CBQUOTE" => CBQUOTE,
            "CVAR" => CVAR,
            "CENDVAR" => CENDVAR,
            "CLP" => CLP,
            "CRP" => CRP,
            "CEOF" => CEOF,
            "CCTL" => CCTL,
            "CSPCL" => CSPCL,
            "ISDIGIT" => ISDIGIT,
            "ISUPPER" => ISUPPER,
            "ISLOWER" => ISLOWER,
            "ISUNDER" => ISUNDER,
            "ISSPECL" => ISSPECL,
            other => panic!("unknown syntax class {other} in the generated syntax.c"),
        }
    }

    #[test]
    fn the_tables_are_the_c_generators_output() {
        let text = match reference("syntax.c") {
            Some(t) => t,
            None => {
                eprintln!(
                    "note: tests/.build/ref absent, skipped the syntax.c comparison \
                     (run tests/build-reference.sh for the stronger assertion)"
                );
                return;
            }
        };
        for (name, ours) in [
            ("basesyntax", &basesyntax),
            ("dqsyntax", &dqsyntax),
            ("sqsyntax", &sqsyntax),
            ("arisyntax", &arisyntax),
            ("is_type", &is_type),
        ] {
            let theirs = table_of(&text, name);
            // 257 entries covering -129..=127. The C array is written
            // without a size and indexed through SYNBASE; `Syntax` pins
            // the length, so a short or long table fails here.
            assert_eq!(theirs.len(), 257, "{name}: entry count");
            for (i, sym) in theirs.iter().enumerate() {
                assert_eq!(ours[i], value_of(sym), "{name}[{i}] (C says {sym})");
            }
        }
    }
}
