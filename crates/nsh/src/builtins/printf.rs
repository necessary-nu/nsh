//! The `printf` builtin.
//!
//! Port of the `printf` half of `src/bltin/printf.c`.
//! Rules: `docs/spec/port/src/bltin/printf.md`.
//!
//! The utility's contract is to read a format string *at runtime* and
//! render each argument by a pattern found in it, so parsing `%`
//! conversions here is inherent to `printf` and not a habit leaking in
//! from the C. What the port does not do is what the C did with the
//! result: `printf.c` found only the *end* of each specification and
//! handed the text straight back to C's `printf` as a format string,
//! which needed `mklong` to widen the conversion to `PRIdMAX` and the
//! `PF`/`ASPF` macros to pick between three varargs arities. Here the
//! specification is parsed into a [`Spec`] -- flags, width and precision
//! as fields -- and [`conv`] renders it against a typed argument through
//! Rust's own formatting and an `io::Write`. See
//! `[dec:nsh:printf-is-parsed-not-interpreted]`.
//!
//! `%b`'s escape dialect and the `\` escapes in the format string are
//! [`crate::escape`], shared with `echo` and the parser.

use std::ffi::CStr;
use std::io::Write as _;

use bstr::{BStr, BString};
use libc::{c_char, c_int};

use crate::escape::{CONV_ESCAPE_SLOP, conv_escape, conv_escape_str};

mod conv;

use conv::{LIMIT, Spec};

/// What the C skipped with `strspn(fmt, SKIP2)` when the width or
/// precision was written out rather than taken from an argument.
///
/// The C's companion `SKIP1`, the flag characters, has no constant here:
/// [`Spec::flag`] recognises them one at a time and records what each
/// means, so a second spelling of the same set could only disagree with
/// it.
const WIDTH: &[u8] = b"*0123456789";

/// Write one rendered conversion to standard output.
///
/// Nothing is checked: `evalbltin` reads `Output`'s sticky error flag
/// after the builtin returns and folds it into the exit status.
unsafe fn emit(bytes: &[u8]) {
    let _ = (&mut *crate::output::stdout()).write_all(bytes);
}

/// Write one rendered conversion, or raise what the C raised when it
/// could not render it.
///
/// `None` is a field longer than `vsnprintf` counts in an `int`. The C
/// asked glibc to lay every conversion out and `xvasprintf` treated the
/// refusal as fatal, so the builtin stops there: whatever the format had
/// already printed stays printed, and the shell's status is 2.
unsafe fn emit_field(rendered: Option<Vec<u8>>) {
    match rendered {
        Some(bytes) => emit(&bytes),
        None => crate::error::sh_error(b"xvsnprintf failed"),
    }
}

/// The operands a conversion reads its value from.
///
/// The C walked a `char **` cursor named `gargv`, and each `get*` helper
/// advanced it and returned a benign default once it ran off the end --
/// which is how one format string renders however many arguments it was
/// given. The cursor is an index here and the defaults are the same.
struct Operands<'a> {
    words: &'a [&'a BStr],
    next: usize,
    /// The C's `rval`: 1 once a numeric argument was malformed, and the
    /// builtin's exit status. Output still proceeds.
    status: c_int,
}

