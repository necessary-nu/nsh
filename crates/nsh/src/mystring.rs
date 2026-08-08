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

use bstr::BString;
use libc::{c_char, c_int, c_uchar, c_void, intmax_t, size_t};

use crate::output::VaArg;
use crate::shell::cstr;

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
pub const DOLATSTRLEN: usize = 6;
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

/* `#define qchars (cqchars + 1)` */
#[inline(always)]
pub fn qchars() -> *const c_char {
    unsafe { cqchars.as_ptr().add(1) }
}

/*
 * `#ifdef HAVE_FNMATCH` … — neither `--enable-fnmatch` nor
 * `--enable-glob` is on by default, so both knobs are 0 in the shipped
 * build.
 */
pub const FNMATCH_IS_ENABLED: c_int = 0;
pub const GLOB_IS_ENABLED: c_int = 0;

/* `#define equal(s1, s2) (strcmp(s1, s2) == 0)` */
#[inline(always)]
pub unsafe fn equal(s1: *const c_char, s2: *const c_char) -> bool {
    libc::strcmp(s1, s2) == 0
}

/* `#define scopy(s1, s2) ((void)strcpy(s2, s1))` */
#[inline(always)]
pub unsafe fn scopy(s1: *const c_char, s2: *mut c_char) {
    libc::strcpy(s2, s1);
}

/*
 * equal - #defined in mystring.h
 */

/*
 * scopy - #defined in mystring.h
 */

/*
 * The C body below is wrapped in `#if 0` and is not compiled; it is
 * carried here so the manifest symbol has a home, and it is never
 * called.
 *
 * scopyn - copy a string from "from" to "to", truncating the string
 *		if necessary.  "To" is always nul terminated, even if
 *		truncation is performed.  "Size" is the size of "to".
 */

// [spec:dash:def:mystring.scopyn-fn]
// [spec:dash:sem:mystring.scopyn-fn]
pub unsafe fn scopyn(from: *const c_char, to: *mut c_char, size: c_int) {
    let mut from = from;
    let mut to = to;
    let mut size = size;

    loop {
        size -= 1;
        if !(size > 0) {
            break;
        }
        let c = *from;
        from = from.add(1);
        *to = c;
        to = to.add(1);
        if c == 0 {
            return;
        }
    }
    *to = 0;
}

/*
 * prefix -- see if pfx is a prefix of string.
 */

// [spec:dash:def:mystring.prefix-fn]
// [spec:dash:sem:mystring.prefix-fn]
pub unsafe fn prefix(string: *const c_char, pfx: *const c_char) -> *mut c_char {
    let mut string = string;
    let mut pfx = pfx;

    while *pfx != 0 {
        let a = *pfx;
        pfx = pfx.add(1);
        let b = *string;
        string = string.add(1);
        if a != b {
            return core::ptr::null_mut();
        }
    }
    string as *mut c_char
}

// [spec:dash:def:mystring.badnum-fn]
// [spec:dash:sem:mystring.badnum-fn]
pub unsafe fn badnum(s: *const c_char) -> ! {
    crate::error::sh_error(illnum.as_ptr(), &[VaArg::Str(s)]);
}

/*
 * Convert a string into an integer of type intmax_t.  Alow trailing spaces.
 */
// [spec:dash:def:mystring.atomax-fn]
// [spec:dash:sem:mystring.atomax-fn]
pub unsafe fn atomax(s: *const c_char, base: c_int) -> intmax_t {
    let mut p: *mut c_char = core::ptr::null_mut();
    let r: intmax_t;

    *libc::__errno_location() = 0;
    r = crate::system::strtoimax(s, &mut p, base);

    /*
     * Disallow completely blank strings in non-arithmetic (base != 0)
     * contexts.
     */
    if p == s as *mut c_char && base != 0 {
        badnum(s);
    }

    while libc::isspace(*p as c_uchar as c_int) != 0 {
        p = p.add(1);
    }

    if *p != 0 {
        badnum(s);
    }

    r
}

// [spec:dash:def:mystring.atomax10-fn]
// [spec:dash:sem:mystring.atomax10-fn]
pub unsafe fn atomax10(s: *const c_char) -> intmax_t {
    atomax(s, 10)
}

/*
 * Convert a string of digits to an integer, printing an error message on
 * failure.
 */

// [spec:dash:def:mystring.number-fn]
// [spec:dash:sem:mystring.number-fn]
pub unsafe fn number(s: *const c_char) -> c_int {
    let n: intmax_t = atomax10(s);

    if n < 0 || n > c_int::MAX as intmax_t {
        badnum(s);
    }

    n as c_int
}

/*
 * Check for a valid number.  This should be elsewhere.
 */

// [spec:dash:def:mystring.is-number-fn]
// [spec:dash:sem:mystring.is-number-fn]
pub unsafe fn is_number(p: *const c_char) -> c_int {
    let mut p = p;

    loop {
        if !crate::syntax::is_digit(*p as c_int) {
            return 0;
        }
        p = p.add(1);
        if *p == 0 {
            break;
        }
    }
    1
}

