//! A magnitude, in each of the four notations C writes one in.
//!
//! `%f`, `%e` and `%g` are three readings of a single decimal expansion,
//! and `%g` is defined in terms of the other two -- it picks between them
//! by where the exponent falls, which is why they are one subject and not
//! three. What they share besides that is [`EXACT_PLACES`]: Rust holds a
//! formatting precision in a `u16`, so the places past a double's exact
//! expansion have to be counted here rather than asked for, and every
//! rendering below returns a [`Number`] with that run left as a length.
//!
//! `%a` is the one conversion Rust's formatting cannot spell, and it is
//! also the one that needs no decimal arithmetic at all: a hexadecimal
//! float *is* the double's IEEE-754 fields written out, so
//! [`Spec::hexadecimal`] transcribes them from [`f64::to_bits`].
//!
//! What is not here is what a conversion *specification* means. [`super`]
//! keeps the flags, the width, the precision and the field; each of these
//! renderings hands it digits and a sign and lets it do the laying out.

use super::Spec;
use super::digits::{Number, hex_digit, push_exponent, split_exponent, trim_fraction};

/// The most places after the point `format!` is ever asked for.
///
/// Rust holds a formatting precision in a `u16`, and a specification may
/// name more places than that -- asking for them panics, which is the
/// one thing a builtin must never do to the shell hosting it. It need
/// not ask: every finite double is a whole multiple of 2^-1074, so its
/// decimal expansion terminates within this many places and every place
/// past them is a zero this module writes for itself.
const EXACT_PLACES: usize = 1074;

impl Spec {
    /// `%a`, `%A`, `%e`, `%E`, `%f`, `%F`, `%g` and `%G`.
    pub(in super::super) fn double(&self, value: f64, conversion: u8) -> Option<Vec<u8>> {
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
            .map_or(FRACTION_DIGITS, |precision| precision.min(FRACTION_DIGITS));
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

// The expectations below are C's in the same way the parent module's are:
// each is what `tests/.build/ref/src/dash` prints for the same conversion,
// not what reading the standard suggests it should.
#[cfg(test)]
mod tests {
    use super::super::tests::{double, signed};

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

    /// The sign comes from the sign *bit*, so a negative zero keeps it
    /// in every notation.
    #[test]
    fn negative_zero_keeps_its_sign() {
        assert_eq!(double("", -0.0, b'f'), "-0.000000");
        assert_eq!(double("", -0.0, b'g'), "-0");
        assert_eq!(double("", -0.0, b'e'), "-0.000000e+00");
        assert_eq!(double("", -0.0, b'a'), "-0x0p+0");
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