impl<'a> Operands<'a> {
    fn new(words: &'a [&'a BStr]) -> Self {
        Self {
            words,
            next: 0,
            status: 0,
        }
    }

    /// The C's `gargv != argv && *gargv`: the format is scanned again
    /// only while arguments remain *and* a pass consumed at least one,
    /// which is what keeps a format that reads nothing from looping for
    /// ever.
    fn reuse_format(&self) -> bool {
        self.next != 0 && self.next < self.words.len()
    }

    fn next_word(&mut self) -> Option<&'a BStr> {
        let word = *self.words.get(self.next)?;
        self.next += 1;
        Some(word)
    }

    /// One argument's first byte, or 0 once they are exhausted.
    // [spec:dash:def:printf.getchr-fn]
    // [spec:dash:sem:printf.getchr-fn]
    fn getchr(&mut self) -> u8 {
        self.next_word()
            .and_then(|word| word.first().copied())
            .unwrap_or(0)
    }

    /// One argument, or the empty string once they are exhausted.
    // [spec:dash:def:printf.getstr-fn]
    // [spec:dash:sem:printf.getstr-fn]
    fn getstr(&mut self) -> &'a [u8] {
        self.next_word().map_or(&[][..], |word| &word[..])
    }

    /// One argument as an integer, or 0 once they are exhausted.
    ///
    /// `signed` picks between the C's `strtoimax` and `strtoumax`, which
    /// differ in where they saturate; both read base 0.
    // [spec:dash:def:printf.getuintmax-fn]
    // [spec:dash:sem:printf.getuintmax-fn]
    fn getuintmax(&mut self, signed: bool) -> u64 {
        let Some(word) = self.next_word() else {
            return 0;
        };
        let bytes = &word[..];

        /* The POSIX rule that lets `printf %d "'A"` print 65: an
         * argument that opens with a quote is the character after it,
         * and nothing else is looked at. */
        if let Some(b'"' | b'\'') = bytes.first() {
            return u64::from(bytes.get(1).copied().unwrap_or(0));
        }

        let (value, end, range) = scan_integer(bytes, signed);
        self.check_conversion(bytes, end, range);
        value
    }

    /// One argument as a floating-point value, or 0 once they are
    /// exhausted.
    // [spec:dash:def:printf.getdouble-fn]
    // [spec:dash:sem:printf.getdouble-fn]
    fn getdouble(&mut self) -> f64 {
        let Some(word) = self.next_word() else {
            return 0.0;
        };
        let bytes = &word[..];

        if let Some(b'"' | b'\'') = bytes.first() {
            return f64::from(bytes.get(1).copied().unwrap_or(0));
        }

        let (value, end, range) = scan_double(bytes);
        self.check_conversion(bytes, end, range);
        value
    }

    /// Report a malformed numeric argument.
    ///
    /// `end` is where the conversion stopped and `range` whether the
    /// value ran past what the type holds. Either sets the exit status to
    /// 1 while the builtin goes on printing the value it did derive --
    /// text left over is the louder complaint of the two, so it is
    /// checked first, exactly as the C checked `*ep` before `errno`.
    // [spec:dash:def:printf.check-conversion-fn]
    // [spec:dash:sem:printf.check-conversion-fn]
    fn check_conversion(&mut self, word: &[u8], end: usize, range: bool) {
        let mut message = word.to_vec();
        if end < word.len() {
            message.extend_from_slice(if end == 0 {
                &b": expected numeric value"[..]
            } else {
                &b": not completely converted"[..]
            });
        } else if range {
            message.extend_from_slice(b": ");
            /* The C's `strerror(ERANGE)`, so the wording is the
             * platform's rather than this file's. */
            let text = unsafe { CStr::from_ptr(libc::strerror(libc::ERANGE)) };
            message.extend_from_slice(text.to_bytes());
        } else {
            return;
        }

        unsafe { crate::error::sh_warnx(&message) };
        self.status = 1;
    }
}

/// Skip the blanks a C `strto*` conversion skips before the sign.
///
/// The set is C's `isspace` in the C locale, spelled out because Rust's
/// `is_ascii_whitespace` is a different set: it leaves out the vertical
/// tab, and a conversion that stopped at one would call
/// `printf '%d' "$(printf '\v42')"` a malformed number where the C reads
/// 42. Every numeric conversion and every `*` width reaches here.
fn skip_blanks(bytes: &[u8]) -> usize {
    bytes
        .iter()
        .position(|byte| !matches!(byte, b' ' | 0x09..=0x0d))
        .unwrap_or(bytes.len())
}

/// The value of `byte` as a digit in `radix`.
fn digit_value(byte: Option<&u8>, radix: u64) -> Option<u64> {
    let digit = u64::from(char::from(*byte?).to_digit(36)?);
    (digit < radix).then_some(digit)
}

