//! Shell-language integer parsing.
//!
//! Shell operands are byte strings and have parsing rules that differ from
//! Rust's `str::parse`: base-zero input accepts shell arithmetic prefixes,
//! overflow saturates at the signed range, and trailing C-locale whitespace
//! is accepted. Keeping those rules here gives callers typed results and
//! errors without a generic string-compatibility module.

use bstr::{BStr, ByteSlice as _};

use crate::error::{Diagnostics, Error};

fn before_nul(bytes: &BStr) -> &BStr {
    let end = bytes.find_byte(0).unwrap_or(bytes.len());
    BStr::new(&bytes[..end])
}

// [spec:dash:def:mystring.badnum-fn]
// [spec:dash:sem:mystring.badnum-fn]
/// Build the shell's diagnostic for an invalid numeric operand.
pub(crate) fn invalid_number(diagnostics: &mut Diagnostics<'_>, input: &BStr) -> Error {
    let mut message = b"Illegal number: ".to_vec();
    message.extend_from_slice(before_nul(input));
    diagnostics.sh_error_value(&message)
}

// [spec:dash:def:mystring.atomax-fn]
// [spec:dash:sem:mystring.atomax-fn]
/// Parse one signed shell integer in `base`, where zero selects a prefix.
// [spec:nsh:req:idiom.no-mystring]
pub(crate) fn parse_integer(
    diagnostics: &mut Diagnostics<'_>,
    input: &BStr,
    requested_base: u32,
) -> Result<i64, Error> {
    debug_assert!(requested_base == 0 || (2..=36).contains(&requested_base));

    let bytes = before_nul(input).as_bytes();
    let mut position = bytes
        .iter()
        .position(|&byte| !is_shell_space(byte))
        .unwrap_or(bytes.len());
    let number_start = position;
    let negative = match bytes.get(position) {
        Some(b'+') => {
            position += 1;
            false
        }
        Some(b'-') => {
            position += 1;
            true
        }
        _ => false,
    };

    let mut base = requested_base;
    if base == 0 {
        base = if bytes.get(position) == Some(&b'0') {
            match (
                bytes.get(position + 1),
                bytes.get(position + 2).and_then(|byte| digit_value(*byte)),
            ) {
                (Some(b'x' | b'X'), Some(digit)) if digit < 16 => {
                    position += 2;
                    16
                }
                (Some(b'b' | b'B'), Some(digit)) if digit < 2 => {
                    position += 2;
                    2
                }
                _ => 8,
            }
        } else {
            10
        };
    } else if ((base == 16 && matches!(bytes.get(position + 1), Some(b'x' | b'X')))
        || (base == 2 && matches!(bytes.get(position + 1), Some(b'b' | b'B'))))
        && bytes.get(position) == Some(&b'0')
        && bytes
            .get(position + 2)
            .and_then(|byte| digit_value(*byte))
            .is_some_and(|digit| digit < base)
    {
        position += 2;
    }

    let digits_start = position;
    let limit = if negative {
        i64::MAX as u64 + 1
    } else {
        i64::MAX as u64
    };
    let mut magnitude = 0_u64;
    while let Some(digit) = bytes.get(position).and_then(|byte| digit_value(*byte)) {
        if digit >= base {
            break;
        }
        magnitude = magnitude
            .saturating_mul(base as u64)
            .saturating_add(digit as u64)
            .min(limit);
        position += 1;
    }

    if position == digits_start {
        if requested_base == 0
            && number_start == bytes.len()
            && bytes[..number_start]
                .iter()
                .all(|&byte| is_shell_space(byte))
        {
            return Ok(0);
        }
        return Err(invalid_number(diagnostics, input));
    }

    while bytes
        .get(position)
        .is_some_and(|&byte| is_shell_space(byte))
    {
        position += 1;
    }
    if position != bytes.len() {
        return Err(invalid_number(diagnostics, input));
    }

    Ok(if negative {
        if magnitude == i64::MAX as u64 + 1 {
            i64::MIN
        } else {
            -(magnitude as i64)
        }
    } else {
        magnitude as i64
    })
}

// [spec:dash:def:mystring.number-fn]
// [spec:dash:sem:mystring.number-fn]
// [spec:dash:def:mystring.atomax10-fn]
// [spec:dash:sem:mystring.atomax10-fn]
/// Parse a non-negative decimal shell operand in the `i32` range.
pub(crate) fn parse_nonnegative(
    diagnostics: &mut Diagnostics<'_>,
    input: &BStr,
) -> Result<i32, Error> {
    let number = parse_integer(diagnostics, input, 10)?;
    i32::try_from(number)
        .ok()
        .filter(|number| *number >= 0)
        .ok_or_else(|| invalid_number(diagnostics, input))
}

// [spec:dash:def:mystring.is-number-fn]
// [spec:dash:sem:mystring.is-number-fn]
/// Parse a non-empty string of ASCII decimal digits with saturation.
pub(crate) fn parse_decimal(input: &BStr) -> Option<u64> {
    (!input.is_empty() && input.iter().all(u8::is_ascii_digit)).then(|| {
        input.iter().fold(0_u64, |value, byte| {
            value
                .saturating_mul(10)
                .saturating_add((byte - b'0') as u64)
        })
    })
}

