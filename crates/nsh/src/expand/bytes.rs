//! Byte access for the translated expansion buffer.

use bstr::BStr;

#[inline]
pub(super) fn at(bytes: &[u8], index: usize) -> u8 {
    bytes.get(index).copied().unwrap_or(0)
}

impl super::ExpandedField {
    /// The shell-visible field bytes.
    pub fn as_bstr(&self) -> &BStr {
        BStr::new(&self.text)
    }
}