/// Parse a leading integer, returning its value, how many bytes it
/// consumed, and whether the magnitude ran past what the type holds.
///
/// This is C's `strtoimax`/`strtoumax` with a base of 0, written out
/// because the base detection *is* the specification: `0x`/`0X`
/// hexadecimal, `0b`/`0B` binary, a leading `0` octal, decimal
/// otherwise. The binary form is not C99's -- glibc's C23 `strtol`
/// semantics accept it and the dash build enables them, so
/// `printf '%d' 0b11` prints 3 and a port that dropped it would differ.
///
/// A signed overflow saturates at the ends of the range and an unsigned
/// one wraps, which is what the C's two functions do.
fn scan_integer(bytes: &[u8], signed: bool) -> (u64, usize, bool) {
    let mut at = skip_blanks(bytes);
    let negative = matches!(bytes.get(at), Some(b'-'));
    if let Some(b'-' | b'+') = bytes.get(at) {
        at += 1;
    }

    let radix = match (bytes.get(at), bytes.get(at + 1)) {
        (Some(b'0'), Some(b'x' | b'X')) if digit_value(bytes.get(at + 2), 16).is_some() => {
            at += 2;
            16
        }
        (Some(b'0'), Some(b'b' | b'B')) if digit_value(bytes.get(at + 2), 2).is_some() => {
            at += 2;
            2
        }
        /* The leading `0` is itself an octal digit, so the loop below
         * consumes it and a lone `0` converts cleanly. */
        (Some(b'0'), _) => 8,
        _ => 10,
    };

    let mut magnitude = 0u64;
    let mut overflow = false;
    let mut digits = 0usize;
    while let Some(digit) = digit_value(bytes.get(at), radix) {
        digits += 1;
        at += 1;
        match magnitude
            .checked_mul(radix)
            .and_then(|shifted| shifted.checked_add(digit))
        {
            Some(next) => magnitude = next,
            None => overflow = true,
        }
    }

    /* No conversion at all: C leaves `ep` at the string's start, blanks
     * and sign included, which is what makes the diagnostic "expected
     * numeric value" rather than "not completely converted". */
    if digits == 0 {
        return (0, 0, false);
    }

    let limit = if signed {
        if negative { 1u64 << 63 } else { i64::MAX as u64 }
    } else {
        u64::MAX
    };
    if overflow || magnitude > limit {
        let saturated = if !signed {
            u64::MAX
        } else if negative {
            i64::MIN as u64
        } else {
            i64::MAX as u64
        };
        return (saturated, at, true);
    }

    let value = if negative {
        magnitude.wrapping_neg()
    } else {
        magnitude
    };
    (value, at, false)
}

/// True when `bytes` opens with `word`, ignoring case.
fn starts_with_ignoring_case(bytes: &[u8], word: &[u8]) -> bool {
    bytes.len() >= word.len() && bytes[..word.len()].eq_ignore_ascii_case(word)
}

