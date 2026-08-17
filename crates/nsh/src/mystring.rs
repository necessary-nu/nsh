//! Literal port of `src/mystring.c` / `src/mystring.h`.
//! Rules: `docs/spec/port/src/mystring.md`.
//!
//! String functions.
//!
//!	equal(s1, s2)		Return true if strings are equal.
//!	scopy(from, to)		Copy a string.
//!	scopyn(from, to, n)	Like scopy, but checks for overflow.
//!	number(s)		Convert a string of digits to an integer.
//!	is_number(s)		Return true if s is a string of digits.
//!
//! Eight of those exported items are gone, because `<[u8]>` spells them and
//! nothing called the port's copy. `equal` and `scopy` are `mystring.h`
//! macros that `parser.rs` and `exec.rs` each expanded for themselves;
//! `scopyn` is inside `#if 0` in the C; `sstrdup` copied onto
//! the region allocator that `delete-memalloc` emptied; `findstring` was a
//! `bsearch` over one sorted table, which `parser::findkwd` now does with
//! `binary_search_by`; `pstrcmp` existed only to adapt that search to C's
//! `bsearch`; and `DOLATSTRLEN` and `qchars` had no reader on either side.
//!
//! What is left is here because std is not the same function. Each
//! survivor says which.

use crate::error::Error;
use bstr::{BStr, BString};
use core::ffi::{c_char, c_int};

/*
 * C's `const char foo[] = "…"` becomes `[c_char; N]` here; this const fn
 * spells the conversion from a NUL-terminated byte string.
 */
const fn to_cchar<const N: usize>(b: &[u8; N]) -> [c_char; N] {
    let mut out = [0 as c_char; N];
    let mut i = 0;
    while i < N {
        out[i] = b[i] as c_char;
        i += 1;
    }
    out
}

/* Callers compare `nullstr` by *address*, not by content — `cd.rs:241`
 * distinguishes "unset" from "empty" that way — so it cannot become a
 * shared `b""` literal, which the compiler is free to coalesce. */
pub static nullstr: [c_char; 1] = [0]; /* zero length string */
pub static spcstr: [c_char; 2] = to_cchar(b" \0");
pub static snlfmt: [c_char; 4] = to_cchar(b"%s\n\0");
pub static dolatstr: [c_char; 7] = [
    crate::parser::CTLQUOTEMARK as c_char,
    crate::parser::CTLVAR as c_char,
    (crate::parser::VSNORMAL | crate::parser::VSBIT) as c_char,
    b'@' as c_char,
    b'=' as c_char,
    crate::parser::CTLQUOTEMARK as c_char,
    b'\0' as c_char,
];
pub static cqchars: [c_char; 5] = [
    b'\\' as c_char,
    crate::parser::CTLESC as c_char,
    crate::parser::CTLMBCHAR as c_char,
    crate::parser::CTLQUOTEMARK as c_char,
    0,
];
pub static illnum: [c_char; 19] = to_cchar(b"Illegal number: %s\0");
pub static homestr: [c_char; 5] = to_cchar(b"HOME\0");
pub static dotdir: [c_char; 2] = to_cchar(b".\0");

/*
 * `#ifdef HAVE_FNMATCH` … — neither `--enable-fnmatch` nor
 * `--enable-glob` is on by default, so both knobs are 0 in the shipped
 * build.
 */
pub const FNMATCH_IS_ENABLED: c_int = 0;
pub const GLOB_IS_ENABLED: c_int = 0;

/*
 * prefix -- see if pfx is a prefix of string.
 *
 * The callers still use the returned interior pointer as a parse position,
 * but the comparison itself is slice work: the offset of a successful
 * `<[u8]>::strip_prefix` is the same pointer the C loop returned.
 */

// `prefix` became `<[u8]>::strip_prefix` at both callers.
// [spec:dash:def:mystring.prefix-fn]
// [spec:dash:sem:mystring.prefix-fn]

// [spec:dash:def:mystring.badnum-fn]
// [spec:dash:sem:mystring.badnum-fn]
// The C's `badnum` does not return; here it builds the diagnostic and the
// caller's `?` does the leaving. Same bytes, same point, same funnel.
pub fn bad_number(sh: &mut crate::context::Shell, s: &BStr) -> Error {
    let mut message = b"Illegal number: ".to_vec();
    message.extend_from_slice(cstr_prefix(s.as_ref()));
    sh.sh_error_value(&message)
}

