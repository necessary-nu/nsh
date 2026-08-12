//! The text of a number, before a conversion's flags get to it.
//!
//! [`super`] is about what C's flags *mean* -- sign, radix prefix, field
//! padding, `%g`'s choice of style. This is the layer under that: the
//! digits themselves, and the small differences between how Rust writes
//! them and how C does. Nothing here knows what a width is.

/// A conversion's digits, with the zeros a precision asks for counted
/// rather than built.
///
/// A precision may name more places than a `Vec` should hold on the way
/// to deciding whether the conversion fits at all: `%.2147483646f` is
/// refused outright, and `%.2000000000f` is two gigabytes of zeros in
/// either shell. Keeping the run as a length lets the field's size be
/// known before any of it is allocated.
#[derive(Default)]
pub(super) struct Number {
    /// What precedes the zero run.
    pub(super) head: Vec<u8>,
    /// The zeros themselves.
    pub(super) zeros: usize,
    /// What follows it -- an exponent, or the digits a precision padded
    /// on the left.
    pub(super) tail: Vec<u8>,
}

impl Number {
    /// Digits with no counted run in them.
    pub(super) fn plain(head: Vec<u8>) -> Self {
        Self {
            head,
            ..Self::default()
        }
    }

    pub(super) fn len(&self) -> usize {
        self.head.len() + self.zeros + self.tail.len()
    }

    /// Put a leading zero in front unless there is one already, which is
    /// how C words `%#o`: raise the precision just far enough to force
    /// one.
    pub(super) fn force_leading_zero(&mut self) {
        let leading = if let Some(&byte) = self.head.first() {
            Some(byte)
        } else if self.zeros > 0 {
            Some(b'0')
        } else {
            self.tail.first().copied()
        };
        if leading != Some(b'0') {
            self.head.insert(0, b'0');
        }
    }

    pub(super) fn write_to(&self, out: &mut Vec<u8>) {
        out.extend_from_slice(&self.head);
        out.resize(out.len() + self.zeros, b'0');
        out.extend_from_slice(&self.tail);
    }
}

/// One hexadecimal digit, in the case the conversion character asked for.
pub(super) fn hex_digit(nibble: u8, upper: bool) -> u8 {
    match nibble {
        0..=9 => b'0' + nibble,
        _ if upper => b'A' + nibble - 10,
        _ => b'a' + nibble - 10,
    }
}

/// Split Rust's `{:e}` rendering into its significand and exponent.
pub(super) fn split_exponent(rendered: &str) -> (&str, i32) {
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
pub(super) fn push_exponent(text: &mut String, exponent: i32, upper: bool) {
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
pub(super) fn trim_fraction(text: &mut String) {
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

#[cfg(test)]
mod tests {
    use super::*;

    fn written(number: &Number) -> Vec<u8> {
        let mut out = Vec::new();
        number.write_to(&mut out);
        out
    }

    /// The counted run is written where it sits, and the length agrees
    /// with what gets written -- which is what the field measures before
    /// it decides whether to build any of it.
    #[test]
    fn a_counted_run_writes_in_place() {
        let number = Number {
            head: b"1.".to_vec(),
            zeros: 3,
            tail: b"e+00".to_vec(),
        };
        assert_eq!(written(&number), b"1.000e+00");
        assert_eq!(number.len(), 9);

        let plain = Number::plain(b"42".to_vec());
        assert_eq!(written(&plain), b"42");
        assert_eq!(plain.len(), 2);
    }

    /// The zero `%#o` forces is added only when no part of the digits
    /// already leads with one.
    #[test]
    fn a_forced_zero_lands_once() {
        let mut digits = Number::plain(b"10".to_vec());
        digits.force_leading_zero();
        assert_eq!(written(&digits), b"010");

        /* Already there, in the head, in the counted run, and in the
         * digits a precision padded on the left. */
        for mut already in [
            Number::plain(b"0".to_vec()),
            Number {
                head: Vec::new(),
                zeros: 2,
                tail: b"7".to_vec(),
            },
            Number {
                head: Vec::new(),
                zeros: 0,
                tail: b"07".to_vec(),
            },
        ] {
            let before = written(&already);
            already.force_leading_zero();
            assert_eq!(written(&already), before);
        }

        /* A precision of zero prints nothing of a zero value, and `#`
         * puts the digit back. */
        let mut nothing = Number::default();
        nothing.force_leading_zero();
        assert_eq!(written(&nothing), b"0");
    }

    /// C always signs an exponent and pads it to two digits.
    #[test]
    fn an_exponent_is_signed_and_padded() {
        let mut text = String::new();
        push_exponent(&mut text, 0, false);
        assert_eq!(text, "e+00");

        for (exponent, upper, want) in [
            (-4, false, "e-04"),
            (5, false, "e+05"),
            (300, false, "e+300"),
            (-308, true, "E-308"),
            (2, true, "E+02"),
        ] {
            let mut text = String::new();
            push_exponent(&mut text, exponent, upper);
            assert_eq!(text, want);
        }
    }

    #[test]
    fn a_rendered_exponent_splits_off() {
        assert_eq!(split_exponent("1.5e3"), ("1.5", 3));
        assert_eq!(split_exponent("1e-4"), ("1", -4));
        assert_eq!(split_exponent("9.999e0"), ("9.999", 0));
    }

    /// Trimming takes the point with the last zero, and leaves digits
    /// that were never a fraction alone.
    #[test]
    fn trimming_stops_at_the_point() {
        for (before, after) in [
            ("1.500", "1.5"),
            ("1.000", "1"),
            ("0.000", "0"),
            ("100", "100"),
            ("0", "0"),
        ] {
            let mut text = String::from(before);
            trim_fraction(&mut text);
            assert_eq!(text, after);
        }
    }

    #[test]
    fn a_nibble_takes_the_conversions_case() {
        assert_eq!(hex_digit(9, false), b'9');
        assert_eq!(hex_digit(10, false), b'a');
        assert_eq!(hex_digit(10, true), b'A');
        assert_eq!(hex_digit(15, true), b'F');
    }
}