/// Parse a leading floating-point value, returning it, how many bytes it
/// consumed, and whether it fell outside the range a double holds.
///
/// C's `strtod`, which converts the longest valid prefix. Rust's own
/// parser wants a whole string and no hexadecimal, so the prefix is
/// measured here and handed over -- except the hexadecimal form, which
/// is exact bit-laying and done in [`scan_hexadecimal`].
fn scan_double(bytes: &[u8]) -> (f64, usize, bool) {
    let from = skip_blanks(bytes);
    let mut at = from;
    let negative = matches!(bytes.get(at), Some(b'-'));
    if let Some(b'-' | b'+') = bytes.get(at) {
        at += 1;
    }
    let signed = |value: f64| if negative { -value } else { value };

    /* `inf`, `infinity` and `nan` -- the longer spelling first, so that
     * `infinity` is not read as `inf` with five bytes left over. */
    for word in [&b"infinity"[..], &b"inf"[..]] {
        if starts_with_ignoring_case(&bytes[at..], word) {
            return (signed(f64::INFINITY), at + word.len(), false);
        }
    }
    if starts_with_ignoring_case(&bytes[at..], b"nan") {
        at += 3;
        /* C allows a parenthesised tag after `nan`, which names a
         * payload this port has no use for but must still step over. */
        if bytes.get(at) == Some(&b'(') {
            if let Some(close) = bytes[at..].iter().position(|&byte| byte == b')') {
                at += close + 1;
            }
        }
        return (signed(f64::NAN), at, false);
    }

    if let (Some(b'0'), Some(b'x' | b'X')) = (bytes.get(at), bytes.get(at + 1)) {
        let hex = |offset: usize| bytes.get(offset).is_some_and(u8::is_ascii_hexdigit);
        if hex(at + 2) || (bytes.get(at + 2) == Some(&b'.') && hex(at + 3)) {
            let (magnitude, end) = scan_hexadecimal(bytes, at + 2);
            return (signed(magnitude), end, false);
        }
    }

    let mut nonzero = false;
    let mut digits = 0usize;
    let mut count = |at: &mut usize| {
        while let Some(byte) = bytes.get(*at).filter(|byte| byte.is_ascii_digit()) {
            nonzero |= *byte != b'0';
            digits += 1;
            *at += 1;
        }
    };
    count(&mut at);
    if bytes.get(at) == Some(&b'.') {
        at += 1;
        count(&mut at);
    }
    if digits == 0 {
        return (0.0, 0, false);
    }

    /* An exponent counts only when it has digits of its own: `1e` is
     * the value 1 with an `e` left over, not a malformed exponent. */
    if let Some(b'e' | b'E') = bytes.get(at) {
        let mut probe = at + 1;
        if let Some(b'-' | b'+') = bytes.get(probe) {
            probe += 1;
        }
        if bytes.get(probe).is_some_and(u8::is_ascii_digit) {
            while bytes.get(probe).is_some_and(u8::is_ascii_digit) {
                probe += 1;
            }
            at = probe;
        }
    }

    let text = std::str::from_utf8(&bytes[from..at]).expect("a scanned literal is ASCII");
    let value: f64 = text.parse().expect("a scanned literal parses");

    /* C reports a magnitude too large to hold and one too small to hold
     * normally as the same out-of-range condition. */
    let range = value.is_infinite() || (nonzero && value.abs() < f64::MIN_POSITIVE);
    (value, at, range)
}

/// Parse C's hexadecimal floating literal, from just past the `0x`.
///
/// The digits are the value's bits, so this lays them straight into a
/// mantissa and a binary exponent rather than converting anything.
/// Digits past the 16th cannot change a double except by rounding, so
/// they are gathered into a sticky bit and left to the one rounding
/// `u128 as f64` already does.
fn scan_hexadecimal(bytes: &[u8], from: usize) -> (f64, usize) {
    let mut at = from;
    let mut mantissa = 0u128;
    let mut sticky = false;
    let mut exponent = 0i32;
    let mut fraction = false;

    loop {
        match bytes.get(at) {
            Some(b'.') if !fraction => fraction = true,
            Some(byte) if byte.is_ascii_hexdigit() => {
                let digit = u128::from(char::from(*byte).to_digit(16).expect("a hex digit"));
                if mantissa < 1u128 << 124 {
                    mantissa = (mantissa << 4) | digit;
                    /* Every digit kept after the point costs the
                     * exponent four; every one before it is already in
                     * the mantissa's place value. */
                    exponent -= 4 * i32::from(fraction);
                } else {
                    sticky |= digit != 0;
                    exponent += 4 * i32::from(!fraction);
                }
            }
            _ => break,
        }
        at += 1;
    }

    if let Some(b'p' | b'P') = bytes.get(at) {
        let mut probe = at + 1;
        let negative = matches!(bytes.get(probe), Some(b'-'));
        if let Some(b'-' | b'+') = bytes.get(probe) {
            probe += 1;
        }
        if bytes.get(probe).is_some_and(u8::is_ascii_digit) {
            let mut value = 0i32;
            while let Some(digit) = bytes.get(probe).filter(|byte| byte.is_ascii_digit()) {
                value = value.saturating_mul(10).saturating_add(i32::from(digit - b'0'));
                probe += 1;
            }
            exponent = exponent.saturating_add(if negative { -value } else { value });
            at = probe;
        }
    }

    let mut magnitude = mantissa as f64;
    if sticky {
        /* Below the last bit `mantissa` kept, so it only ever breaks a
         * tie away from even -- which is what the discarded digits do. */
        magnitude = f64::from_bits(magnitude.to_bits() | 1);
    }
    (magnitude * exp2(exponent), at)
}

