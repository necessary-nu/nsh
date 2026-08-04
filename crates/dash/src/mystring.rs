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
    let mut p: *mut c_char;
    let mut s = s;

    crate::STARTSTACKSTR!(p);

    loop {
        let mut q: *mut c_char;
        let mut len: size_t;

        len = (crate::system::strchrnul(s, '\'' as c_int) as usize).wrapping_sub(s as usize);

        p = crate::memalloc::makestrspace(len + 3, p);
        q = p;

        *q = '\'' as c_char;
        q = q.add(1);
        q = crate::system::mempcpy(q as *mut c_void, s as *const c_void, len) as *mut c_char;
        *q = '\'' as c_char;
        q = q.add(1);
        s = s.add(len);

        crate::STADJUST!((q as usize).wrapping_sub(p as usize), p);

        len = libc::strspn(s, cstr(b"'\0"));
        if len == 0 {
            break;
        }

        p = crate::memalloc::makestrspace(len + 3, p);
        q = p;

        *q = '"' as c_char;
        q = q.add(1);
        q = crate::system::mempcpy(q as *mut c_void, s as *const c_void, len) as *mut c_char;
        *q = '"' as c_char;
        q = q.add(1);
        s = s.add(len);

        crate::STADJUST!((q as usize).wrapping_sub(p as usize), p);

        if *s == 0 {
            break;
        }
    }

    crate::USTPUTC!(0, p);

    crate::memalloc::stackblock() as *mut c_char
}

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
