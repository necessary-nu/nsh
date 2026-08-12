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
            let (magnitude, end, range) = scan_hexadecimal(bytes, at + 2);
            return (signed(magnitude), end, range);
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
/// Digits past the 31st cannot change a double except by rounding, so
/// they are gathered into a sticky bit and left to [`round_to_double`],
/// which rounds the whole thing exactly once.
fn scan_hexadecimal(bytes: &[u8], from: usize) -> (f64, usize, bool) {
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

    let (magnitude, range) = round_to_double(mantissa, sticky, exponent);
    (magnitude, at, range)
}

/// The double nearest `mantissa * 2^exponent`, and whether C would call
/// the conversion out of range.
///
/// Scaling a rounded mantissa by a power of two was the wrong shape for
/// this: a value in the subnormal range is rounded twice that way, and
/// the power itself underflows to zero well before the value does -- so
/// `0x1.fffffffffffffp-1023` came out `0` rather than the smallest
/// normal. Here the bits are shifted into their place and rounded once,
/// which is the whole of what the conversion is.
///
/// `sticky` says nonzero bits already fell off the bottom of `mantissa`,
/// so the value sits just above what the bits alone say.
///
/// Out of range is C's `ERANGE` from `strtod`: an infinite result, or an
/// underflowing one. IEEE detects underflow as tininess *after*
/// rounding, together with a result that could not be held exactly, and
/// both halves of that are load-bearing here. `0x1p-1074` is the
/// smallest subnormal exactly, so it is tiny and silent; `0x1.8p-1074`
/// is tiny and cannot be held, so it warns; and
/// `0x1.fffffffffffff8p-1023` warns in neither shell because rounding it
/// to 53 bits lands on the smallest normal, which is not tiny at all --
/// where `0x1.fffffffffffffp-1023`, which needs no rounding at 53 bits
/// and stays below it, does warn. Both reach the same double.
fn round_to_double(mantissa: u128, sticky: bool, exponent: i32) -> (f64, bool) {
    if mantissa == 0 {
        return (0.0, false);
    }
    let bits = i64::from(128 - mantissa.leading_zeros());
    /* Where the value's leading bit sits: it is `1.f * 2^top`. */
    let top = i64::from(exponent) + bits - 1;
    if top > 1023 {
        return (f64::INFINITY, true);
    }

    /* Tininess is asked of the value rounded to 53 bits with no bound on
     * the exponent, where only a carry out of them moves it up a
     * binade. */
    let (at_full_precision, _) = round_bits(mantissa, sticky, bits - 53);
    let tiny = top + i64::from(at_full_precision >> 53 != 0) < -1022;

    /* The result is a multiple of this power of two: the last of the 53
     * bits for a normal, and the smallest subnormal's own step once the
     * value falls below the normal range. */
    let step = top.max(-1022) - 52;
    let (digits, exact) = round_bits(mantissa, sticky, step - i64::from(exponent));
    /* Bits narrower than the step are left where they were rather than
     * padded up to it, so they still count in the literal's own place. */
    let place = step.max(i64::from(exponent));
    let value = digits as f64 * power_of_two(place as i32);

    (value, value.is_infinite() || (tiny && !exact))
}

/// Round `mantissa` to what survives dropping its `shift` lowest bits,
/// half to even. `sticky` puts the value just above the bits, so it
/// breaks a tie upwards.
///
/// Returns the rounded bits -- one place wider than the shift left room
/// for, when the rounding carried -- and whether nothing was lost.
fn round_bits(mantissa: u128, sticky: bool, shift: i64) -> (u128, bool) {
    if shift <= 0 {
        return (mantissa, !sticky);
    }
    if shift >= 128 {
        /* Nothing of the mantissa survives, so only a value at or past
         * the halfway point rounds up -- which needs every bit of it to
         * sit exactly one place below the last one kept. A tie there
         * rounds towards the even zero. */
        let half = 1u128 << 127;
        let up = shift == 128 && (mantissa > half || (mantissa == half && sticky));
        return (u128::from(up), false);
    }

    let shift = shift as u32;
    let dropped = mantissa & ((1u128 << shift) - 1);
    let half = 1u128 << (shift - 1);
    let kept = mantissa >> shift;
    let up = dropped > half || (dropped == half && (sticky || kept & 1 != 0));
    (kept + u128::from(up), dropped == 0 && !sticky)
}

