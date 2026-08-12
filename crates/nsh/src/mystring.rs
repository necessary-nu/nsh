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
use bstr::BString;
use libc::{c_char, c_int, c_uchar, intmax_t};

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
pub static mut nullstr: [c_char; 1] = [0]; /* zero length string */
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

// [spec:dash:def:mystring.prefix-fn]
// [spec:dash:sem:mystring.prefix-fn]
pub unsafe fn prefix(string: *const c_char, pfx: *const c_char) -> *mut c_char {
    let string_bytes = core::ffi::CStr::from_ptr(string).to_bytes();
    let prefix_bytes = core::ffi::CStr::from_ptr(pfx).to_bytes();
    if string_bytes.strip_prefix(prefix_bytes).is_some() {
        string.add(prefix_bytes.len()) as *mut c_char
    } else {
        core::ptr::null_mut()
    }
}

// [spec:dash:def:mystring.badnum-fn]
// [spec:dash:sem:mystring.badnum-fn]
// The C's `badnum` does not return; here it builds the diagnostic and the
// caller's `?` does the leaving. Same bytes, same point, same funnel.
pub unsafe fn badnum(s: *const c_char) -> Error {
    let mut message = b"Illegal number: ".to_vec();
    message.extend_from_slice(core::ffi::CStr::from_ptr(s).to_bytes());
    crate::error::sh_error_value(&message)
}

/*
 * Convert a string into an integer of type intmax_t.  Alow trailing spaces.
 *
 * `str::parse` is not this function: `strtoimax` saturates at
 * `INTMAX_MAX` and reports `ERANGE` where `parse` returns `Err`, it takes
 * base 0 and — through the `__isoc23_` binding — `0b` literals, and it
 * stops at the first unconvertible byte instead of rejecting the string.
 * `docs/std-replacements.md` §5.8 measures all three.
 */
// [spec:dash:def:mystring.atomax-fn]
// [spec:dash:sem:mystring.atomax-fn]
pub unsafe fn atomax(s: *const c_char, base: c_int) -> Result<intmax_t, Error> {
    let mut p: *mut c_char = core::ptr::null_mut();
    let r: intmax_t;

    *libc::__errno_location() = 0;
    r = crate::system::strtoimax(s, &mut p, base);

    /*
     * Disallow completely blank strings in non-arithmetic (base != 0)
     * contexts.
     */
    if p == s as *mut c_char && base != 0 {
        return Err(badnum(s));
    }

    /*
     * `u8::is_ascii_whitespace` is not `isspace`: it excludes vertical
     * tab, and `exit $'1\v'` exits 1 in both shells, so the substitution
     * is observable from two tokens of shell.  Measured across `C`,
     * `en_US.utf8` and a generated `en_US.ISO-8859-1`, no byte 0x80-0xFF
     * is in the space class, so the locale is not what keeps this call —
     * 0x0B is.
     */
    while libc::isspace(*p as c_uchar as c_int) != 0 {
        p = p.add(1);
    }

    if *p != 0 {
        return Err(badnum(s));
    }

    Ok(r)
}

// [spec:dash:def:mystring.atomax10-fn]
// [spec:dash:sem:mystring.atomax10-fn]
pub unsafe fn atomax10(s: *const c_char) -> Result<intmax_t, Error> {
    atomax(s, 10)
}

/*
 * Convert a string of digits to an integer, printing an error message on
 * failure.
 */