/// Two raised to `exponent`, in steps a double can hold, so that a value
/// which is representable only after the whole scaling still arrives.
fn exp2(exponent: i32) -> f64 {
    let mut value = 1.0f64;
    let mut left = exponent;
    while left > 1000 {
        value *= f64::from_bits(0x7fe0_0000_0000_0000); /* 2^1023 */
        left -= 1023;
        if value.is_infinite() {
            return value;
        }
    }
    while left < -1000 {
        value *= f64::from_bits(0x0010_0000_0000_0000); /* 2^-1022 */
        left += 1022;
        if value == 0.0 {
            return value;
        }
    }
    value * (left as f64).exp2()
}

/// The width or precision a specification wrote out in digits.
///
/// The C let `strspn` run over `*` as well as the digits and then handed
/// the text to `printf`, which had no argument to match a `*` that
/// arrived this way; the number is what the specification actually
/// asked for.
///
/// Digits running past what the C could hold saturate one place beyond
/// [`LIMIT`], which is the only thing anything downstream asks about
/// them: a width or precision over the limit is refused whatever its
/// value, so there is nothing further to count.
fn leading_number(bytes: &[u8]) -> usize {
    let mut value = 0usize;
    for byte in bytes.iter().filter(|byte| byte.is_ascii_digit()) {
        value = value
            .saturating_mul(10)
            .saturating_add(usize::from(byte - b'0'))
            .min(LIMIT + 1);
    }
    value
}

/// How many leading bytes of `bytes` from `at` are in `set`.
fn span(bytes: &[u8], at: usize, set: &[u8]) -> usize {
    bytes[at..]
        .iter()
        .position(|byte| !set.contains(byte))
        .unwrap_or(bytes.len() - at)
}

/// `%b`: the argument with its escapes expanded, laid out in the field.
///
/// Returns non-zero when a `\c` stopped the conversion, which stops the
/// whole builtin.
///
/// The C could not lay this out itself. A `%b` result may contain NUL,
/// so it could not be handed to a C string function at all: the C
/// formatted a run of `X`s of the same length, let `printf` compute the
/// padding around them, then copied the real bytes back over the run.
/// Laying out bytes needs no stand-in.
// [spec:dash:def:printf.print-escape-str-fn]
// [spec:dash:sem:printf.print-escape-str-fn]
unsafe fn print_escape_str(spec: &Spec, word: &CStr) -> c_int {
    let mut buf = BString::default();
    let done = conv_escape_str(word.as_ptr(), &mut buf);

    /* `conv_escape_str` exits on the iteration that writes the
     * terminating NUL, so there is always one to drop. The C overwrote
     * it with `echo`'s separator; every route in from here had a NUL
     * after the conversion character and appended nothing. */
    debug_assert!(!buf.is_empty());
    let text = &buf[..buf.len() - 1];

    emit_field(spec.string(text));
    done
}

