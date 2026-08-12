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
//!
//! The digit text those conversions are assembled from -- and the ways
//! Rust spells it differently from C -- is [`digits`].

use libc::c_int;

mod digits;

use digits::{Number, hex_digit, push_exponent, split_exponent, trim_fraction};

/// The most bytes one conversion may render.
///
/// The C printed each conversion through `vsnprintf`, which counts what
/// it wrote in an `int`: glibc refuses a result it cannot count, hands
/// back -1, and `xvasprintf` turns that into a fatal `xvsnprintf failed`.
/// So `printf '%2147483647d' 1` is two gigabytes of spaces and
/// `%2147483648d` is an error with no output at all. Both the width and
/// precision a specification writes out and the length of what it
/// renders are held to this.
pub(super) const LIMIT: usize = c_int::MAX as usize;

/// The most places after the point `format!` is ever asked for.
///
/// Rust holds a formatting precision in a `u16`, and a specification may
/// name more places than that -- asking for them panics, which is the
/// one thing a builtin must never do to the shell hosting it. It need
/// not ask: every finite double is a whole multiple of 2^-1074, so its
/// decimal expansion terminates within this many places and every place
/// past them is a zero this module writes for itself.
const EXACT_PLACES: usize = 1074;

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
    /// A width or precision written out in digits that ran past what the
    /// C held it in. The conversion is refused rather than clamped, so
    /// the flag travels to the field instead of being acted on here.
    over: bool,
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
            over: false,
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

    /// Set the field width from a `*` argument, which may arrive
    /// negative -- C reads that as the `-` flag applied to its
    /// magnitude, and a magnitude one past the range is how `INT_MIN`
    /// reaches [`LIMIT`].
    pub(super) fn set_width(&mut self, value: c_int) {
        self.left |= value < 0;
        self.width = value.unsigned_abs() as usize;
    }

    /// Set the precision from a `*` argument. A negative one means "no
    /// precision".
    pub(super) fn set_precision(&mut self, value: c_int) {
        self.precision = if value < 0 {
            None
        } else {
            Some(value as usize)
        };
    }

    /// Set the field width a specification wrote out in digits.
    pub(super) fn set_written_width(&mut self, value: usize) {
        self.over |= value > LIMIT;
        self.width = value;
    }

    /// Set the precision a specification wrote out in digits.
    pub(super) fn set_written_precision(&mut self, value: usize) {
        self.over |= value > LIMIT;
        self.precision = Some(value);
    }

    /// Lay a number out in the field, or refuse a conversion the C could
    /// not have counted.
    ///
    /// Zero padding goes *after* the sign and any radix prefix, which is
    /// what distinguishes it from the space padding either side of a
    /// string; a string reaches here with no prefix and `zeros` false.
    fn field(&self, prefix: &[u8], body: &Number, zeros: bool) -> Option<Vec<u8>> {
        let len = prefix.len() + body.len();
        if self.over || len > LIMIT || self.width > LIMIT {
            return None;
        }
        let fill = self.width.saturating_sub(len);
        let mut out = Vec::with_capacity(len.max(self.width));

        if !self.left && !zeros {
            out.resize(fill, b' ');
        }
        out.extend_from_slice(prefix);
        if !self.left && zeros {
            out.resize(out.len() + fill, b'0');
        }
        body.write_to(&mut out);
        if self.left {
            out.resize(self.width.max(len), b' ');
        }
        Some(out)
    }

    /// `%s`: the argument's bytes, truncated by the precision.
    ///
    /// Byte-oriented, so an argument that is not valid UTF-8 keeps its
    /// bytes and a precision counts them the way C counts them.
    pub(super) fn string(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        let take = self.precision.map_or(bytes.len(), |p| p.min(bytes.len()));
        self.field(&[], &Number::plain(bytes[..take].to_vec()), false)
    }

    /// `%c`: one byte, which may be NUL.
    pub(super) fn character(&self, value: u8) -> Option<Vec<u8>> {
        self.field(&[], &Number::plain(vec![value]), false)
    }

    /// Apply an integer precision -- a minimum digit count, where a
    /// precision of zero prints a zero value as nothing at all.
    fn integer_digits(&self, text: &str) -> Number {
        let Some(precision) = self.precision else {
            return Number::plain(text.as_bytes().to_vec());
        };
        if text == "0" && precision == 0 {
            return Number::plain(Vec::new());
        }
        Number {
            head: Vec::new(),
            zeros: precision.saturating_sub(text.len()),
            tail: text.as_bytes().to_vec(),
        }
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
    pub(super) fn signed(&self, value: i64) -> Option<Vec<u8>> {
        let digits = self.integer_digits(&format!("{}", value.unsigned_abs()));
        /* C ignores `0` once a precision has said how many digits there
         * are to print. */
        let zeros = self.zero && self.precision.is_none();
        self.field(self.sign(value < 0), &digits, zeros)
    }

    /// `%o`, `%u`, `%x` and `%X`.
    pub(super) fn unsigned(&self, value: u64, conversion: u8) -> Option<Vec<u8>> {
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
                b'o' => digits.force_leading_zero(),
                b'x' if value != 0 => prefix = b"0x",
                b'X' if value != 0 => prefix = b"0X",
                _ => {}
            }
        }

        let zeros = self.zero && self.precision.is_none();
        self.field(prefix, &digits, zeros)
    }

    /// `%a`, `%A`, `%e`, `%E`, `%f`, `%F`, `%g` and `%G`.
    pub(super) fn double(&self, value: f64, conversion: u8) -> Option<Vec<u8>> {
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
            return self.field(sign, &Number::plain(word.to_vec()), false);
        }

        let magnitude = value.abs();
        match conversion.to_ascii_lowercase() {
            b'f' => self.field(sign, &self.fixed(magnitude), self.zero),
            b'e' => {
                let body = self.scientific(magnitude, self.precision.unwrap_or(6), upper);
                self.field(sign, &body, self.zero)
            }
            b'g' => self.field(sign, &self.general(magnitude, upper), self.zero),
            _ => {
                /* `%a`'s radix prefix joins the sign, so zero padding
                 * lands after it the way it does for `%#x`. */
                let (marker, body) = self.hexadecimal(magnitude, upper);
                let mut prefix = sign.to_vec();
                prefix.extend_from_slice(marker);
                self.field(&prefix, &body, self.zero)
            }
        }
    }

    /// `%f`: Rust's fixed-point formatting, with C's default precision.
    fn fixed(&self, magnitude: f64) -> Number {
        let precision = self.precision.unwrap_or(6);
        let places = precision.min(EXACT_PLACES);
        let mut text = format!("{magnitude:.places$}");
        if precision == 0 && self.alt {
            text.push('.');
        }
        Number {
            head: text.into_bytes(),
            zeros: precision - places,
            tail: Vec::new(),
        }
    }

    /// `%e`: Rust's `{:e}`, with C's exponent spelling.
    fn scientific(&self, magnitude: f64, precision: usize, upper: bool) -> Number {
        let places = precision.min(EXACT_PLACES);
        let rendered = format!("{magnitude:.places$e}");
        let (significand, exponent) = split_exponent(&rendered);
        let mut text = String::from(significand);
        if precision == 0 && self.alt {
            text.push('.');
        }
        let mut tail = String::new();
        push_exponent(&mut tail, exponent, upper);
        Number {
            head: text.into_bytes(),
            zeros: precision - places,
            tail: tail.into_bytes(),
        }
    }

    /// `%g`: C chooses between `%e` and `%f` by where the exponent falls,
    /// then drops trailing fractional zeros unless `#` asked to keep them.
    fn general(&self, magnitude: f64, upper: bool) -> Number {
        /* "Let P equal the precision, 6 if it is omitted, 1 if it is
         * zero" -- then style e is used when the exponent X of the value
         * rendered with precision P-1 is below -4 or at least P. */
        let significant = self.precision.unwrap_or(6).max(1);
        let places = (significant - 1).min(EXACT_PLACES);
        let rendered = format!("{magnitude:.places$e}");
        let (significand, exponent) = split_exponent(&rendered);

        if i64::from(exponent) < -4 || i64::from(exponent) >= significant as i64 {
            let mut text = String::from(significand);
            let mut zeros = significant - 1 - places;
            if self.alt {
                /* Style e reached only because rounding carried the
                 * value up a decade -- its own exponent, one lower,
                 * would have chosen style f. C has a single digit to
                 * show for that and no zeros behind it to keep. */
                let carried = i64::from(exponent) == significant as i64
                    && carried_decade(magnitude, exponent);
                if significant == 1 || carried {
                    text.truncate(1);
                    text.push('.');
                    zeros = 0;
                }
            } else {
                /* Every counted zero is a trailing one, so trimming
                 * takes the whole run with it. */
                trim_fraction(&mut text);
                zeros = 0;
            }
            let mut tail = String::new();
            push_exponent(&mut tail, exponent, upper);
            return Number {
                head: text.into_bytes(),
                zeros,
                tail: tail.into_bytes(),
            };
        }

        /* Wider than the precision itself: a negative exponent adds
         * places to it, so the sum is counted where it cannot wrap into
         * a length the field would accept. */
        let scale = significant as i64 - 1 - i64::from(exponent);
        let places = scale.min(EXACT_PLACES as i64) as usize;
        let mut text = format!("{magnitude:.places$}");
        let mut zeros = (scale - places as i64) as usize;
        if self.alt {
            if scale == 0 {
                text.push('.');
            }
        } else {
            trim_fraction(&mut text);
            zeros = 0;
        }
        Number {
            head: text.into_bytes(),
            zeros,
            tail: Vec::new(),
        }
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
    fn hexadecimal(&self, magnitude: f64, upper: bool) -> (&'static [u8], Number) {
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
        /* Without a precision C writes the shortest exact form; with one
         * it writes exactly that many digits, and every digit past the
         * mantissa's last bit is a zero. */
        let zeros = match self.precision {
            None => {
                while nibbles.last() == Some(&0) {
                    nibbles.pop();
                }
                0
            }
            Some(precision) => {
                nibbles.truncate(kept);
                precision - kept
            }
        };

        let mut text = vec![b'0' + lead];
        if !nibbles.is_empty() || zeros > 0 || self.alt {
            text.push(b'.');
        }
        text.extend(nibbles.into_iter().map(|nibble| hex_digit(nibble, upper)));

        let mut tail = Vec::new();
        tail.push(if upper { b'P' } else { b'p' });
        tail.push(if exponent < 0 { b'-' } else { b'+' });
        tail.extend_from_slice(format!("{}", exponent.unsigned_abs()).as_bytes());

        let body = Number {
            head: text,
            zeros,
            tail,
        };
        (if upper { b"0X" } else { b"0x" }, body)
    }
}

