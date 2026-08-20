//! Byte access for the translated expansion buffer.

use bstr::BStr;
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

impl super::strlist {
    /// The shell-visible field bytes.
    pub fn as_bstr(&self) -> &BStr {
        BStr::new(&self.text)
    }
}