// [spec:dash:def:printf.printfcmd-fn]
// [spec:dash:sem:printf.printfcmd-fn]
pub unsafe fn printfcmd(args: &[&BStr]) -> c_int {
    let mut options = crate::options::Options::new(args);
    /* `nextopt(nullstr)`: printf takes no options, so this exists to
     * reject `-x` and to step over a `--`. */
    while options.next(b"").is_some() {}

    let Some((format, arguments)) = options.operands().split_first() else {
        crate::error::sh_error(b"usage: printf format [arg ...]");
    };

    /* `conv_escape` reads through a raw cursor and stops at a NUL, so
     * the format is copied once with a terminator on it. */
    let mut format = crate::shell::cstring(format).into_bytes_with_nul();
    let end = format.len() - 1;
    let mut operands = Operands::new(arguments);

    'out: loop {
        /*
         * Basic algorithm is to scan the format string for conversion
         * specifications -- once one is found, find out if the field
         * width or precision is a '*'; if it is, gather up value.
         * Note, format strings are reused as necessary to use up the
         * provided arguments, arguments of zero/null string are
         * provided to use up the format string.
         */
        let mut at = 0usize;
        while at < end {
            let ch = format[at];
            at += 1;

            if ch == b'\\' {
                /* `STARTSTACKSTR(cp); CHECKSTRSPACE(4, cp)` -- one
                 * escape's worth of scratch and nothing else; see
                 * `CONV_ESCAPE_SLOP` for why 4 is not the bound. */
                let mut scratch: [c_char; CONV_ESCAPE_SLOP] = [0; CONV_ESCAPE_SLOP];
                let ret = conv_escape(
                    format.as_mut_ptr().add(at) as *mut c_char,
                    scratch.as_mut_ptr(),
                    false,
                );
                at += (ret >> 4) as usize;
                debug_assert!((ret & 15) as usize <= CONV_ESCAPE_SLOP);
                emit(core::slice::from_raw_parts(
                    scratch.as_ptr() as *const u8,
                    (ret & 15) as usize,
                ));
                continue;
            }
            /* A `%%` is one `%`; a `%` at the very end of the format
             * falls through and is the missing-conversion error. */
            if ch != b'%' || format[at] == b'%' {
                if ch == b'%' {
                    at += 1;
                }
                emit(&[ch]);
                continue;
            }

            /* Ok - we've found a format specification. The C saved its
             * address to hand back to `printf`; the port collects it. */
            let start = at - 1;
            let mut spec = Spec::bare();

            /* skip to field width */
            while at < end && spec.flag(format[at]) {
                at += 1;
            }
            if format[at] == b'*' {
                at += 1;
                spec.set_width(operands.getuintmax(true) as c_int);
            } else {
                /* skip to possible '.', get following precision */
                let digits = span(&format[..end], at, WIDTH);
                spec.set_written_width(leading_number(&format[at..at + digits]));
                at += digits;
            }

            if format[at] == b'.' {
                at += 1;
                if format[at] == b'*' {
                    at += 1;
                    spec.set_precision(operands.getuintmax(true) as c_int);
                } else {
                    let digits = span(&format[..end], at, WIDTH);
                    spec.set_written_precision(leading_number(&format[at..at + digits]));
                    at += digits;
                }
            }

            let conversion = format[at];
            if conversion == 0 {
                crate::error::sh_error(b"missing format character");
            }
            at += 1;

            match conversion {
                b'b' => {
                    let word = crate::shell::cstring(BStr::new(operands.getstr()));
                    /* escape if a \c was encountered */
                    if print_escape_str(&spec, &word) != 0 {
                        break 'out;
                    }
                }
                b'c' => {
                    let value = operands.getchr();
                    emit_field(spec.character(value));
                }
                b's' => {
                    let value = operands.getstr();
                    emit_field(spec.string(value));
                }
                /* `mklong` widened the specification to `PRIdMAX` so
                 * that C's printf would pull a whole `intmax_t` off the
                 * varargs. The value arrives typed. */
                b'd' | b'i' => {
                    let value = operands.getuintmax(true);
                    emit_field(spec.signed(value as i64));
                }
                b'o' | b'u' | b'x' | b'X' => {
                    let value = operands.getuintmax(false);
                    emit_field(spec.unsigned(value, conversion));
                }
                b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                    let value = operands.getdouble();
                    emit_field(spec.double(value, conversion));
                }
                _ => {
                    let mut message = format[start..at].to_vec();
                    message.extend_from_slice(b": invalid directive");
                    crate::error::sh_error(&message);
                }
            }
        }

        if !operands.reuse_format() {
            break;
        }
    }

    // out:
    operands.status
}

