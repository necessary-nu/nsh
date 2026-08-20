//! Byte access for the translated expansion buffer.

use bstr::{BStr, ByteSlice as _};
use core::ffi::c_char;

#[inline]
pub(super) fn at(bytes: &[u8], index: usize) -> c_char {
    bytes.get(index).copied().unwrap_or(0) as c_char
}

#[inline]
pub(super) fn at_signed(bytes: &[u8], index: isize) -> c_char {
    usize::try_from(index)
        .ok()
        .map_or(0, |index| at(bytes, index))
}

#[inline]
pub(super) fn before_nul(bytes: &[u8]) -> &BStr {
    let end = bytes.find_byte(0).unwrap_or(bytes.len());
    BStr::new(&bytes[..end])
}

impl super::strlist {
    /// The shell-visible field bytes, excluding the stored terminator.
    pub fn as_bstr(&self) -> &BStr {
        before_nul(&self.text)
    }
}
