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
//! What that leaves here is what a conversion *specification* means: the
//! flags, the width, the precision, and the field they lay a number out
//! in. How a magnitude becomes the digits of one of C's four float
//! notations is [`float`]; the digit text every conversion is assembled
//! from, and the ways Rust spells it differently from C, is [`digits`].

mod digits;
mod float;

use digits::Number;

/// The most bytes one conversion may render.
///
/// The C printed each conversion through `vsnprintf`, which counts what
/// it wrote in an `int`: glibc refuses a result it cannot count, hands
/// back -1, and `xvasprintf` turns that into a fatal `xvsnprintf failed`.
/// So `printf '%2147483647d' 1` is two gigabytes of spaces and
/// `%2147483648d` is an error with no output at all. Both the width and
/// precision a specification writes out and the length of what it
/// renders are held to this.
pub(super) const LIMIT: usize = i32::MAX as usize;

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
    /// The specification's own text, when it is one glibc could not
    /// read. See [`Spec::set_literal`].
    literal: Option<Vec<u8>>,
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
            literal: None,
        }
    }

    /// Record one flag character, or report that `ch` ends the flags.
    pub(super) fn flag(&mut self, character: u8) -> bool {
        match character {
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
    pub(super) fn set_width(&mut self, value: i32) {
        self.left |= value < 0;
        self.width = value.unsigned_abs() as usize;
    }

    /// Set the precision from a `*` argument. A negative one means "no
    /// precision".
    pub(super) fn set_precision(&mut self, value: i32) {
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

    /// Record that this specification renders as itself, having stopped
    /// C's `printf` at `stop` with `rest` still unread.
    ///
    /// A `*` after the digits of a width or precision is a `*` where C's
    /// `printf` has no argument waiting for one. glibc answers a
    /// character it has no rule for by writing back what it had
    /// understood, then that character, and then carrying on with the
    /// rest of the format as ordinary text -- so `rest` is emitted
    /// verbatim and never read as a width or precision at all.
    ///
    /// What it writes back is its own spelling, not the one the user
    /// typed. The flags come out in a fixed order however they were
    /// written, `+` hides a ` `, `-` takes the padding away from `0`, a
    /// width of zero is not written, and a width that arrived as a `*`
    /// is written as the digits it stood for.
    pub(super) fn set_unreadable(&mut self, stop: u8, rest: &[u8]) {
        let mut text = vec![b'%'];
        if self.alt {
            text.push(b'#');
        }
        if self.plus {
            text.push(b'+');
        } else if self.space {
            text.push(b' ');
        }
        if self.left {
            text.push(b'-');
        }
        if self.zero && !self.left {
            text.push(b'0');
        }
        if self.width != 0 {
            /* C holds the width in an `int` and negates a `*` that
             * arrived negative, which leaves `INT_MIN` negative, then
             * writes it back through an unsigned conversion -- so the
             * one magnitude an `int` cannot hold is spelt as its sign
             * extension. A written-out width that large never reaches
             * here, having been refused for running past the limit. */
            let spelt = if self.width > LIMIT {
                i64::from(i32::MIN) as u64
            } else {
                self.width as u64
            };
            text.extend_from_slice(format!("{spelt}").as_bytes());
        }
        if let Some(precision) = self.precision {
            text.push(b'.');
            text.extend_from_slice(format!("{precision}").as_bytes());
        }
        text.push(stop);
        text.extend_from_slice(rest);
        self.literal = Some(text);
    }

    /// Lay a number out in the field, or refuse a conversion the C could
    /// not have counted.
    ///
    /// Zero padding goes *after* the sign and any radix prefix, which is
    /// what distinguishes it from the space padding either side of a
    /// string; a string reaches here with no prefix and `zeros` false.
    fn field(&self, prefix: &[u8], body: &Number, zeros: bool) -> Option<Vec<u8>> {
        /* The three answers in the order glibc arrives at them. Digits
         * past `INT_MAX` are read before the `*` that would have stopped
         * it, so they outrank the echo; the echo in turn outranks any
         * length, because a specification glibc cannot read is one it
         * never renders and so never measures. */
        if self.over {
            return None;
        }
        if let Some(text) = &self.literal {
            return Some(text.clone());
        }
        let len = prefix.len() + body.len();
        if len > LIMIT {
            return None;
        }
        if self.width > LIMIT {
            /* A width whose magnitude an `int` cannot hold stayed
             * negative through C's negation, so it never padded
             * anything. An empty field prints nothing at all; one with
             * bytes in it is refused. */
            return (len == 0).then(Vec::new);
        }

        let fill = self.width.saturating_sub(len);
        let mut output = Vec::with_capacity(len.max(self.width));

        if !self.left && !zeros {
            output.resize(fill, b' ');
        }
        output.extend_from_slice(prefix);
        if !self.left && zeros {
            output.resize(output.len() + fill, b'0');
        }
        body.write_to(&mut output);
        if self.left {
            output.resize(self.width.max(len), b' ');
        }
        Some(output)
    }

    /// `%s`: the argument's bytes, truncated by the precision.
    ///
    /// Byte-oriented, so an argument that is not valid UTF-8 keeps its
    /// bytes and a precision counts them the way C counts them.
    pub(super) fn string(&self, bytes: &[u8]) -> Option<Vec<u8>> {
        let take = self
            .precision
            .map_or(bytes.len(), |precision| precision.min(bytes.len()));
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

    pub(super) fn signed(spelling: &str, value: i64) -> String {
        text(spec(spelling).signed(value))
    }

    fn unsigned(spelling: &str, value: u64, conversion: u8) -> String {
        text(spec(spelling).unsigned(value, conversion))
    }

    pub(super) fn double(spelling: &str, value: f64, conversion: u8) -> String {
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

    /// `#` on octal raises the precision just far enough to force one
    /// leading zero, and does nothing when a precision already did.
    #[test]
    fn octal_alternate_form_adds_one_zero() {
        assert_eq!(unsigned("#.3", 8, b'o'), "010");
        assert_eq!(unsigned("#.4", 8, b'o'), "0010");
        assert_eq!(unsigned("#", 8, b'o'), "010");
        assert_eq!(unsigned("", 0, b'o'), "0");
    }

    /// A specification glibc could not read prints as itself, whatever
    /// the conversion would have rendered -- but a width past the limit
    /// is still the error, because glibc reads those digits first.
    #[test]
    fn an_unreadable_spec_prints_itself() {
        let mut left = spec("-5");
        left.set_unreadable(b'*', b"ld");
        assert_eq!(text(left.signed(42)), "%-5*ld");
        assert_eq!(bytes(left.string(b"ab")), b"%-5*ld");

        /* C's own spelling of the flags, not the one that was typed:
         * one order, `+` over ` `, and `-` taking the padding. */
        let mut flags = spec("#0-5");
        flags.set_unreadable(b'*', b"lo");
        assert_eq!(text(flags.signed(1)), "%#-5*lo");
        let mut signs = spec("+ 5");
        signs.set_unreadable(b'*', b"ld");
        assert_eq!(text(signs.signed(1)), "%+5*ld");

        /* A width of zero is not written; a precision of zero is. */
        let mut none = spec("");
        none.set_unreadable(b'*', b"s");
        assert_eq!(text(none.string(b"ab")), "%*s");

        /* The width C could not negate is spelt as its sign extension. */
        let mut floor = Spec::bare();
        floor.set_width(i32::MIN);
        floor.set_unreadable(b'*', b"ld");
        assert_eq!(text(floor.signed(1)), "%-18446744071562067968*ld");
        let mut empty = spec(".0");
        empty.set_unreadable(b'*', b"ld");
        assert_eq!(text(empty.signed(1)), "%.0*ld");

        /* Digits past the limit are read before the `*` that stops the
         * read, so they are still the error. */
        let mut over = spec("2147483648");
        over.set_unreadable(b'*', b"ld");
        assert!(over.signed(42).is_none());

        /* A length only the rendering would have had is no length at
         * all, because the rendering never happens. */
        let mut long = spec(".2147483646");
        long.set_unreadable(b'*', b"f");
        assert_eq!(text(long.double(1.0, b'f')), "%.2147483646*f");
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
        /* `INT_MIN` is a magnitude one past the range, which C could not
         * negate and so never padded with: it refuses a field with
         * bytes in it and prints an empty one as nothing. */
        let mut star = Spec::bare();
        star.set_width(i32::MIN);
        assert!(star.signed(1).is_none());
        assert_eq!(bytes(star.string(b"")), b"");
        star.set_precision(0);
        assert_eq!(bytes(star.string(b"abc")), b"");
        assert_eq!(bytes(star.signed(0)), b"");
        /* A precision at the limit that no field grows by is not. */
        assert_eq!(bytes(spec(".2147483647").string(b"abc")), b"abc");
    }
}