/*
 * Convert a string into an integer of type i64.  Alow trailing spaces.
 *
 * `str::parse` is not this function: `strtoimax` saturates at
 * `INTMAX_MAX` and reports `ERANGE` where `parse` returns `Err`, it takes
 * base 0 and — through the `__isoc23_` binding — `0b` literals, and it
 * stops at the first unconvertible byte instead of rejecting the string.
 * `docs/std-replacements.md` §5.8 measures all three.
 */
// [spec:dash:def:mystring.atomax-fn]
// [spec:dash:sem:mystring.atomax-fn]
pub fn parse_integer(
    sh: &mut crate::context::Shell,
    s: &BStr,
    requested_base: u32,
) -> Result<i64, Error> {
    debug_assert!(requested_base == 0 || (2..=36).contains(&requested_base));

    let bytes: &[u8] = cstr_prefix(s.as_ref()).as_ref();
    let mut pos = bytes.iter().position(|&b| !is_c_space(b)).unwrap_or(bytes.len());
    let number_start = pos;
    let negative = match bytes.get(pos) {
        Some(b'+') => {
            pos += 1;
            false
        }
        Some(b'-') => {
            pos += 1;
            true
        }
        _ => false,
    };

    let mut base = requested_base;
    if base == 0 {
        base = if bytes.get(pos) == Some(&b'0') {
            match (bytes.get(pos + 1), bytes.get(pos + 2).and_then(|b| digit_value(*b))) {
                (Some(b'x' | b'X'), Some(d)) if d < 16 => {
                    pos += 2;
                    16
                }
                (Some(b'b' | b'B'), Some(d)) if d < 2 => {
                    pos += 2;
                    2
                }
                _ => 8,
            }
        } else {
            10
        };
    } else if ((base == 16 && matches!(bytes.get(pos + 1), Some(b'x' | b'X')))
        || (base == 2 && matches!(bytes.get(pos + 1), Some(b'b' | b'B'))))
        && bytes.get(pos) == Some(&b'0')
        && bytes
            .get(pos + 2)
            .and_then(|b| digit_value(*b))
            .is_some_and(|d| d < base)
    {
        pos += 2;
    }

    let digits_start = pos;
    let limit = if negative {
        i64::MAX as u64 + 1
    } else {
        i64::MAX as u64
    };
    let mut magnitude = 0_u64;
    while let Some(digit) = bytes.get(pos).and_then(|b| digit_value(*b)) {
        if digit >= base {
            break;
        }
        magnitude = magnitude
            .saturating_mul(base as u64)
            .saturating_add(digit as u64)
            .min(limit);
        pos += 1;
    }

    if pos == digits_start {
        // The base-zero caller deliberately accepts a wholly blank value as
        // zero. That is the one oddity the original `strtoimax` adapter
        // exposed to arithmetic variable lookup.
        if requested_base == 0
            && number_start == bytes.len()
            && bytes[..number_start].iter().all(|&b| is_c_space(b))
        {
            return Ok(0);
        }
        return Err(bad_number(sh, s));
    }

    while bytes.get(pos).is_some_and(|&b| is_c_space(b)) {
        pos += 1;
    }
    if pos != bytes.len() {
        return Err(bad_number(sh, s));
    }

    Ok(if negative {
        if magnitude == i64::MAX as u64 + 1 {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else {
        magnitude as i64
    })
}

fn digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u32),
        b'a'..=b'z' => Some((byte - b'a' + 10) as u32),
        b'A'..=b'Z' => Some((byte - b'A' + 10) as u32),
        _ => None,
    }
}

fn is_c_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

/*
 * Convert a string of digits to an integer, printing an error message on
 * failure.
 */

// [spec:dash:def:mystring.number-fn]
// [spec:dash:sem:mystring.number-fn]
pub fn number(sh: &mut crate::context::Shell, s: &BStr) -> Result<c_int, Error> {
    let n = parse_integer(sh, s, 10)?;

    if n < 0 || n > c_int::MAX as i64 {
        return Err(bad_number(sh, s));
    }

    Ok(n as c_int)
}

