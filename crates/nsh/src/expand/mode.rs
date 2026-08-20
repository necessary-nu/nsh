//! Typed requests for expansion and quote removal.

/// Orthogonal requests made of one word expansion.
///
/// The set is deliberately private to the shell crate and can only contain
/// these named facts. Combining facts is valid; callers use small named
/// combinations for ordinary words, assignment words, redirection operands,
/// case patterns, and quoted here-documents.
// [spec:nsh:req:idiom.operation-modes]
#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ExpansionMode(u16);

impl ExpansionMode {
    pub(crate) const SPLIT: Self = Self(1 << 0);
    pub(crate) const TILDE: Self = Self(1 << 1);
    pub(crate) const ASSIGNMENT_TILDE: Self = Self(1 << 2);
    pub(crate) const REDIRECTION: Self = Self(1 << 3);
    pub(crate) const PRESERVE_MULTIBYTE: Self = Self(1 << 5);
    pub(crate) const COLON_TILDE: Self = Self(1 << 6);
    pub(crate) const QUOTED: Self = Self(1 << 8);

    pub(crate) const fn contains(self, other: Self) -> bool {
        self.0 & other.0 == other.0
    }

    pub(super) const fn intersects(self, other: Self) -> bool {
        self.0 & other.0 != 0
    }

    const fn union(self, other: Self) -> Self {
        Self(self.0 | other.0)
    }
}

impl core::ops::BitOr for ExpansionMode {
    type Output = Self;

    fn bitor(self, rhs: Self) -> Self::Output {
        self.union(rhs)
    }
}