#[cfg(test)]
mod tests {
    use super::*;

    fn integer(text: &str, signed: bool) -> (u64, usize, bool) {
        scan_integer(text.as_bytes(), signed)
    }

    fn double(text: &str) -> (f64, usize, bool) {
        scan_double(text.as_bytes())
    }

    /// The base a `strto*` conversion picks is written into the digits,
    /// and a leading `0` means octal even with nothing after it.
    #[test]
    fn base_zero_reads_every_radix() {
        assert_eq!(integer("42", true).0, 42);
        assert_eq!(integer("017", true).0, 15);
        assert_eq!(integer("0x1f", true).0, 31);
        assert_eq!(integer("0X1F", true).0, 31);
        assert_eq!(integer("0b101", true).0, 5);
        assert_eq!(integer("0", true), (0, 1, false));
        assert_eq!(integer("  -5", true).0, (-5i64) as u64);
        assert_eq!(integer("+7", true).0, 7);
    }

    /// Every blank C's `isspace` names is skipped before the sign, the
    /// vertical tab included.
    #[test]
    fn every_c_blank_precedes_a_number() {
        for blank in [" ", "\t", "\n", "\x0b", "\x0c", "\r"] {
            assert_eq!(integer(&format!("{blank}42"), true), (42, 3, false));
            assert_eq!(double(&format!("{blank}2.5")).0, 2.5);
        }
        /* Not a blank: the conversion stops before it and converts
         * nothing at all. */
        assert_eq!(integer("\x0e42", true), (0, 0, false));
    }

    /// A prefix with no digits behind it converts the `0` and stops at
    /// the letter, which is a partial conversion and not a failed one.
    #[test]
    fn an_empty_prefix_converts_the_zero() {
        assert_eq!(integer("0x", true), (0, 1, false));
        assert_eq!(integer("0b", true), (0, 1, false));
        assert_eq!(integer("08", true), (0, 1, false));
    }

    /// Where the conversion stopped is what picks the diagnostic: at the
    /// start means nothing converted, past it means text left over.
    #[test]
    fn a_stop_position_picks_the_complaint() {
        assert_eq!(integer("abc", true), (0, 0, false));
        assert_eq!(integer("", true), (0, 0, false));
        assert_eq!(integer("12abc", true), (12, 2, false));
        assert_eq!(double("abc"), (0.0, 0, false));
        assert_eq!(double("1e").0, 1.0);
        assert_eq!(double("1e").1, 1);
    }

    /// Signed conversions saturate at the ends of the range; unsigned
    /// ones wrap, so a negative argument to `%u` is its complement.
    #[test]
    fn overflow_saturates_then_wraps() {
        assert_eq!(integer("99999999999999999999999", true), (i64::MAX as u64, 23, true));
        assert_eq!(
            integer("-99999999999999999999999", false),
            (u64::MAX, 24, true)
        );
        assert_eq!(integer("-1", false).0, u64::MAX);
        assert_eq!(integer("-9223372036854775808", true), (i64::MIN as u64, 20, false));
        assert_eq!(integer("18446744073709551615", false), (u64::MAX, 20, false));
    }

    #[test]
    fn doubles_take_the_longest_prefix() {
        assert_eq!(double("1e3").0, 1000.0);
        assert_eq!(double(".5").0, 0.5);
        assert_eq!(double("5.").0, 5.0);
        assert_eq!(double("+.5e1").0, 5.0);
        assert_eq!(double("  -2.5xyz"), (-2.5, 6, false));
        assert!(double("inf").0.is_infinite());
        assert!(double("-INFINITY").0.is_infinite() && double("-INFINITY").0 < 0.0);
        assert_eq!(double("infinity").1, 8);
        assert!(double("nan").0.is_nan());
        assert!(double("nan(x)").0.is_nan() && double("nan(x)").1 == 6);
    }

