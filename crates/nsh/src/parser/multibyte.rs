//! Typed parser modes for locale-dependent input characters.

/// How a multibyte input character is represented in parser output.
///
/// These are the five combinations the parser actually uses; impossible
/// integer values can no longer alter framing through `mode & 3`.
// [spec:nsh:req:idiom.operation-modes]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MultibyteMode {
    Framed,
    Escaped,
    Raw,
    FieldBoundary,
    RawField,
}

impl MultibyteMode {
    pub(crate) const fn for_word(field_splitting: bool, raw: bool) -> Self {
        match (field_splitting, raw) {
            (false, false) => Self::Framed,
            (false, true) => Self::Raw,
            (true, false) => Self::FieldBoundary,
            (true, true) => Self::RawField,
        }
    }
}
