//! Rendering one POSIX `printf` conversion.
//!
//! `printf.c` handed every conversion straight back to C's `printf`,
//! passing the user's own specification as the format string. The port has
//! no runtime format-string API and does no libc formatting, so the
//! specification arrives here as a [`Spec`] -- flags, width and precision
//! already parsed into fields -- and is rendered against a typed argument.
//!
//! Every decimal digit printed below comes out of `format!`: `{}`, `{:o}`,
//! `{:x}` and `{:X}` supply integer digits, `{:.p$}` and `{:.p$e}` the
//! significand of a double. What this module adds is what C's flags *mean*
//! on top of those digits -- sign, radix prefix, precision zeros, field
//! padding, and `%g`'s choice between fixed and scientific. There is no
//! decimal digit generation, no rounding table and no float-to-decimal
//! loop here; that is Rust's job.
//!
//! `%a` is the one conversion Rust's formatting cannot spell, and it is
//! also the one that needs no decimal arithmetic at all: a hexadecimal
//! float *is* the double's IEEE-754 fields written out, so [`Spec::
//! hexadecimal`] transcribes them from [`f64::to_bits`].

use libc::c_int;

/// A parsed `%` conversion specification: the flags, field width and
/// precision between the `%` and the conversion character.
pub(super) struct Spec {
    /// `-`: left-justify within the field.
    pub(super) left: bool,
    /// `+`: always sign a signed conversion.
    plus: bool,
    /// ` `: sign a non-negative signed conversion with a space.
    space: bool,
    /// `#`: alternate form -- a radix prefix, or a forced radix point.
    alt: bool,
    /// `0`: pad the field with zeros rather than spaces.
    zero: bool,
    width: usize,
    precision: Option<usize>,
}

impl Spec {
    /// The specification of a bare `%<conversion>`.
    pub(super) const fn bare() -> Self {
        Self {
            left: false,
            plus: false,
            space: false,
            alt: false,
            zero: false,
            width: 0,
            precision: None,
        }
    }

    /// Record one flag character, or report that `ch` ends the flags.
    pub(super) fn flag(&mut self, ch: u8) -> bool {
        match ch {
            b'#' => self.alt = true,
            b'-' => self.left = true,
            b'+' => self.plus = true,
            b' ' => self.space = true,
            b'0' => self.zero = true,
            _ => return false,
        }
        true
    }

    /// Set the field width, which a `*` argument may deliver negative --
    /// C reads that as the `-` flag applied to its magnitude.
    pub(super) fn set_width(&mut self, value: c_int) {
        if value < 0 {
            self.left = true;
            self.width = value.unsigned_abs() as usize;
        } else {
            self.width = value as usize;
        }
    }

    /// Set the precision. A negative `*` argument means "no precision".
    pub(super) fn set_precision(&mut self, value: c_int) {
        self.precision = if value < 0 {
            None
        } else {
            Some(value as usize)
        };
    }

    /// Lay `body` out in the field, padding with spaces.
    fn pad(&self, body: &[u8]) -> Vec<u8> {
        let mut out = Vec::with_capacity(body.len().max(self.width));
        if self.left {
            out.extend_from_slice(body);
            out.resize(self.width.max(body.len()), b' ');
        } else {
            out.resize(self.width.saturating_sub(body.len()), b' ');
            out.extend_from_slice(body);
        }
        out
    }

    /// Lay a number out in the field. Zero padding goes *after* the sign
    /// and any radix prefix, which is what distinguishes it from `pad`.
    fn pad_number(&self, prefix: &[u8], digits: &[u8], zeros: bool) -> Vec<u8> {
        let len = prefix.len() + digits.len();
        let fill = self.width.saturating_sub(len);
        let mut out = Vec::with_capacity(len.max(self.width));

        if !self.left && !zeros {
            out.resize(fill, b' ');
        }
        out.extend_from_slice(prefix);
        if !self.left && zeros {
            out.resize(out.len() + fill, b'0');
        }
        out.extend_from_slice(digits);
        if self.left {
            out.resize(self.width.max(len), b' ');
        }
        out
    }

    /// `%s`: the argument's bytes, truncated by the precision.
    ///
    /// Byte-oriented, so an argument that is not valid UTF-8 keeps its
    /// bytes and a precision counts them the way C counts them.
    pub(super) fn string(&self, bytes: &[u8]) -> Vec<u8> {
        let take = self.precision.map_or(bytes.len(), |p| p.min(bytes.len()));
        self.pad(&bytes[..take])
    }

