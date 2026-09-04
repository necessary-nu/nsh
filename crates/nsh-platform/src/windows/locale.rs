//! Locale objects for a host whose shell text is always UTF-8.
//!
//! Windows offers no POSIX locale for the shell to select, and nsh fixes
//! its text encoding as UTF-8 whatever `LC_ALL` names -- so a locale
//! here is a *name* that stays observable to the script, and every
//! question asked of it is answered from Rust's own Unicode tables and
//! from `io::Error` instead of from a C library. The module exists all
//! the same because the shell asks the same list on both hosts:
//! character classes, collation, multibyte length, and the text behind
//! an error number and a signal number.

use std::cmp::Ordering;

use crate::Signal;

pub enum LocaleDecode {
    Incomplete,
    Complete(i32),
    Invalid,
}

/// One character read from the front of a byte string, and what it cost
/// in bytes. `Incomplete` says the bytes are a valid beginning the string
/// ends too soon to finish, which is why this is not `Option`.
pub enum LocaleCharacter {
    Complete { wide: i32, width: usize },
    Incomplete,
    Invalid,
}

pub struct LocaleDecoder {
    bytes: Vec<u8>,
}

impl LocaleDecoder {
    pub fn push(&mut self, byte: u8) -> LocaleDecode {
        self.bytes.push(byte);
        let expected = match self.bytes[0] {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => {
                self.bytes.clear();
                return LocaleDecode::Invalid;
            }
        };
        if self.bytes.len() < expected {
            return LocaleDecode::Incomplete;
        }
        let result = std::str::from_utf8(&self.bytes)
            .ok()
            .and_then(|value| value.chars().next())
            .map(|value| LocaleDecode::Complete(value as i32))
            .unwrap_or(LocaleDecode::Invalid);
        self.bytes.clear();
        result
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocaleCategory {
    Collate,
    Ctype,
    Messages,
    Monetary,
    Numeric,
    Time,
}

/// Windows builds use Unicode classification and UTF-8 shell text. The name
/// is retained so locale environment variables remain observable, while no
/// process-global C locale is changed.
#[derive(Clone)]
pub struct Locale {
    _name: Vec<u8>,
}

impl Locale {
    pub fn new(base: &[u8], overrides: &[(LocaleCategory, &[u8])]) -> std::io::Result<Self> {
        fn supported(name: &[u8]) -> bool {
            let Ok(name) = std::str::from_utf8(name) else {
                return false;
            };
            name.eq_ignore_ascii_case("C")
                || name.eq_ignore_ascii_case("POSIX")
                || name.eq_ignore_ascii_case("UTF-8")
                || name.eq_ignore_ascii_case("UTF8")
                || name.to_ascii_uppercase().ends_with(".UTF-8")
                || name.to_ascii_uppercase().ends_with(".UTF8")
        }

        if !supported(base) || overrides.iter().any(|(_, name)| !supported(name)) {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "Windows nsh supports C, POSIX, and UTF-8 locales",
            ));
        }
        Ok(Self {
            _name: overrides.last().map_or(base, |(_, name)| *name).to_vec(),
        })
    }

    pub fn c() -> std::io::Result<Self> {
        Self::new(b"C", &[])
    }

    pub fn decoder(&self) -> LocaleDecoder {
        LocaleDecoder { bytes: Vec::new() }
    }

    pub fn is_alpha(&self, byte: u8) -> bool {
        byte.is_ascii_alphabetic()
    }

    pub fn is_alphanumeric(&self, byte: u8) -> bool {
        byte.is_ascii_alphanumeric()
    }

    pub fn is_space(&self, byte: u8) -> bool {
        matches!(byte, b' ' | b'\t' | b'\n' | 0x0b | 0x0c | b'\r')
    }

    pub fn wide_is_blank(&self, wide: i32) -> bool {
        crate::work::record_character_queries(1);
        char::from_u32(wide as u32).is_some_and(|value| matches!(value, ' ' | '\t'))
    }

    pub fn wide_is_space(&self, wide: i32) -> bool {
        crate::work::record_character_queries(1);
        char::from_u32(wide as u32).is_some_and(char::is_whitespace)
    }

    pub fn decode_prefix(&self, bytes: &[u8]) -> LocaleCharacter {
        let Some(first) = bytes.first() else {
            return LocaleCharacter::Incomplete;
        };
        let width = match first {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return LocaleCharacter::Invalid,
        };
        if bytes.len() < width {
            return LocaleCharacter::Incomplete;
        }
        match std::str::from_utf8(&bytes[..width])
            .ok()
            .and_then(|text| text.chars().next())
        {
            Some(wide) => LocaleCharacter::Complete {
                wide: wide as i32,
                width,
            },
            None => LocaleCharacter::Invalid,
        }
    }