/// Whether rounding carried `magnitude` up into the decade `exponent`
/// names.
///
/// It is the only way `%g` reaches style e from a value whose own
/// exponent would have chosen style f, and C shows a single digit for
/// it: the carry replaces the whole digit string with one `1`, so `#`
/// finds no trailing zeros to keep and `printf '%#g' 999999.5` is
/// `1.e+06` rather than the `1.00000e+06` the standard describes. The
/// comparison is made against the value's exact decimal expansion, which
/// terminates well inside the places asked for here, so no second
/// rounding stands between the two exponents.
fn carried_decade(magnitude: f64, exponent: i32) -> bool {
    /// More significant digits than a double's exact expansion has.
    const EXACT_DIGITS: usize = 767;

    let exact = format!("{magnitude:.*e}", EXACT_DIGITS);
    split_exponent(&exact).1 < exponent
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
            spec.set_written_width(text[start..at].parse().unwrap());
        }
        if at < bytes.len() && bytes[at] == b'.' {
            at += 1;
            let start = at;
            while at < bytes.len() && bytes[at].is_ascii_digit() {
                at += 1;
            }
            spec.set_written_precision(text[start..at].parse().unwrap_or(0));
        }
        spec
    }

    /// The bytes of a field the C would have laid out. A conversion it
    /// would have refused is `None`, which [`an_oversized_field_is_refused`]
    /// is about.
    fn bytes(field: Option<Vec<u8>>) -> Vec<u8> {
        field.expect("a field inside the limit")
    }

    /// The same, for the conversions that cannot render a byte outside
    /// ASCII.
    fn text(field: Option<Vec<u8>>) -> String {
        String::from_utf8(bytes(field)).unwrap()
    }

    fn signed(spelling: &str, value: i64) -> String {
        text(spec(spelling).signed(value))
    }

    fn unsigned(spelling: &str, value: u64, conversion: u8) -> String {
        text(spec(spelling).unsigned(value, conversion))
    }

    fn double(spelling: &str, value: f64, conversion: u8) -> String {
        text(spec(spelling).double(value, conversion))
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
        assert_eq!(text(left.signed(42)), "42   ");

        let mut none = Spec::bare();
        none.set_precision(-1);
        assert_eq!(text(none.signed(0)), "0");
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
        let word = [b'a', 0, 0xff, b'z'];
        assert_eq!(bytes(spec("").string(&word)), word);
        assert_eq!(bytes(spec(".2").string(&word)), b"a\0");
        assert_eq!(bytes(spec("6").string(&word)), b"  a\0\xffz");
        assert_eq!(bytes(spec("-6").string(&word)), b"a\0\xffz  ");
        assert_eq!(bytes(spec(".0").string(&word)), b"");
    }

    #[test]
    fn a_character_may_be_nul() {
        assert_eq!(bytes(spec("").character(0)), b"\0");
        assert_eq!(bytes(spec("3").character(b'x')), b"  x");
        assert_eq!(bytes(spec("").character(0xff)), b"\xff");
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

    /// A `%g` that reaches style e only because rounding carried the
    /// value into the next decade has one digit to show for it, so `#`
    /// finds nothing to keep.
    #[test]
    fn a_switching_carry_shows_one_digit() {
        assert_eq!(double("#", 999999.5, b'g'), "1.e+06");
        assert_eq!(double("#.3", 999.5, b'g'), "1.e+03");
        assert_eq!(double("#.2", 99.5, b'g'), "1.e+02");
        assert_eq!(double("#.1", 9.5, b'g'), "1.e+01");
        assert_eq!(double("#", 999999.5, b'G'), "1.E+06");

        /* Style e the value chose for itself keeps every digit the
         * precision asked for, carry or no carry. */
        assert_eq!(double("#", 1000000.0, b'g'), "1.00000e+06");
        assert_eq!(double("#.3", 999999.5, b'g'), "1.00e+06");
        assert_eq!(double("#", 9999999.5, b'g'), "1.00000e+07");

        /* A carry that stays inside style f zero-fills as ever, and
         * without `#` the zeros go whichever way it went. */
        assert_eq!(double("#", 9.9999995, b'g'), "10.0000");
        assert_eq!(double("#.3", 0.9995, b'g'), "1.00");
        assert_eq!(double("#.2", 0.0000999995, b'g'), "0.00010");
        assert_eq!(double("", 999999.5, b'g'), "1e+06");
    }

    /// A precision naming more places than Rust's formatter holds is a
    /// run of zeros this module writes for itself, so the field arrives
    /// whole where asking for it would have panicked.
    #[test]
    fn huge_precisions_write_their_own_zeros() {
        let fixed = double(".65536", 1.0, b'f');
        assert_eq!(fixed.len(), 65538);
        assert_eq!(fixed.trim_end_matches('0'), "1.");

        let scientific = double(".65536", 1.0, b'e');
        assert_eq!(scientific.len(), 65542);
        assert!(scientific.starts_with("1.0") && scientific.ends_with("0e+00"));

        let hexadecimal = double(".65536", 1.0, b'a');
        assert_eq!(hexadecimal.len(), 65543);
        assert!(hexadecimal.starts_with("0x1.0") && hexadecimal.ends_with("0p+0"));

        assert_eq!(signed(".65536", 1).len(), 65536);

        /* `%g` keeps only the digits it was asked to keep, so the same
         * precision is a handful of bytes. */
        assert_eq!(double(".65537", 1.0, b'g'), "1");
        assert_eq!(
            double(".65536", 0.0001, b'g'),
            "0.000100000000000000004792173602385929598312941379845142364501953125"
        );
    }

    /// A field longer than the C counted is refused, and nothing of it
    /// is built on the way to finding out.
    #[test]
    fn an_oversized_field_is_refused() {
        /* The places fit an `int`; the field they ask for does not. */
        assert!(spec(".2147483646").double(1.0, b'f').is_none());
        assert!(spec(".2147483647").double(1.0, b'a').is_none());
        /* The digits themselves ran past it. */
        assert!(spec("2147483648").signed(1).is_none());
        assert!(spec(".2147483648").string(b"abc").is_none());
        /* `INT_MIN` is a `-` flag over a magnitude one past the range. */
        let mut star = Spec::bare();
        star.set_width(c_int::MIN);
        assert!(star.signed(1).is_none());
        /* A precision at the limit that no field grows by is not. */
        assert_eq!(bytes(spec(".2147483647").string(b"abc")), b"abc");
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
