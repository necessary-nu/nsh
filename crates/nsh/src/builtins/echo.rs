//! The `echo` builtin.
//!
//! Port of the `echo` half of `src/bltin/printf.c`.
//! Rules: `docs/spec/port/src/bltin/printf.md`.
//!
//! The other half of that C file was the `printf` utility, and this shell
//! does not have one: the utility's contract *is* a runtime
//! `%`-conversion interpreter, and nsh carries no such machinery
//! anywhere. Output is an `io::Write` and formatting happens through
//! `write!` at call sites where the arguments already have types.
//! `printf` resolves through `PATH` like any other external utility. See
//! `[dec:nsh:no-format-interpreters]`.
//!
//! What that removed: `printfcmd` and its scanning loop, `mklong`, the
//! `PF`/`ASPF` arity switch, the `libc::snprintf` bridge behind them, the
//! four `get*` argument readers and `check_conversion`.
//! `print_escape_str` kept only what `echo` needs -- the C passed it a
//! format string of which two bytes ever mattered, and it takes those
//! directly now.
//!
//! The escape decoding itself is [`crate::escape`], because the parser
//! shares it: `$\'...\'` is the same decoder with `mbchar` set.

use core::ptr;
use std::io::Write as _;

use bstr::{BStr, BString};
use libc::{c_char, c_int, c_uint};

use crate::escape::{CONV_ESCAPE_SLOP, conv_escape, isodigit};

#[inline]
unsafe fn nullstr() -> *mut c_char {
    ptr::addr_of!(crate::mystring::nullstr) as *const c_char as *mut c_char
}


/// Write one rendered conversion to standard output.
unsafe fn emit(bytes: &[u8]) {
    let _ = (&mut *crate::output::stdout()).write_all(bytes);
}

/// Expand `echo`'s escapes and write the result, followed by `separator`
/// unless a `\c` stopped the conversion.
///
/// The C took a format string and three of its bytes meant something:
/// `f[1]` said whether the conversion character sat right after the `%`,
/// and `f[2]` was the byte to append — `echo`'s space or its closing
/// newline. `echo` is the only caller left and it only ever passed `%s`,
/// `%s ` or `%s\n`, so it passes the byte itself.
// [spec:dash:def:printf.print-escape-str-fn]
// [spec:dash:sem:printf.print-escape-str-fn]
unsafe fn print_escape_str(separator: u8, s: *mut c_char) -> c_int {
    let done: c_int;
    /* The C's `q` is a cursor into the stack block and `stackblock()` its
     * base.  Both are this buffer: `len` is its length and `q[-1]` its
     * last byte. */
    let mut buf = BString::default();

    done = conv_escape_str(s, &mut buf);
    let len = buf.len();

    /* `conv_escape_str`'s do-while exits only on the iteration that writes
     * the terminating NUL, so `q[-1]` always exists. */
    debug_assert!(len >= 1);

    /* `q[-1] = (!!((f[1] - 's') | done) - 1) & f[2];` — the separator
     * overwrites the terminator, and a `\c` suppresses it. */
    let close = if done == 0 { separator } else { 0 };
    buf[len - 1] = close;
    let total = len - 1 + (close != 0) as usize;

    emit(&buf[..total]);

    done
}

/*
 * Print SysV echo(1) style escape string
 *	Halts processing string if a \c escape is encountered.
 */
// [spec:dash:def:printf.conv-escape-str-fn]
// [spec:dash:sem:printf.conv-escape-str-fn]
unsafe fn conv_escape_str(mut str: *mut c_char, cp: &mut BString) -> c_int {
    let mut c: c_int;

    /* convert string into a temporary buffer... */
    /* `STARTSTACKSTR(cp)` — the buffer is the caller's, and the C's `*sp =
     * cp` at the end is its length. */
    debug_assert!(cp.is_empty());

    loop {
        let ret: c_uint;
        let ch: c_int;

        /* `CHECKSTRSPACE(4, cp)` — the room `conv_escape` writes into
         * through the raw cursor below; see `CONV_ESCAPE_SLOP`. */
        cp.reserve(CONV_ESCAPE_SLOP);

        // `goto putchar` is taken from two places; the flag replaces it.
        let mut goto_putchar = false;

        c = *str as c_int;
        str = str.add(1);
        if c != b'\\' as c_int {
            ch = 0; /* unused on this path */
            goto_putchar = true;
        } else {
            ch = *str as c_int;
            if ch == b'c' as c_int {
                /* \c as in SYSV echo - abort all processing.... */
                c = 0x100;
                goto_putchar = true;
            }
        }

        if goto_putchar {
            // putchar:
            /* `USTPUTC(c, cp)` truncates to `char`, which is what turns
             * `\c`'s 0x100 into the terminating NUL. */
            cp.push(c as u8);
        } else {
            /*
             * %b string octal constants are not like those in C.
             * They start with a \0, and are followed by 0, 1, 2,
             * or 3 octal digits.
             */
            if ch == b'0' as c_int && isodigit(*str.add(1) as c_int) {
                str = str.add(1);
            }

            /* Finally test for sequences valid in the format string */
            let at = cp.len();
            ret = conv_escape(str, cp.as_mut_ptr().add(at) as *mut c_char, false);
            str = str.add((ret >> 4) as usize);
            /* `cp += ret & 15` is the commit of what `conv_escape` wrote
             * past the cursor; what it wrote above that stays uncommitted,
             * for the next write to overwrite as the C's does. */
            debug_assert!((ret & 15) as usize <= CONV_ESCAPE_SLOP);
            cp.set_len(at + (ret & 15) as usize);
        }

        // } while (c & 0xff);
        if (c & 0xff) == 0 {
            break;
        }
    }

    c
}

// [spec:dash:def:printf.echocmd-fn]
// [spec:dash:sem:printf.echocmd-fn]
pub unsafe fn echocmd(args: &[&BStr]) -> c_int {
    /* The C picked between the formats `"%s\n"`, `"%s"` and `"%s "`; all
     * that ever differed was the byte after the conversion, so what is
     * chosen here is that byte. `-n` closes with nothing. */
    let mut last: u8 = b'\n';
    let mut nonl: c_int;

    let mut words = &args[1..];
    if words.first().is_some_and(|w| &w[..] == b"-n") {
        words = &words[1..];
        last = 0;
    }

    let mut index = 0usize;
    loop {
        let mut separator: u8 = b' ';
        let s = words.get(index);

        // if (!s || !*++argv) — `++argv` is not evaluated when s is NULL.
        if s.is_none() || {
            index += 1;
            words.get(index).is_none()
        } {
            separator = last;
        }

        let s = s.map(|w| crate::shell::cstring(w));
        nonl = print_escape_str(
            separator,
            s.as_ref().map_or(nullstr(), |w| w.as_ptr() as *mut c_char),
        );

        if !(nonl == 0 && words.get(index).is_some()) {
            break;
        }
    }
    0
}