    pub fn multibyte_len(&self, bytes: &[u8]) -> Option<usize> {
        crate::work::record_character_queries(1);
        let first = *bytes.first()?;
        let length = match first {
            0x00..=0x7f => 1,
            0xc2..=0xdf => 2,
            0xe0..=0xef => 3,
            0xf0..=0xf4 => 4,
            _ => return None,
        };
        (bytes.len() >= length && std::str::from_utf8(&bytes[..length]).is_ok()).then_some(length)
    }

    /// See the POSIX host's `character_widths` for the contract: `bytes`
    /// begins a character, positions the walk steps over are one byte,
    /// and the run stops only where a character does.
    // [spec:nsh:req:cost.only-the-work-the-command-needs]
    pub fn character_widths(&self, bytes: &[u8], offsets: usize) -> Vec<u8> {
        let offsets = offsets.min(bytes.len());
        let mut widths: Vec<u8> = Vec::with_capacity(offsets);
        while widths.len() < offsets {
            let at = widths.len();
            let width = self
                .multibyte_len(&bytes[at..])
                .and_then(|width| u8::try_from(width).ok())
                .filter(|width| *width > 0)
                .unwrap_or(1);
            widths.push(width);
            /* A length is never more than the bytes it was read from,
             * so the interior never runs past the string. */
            widths.resize(at + usize::from(width), 1);
        }
        widths
    }

    pub fn decode_exact(&self, bytes: &[u8], expected_len: usize) -> Option<i32> {
        crate::work::record_character_queries(1);
        let value = std::str::from_utf8(bytes.get(..expected_len)?).ok()?;
        let mut chars = value.chars();
        let first = chars.next()?;
        (first.len_utf8() == expected_len).then_some(first as i32)
    }

    pub fn wide_class_matches(
        &self,
        name: &[u8],
        bytes: &[u8],
        expected_len: usize,
    ) -> Option<bool> {
        let wide = self.decode_exact(bytes, expected_len)?;
        let value = char::from_u32(wide as u32)?;
        Some(match name {
            b"alnum" => value.is_alphanumeric(),
            b"alpha" => value.is_alphabetic(),
            b"blank" => matches!(value, ' ' | '\t'),
            b"cntrl" => value.is_control(),
            b"digit" => value.is_ascii_digit(),
            b"graph" => !value.is_control() && !value.is_whitespace(),
            b"lower" => value.is_lowercase(),
            b"print" => !value.is_control(),
            b"punct" => value.is_ascii_punctuation(),
            b"space" => value.is_whitespace(),
            b"upper" => value.is_uppercase(),
            b"xdigit" => value.is_ascii_hexdigit(),
            _ => return None,
        })
    }

    pub fn wide_chars(&self, bytes: &[u8]) -> (usize, Vec<i32>) {
        if bytes.is_empty() {
            return (0, Vec::new());
        }
        let first_len = self.multibyte_len(bytes).unwrap_or(1);
        let mut decoded = vec![0_i32; bytes.len() + 1];
        if let Ok(text) = std::str::from_utf8(bytes) {
            for (slot, value) in decoded.iter_mut().zip(text.chars()) {
                *slot = value as i32;
            }
        }
        (first_len, decoded)
    }

    pub fn collate(&self, left: &[u8], right: &[u8]) -> Ordering {
        left.cmp(right)
    }

    pub fn collating_bracket_matches(&self, pattern: &[u8], subject: &[u8]) -> bool {
        if pattern.len() >= 7 && pattern.starts_with(b"[[.") && pattern.ends_with(b".]]") {
            let member = &pattern[3..pattern.len() - 3];
            return member.len() == 1 && member == subject;
        }
        if pattern.len() >= 7 && pattern.starts_with(b"[[=") && pattern.ends_with(b"=]]") {
            let member = &pattern[3..pattern.len() - 3];
            return member.len() == 1 && member == subject;
        }
        false
    }

    pub fn error_message(&self, error: &std::io::Error) -> String {
        let Some(code) = error.raw_os_error() else {
            return error.to_string();
        };
        let rendered = error.to_string();
        let suffix = format!(" (os error {code})");
        rendered
            .strip_suffix(&suffix)
            .unwrap_or(&rendered)
            .to_owned()
    }

    pub fn range_error_message(&self) -> String {
        "Result too large".to_owned()
    }

    pub fn signal_description(&self, signal: Signal) -> Vec<u8> {
        let description = match signal.number() {
            1 => "Hangup",
            2 => "Interrupt",
            3 => "Quit",
            9 => "Killed",
            13 => "Broken pipe",
            15 => "Terminated",
            17 => "Child status changed",
            18 => "Continued",
            20 => "Terminal stop",
            _ => return signal.number().to_string().into_bytes(),
        };
        description.as_bytes().to_vec()
    }
}