/*
 * Check for a valid number.  This should be elsewhere.
 *
 * `iter().all(is_digit)` is the wrong answer, not a shorter one: the C
 * loop is do/while, so the empty string tests `is_digit(0)` and returns
 * 0, where `all` on an empty slice returns true.  Four callers
 * (`jobs.rs:995`, `histedit.rs:782,808`, `trap.rs:516`) treat a
 * non-number as "this is a name", so the empty case decides which.
 */

// [spec:dash:def:mystring.is-number-fn]
// [spec:dash:sem:mystring.is-number-fn]
pub fn is_number(p: &BStr) -> bool {
    !p.is_empty() && p.iter().all(u8::is_ascii_digit)
}

pub fn decimal_digits(p: &BStr) -> Option<u64> {
    is_number(p).then(|| {
        p.iter().fold(0_u64, |value, byte| {
            value.saturating_mul(10).saturating_add((byte - b'0') as u64)
        })
    })
}

/*
 * The bytes `strlen` would count, over a slice that is already owned.
 *
 * This is the shape every `CStr::from_ptr(p).to_bytes()` in the port takes
 * once `p` stops being a pointer, and it is *not* "drop the last byte".
 * The buffers this port carries — `strlist::text`, `NodeText`, the
 * expansion buffer — hold a counted terminating NUL, so dropping the last
 * byte agrees whenever the terminator is the only NUL. It stops agreeing
 * the moment the bytes hold an embedded one, and the port reaches those:
 * `read` escapes a NUL out of its input, and a here-document body can
 * carry one. `strlen` stops at the *first* NUL, and so does this.
 *
 * Safe, and that is the point — the `CStr::from_ptr` it replaces is not.
 */
pub fn cstr_prefix(b: &[u8]) -> &bstr::BStr {
    use bstr::ByteSlice;
    let n = b.find_byte(0).unwrap_or(b.len());
    b[..n].as_bstr()
}

/*
 * `strncmp(a, b, n) == 0`, which is the only question either caller asks
 * of `strncmp`, and not a slice compare.
 *
 * The difference is how far it reads. `strncmp` stops at the first shared
 * NUL, and `expand.rs`'s `pmatch` relies on that: the note there records
 * that `mbs` points at a *single stack byte* when the string character is
 * multibyte, and the C gets away with asking for `n` bytes from it only
 * because the comparison ends at the terminator. `from_raw_parts(.., n)`
 * on either side would read `n` unconditionally and turn a reproduced
 * over-read into a certain one, so the loop is here rather than a
 * `<[u8]>` method.
 */
/*
 * Produce a possibly single quoted string suitable as input to the shell.
 * The return string is allocated on the stack.
 *
 * Shell quoting, not a std facility: the alternation between `'…'` and
 * `"…"` runs is what makes the result readable back by the shell, and no
 * escaping API in std produces it.  The scan itself is slice work, so the
 * C's `strchrnul` and `strspn` are gone.
 */

// [spec:dash:def:mystring.single-quote-fn]
// [spec:dash:sem:mystring.single-quote-fn]
pub fn single_quote(mut s: &BStr) -> BString {
    let mut q = BString::new(Vec::new());

    loop {
        let len = s.iter().position(|&c| c == b'\'').unwrap_or(s.len());

        q.push(b'\'');
        q.extend_from_slice(&s[..len]);
        q.push(b'\'');
        s = &s[len..];

        let len = s.iter().position(|&c| c != b'\'').unwrap_or(s.len());
        if len == 0 {
            break;
        }

        q.push(b'"');
        q.extend_from_slice(&s[..len]);
        q.push(b'"');
        s = &s[len..];

        if s.is_empty() {
            break;
        }
    }

    q
}

