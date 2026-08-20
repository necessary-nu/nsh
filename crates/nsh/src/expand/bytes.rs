//! Byte access for the translated expansion buffer.

use bstr::BStr;

#[inline]
pub(super) fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

#[inline]
pub(super) fn at_signed(bytes: &[u8], index: isize) -> u8 {
    usize::try_from(index)
        .ok()
        .map_or(0, |index| at(bytes, index))
}

impl super::ExpandedField {
    /// The shell-visible field bytes.
    pub fn as_bstr(&self) -> &BStr {
        BStr::new(&self.text)
    }
}
