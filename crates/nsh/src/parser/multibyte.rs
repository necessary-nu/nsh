//! Typed parser modes for locale-dependent input characters.

/// How a complete multibyte input character is classified by its caller.
// [spec:nsh:req:idiom.operation-modes]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MultibyteMode {
    Literal,
    Escaped,
    FieldBoundary,
}

impl MultibyteMode {
    pub(crate) const fn for_word(field_splitting: bool, preserve_escapes: bool) -> Self {
        if field_splitting && !preserve_escapes {
            Self::FieldBoundary
        } else {
            Self::Literal
        }
    }
}