// ---------------------------------------------------------------------
// Unit tests for this module's functions.
//
// These live inside the module because most of what the port exposes is
// private to its own file, exactly as the C's `STATIC` functions are.
// An external test crate would only reach the `pub` surface, and the
// manifest's obligation is per function, not per public API.
// ---------------------------------------------------------------------
/// Copy an already-rendered ASCII number into one of the fixed C buffers
/// retained by the variable ABI.
pub(crate) fn copy_ascii_cstr(out: &mut [c_char], text: &str) {
    debug_assert!(text.is_ascii());
    debug_assert!(text.len() < out.len());
    let copied = text.len().min(out.len().saturating_sub(1));
    for (slot, byte) in out.iter_mut().zip(text.bytes()).take(copied) {
        *slot = byte as c_char;
    }
    if let Some(terminator) = out.get_mut(copied) {
        *terminator = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:dash:sem:mystring.is-number-fn/test]
    #[test]
    fn is_number_accepts_only_all_digits() {
        assert!(is_number(BStr::new("0")));
        assert!(is_number(BStr::new("12345")));
        assert!(!is_number(BStr::new("")));
        assert!(!is_number(BStr::new("12a")));
        assert!(!is_number(BStr::new("a12")));
        assert!(!is_number(BStr::new("-1")));
        assert!(!is_number(BStr::new("+1")));
        assert!(!is_number(BStr::new(" 1")));
    }

    // [spec:dash:sem:mystring.is-number-fn/test]
    #[test]
    fn is_number_matches_ascii_digits() {
        assert!(!is_number(BStr::new(b"")));

        for byte in 1_u8..=u8::MAX {
            assert_eq!(
                is_number(BStr::new(&[byte])),
                byte.is_ascii_digit(),
                "classification differed for byte 0x{byte:02x}"
            );
        }

        let all_digits = b"0123456789";
        assert!(is_number(BStr::new(all_digits)));
        for i in 0..all_digits.len() {
            let mut candidate = *all_digits;
            candidate[i] = b'x';
            assert!(!is_number(BStr::new(&candidate)));
        }
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    #[test]
    fn atomax_parses_in_base_and_allows_trailing_space() {
        let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned_sh;
        let _g = crate::testutil::lock();
        assert_eq!(parse_integer(sh, BStr::new("42"), 10).unwrap(), 42);
        assert_eq!(parse_integer(sh, BStr::new("-42"), 10).unwrap(), -42);
        assert_eq!(parse_integer(sh, BStr::new("ff"), 16).unwrap(), 255);
        assert_eq!(parse_integer(sh, BStr::new("777"), 8).unwrap(), 511);
        // "Alow trailing spaces" -- the comment's typo is in the C too.
        assert_eq!(parse_integer(sh, BStr::new("42   "), 10).unwrap(), 42);
        assert_eq!(parse_integer(sh, BStr::new("42\t\n"), 10).unwrap(), 42);
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    // [spec:dash:sem:mystring.badnum-fn/test]
    #[test]
    fn atomax_raises_through_badnum_on_junk() {
        let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned_sh;
        let _g = crate::testutil::lock();
        // Trailing junk is rejected, and the diagnostic is the value
        // now rather than an unwind, so the test can read it.
        let e = parse_integer(sh, BStr::new("42x"), 10).expect_err("trailing junk");
        assert_eq!(e.message().to_vec(), b"Illegal number: 42x".to_vec());
        // ...and so is a wholly blank string, but only when base != 0.
        // At base 0 the blank check is skipped, which is what lets
        // arithmetic variable lookup treat an unset value as zero.
        assert!(parse_integer(sh, BStr::new(""), 10).is_err());
        assert!(parse_integer(sh, BStr::new("   "), 10).is_err());
        assert_eq!(parse_integer(sh, BStr::new(""), 0).unwrap(), 0);
        assert_eq!(parse_integer(sh, BStr::new("   "), 0).unwrap(), 0);
        // bad_number builds the diagnostic rather than raising it.
        assert_eq!(
            bad_number(sh, BStr::new("zzz")).message().to_vec(),
            b"Illegal number: zzz".to_vec()
        );
    }

    // [spec:dash:sem:mystring.atomax10-fn/test]
    #[test]
    fn atomax10_is_atomax_base_ten() {
        let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned_sh;
        let _g = crate::testutil::lock();
        assert_eq!(parse_integer(sh, BStr::new("99"), 10).unwrap(), 99);
        // Base 10, so a leading 0 is not octal and 0x is not hex.
        assert_eq!(parse_integer(sh, BStr::new("010"), 10).unwrap(), 10);
        assert!(parse_integer(sh, BStr::new("0x10"), 10).is_err());
    }

    // [spec:dash:sem:mystring.number-fn/test]
    #[test]
    fn number_is_atomax10_clamped_to_int() {
        let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned_sh;
        let _g = crate::testutil::lock();
        assert_eq!(number(sh, BStr::new("7")).unwrap(), 7);
        assert_eq!(
            number(sh, BStr::new(c_int::MAX.to_string().as_bytes())).unwrap(),
            c_int::MAX
        );
        // Negative and out-of-range both go through bad_number.
        assert!(number(sh, BStr::new("-1")).is_err());
        let too_big = (c_int::MAX as i64 + 1).to_string();
        assert!(number(sh, BStr::new(too_big.as_bytes())).is_err());
    }

    // [spec:dash:sem:mystring.single-quote-fn/test]
    #[test]
    fn single_quote_produces_a_shell_requotable_string() {
        assert_eq!(single_quote(BStr::new(b"abc")), b"'abc'".as_slice());
        assert_eq!(single_quote(BStr::new(b"")), b"''".as_slice());
        // An embedded quote has to leave the single-quoted run and come
        // back. dash uses the '"'"' form, not a backslash escape.
        assert_eq!(single_quote(BStr::new(b"a'b")), b"'a'\"'\"'b'".as_slice());
        // A lone quote gives ''"'" -- dash stops without reopening a
        // trailing empty pair of quotes.
        assert_eq!(single_quote(BStr::new(b"'")), b"''\"'\"".as_slice());
        assert_eq!(
            single_quote(BStr::new(b"a b|c$d")),
            b"'a b|c$d'".as_slice()
        );
    }

    // [spec:dash:sem:mystring.single-quote-fn/test]
    #[test]
    fn single_quote_handles_all_bytes() {
        for byte in 1_u8..=u8::MAX {
            let input = [byte];
            let actual = single_quote(BStr::new(&input));

            if byte == b'\'' {
                assert_eq!(actual, b"''\"'\"".as_slice());
            } else {
                assert_eq!(actual.len(), 3);
                assert_eq!(actual[0], b'\'');
                assert_eq!(actual[1], byte);
                assert_eq!(actual[2], b'\'');
            }
        }
    }
}

/* -------------------------------------------------------------------
 * Reading a NUL-terminated byte string as a slice.
 *
 * `pmatch` and its helpers walk two counted strings that still carry
 * their terminator (`to_bytes_with_nul`), so an index landing on the
 * terminator reads 0 exactly as the C's `*p` did and the loops need no
 * separate length test.
 *
 * `byte_at` answers 0 past the terminator as well.  The C read whatever
 * followed the allocation there; a well-formed pattern or string stops
 * at its terminator long before, so the two agree wherever the C was
 * defined, and where it was not this one at least gives the same answer
 * twice.  The signed variant exists for the two `p[-1]` reads in
 * `pmatch`'s CTLMBCHAR arms, which are in bounds whenever both sides are
 * multibyte-encoded and read before the buffer when only one is.
 * ------------------------------------------------------------------- */

#[inline]
pub(crate) fn byte_at(s: &[u8], i: usize) -> c_char {
    match s.get(i) {
        Some(&b) => b as c_char,
        None => 0,
    }
}

#[inline]
pub(crate) fn byte_at_i(s: &[u8], i: isize) -> c_char {
    if i < 0 { 0 } else { byte_at(s, i as usize) }
}

#[inline]
pub(crate) fn slice_from(s: &[u8], i: usize) -> &[u8] {
    match s.get(i..) {
        Some(t) => t,
        None => &[],
    }
}

// `strncmp(a + ai, b + bi, n) == 0`, stopping at the first difference and
// at a shared NUL, which is what `mystring::ncmp_eq` does for pointers.
pub(crate) fn ncmp_eq_at(a: &[u8], ai: isize, b: &[u8], bi: isize, n: usize) -> bool {
    for k in 0..n as isize {
        let x = byte_at_i(a, ai + k);
        if x != byte_at_i(b, bi + k) {
            return false;
        }
        if x == 0 {
            break;
        }
    }
    true
}