    /// `%c`: one byte, which may be NUL.
    pub(super) fn character(&self, value: u8) -> Vec<u8> {
        self.pad(&[value])
    }

    /// Apply an integer precision -- a minimum digit count, where a
    /// precision of zero prints a zero value as nothing at all.
    fn integer_digits(&self, text: &str) -> Vec<u8> {
        let Some(precision) = self.precision else {
            return text.as_bytes().to_vec();
        };
        if text == "0" && precision == 0 {
            return Vec::new();
        }
        let mut digits = vec![b'0'; precision.saturating_sub(text.len())];
        digits.extend_from_slice(text.as_bytes());
        digits
    }

    /// The sign a signed conversion carries, given the flags.
    fn sign(&self, negative: bool) -> &'static [u8] {
        if negative {
            b"-"
        } else if self.plus {
            b"+"
        } else if self.space {
            b" "
        } else {
            b""
        }
    }

    /// `%d` and `%i`.
    pub(super) fn signed(&self, value: i64) -> Vec<u8> {
        let digits = self.integer_digits(&format!("{}", value.unsigned_abs()));
        /* C ignores `0` once a precision has said how many digits there
         * are to print. */
        let zeros = self.zero && self.precision.is_none();
        self.pad_number(self.sign(value < 0), &digits, zeros)
    }

    /// `%o`, `%u`, `%x` and `%X`.
    pub(super) fn unsigned(&self, value: u64, conversion: u8) -> Vec<u8> {
        let text = match conversion {
            b'o' => format!("{value:o}"),
            b'x' => format!("{value:x}"),
            b'X' => format!("{value:X}"),
            _ => format!("{value}"),
        };
        let mut digits = self.integer_digits(&text);
        let mut prefix: &[u8] = b"";

        if self.alt {
            match conversion {
                /* C words `#` on octal as raising the precision just far
                 * enough to force a leading zero. */
                b'o' if digits.first() != Some(&b'0') => digits.insert(0, b'0'),
                b'x' if value != 0 => prefix = b"0x",
                b'X' if value != 0 => prefix = b"0X",
                _ => {}
            }
        }

        let zeros = self.zero && self.precision.is_none();
        self.pad_number(prefix, &digits, zeros)
    }

    /// `%a`, `%A`, `%e`, `%E`, `%f`, `%F`, `%g` and `%G`.
    pub(super) fn double(&self, value: f64, conversion: u8) -> Vec<u8> {
        let upper = conversion.is_ascii_uppercase();
        let sign = self.sign(value.is_sign_negative());

        if !value.is_finite() {
            let word: &[u8] = match (value.is_nan(), upper) {
                (true, false) => b"nan",
                (true, true) => b"NAN",
                (false, false) => b"inf",
                (false, true) => b"INF",
            };
            /* An infinity or a NaN is padded with spaces even under `0`. */
            return self.pad_number(sign, word, false);
        }

        let magnitude = value.abs();
        match conversion.to_ascii_lowercase() {
            b'f' => self.pad_number(sign, &self.fixed(magnitude), self.zero),
            b'e' => {
                let body = self.scientific(magnitude, self.precision.unwrap_or(6), upper);
                self.pad_number(sign, &body, self.zero)
            }
            b'g' => self.pad_number(sign, &self.general(magnitude, upper), self.zero),
            _ => {
                /* `%a`'s radix prefix joins the sign, so zero padding
                 * lands after it the way it does for `%#x`. */
                let (marker, body) = self.hexadecimal(magnitude, upper);
                let mut prefix = sign.to_vec();
                prefix.extend_from_slice(marker);
                self.pad_number(&prefix, &body, self.zero)
            }
        }
    }

    /// `%f`: Rust's fixed-point formatting, with C's default precision.
    fn fixed(&self, magnitude: f64) -> Vec<u8> {
        let precision = self.precision.unwrap_or(6);
        let mut text = format!("{magnitude:.precision$}");
        if precision == 0 && self.alt {
            text.push('.');
        }
        text.into_bytes()
    }

    /// `%e`: Rust's `{:e}`, with C's exponent spelling.
    fn scientific(&self, magnitude: f64, precision: usize, upper: bool) -> Vec<u8> {
        let rendered = format!("{magnitude:.precision$e}");
        let (significand, exponent) = split_exponent(&rendered);
        let mut text = String::from(significand);
        if precision == 0 && self.alt {
            text.push('.');
        }
        push_exponent(&mut text, exponent, upper);
        text.into_bytes()
    }

    /// `%g`: C chooses between `%e` and `%f` by where the exponent falls,
    /// then drops trailing fractional zeros unless `#` asked to keep them.
    fn general(&self, magnitude: f64, upper: bool) -> Vec<u8> {
        /* "Let P equal the precision, 6 if it is omitted, 1 if it is
         * zero" -- then style e is used when the exponent X of the value
         * rendered with precision P-1 is below -4 or at least P. */
        let significant = self.precision.unwrap_or(6).max(1);
        let rendered = format!("{magnitude:.*e}", significant - 1);
        let (significand, exponent) = split_exponent(&rendered);

        if exponent < -4 || exponent >= significant as i32 {
            let mut text = String::from(significand);
            if self.alt {
                if significant == 1 {
                    text.push('.');
                }
            } else {
                trim_fraction(&mut text);
            }
            push_exponent(&mut text, exponent, upper);
            return text.into_bytes();
        }

        let precision = (significant as i32 - 1 - exponent) as usize;
        let mut text = format!("{magnitude:.precision$}");
        if self.alt {
            if precision == 0 {
                text.push('.');
            }
        } else {
            trim_fraction(&mut text);
        }
        text.into_bytes()
    }

    /// `%a`: the double's own IEEE-754 fields, written in hexadecimal.
    ///
    /// Returns the radix marker separately from the body, because `0`
    /// padding belongs between them.
    ///
    /// This conversion is a transcription rather than a conversion: four
    /// mantissa bits *are* one hexadecimal digit, so there is no decimal
    /// arithmetic to get wrong and nothing for `format!` to do beyond the
    /// exponent's digits. A precision rounds the mantissa half-to-even,
    /// and the carry out of that can reach the digit before the point --
    /// where C leaves it rather than renormalising, so `%.0a` of `1.5` is
    /// `0x2p+0` and not `0x1p+1`.
    fn hexadecimal(&self, magnitude: f64, upper: bool) -> (&'static [u8], Vec<u8>) {
        /* 52 mantissa bits, four to a hexadecimal digit. */
        const FRACTION_DIGITS: usize = 13;

        let bits = magnitude.to_bits();
        let exponent_field = ((bits >> 52) & 0x7ff) as i32;
        let mut fraction = bits & 0x000f_ffff_ffff_ffff;

        /* A subnormal carries no implicit leading one and pins its
         * exponent at the bottom of the normal range; zero is the
         * subnormal with an empty mantissa, which C writes `0x0p+0`. */
        let (mut lead, exponent) = if exponent_field == 0 {
            (0u8, if fraction == 0 { 0 } else { -1022 })
        } else {
            (1u8, exponent_field - 1023)
        };

        let kept = self
            .precision
            .map_or(FRACTION_DIGITS, |p| p.min(FRACTION_DIGITS));
        if kept < FRACTION_DIGITS {
            let shift = (FRACTION_DIGITS - kept) * 4;
            let dropped = fraction & ((1u64 << shift) - 1);
            let half = 1u64 << (shift - 1);
            fraction >>= shift;
            /* Ties go to the even last *kept* digit, which with no
             * fraction digits left is the one before the point. */
            let ties_on = if kept == 0 { u64::from(lead) } else { fraction };
            if dropped > half || (dropped == half && ties_on & 1 != 0) {
                fraction += 1;
                if fraction >> (kept * 4) != 0 {
                    fraction = 0;
                    lead += 1;
                }
            }
            fraction <<= shift;
        }

        let mut nibbles: Vec<u8> = (0..FRACTION_DIGITS)
            .map(|index| ((fraction >> (48 - index * 4)) & 0xf) as u8)
            .collect();
        match self.precision {
            /* Without a precision C writes the shortest exact form. */
            None => while nibbles.last() == Some(&0) {
                nibbles.pop();
            },
            /* With one it writes exactly that many, zero-filling past
             * the last bit the mantissa has. */
            Some(precision) => {
                nibbles.truncate(kept);
                nibbles.resize(precision, 0);
            }
        }

        let mut text = vec![b'0' + lead];
        if !nibbles.is_empty() || self.alt {
            text.push(b'.');
        }
        text.extend(nibbles.into_iter().map(|nibble| hex_digit(nibble, upper)));
        text.push(if upper { b'P' } else { b'p' });
        text.push(if exponent < 0 { b'-' } else { b'+' });
        text.extend_from_slice(format!("{}", exponent.unsigned_abs()).as_bytes());

        (if upper { b"0X" } else { b"0x" }, text)
    }
}