    /// A hexadecimal literal is laid out bit by bit, so it is exact.
    #[test]
    fn hexadecimal_doubles_are_exact() {
        assert_eq!(double("0x1.8p3").0, 12.0);
        assert_eq!(double("0x1f").0, 31.0);
        assert_eq!(double("0x1p-1").0, 0.5);
        assert_eq!(double("0x.8p1").0, 1.0);
        assert_eq!(double("0x1.999999999999ap-4").0, 0.1);
        /* No `p` at all is still a hexadecimal integer. */
        assert_eq!(double("0xffz"), (255.0, 4, false));
    }

    /// Out of range is a magnitude a double cannot hold normally, at
    /// either end, and both still yield the value C yields.
    #[test]
    fn out_of_range_magnitudes_are_reported() {
        assert_eq!(double("1e400"), (f64::INFINITY, 5, true));
        assert_eq!(double("1e-400"), (0.0, 6, true));
        assert!(double("1e-310").2);
        assert!(!double("1e3").2);
        assert!(!double("0").2);
        assert!(!double("0.0").2);
    }

    /// A radix's digits are read in either case.
    #[test]
    fn radix_digits_are_case_blind() {
        assert_eq!(integer("0XFF", true).0, 255);
        assert_eq!(integer("0B11", true).0, 3);
        assert_eq!(integer("0777", true).0, 511);
        /* 9 is not an octal digit, so the conversion stops before it. */
        assert_eq!(integer("0779", true), (63, 3, false));
    }

    /// A `p` with no digits behind it is not an exponent, and a
    /// hexadecimal literal needs no exponent at all.
    #[test]
    fn hexadecimal_exponents_scale_the_mantissa() {
        assert_eq!(double("0x1p10").0, 1024.0);
        assert_eq!(double("0x1P-10").0, 1.0 / 1024.0);
        assert_eq!(double("0x1p"), (1.0, 3, false));
        assert_eq!(double("0x10").0, 16.0);
        /* Scaling runs in steps a double can hold, so the extremes
         * arrive rather than overflowing the step itself. */
        assert!(double("0x1p2000").0.is_infinite());
        assert_eq!(double("0x1p-2000").0, 0.0);
    }

    /// The C's `get*` helpers each read one argument and hand back a
    /// benign default once the list runs out.
    #[test]
    fn exhausted_operands_yield_defaults() {
        let words: Vec<&BStr> = vec![BStr::new("ab")];
        let mut operands = Operands::new(&words);
        assert_eq!(operands.getchr(), b'a');
        assert_eq!(operands.getchr(), 0);
        assert_eq!(operands.getstr(), b"");
        assert_eq!(operands.getuintmax(true), 0);
        assert_eq!(operands.getdouble(), 0.0);
        assert_eq!(operands.status, 0);
    }

    /// The format is scanned again only while arguments remain and a
    /// pass consumed one.
    #[test]
    fn a_format_repeats_while_words_remain() {
        let words: Vec<&BStr> = vec![BStr::new("a"), BStr::new("b")];
        let mut operands = Operands::new(&words);
        assert!(!operands.reuse_format());
        operands.getstr();
        assert!(operands.reuse_format());
        operands.getstr();
        assert!(!operands.reuse_format());
    }

    /// An argument that opens with a quote is the character after it,
    /// whichever quote it is, and a lone quote is nothing.
    #[test]
    fn a_quote_argument_is_one_byte() {
        let words: Vec<&BStr> = vec![BStr::new("'A"), BStr::new("\"z"), BStr::new("'")];
        let mut operands = Operands::new(&words);
        assert_eq!(operands.getuintmax(true), 65);
        assert_eq!(operands.getdouble(), 122.0);
        assert_eq!(operands.getuintmax(false), 0);
        assert_eq!(operands.status, 0);
    }

    /// The digits of a written-out width are what the field asks for.
    #[test]
    fn a_written_width_is_its_digits() {
        assert_eq!(leading_number(b""), 0);
        assert_eq!(leading_number(b"12"), 12);
        assert_eq!(span(b"12.3d", 0, WIDTH), 2);
        assert_eq!(span(b"*", 0, WIDTH), 1);
        assert_eq!(span(b"d", 0, WIDTH), 0);
    }
}
