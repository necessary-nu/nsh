//! Converting between the shell's bytes and Windows' native strings, and
//! where this host puts the end of a line.
//!
//! Neither direction is free here. A Windows native string is
//! potentially ill-formed UTF-16 and the shell's is bytes, so the
//! interchange is WTF-8 -- written out longhand rather than borrowed
//! from Rust's `OsStr`, whose internal encoding is unspecified and would
//! silently redefine the format if it ever changed. The decoder rejects
//! bytes that are not WTF-8 instead of letting a shell-authored name
//! quietly become a different one.
//!
//! The line ending is the same subject seen from the reader's side: a
//! CRLF is one newline to whoever is reading lines and two bytes to
//! whoever counted them, and command substitution has to strip whichever
//! of the two it was given.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::{Path, PathBuf};

/// Shell-specific operations on native strings without exposing Windows'
/// UTF-16 representation to the shell crate.
pub trait NativeStrExt {
    fn to_shell_bytes(&self) -> Vec<u8>;
}

impl NativeStrExt for OsStr {
    fn to_shell_bytes(&self) -> Vec<u8> {
        encode_wtf8(self)
    }
}

impl NativeStrExt for Path {
    fn to_shell_bytes(&self) -> Vec<u8> {
        self.as_os_str().to_shell_bytes()
    }
}

pub trait ShellBytesExt {
    fn try_to_os_string(&self) -> std::io::Result<OsString>;
    fn try_to_path_buf(&self) -> std::io::Result<PathBuf>;
}

impl ShellBytesExt for [u8] {
    fn try_to_os_string(&self) -> std::io::Result<OsString> {
        decode_wtf8(self).map(|wide| OsString::from_wide(&wide))
    }

    fn try_to_path_buf(&self) -> std::io::Result<PathBuf> {
        self.try_to_os_string().map(PathBuf::from)
    }
}

/// Encode Windows' potentially ill-formed UTF-16 as the shell's stable WTF-8
/// interchange representation. This deliberately does not depend on Rust's
/// unspecified internal `OsStr` encoding.
fn encode_wtf8(value: &OsStr) -> Vec<u8> {
    let mut bytes = Vec::new();
    let mut wide = value.encode_wide().peekable();
    while let Some(unit) = wide.next() {
        let scalar = if (0xd800..=0xdbff).contains(&unit)
            && wide
                .peek()
                .is_some_and(|next| (0xdc00..=0xdfff).contains(next))
        {
            let low = wide.next().expect("peeked low surrogate exists");
            0x10000 + ((u32::from(unit) - 0xd800) << 10) + (u32::from(low) - 0xdc00)
        } else {
            u32::from(unit)
        };
        match scalar {
            0x0000..=0x007f => bytes.push(scalar as u8),
            0x0080..=0x07ff => {
                bytes.push(0xc0 | (scalar >> 6) as u8);
                bytes.push(0x80 | (scalar & 0x3f) as u8);
            }
            0x0800..=0xffff => {
                bytes.push(0xe0 | (scalar >> 12) as u8);
                bytes.push(0x80 | ((scalar >> 6) & 0x3f) as u8);
                bytes.push(0x80 | (scalar & 0x3f) as u8);
            }
            _ => {
                bytes.push(0xf0 | (scalar >> 18) as u8);
                bytes.push(0x80 | ((scalar >> 12) & 0x3f) as u8);
                bytes.push(0x80 | ((scalar >> 6) & 0x3f) as u8);
                bytes.push(0x80 | (scalar & 0x3f) as u8);
            }
        }
    }
    bytes
}

/// Decode the shell's WTF-8 interchange representation. Shell-authored byte
/// sequences which are not valid WTF-8 cannot name a Windows native string
/// and are rejected instead of being silently redirected to a different name.
fn decode_wtf8(bytes: &[u8]) -> std::io::Result<Vec<u16>> {
    let mut wide = Vec::with_capacity(bytes.len());
    let mut offset = 0;
    while offset < bytes.len() {
        let first = bytes[offset];
        let (length, mut scalar, minimum) = match first {
            0x00..=0x7f => (1, u32::from(first), 0),
            0xc2..=0xdf => (2, u32::from(first & 0x1f), 0x80),
            0xe0..=0xef => (3, u32::from(first & 0x0f), 0x800),
            0xf0..=0xf4 => (4, u32::from(first & 0x07), 0x10000),
            _ => return Err(invalid_shell_string()),
        };
        if offset + length > bytes.len()
            || bytes[offset + 1..offset + length]
                .iter()
                .any(|byte| byte & 0xc0 != 0x80)
        {
            return Err(invalid_shell_string());
        }
        for byte in &bytes[offset + 1..offset + length] {
            scalar = (scalar << 6) | u32::from(byte & 0x3f);
        }
        if scalar < minimum || scalar > 0x10ffff {
            return Err(invalid_shell_string());
        }
        if scalar <= 0xffff {
            wide.push(scalar as u16);
        } else {
            let scalar = scalar - 0x10000;
            wide.push(0xd800 | ((scalar >> 10) as u16));
            wide.push(0xdc00 | ((scalar & 0x3ff) as u16));
        }
        offset += length;
    }
    Ok(wide)
}

fn invalid_shell_string() -> std::io::Error {
    std::io::Error::new(
        std::io::ErrorKind::InvalidData,
        "shell bytes are not valid WTF-8",
    )
}

pub const fn input_newline_width(previous: Option<u8>) -> usize {
    match previous {
        Some(b'\r') => 2,
        _ => 1,
    }
}

pub fn trim_command_substitution_output(output: &mut Vec<u8>, start: usize) {
    while output.len() > start && output.last() == Some(&b'\n') {
        output.pop();
        if output.len() > start && output.last() == Some(&b'\r') {
            output.pop();
        }
    }
}