/// One hexadecimal digit, in the case the conversion character asked for.
fn hex_digit(nibble: u8, upper: bool) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ if upper => b'A' + nibble - 10,
        _ => b'a' + nibble - 10,
    }
}

/// Split Rust's `{:e}` rendering into its significand and exponent.
fn split_exponent(rendered: &str) -> (&str, i32) {
    let (significand, exponent) = rendered
        .split_once('e')
        .expect("`{:e}` always writes an exponent");
    let exponent = exponent
        .parse()
        .expect("`{:e}` always writes a decimal exponent");
    (significand, exponent)
}

/// C always signs an exponent and pads it to two digits; Rust writes
/// neither the sign nor the padding.
fn push_exponent(text: &mut String, exponent: i32, upper: bool) {
    text.push(if upper { 'E' } else { 'e' });
    text.push(if exponent < 0 { '-' } else { '+' });
    let magnitude = exponent.unsigned_abs();
    if magnitude < 10 {
        text.push('0');
    }
    text.push_str(&format!("{magnitude}"));
}

/// Drop a fraction's trailing zeros, and the radix point with them when
/// nothing of the fraction survives.
fn trim_fraction(text: &mut String) {
    if !text.contains('.') {
        return;
    }
    while text.ends_with('0') {
        text.pop();
    }
    if text.ends_with('.') {
        text.pop();
    }
}

