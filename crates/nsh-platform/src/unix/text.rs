//! Converting between the shell's bytes and the host's native strings,
//! and where this host puts the end of a line.
//!
//! On a POSIX host both answers are nearly free -- a native string is
//! already a byte string, so neither conversion can fail, and a line ends
//! in one byte. The subject earns a file of its own because the Windows
//! side of it spends a hundred lines on the same two questions: native
//! strings there are potentially ill-formed UTF-16, and a line ends in
//! two bytes. Keeping the pair together means the whole difference
//! between the hosts is one file rather than a scatter.

use std::ffi::{OsStr, OsString};
use std::os::unix::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};

/// Shell-specific operations on native strings without exposing the host
/// representation to the shell crate.
pub trait NativeStrExt {
    fn to_shell_bytes(&self) -> Vec<u8>;
}

impl NativeStrExt for OsStr {
    fn to_shell_bytes(&self) -> Vec<u8> {
        self.as_bytes().to_vec()
    }
}

impl NativeStrExt for Path {
    fn to_shell_bytes(&self) -> Vec<u8> {
        self.as_os_str().to_shell_bytes()
    }
}

/// Native-string conversions for byte-oriented shell values.
pub trait ShellBytesExt {
    fn try_to_os_string(&self) -> std::io::Result<OsString>;
    fn try_to_path_buf(&self) -> std::io::Result<PathBuf>;
}

impl ShellBytesExt for [u8] {
    fn try_to_os_string(&self) -> std::io::Result<OsString> {
        Ok(OsString::from_vec(self.to_vec()))
    }

    fn try_to_path_buf(&self) -> std::io::Result<PathBuf> {
        self.try_to_os_string().map(PathBuf::from)
    }
}

pub const fn input_newline_width(_previous: Option<u8>) -> usize {
    1
}

pub fn trim_command_substitution_output(output: &mut Vec<u8>, start: usize) {
    while output.len() > start && output.last() == Some(&b'\n') {
        output.pop();
    }
}