fn digit_value(byte: u8) -> Option<u32> {
    match byte {
        b'0'..=b'9' => Some((byte - b'0') as u32),
        b'a'..=b'z' => Some((byte - b'a' + 10) as u32),
        b'A'..=b'Z' => Some((byte - b'A' + 10) as u32),
        _ => None,
    }
}

fn is_shell_space(byte: u8) -> bool {
    matches!(byte, b' ' | b'\t' | b'\n' | b'\r' | 0x0b | 0x0c)
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:dash:sem:mystring.is-number-fn/test]
    #[test]
    fn decimal_requires_ascii_digits() {
        assert_eq!(parse_decimal(BStr::new(b"")), None);
        assert_eq!(parse_decimal(BStr::new(b"12345")), Some(12345));
        for byte in 1_u8..=u8::MAX {
            assert_eq!(
                parse_decimal(BStr::new(&[byte])).is_some(),
                byte.is_ascii_digit(),
                "classification differed for byte 0x{byte:02x}"
            );
        }
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    #[test]
    fn integer_parsing_accepts_trailing_space() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let diagnostics = &mut shell.diagnostics();
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"-42"), 10).unwrap(),
            -42
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"ff"), 16).unwrap(),
            255
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"777"), 8).unwrap(),
            511
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"42\t\n"), 10).unwrap(),
            42
        );
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    // [spec:dash:sem:mystring.badnum-fn/test]
    #[test]
    fn invalid_integer_returns_diagnostic() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let diagnostics = &mut shell.diagnostics();
        let error = parse_integer(diagnostics, BStr::new(b"42x"), 10).unwrap_err();
        assert_eq!(error.message(), BStr::new(b"Illegal number: 42x"));
        assert!(parse_integer(diagnostics, BStr::new(b""), 10).is_err());
        assert!(parse_integer(diagnostics, BStr::new(b"   "), 10).is_err());
        assert_eq!(parse_integer(diagnostics, BStr::new(b"   "), 0).unwrap(), 0);
    }

    // [spec:dash:sem:mystring.atomax10-fn/test]
    #[test]
    fn decimal_parser_uses_base_ten() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let diagnostics = &mut shell.diagnostics();
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"010"), 10).unwrap(),
            10
        );
        assert!(parse_integer(diagnostics, BStr::new(b"0x10"), 10).is_err());
    }

    // [spec:dash:sem:mystring.number-fn/test]
    #[test]
    fn nonnegative_parser_checks_i32_range() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let diagnostics = &mut shell.diagnostics();
        assert_eq!(parse_nonnegative(diagnostics, BStr::new(b"7")).unwrap(), 7);
        assert_eq!(
            parse_nonnegative(diagnostics, BStr::new(i32::MAX.to_string().as_bytes())).unwrap(),
            i32::MAX
        );
        assert!(parse_nonnegative(diagnostics, BStr::new(b"-1")).is_err());
        let too_big = (i32::MAX as i64 + 1).to_string();
        assert!(parse_nonnegative(diagnostics, BStr::new(too_big.as_bytes())).is_err());
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    #[test]
    fn integer_parser_accepts_shell_prefixes() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let diagnostics = &mut shell.diagnostics();

        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"0x2a"), 0).unwrap(),
            42
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"0X2A"), 0).unwrap(),
            42
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"0b101010"), 0).unwrap(),
            42
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"052"), 0).unwrap(),
            42
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"+42"), 0).unwrap(),
            42
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b" \t-052\n"), 0).unwrap(),
            -42
        );
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    #[test]
    fn integer_parser_saturates_signed_range() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let diagnostics = &mut shell.diagnostics();

        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"999999999999999999999999"), 10).unwrap(),
            i64::MAX
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"-999999999999999999999999"), 10,).unwrap(),
            i64::MIN
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(i64::MAX.to_string().as_bytes()), 10).unwrap(),
            i64::MAX
        );
        assert_eq!(
            parse_integer(diagnostics, BStr::new(i64::MIN.to_string().as_bytes()), 10).unwrap(),
            i64::MIN
        );
    }

    // [spec:dash:sem:mystring.atomax-fn/test]
    // [spec:dash:sem:mystring.badnum-fn/test]
    #[test]
    fn nul_ends_numeric_operand() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let diagnostics = &mut shell.diagnostics();

        assert_eq!(
            parse_integer(diagnostics, BStr::new(b"42\0junk"), 10).unwrap(),
            42
        );
        let error = invalid_number(diagnostics, BStr::new(b"junk\0hidden"));
        assert_eq!(error.message(), BStr::new(b"Illegal number: junk"));
    }

    // [spec:dash:sem:mystring.is-number-fn/test]
    #[test]
    fn decimal_parser_saturates_overflow() {
        assert_eq!(
            parse_decimal(BStr::new(b"999999999999999999999999")),
            Some(u64::MAX)
        );
        assert_eq!(parse_decimal(BStr::new(b"0123456789")), Some(123456789));
        assert_eq!(parse_decimal(BStr::new(b"12x")), None);
    }
}