// ---------------------------------------------------------------------
// The expectations below are C's, and were taken from the reference
// shell rather than from reading the standard: each is what
// `tests/.build/ref/src/dash` prints for the same conversion.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    /// Build a spec from its C spelling, e.g. `"-+08.3"`.
    fn spec(text: &str) -> Spec {
        let mut spec = Spec::bare();
        let bytes = text.as_bytes();
        let mut at = 0;
        while at < bytes.len() && spec.flag(bytes[at]) {
            at += 1;
        }
        let start = at;
        while at < bytes.len() && bytes[at].is_ascii_digit() {
            at += 1;
        }
        if at > start {
            spec.set_width(text[start..at].parse().unwrap());
        }
        if at < bytes.len() && bytes[at] == b'.' {
            at += 1;
            let start = at;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
            spec.set_precision(text[start..at].parse().unwrap_or(0));
        }
        spec
    }

    fn signed(text: &str, value: i64) -> String {
        String::from_utf8(spec(text).signed(value)).unwrap()
    }

    fn unsigned(text: &str, value: u64, conversion: u8) -> String {
        String::from_utf8(spec(text).unsigned(value, conversion)).unwrap()
    }

    fn double(text: &str, value: f64, conversion: u8) -> String {
        String::from_utf8(spec(text).double(value, conversion)).unwrap()
    }

    #[test]
    fn bare_spec_prints_digits_alone() {
        assert_eq!(signed("", 0), "0");
        assert_eq!(signed("", -42), "-42");
        assert_eq!(signed("", i64::MIN), "-9223372036854775808");
        assert_eq!(unsigned("", u64::MAX, b'u'), "18446744073709551615");
    }

    #[test]
    fn width_pads_and_never_truncates() {
        assert_eq!(signed("5", 42), "   42");
        assert_eq!(signed("-5", 42), "42   ");
        assert_eq!(signed("05", 42), "00042");
        assert_eq!(signed("05", -42), "-0042");
        assert_eq!(signed("-05", 42), "42   ");
        assert_eq!(signed("2", 12345), "12345");
    }

    #[test]
    fn a_negative_star_width_left_justifies() {
        let mut left = Spec::bare();
        left.set_width(-5);
        assert_eq!(String::from_utf8(left.signed(42)).unwrap(), "42   ");

        let mut none = Spec::bare();
        none.set_precision(-1);
        assert_eq!(String::from_utf8(none.signed(0)).unwrap(), "0");
    }

    #[test]
    fn sign_flags_apply_to_signed_values() {
        assert_eq!(signed("+", 42), "+42");
        assert_eq!(signed(" ", 42), " 42");
        assert_eq!(signed("+", -42), "-42");
        /* Both flags: C ignores the space. */
        assert_eq!(signed("+ ", 42), "+42");
        assert_eq!(unsigned("+", 42, b'u'), "42");
    }

    #[test]
    fn integer_precision_sets_a_digit_floor() {
        assert_eq!(signed(".5", 42), "00042");
        assert_eq!(signed(".0", 0), "");
        assert_eq!(signed(".0", 1), "1");
        /* A precision defeats the zero flag, so the field pads with
         * spaces around the zero-extended digits. */
        assert_eq!(signed("08.3", 42), "     042");
        assert_eq!(signed("-8.3", 42), "042     ");
    }

    #[test]
    fn radix_conversions_use_their_own_digits() {
        assert_eq!(unsigned("", 255, b'o'), "377");
        assert_eq!(unsigned("", 255, b'x'), "ff");
        assert_eq!(unsigned("", 255, b'X'), "FF");
        assert_eq!(unsigned("#", 255, b'x'), "0xff");
        assert_eq!(unsigned("#", 255, b'X'), "0XFF");
        assert_eq!(unsigned("#", 255, b'o'), "0377");
        /* `#` prints no prefix for zero, but octal still gets its zero. */
        assert_eq!(unsigned("#", 0, b'x'), "0");
        assert_eq!(unsigned("#", 0, b'o'), "0");
        assert_eq!(unsigned("#.0", 0, b'o'), "0");
        assert_eq!(unsigned("#08", 255, b'x'), "0x0000ff");
    }

    #[test]
    fn strings_and_precision_count_bytes() {
        let bytes = [b'a', 0, 0xff, b'z'];
        assert_eq!(spec("").string(&bytes), bytes);
        assert_eq!(spec(".2").string(&bytes), b"a\0");
        assert_eq!(spec("6").string(&bytes), b"  a\0\xffz");
        assert_eq!(spec("-6").string(&bytes), b"a\0\xffz  ");
        assert_eq!(spec(".0").string(&bytes), b"");
    }

    #[test]
    fn a_character_may_be_nul() {
        assert_eq!(spec("").character(0), b"\0");
        assert_eq!(spec("3").character(b'x'), b"  x");
        assert_eq!(spec("").character(0xff), b"\xff");
    }

    #[test]
    fn fixed_notation_defaults_to_six_places() {
        assert_eq!(double("", 1.5, b'f'), "1.500000");
        assert_eq!(double(".0", 2.5, b'f'), "2");
        assert_eq!(double(".0", 3.5, b'f'), "4");
        assert_eq!(double("#.0", 3.5, b'f'), "4.");
        assert_eq!(double(".2", -0.125, b'f'), "-0.12");
        assert_eq!(double("010.2", -0.125, b'f'), "-000000.12");
    }

    #[test]
    fn scientific_notation_signs_its_exponent() {
        assert_eq!(double("", 123.456, b'e'), "1.234560e+02");
        assert_eq!(double("", 123.456, b'E'), "1.234560E+02");
        assert_eq!(double("", 0.0, b'e'), "0.000000e+00");
        assert_eq!(double(".2", 0.000123, b'e'), "1.23e-04");
        assert_eq!(double(".0", 1.0, b'e'), "1e+00");
        assert_eq!(double("#.0", 1.0, b'e'), "1.e+00");
        assert_eq!(double(".2", 1e300, b'e'), "1.00e+300");
    }

    #[test]
    fn general_notation_picks_and_trims() {
        assert_eq!(double("", 100000.0, b'g'), "100000");
        assert_eq!(double("", 1000000.0, b'g'), "1e+06");
        assert_eq!(double("", 0.0001, b'g'), "0.0001");
        assert_eq!(double("", 0.00001, b'g'), "1e-05");
        assert_eq!(double("", 1.5, b'g'), "1.5");
        assert_eq!(double("", 0.0, b'g'), "0");
        assert_eq!(double("#", 0.0, b'g'), "0.00000");
        assert_eq!(double(".1", 1234.0, b'g'), "1e+03");
        assert_eq!(double(".0", 1234.0, b'g'), "1e+03");
        assert_eq!(double("", 1000000.0, b'G'), "1E+06");
    }

    #[test]
    fn infinities_and_nan_ignore_zero_padding() {
        assert_eq!(double("", f64::INFINITY, b'f'), "inf");
        assert_eq!(double("", f64::NEG_INFINITY, b'f'), "-inf");
        assert_eq!(double("", f64::INFINITY, b'F'), "INF");
        assert_eq!(double("", f64::NAN, b'f'), "nan");
        assert_eq!(double("", -f64::NAN, b'f'), "-nan");
        assert_eq!(double("08", f64::INFINITY, b'f'), "     inf");
        assert_eq!(double("+8", f64::INFINITY, b'e'), "    +inf");
    }

    #[test]
    fn large_precisions_expand_the_exact_value() {
        /* The exact binary value of 0.1, which is what C prints too. */
        assert_eq!(double(".20", 0.1, b'f'), "0.10000000000000000555");
        assert_eq!(double(".0", 1e21, b'f'), "1000000000000000000000");
    }

    #[test]
    fn width_and_precision_combine() {
        assert_eq!(double("10.3", 1.5, b'f'), "     1.500");
        assert_eq!(double("-10.3", 1.5, b'f'), "1.500     ");
        assert_eq!(double("010.3", 1.5, b'f'), "000001.500");
        assert_eq!(signed("+08", 42), "+0000042");
        assert_eq!(signed(" 08", 42), " 0000042");
        assert_eq!(unsigned("08", 255, b'x'), "000000ff");
    }

    /// Left justification is laid out with spaces, so it defeats `0`
    /// however the two flags are written.
    #[test]
    fn a_left_flag_defeats_zero_padding() {
        assert_eq!(signed("-08", 42), "42      ");
        assert_eq!(unsigned("-#08", 255, b'x'), "0xff    ");
        assert_eq!(double("-010.2", 1.5, b'f'), "1.50      ");
    }

    /// The sign comes from the sign *bit*, so a negative zero keeps it
    /// in every notation.
    #[test]
    fn negative_zero_keeps_its_sign() {
        assert_eq!(double("", -0.0, b'f'), "-0.000000");
        assert_eq!(double("", -0.0, b'g'), "-0");
        assert_eq!(double("", -0.0, b'e'), "-0.000000e+00");
        assert_eq!(double("", -0.0, b'a'), "-0x0p+0");
    }

    /// `#` on octal raises the precision just far enough to force one
    /// leading zero, and does nothing when a precision already did.
    #[test]
    fn octal_alternate_form_adds_one_zero() {
        assert_eq!(unsigned("#.3", 8, b'o'), "010");
        assert_eq!(unsigned("#.4", 8, b'o'), "0010");
        assert_eq!(unsigned("#", 8, b'o'), "010");
        assert_eq!(unsigned("", 0, b'o'), "0");
    }

    #[test]
    fn hexadecimal_transcribes_the_mantissa() {
        assert_eq!(double("", 1.0, b'a'), "0x1p+0");
        assert_eq!(double("", 1.5, b'a'), "0x1.8p+0");
        assert_eq!(double("", 3.0, b'a'), "0x1.8p+1");
        assert_eq!(double("", 0.1, b'a'), "0x1.999999999999ap-4");
        assert_eq!(double("", 1e300, b'a'), "0x1.7e43c8800759cp+996");
        assert_eq!(double("", 1.5, b'A'), "0X1.8P+0");
        /* Zero and its sign, and the subnormal with a single bit set. */
        assert_eq!(double("", 0.0, b'a'), "0x0p+0");
        assert_eq!(double("", -0.0, b'a'), "-0x0p+0");
        assert_eq!(double("", 5e-324, b'a'), "0x0.0000000000001p-1022");
    }

    #[test]
    fn hexadecimal_rounds_to_even() {
        /* A carry out of the mantissa lands in the digit before the
         * point; C does not renormalise it away. */
        assert_eq!(double(".0", 1.5, b'a'), "0x2p+0");
        assert_eq!(double(".0", 1.9, b'a'), "0x2p+0");
        /* Below the halfway point, and exactly on it with an even digit
         * before the point. */
        assert_eq!(double(".0", 2.5, b'a'), "0x1p+1");
        assert_eq!(double(".2", 0.1, b'a'), "0x1.9ap-4");
        /* `#` forces the point that a bare rendering drops, and a
         * precision past the mantissa's last bit zero-fills. */
        assert_eq!(double("#", 1.0, b'a'), "0x1.p+0");
        assert_eq!(double(".15", 1.5, b'a'), "0x1.800000000000000p+0");
        /* Zero padding falls between the `0x` and the digits. */
        assert_eq!(double("012", 1.0, b'a'), "0x0000001p+0");
    }
}