/// Two raised to `exponent`, exactly, for every exponent a double holds
/// -- the subnormal ones included.
fn power_of_two(exponent: i32) -> f64 {
    if exponent >= -1022 {
        f64::from_bits(((exponent + 1023) as u64) << 52)
    } else {
        f64::from_bits(1u64 << (exponent + 1074))
    }
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

/// The tail of a specification, as the C handed it to `printf`.
///
/// `mklong` had rewritten the integer conversions to `PRIdMAX` before
/// the text was passed, so the length modifier it inserted is part of
/// what a specification glibc could not read prints -- one `l` on this
/// target, where `intmax_t` is a `long`. `%b` was passed with its
/// conversion character set to `s`, which is how the C reached `printf`
/// with a conversion character C has.
fn passed_tail(tail: &[u8], conversion: u8) -> Vec<u8> {
    let mut text = tail.to_vec();
    if matches!(conversion, b'd' | b'i' | b'o' | b'u' | b'x' | b'X') {
        text.push(b'l');
    }
    text.push(if conversion == b'b' { b's' } else { conversion });
    text
}

/// Where a written-out width or precision holds a `*`, which is where
/// C's `printf` stops reading the specification.
///
/// The C's own scan ran `strspn` over `*` along with the digits and
/// never looked further, so the digits it collected may straddle a `*`
/// that C's `printf` would never have got past. Only the ones in front
/// of it were ever read as a number.
fn unreadable_at(field: &[u8]) -> Option<usize> {
    field.iter().position(|&byte| byte == b'*')
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
            /* Where C's `printf` stopped reading. The C's scan carries on
             * past it either way -- it is the C's scan that says where
             * the specification ends and which operands it takes. */
            let mut stop = None;

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
                let field = &format[at..at + digits];
                let read = unreadable_at(field);
                if let Some(k) = read {
                    stop = Some(at + k);
                }
                spec.set_written_width(leading_number(&field[..read.unwrap_or(digits)]));
                at += digits;
            }

            if format[at] == b'.' {
                at += 1;
                if format[at] == b'*' {
                    at += 1;
                    let value = operands.getuintmax(true) as c_int;
                    if stop.is_none() {
                        spec.set_precision(value);
                    }
                } else {
                    let digits = span(&format[..end], at, WIDTH);
                    let field = &format[at..at + digits];
                    let read = unreadable_at(field);
                    if stop.is_none() {
                        if let Some(k) = read {
                            stop = Some(at + k);
                        }
                        spec.set_written_precision(leading_number(&field[..read.unwrap_or(digits)]));
                    }
                    at += digits;
                }
            }

            let conversion = format[at];
            if conversion == 0 {
                crate::error::sh_error(b"missing format character");
            }
            at += 1;
            if let Some(stop) = stop {
                let tail = passed_tail(&format[stop + 1..at - 1], conversion);
                spec.set_unreadable(format[stop], &tail);
            }

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
        assert!(double("0x1p2000").0.is_infinite());
        assert_eq!(double("0x1p-2000").0, 0.0);
    }

    /// The bits are placed and rounded once, so a literal only a
    /// subnormal can hold arrives instead of collapsing through a scale
    /// that underflowed before the value did.
    #[test]
    fn subnormal_hexadecimal_literals_arrive() {
        let smallest = 5e-324;
        assert_eq!(double("0x1p-1074").0, smallest);
        assert_eq!(double("0x1.8p-1074").0, 2.0 * smallest);
        assert_eq!(double("0x3p-1075").0, 2.0 * smallest);
        assert_eq!(double("0x1.8p-1075").0, smallest);
        /* Exactly half the smallest, which ties towards the even zero. */
        assert_eq!(double("0x1p-1075").0, 0.0);
        /* Just under the smallest normal, from either side of the tie. */
        assert_eq!(double("0x1.fffffffffffffp-1023").0, f64::MIN_POSITIVE);
        assert_eq!(double("0x1.fffffffffffff8p-1023").0, f64::MIN_POSITIVE);
        assert_eq!(
            double("0x1.ffffffffffffep-1023").0,
            f64::MIN_POSITIVE - smallest
        );
        /* More digits than a double has bits, rounded once. */
        assert_eq!(double("0x1.ffffffffffffffffffffp0").0, 2.0);
        assert_eq!(double("0x1.fffffffffffff7fffp1023").0, f64::MAX);
    }

    /// A hexadecimal literal reports the range C reports: an infinite
    /// result, or one tiny once rounded that could not be held exactly.
    #[test]
    fn hexadecimal_literals_report_range() {
        for out_of_range in [
            "0x1p9999",
            "0x1p-9999",
            "0x1.8p-1074",
            "0x1p-1075",
            "0x1.fffffffffffffp-1023",
            "0x1.ffffffffffffe8p-1023",
            "0x1.fffffffffffff8p1023",
        ] {
            assert!(double(out_of_range).2, "{out_of_range} is out of range");
        }
        for held in [
            "0x1p0",
            "0x0p0",
            "0x1p-1022",
            "0x1p-1074",
            "0x1.ffffffffffffep-1023",
            /* Tiny before rounding, but rounded to 53 bits it lands on
             * the smallest normal, which is not tiny at all. */
            "0x1.fffffffffffff8p-1023",
            "0x1.fffffffffffff7fffp1023",
        ] {
            assert!(!double(held).2, "{held} is in range");
        }
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
        /* Digits past what the C held saturate one place beyond the
         * limit, which is all anything asks of them. */
        assert_eq!(leading_number(b"2147483647"), LIMIT);
        assert_eq!(leading_number(b"2147483648"), LIMIT + 1);
        assert_eq!(leading_number(b"99999999999999999999"), LIMIT + 1);
    }

    /// The C widened the integer conversions before handing the text
    /// over, and passed `%b` as `%s`.
    #[test]
    fn a_passed_tail_carries_c_rewrites() {
        assert_eq!(passed_tail(b"", b'd'), b"ld");
        assert_eq!(passed_tail(b"2.5", b'i'), b"2.5li");
        assert_eq!(passed_tail(b".5", b'X'), b".5lX");
        assert_eq!(passed_tail(b"", b's'), b"s");
        assert_eq!(passed_tail(b"3", b'f'), b"3f");
        assert_eq!(passed_tail(b"", b'b'), b"s");
    }

    /// Only the digits in front of a `*` were ever read as a number.
    #[test]
    fn a_star_stops_the_digits() {
        assert_eq!(unreadable_at(b"5*"), Some(1));
        assert_eq!(unreadable_at(b"1*2"), Some(1));
        assert_eq!(unreadable_at(b"12"), None);
        assert_eq!(unreadable_at(b""), None);
    }
}