/*
 * Produce a possibly single quoted string suitable as input to the shell.
 * The return string is allocated on the stack.
 */

// [spec:dash:def:mystring.single-quote-fn]
// [spec:dash:sem:mystring.single-quote-fn]
pub unsafe fn single_quote(s: *const c_char) -> *mut c_char {
    let mut s = s;
    /* The C leaves the result in the stack block without grabbing it, so
     * the next call overwrites it and every caller reads it before making
     * another. One buffer, reused, is that contract exactly. */
    let q = &mut *core::ptr::addr_of_mut!(quoted);
    q.clear();

    loop {
        let mut len: size_t;

        len = (crate::system::strchrnul(s, '\'' as c_int) as usize).wrapping_sub(s as usize);

        q.push(b'\'');
        q.extend_from_slice(core::slice::from_raw_parts(s as *const u8, len));
        q.push(b'\'');
        s = s.add(len);

        len = libc::strspn(s, cstr(b"'\0"));
        if len == 0 {
            break;
        }

        q.push(b'"');
        q.extend_from_slice(core::slice::from_raw_parts(s as *const u8, len));
        q.push(b'"');
        s = s.add(len);

        if *s == 0 {
            break;
        }
    }

    q.push(0);

    q.as_mut_ptr() as *mut c_char
}

/// [`single_quote`]'s result, which the C left in the stack block.
static mut quoted: BString = BString::new(Vec::new());

/*
 * Like strdup but works with the ash stack.
 */

// [spec:dash:def:mystring.sstrdup-fn]
// [spec:dash:sem:mystring.sstrdup-fn]
pub unsafe fn sstrdup(p: *const c_char) -> *mut c_char {
    let len: size_t = libc::strlen(p) + 1;
    libc::memcpy(crate::memalloc::stalloc(len), p as *const c_void, len) as *mut c_char
}

/*
 * Wrapper around strcmp for qsort/bsearch/...
 */
// [spec:dash:def:mystring.pstrcmp-fn]
// [spec:dash:sem:mystring.pstrcmp-fn]
pub unsafe extern "C" fn pstrcmp(a: *const c_void, b: *const c_void) -> c_int {
    libc::strcmp(
        *(a as *const *const c_char),
        *(b as *const *const c_char),
    )
}

/*
 * Find a string is in a sorted array.
 */
// [spec:dash:def:mystring.findstring-fn]
// [spec:dash:sem:mystring.findstring-fn]
pub unsafe fn findstring(
    s: *const c_char,
    array: *const *const c_char,
    nmemb: size_t,
) -> *const *const c_char {
    let s = s;
    crate::system::bsearch(
        &s as *const *const c_char as *const c_void,
        array as *const c_void,
        nmemb,
        core::mem::size_of::<*const c_char>(),
        pstrcmp,
    ) as *const *const c_char
}