// [spec:dash:def:mystring.number-fn]
// [spec:dash:sem:mystring.number-fn]
pub unsafe fn number(s: *const c_char) -> Result<c_int, Error> {
    let n: intmax_t = atomax10(s)?;

    if n < 0 || n > c_int::MAX as intmax_t {
        return Err(badnum(s));
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
pub unsafe fn is_number(p: *const c_char) -> c_int {
    let bytes = core::ffi::CStr::from_ptr(p).to_bytes();
    c_int::from(!bytes.is_empty() && bytes.iter().all(u8::is_ascii_digit))
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
pub unsafe fn ncmp_eq(a: *const c_char, b: *const c_char, n: usize) -> bool {
    for i in 0..n {
        let x = *a.add(i);
        if x != *b.add(i) {
            return false;
        }
        if x == 0 {
            break;
        }
    }
    true
}

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
pub unsafe fn single_quote(s: *const c_char) -> *mut c_char {
    let mut s = core::ffi::CStr::from_ptr(s).to_bytes();
    /* The C leaves the result in the stack block without grabbing it, so
     * the next call overwrites it and every caller reads it before making
     * another. One buffer, reused, is that contract exactly. */
    let q = &mut *core::ptr::addr_of_mut!(quoted);
    q.clear();

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

    q.push(0);

    q.as_mut_ptr() as *mut c_char
}

/// [`single_quote`]'s result, which the C left in the stack block.
static mut quoted: BString = BString::new(Vec::new());

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
pub(crate) unsafe fn copy_ascii_cstr(out: *mut c_char, capacity: usize, text: &str) {
    debug_assert!(text.is_ascii());
    debug_assert!(text.len() < capacity);
    let copied = text.len().min(capacity.saturating_sub(1));
    core::ptr::copy_nonoverlapping(text.as_ptr(), out as *mut u8, copied);
    if capacity != 0 {
        *out.add(copied) = 0;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{CStr0, s};

    // [spec:dash:sem:mystring.prefix-fn/test]
    #[test]
    fn prefix_returns_the_tail_or_null() {
        unsafe {
            let str_ = CStr0::new("foobar");
            let tail = prefix(str_.p(), CStr0::new("foo").p());
            assert!(!tail.is_null());
            assert_eq!(s(tail), "bar");
            // A complete match leaves an empty tail, which is still a
            // non-NULL pointer -- callers test the pointer, not the byte.
            assert_eq!(s(prefix(str_.p(), CStr0::new("foobar").p())), "");
            assert!(!prefix(str_.p(), CStr0::new("foobar").p()).is_null());
            assert!(prefix(str_.p(), CStr0::new("fox").p()).is_null());
            // An empty prefix matches anything and consumes nothing.
            let tail = prefix(str_.p(), CStr0::new("").p());
            assert!(!tail.is_null());
            assert_eq!(s(tail), "foobar");
            // A prefix longer than the string stops at the NUL.
            assert!(prefix(CStr0::new("fo").p(), CStr0::new("foo").p()).is_null());
        }
    }

    // [spec:dash:sem:mystring.prefix-fn/test]
    #[test]
    fn prefix_handles_all_non_nul_bytes() {
        unsafe {
            for byte in 1_u8..=u8::MAX {
                let haystack = [byte, b'x', 0];
                let matching = [byte, 0];
                let tail = prefix(haystack.as_ptr().cast(), matching.as_ptr().cast());

                assert_eq!(tail.cast_const().cast::<u8>(), haystack.as_ptr().add(1));
                assert_eq!(core::ffi::CStr::from_ptr(tail).to_bytes(), b"x");

                let different = if byte == 1 { 2 } else { 1 };
                let missing = [different, 0];
                assert!(prefix(haystack.as_ptr().cast(), missing.as_ptr().cast()).is_null());
            }
        }
    }

    // [spec:dash:sem:mystring.is-number-fn/test]
    #[test]
    fn is_number_accepts_only_all_digits() {
        unsafe {
            assert_eq!(is_number(CStr0::new("0").p()), 1);
            assert_eq!(is_number(CStr0::new("12345").p()), 1);
            assert_eq!(is_number(CStr0::new("").p()), 0);
            assert_eq!(is_number(CStr0::new("12a").p()), 0);
            assert_eq!(is_number(CStr0::new("a12").p()), 0);
            // No sign is accepted: this is a digit test, not a number
            // parser, which is why `number()` exists separately.
            assert_eq!(is_number(CStr0::new("-1").p()), 0);
            assert_eq!(is_number(CStr0::new("+1").p()), 0);
            assert_eq!(is_number(CStr0::new(" 1").p()), 0);
        }
    }

    // [spec:dash:sem:mystring.is-number-fn/test]
    #[test]
    fn is_number_matches_ascii_digits() {
        unsafe {
            let empty = [0_u8];
            assert_eq!(is_number(empty.as_ptr().cast()), 0);

            for byte in 1_u8..=u8::MAX {
                let candidate = [byte, 0];
                assert_eq!(
                    is_number(candidate.as_ptr().cast()),
                    c_int::from(byte.is_ascii_digit()),
                    "classification differed for byte 0x{byte:02x}"
                );
            }

            let all_digits = b"0123456789\0";
            assert_eq!(is_number(all_digits.as_ptr().cast()), 1);
            for i in 0..all_digits.len() - 1 {
                let mut candidate = *all_digits;
                candidate[i] = b'x';
                assert_eq!(is_number(candidate.as_ptr().cast()), 0);
            }
        }
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    #[test]
    fn atomax_parses_in_base_and_allows_trailing_space() {
        let _g = crate::testutil::lock();
        unsafe {
            assert_eq!(atomax(CStr0::new("42").p(), 10).unwrap(), 42);
            assert_eq!(atomax(CStr0::new("-42").p(), 10).unwrap(), -42);
            assert_eq!(atomax(CStr0::new("ff").p(), 16).unwrap(), 255);
            assert_eq!(atomax(CStr0::new("777").p(), 8).unwrap(), 511);
            // "Alow trailing spaces" -- the comment's typo is in the C too.
            assert_eq!(atomax(CStr0::new("42   ").p(), 10).unwrap(), 42);
            assert_eq!(atomax(CStr0::new("42\t\n").p(), 10).unwrap(), 42);
        }
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    // [spec:dash:sem:mystring.badnum-fn/test]
    #[test]
    fn atomax_raises_through_badnum_on_junk() {
        let _g = crate::testutil::lock();
        unsafe {
            // Trailing junk is rejected, and the diagnostic is the value
            // now rather than an unwind, so the test can read it.
            let e = atomax(CStr0::new("42x").p(), 10).expect_err("trailing junk");
            assert_eq!(e.message().to_vec(), b"Illegal number: 42x".to_vec());
            // ...and so is a wholly blank string, but only when base != 0.
            // At base 0 the blank check is skipped, which is what lets the
            // arithmetic lexer call this on an empty token.
            assert!(atomax(CStr0::new("").p(), 10).is_err());
            assert!(atomax(CStr0::new("   ").p(), 10).is_err());
            assert_eq!(atomax(CStr0::new("").p(), 0).unwrap(), 0);
            // badnum builds the diagnostic rather than raising it.
            assert_eq!(
                badnum(CStr0::new("zzz").p()).message().to_vec(),
                b"Illegal number: zzz".to_vec()
            );
        }
    }

    // [spec:dash:sem:mystring.atomax10-fn/test]
    #[test]
    fn atomax10_is_atomax_base_ten() {
        let _g = crate::testutil::lock();
        unsafe {
            assert_eq!(atomax10(CStr0::new("99").p()).unwrap(), 99);
            // Base 10, so a leading 0 is not octal and 0x is not hex.
            assert_eq!(atomax10(CStr0::new("010").p()).unwrap(), 10);
            assert!(atomax10(CStr0::new("0x10").p()).is_err());
        }
    }

    // [spec:dash:sem:mystring.number-fn/test]
    #[test]
    fn number_is_atomax10_clamped_to_int() {
        let _g = crate::testutil::lock();
        unsafe {
            assert_eq!(number(CStr0::new("7").p()).unwrap(), 7);
            assert_eq!(
                number(CStr0::new(&c_int::MAX.to_string()).p()).unwrap(),
                c_int::MAX
            );
            // Negative and out-of-range both go through badnum.
            assert!(number(CStr0::new("-1").p()).is_err());
            let too_big = (c_int::MAX as i64 + 1).to_string();
            assert!(number(CStr0::new(&too_big).p()).is_err());
        }
    }

    // [spec:dash:sem:mystring.single-quote-fn/test]
    #[test]
    fn single_quote_produces_a_shell_requotable_string() {
        let _g = crate::testutil::lock();
        unsafe {
            assert_eq!(s(single_quote(CStr0::new("abc").p())), "'abc'");
            assert_eq!(s(single_quote(CStr0::new("").p())), "''");
            // An embedded quote has to leave the single-quoted run and
            // come back. dash uses the '"'"' form, NOT a backslash
            // escape -- verified against the C, whose `set` prints
            // A='a'"'"'b' for the same value.
            assert_eq!(s(single_quote(CStr0::new("a'b").p())), "'a'\"'\"'b'");
            // A lone quote gives ''"'" -- dash stops without reopening a
            // trailing empty '' , which both shells agree on:
            //     A="'" ; set  =>  A=''"'"
            assert_eq!(s(single_quote(CStr0::new("'").p())), "''\"'\"");
            // Everything else, blanks and metacharacters included, is
            // literal inside the quotes.
            assert_eq!(s(single_quote(CStr0::new("a b|c$d").p())), "'a b|c$d'");
        }
    }

    // [spec:dash:sem:mystring.single-quote-fn/test]
    #[test]
    fn single_quote_handles_all_bytes() {
        let _g = crate::testutil::lock();
        unsafe {
            for byte in 1_u8..=u8::MAX {
                let input = [byte, 0];
                let actual =
                    core::ffi::CStr::from_ptr(single_quote(input.as_ptr().cast())).to_bytes();

                if byte == b'\'' {
                    assert_eq!(actual, b"''\"'\"");
                } else {
                    assert_eq!(actual.len(), 3);
                    assert_eq!(actual[0], b'\'');
                    assert_eq!(actual[1], byte);
                    assert_eq!(actual[2], b'\'');
                }
            }
        }
    }
}