// ---------------------------------------------------------------------
// Unit tests for this module's functions.
//
// These live inside the module because most of what the port exposes is
// private to its own file, exactly as the C's `STATIC` functions are.
// An external test crate would only reach the `pub` surface, and the
// manifest's obligation is per function, not per public API.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{raises, s, CStr0};

    // [spec:dash:sem:mystring.prefix-fn/test]
    #[test]
    fn prefix_returns_the_tail_or_null() {
        unsafe {
            let str_ = CStr0::new("foobar");
            assert_eq!(s(prefix(str_.p(), CStr0::new("foo").p())), "bar");
            // A complete match leaves an empty tail, which is still a
            // non-NULL pointer -- callers test the pointer, not the byte.
            assert_eq!(s(prefix(str_.p(), CStr0::new("foobar").p())), "");
            assert!(!prefix(str_.p(), CStr0::new("foobar").p()).is_null());
            assert!(prefix(str_.p(), CStr0::new("fox").p()).is_null());
            // An empty prefix matches anything and consumes nothing.
            assert_eq!(s(prefix(str_.p(), CStr0::new("").p())), "foobar");
            // A prefix longer than the string stops at the NUL.
            assert!(prefix(CStr0::new("fo").p(), CStr0::new("foo").p()).is_null());
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

    // [spec:dash:sem:mystring.atomax-fn/test]
    #[test]
    fn atomax_parses_in_base_and_allows_trailing_space() {
        let _g = crate::testutil::lock();
        unsafe {
            assert_eq!(atomax(CStr0::new("42").p(), 10), 42);
            assert_eq!(atomax(CStr0::new("-42").p(), 10), -42);
            assert_eq!(atomax(CStr0::new("ff").p(), 16), 255);
            assert_eq!(atomax(CStr0::new("777").p(), 8), 511);
            // "Alow trailing spaces" -- the comment's typo is in the C too.
            assert_eq!(atomax(CStr0::new("42   ").p(), 10), 42);
            assert_eq!(atomax(CStr0::new("42\t\n").p(), 10), 42);
        }
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    // [spec:dash:sem:mystring.badnum-fn/test]
    #[test]
    fn atomax_raises_through_badnum_on_junk() {
        let _g = crate::testutil::lock();
        unsafe {
            // Trailing junk is rejected...
            assert!(raises(|| {
                atomax(CStr0::new("42x").p(), 10);
            }));
            // ...and so is a wholly blank string, but only when base != 0.
            // At base 0 the blank check is skipped, which is what lets the
            // arithmetic lexer call this on an empty token.
            assert!(raises(|| {
                atomax(CStr0::new("").p(), 10);
            }));
            assert!(raises(|| {
                atomax(CStr0::new("   ").p(), 10);
            }));
            assert!(!raises(|| {
                assert_eq!(atomax(CStr0::new("").p(), 0), 0);
            }));
            // badnum itself never returns.
            assert!(raises(|| badnum(CStr0::new("zzz").p())));
        }
    }

    // [spec:dash:sem:mystring.atomax10-fn/test]
    #[test]
    fn atomax10_is_atomax_base_ten() {
        let _g = crate::testutil::lock();
        unsafe {
            assert_eq!(atomax10(CStr0::new("99").p()), 99);
            // Base 10, so a leading 0 is not octal and 0x is not hex.
            assert_eq!(atomax10(CStr0::new("010").p()), 10);
            assert!(raises(|| {
                atomax10(CStr0::new("0x10").p());
            }));
        }
    }

    // [spec:dash:sem:mystring.number-fn/test]
    #[test]
    fn number_is_atomax10_clamped_to_int() {
        let _g = crate::testutil::lock();
        unsafe {
            assert_eq!(number(CStr0::new("7").p()), 7);
            assert_eq!(
                number(CStr0::new(&c_int::MAX.to_string()).p()),
                c_int::MAX
            );
            // Negative and out-of-range both go through badnum.
            assert!(raises(|| {
                number(CStr0::new("-1").p());
            }));
            let too_big = (c_int::MAX as i64 + 1).to_string();
            assert!(raises(|| {
                number(CStr0::new(&too_big).p());
            }));
        }
    }

    // [spec:dash:sem:mystring.single-quote-fn/test]
    #[test]
    fn single_quote_produces_a_shell_requotable_string() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut mark: crate::memalloc::stackmark = core::mem::zeroed();
            crate::memalloc::setstackmark(&mut mark);
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
            crate::memalloc::popstackmark(&mut mark);
        }
    }

    // [spec:dash:sem:mystring.sstrdup-fn/test]
    #[test]
    fn sstrdup_copies_onto_the_shell_stack() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut mark: crate::memalloc::stackmark = core::mem::zeroed();
            crate::memalloc::setstackmark(&mut mark);
            let src = CStr0::new("hello");
            let copy = sstrdup(src.p());
            assert_eq!(s(copy), "hello");
            // A copy, not the original pointer.
            assert_ne!(copy as *const c_char, src.p());
            assert_eq!(s(sstrdup(CStr0::new("").p())), "");
            crate::memalloc::popstackmark(&mut mark);
        }
    }

    // [spec:dash:sem:mystring.pstrcmp-fn/test]
    #[test]
    fn pstrcmp_compares_through_two_levels_of_pointer() {
        unsafe {
            let a = CStr0::new("aaa");
            let b = CStr0::new("bbb");
            let (pa, pb) = (a.p(), b.p());
            let cmp = |x: &*const c_char, y: &*const c_char| {
                pstrcmp(
                    x as *const *const c_char as *const c_void,
                    y as *const *const c_char as *const c_void,
                )
            };
            assert!(cmp(&pa, &pb) < 0);
            assert!(cmp(&pb, &pa) > 0);
            assert_eq!(cmp(&pa, &pa), 0);
        }
    }

    // [spec:dash:sem:mystring.findstring-fn/test]
    #[test]
    fn findstring_bsearches_a_sorted_array() {
        unsafe {
            let owned = ["alpha", "delta", "kilo", "zulu"].map(CStr0::new);
            let array: Vec<*const c_char> = owned.iter().map(|c| c.p()).collect();
            let n = array.len() as size_t;

            for (i, name) in ["alpha", "delta", "kilo", "zulu"].iter().enumerate() {
                let hit = findstring(CStr0::new(name).p(), array.as_ptr(), n);
                assert!(!hit.is_null(), "{name} should be found");
                assert_eq!(s(*hit), *name);
                // The result points INTO the array, which is what callers
                // rely on to recover the index.
                assert_eq!(hit.offset_from(array.as_ptr()) as usize, i);
            }
            assert!(findstring(CStr0::new("mike").p(), array.as_ptr(), n).is_null());
            assert!(findstring(CStr0::new("").p(), array.as_ptr(), n).is_null());
        }
    }
}
