//! Where the locale says a byte string's characters end.
//!
//! `mbrlen` has no locale-taking form, so every answer costs the thread
//! locale being selected and restored around one C call -- the short,
//! restoring scope `[dec:nsh:per-shell-locale]` permits, and the reason
//! an answer is worth asking for in bulk.
//!
//! Three shapes of caller ask, and they want different things, which is
//! why this module offers three entry points rather than one:
//!
//! * A walk that revisits offsets -- a backtracking matcher, or a scan
//!   that asks where a character ends and then asks again from the same
//!   place -- wants [`Characters`], which keeps what it was told.
//! * A walk of a whole string that visits each offset once wants
//!   [`boundaries`], which asks about the whole string under one
//!   selection and never asks again.
//! * A caller that wants one character out of a string it will not walk
//!   wants [`width`], which asks about exactly that character.
//!
//! Giving all three the memo would be worse than giving none of them it:
//! a table answers for every byte position, so a walk that steps by
//! whole characters pays for the interior bytes it never asks about, and
//! a table built for one question is a heap allocation spent to save one
//! locale selection.
//!
//! One table or one answer belongs to one string in one locale. Sharing
//! either beyond that would answer for the wrong charmap, which is what
//! `[spec:nsh:req:shell-locale.instance-isolation]` forbids.

/// How wide the character beginning at the start of `bytes` is, in bytes.
///
/// A position where no character begins -- an invalid sequence, one the
/// string ends too soon to complete, or the null character -- is one
/// byte wide, because one byte is what a caller has to step over to make
/// progress.
pub(crate) fn width(locale: &nsh_platform::Locale, bytes: &[u8]) -> usize {
    locale
        .multibyte_len(bytes)
        .filter(|width| *width > 0)
        .unwrap_or(1)
}

/// Every offset at which a character begins in `bytes`, then `bytes.len()`.
///
/// One locale selection for the whole string. A caller that already
/// knows it will visit every character has no use for a memo -- it would
/// ask each question once either way -- and every use for asking the
/// whole question at once.
pub(crate) fn boundaries(locale: &nsh_platform::Locale, bytes: &[u8]) -> Vec<usize> {
    let widths = locale.character_widths(bytes, bytes.len());
    let mut boundaries = vec![0];
    let mut at = 0;
    /* `get` rather than a length test: it ends the walk at the last byte
     * position and takes the width in the same step. The width cannot be
     * zero, and saying `max(1)` anyway is what makes that a property of
     * this loop rather than of the platform crate. */
    while let Some(width) = widths.get(at) {
        at += usize::from(*width).max(1);
        boundaries.push(at);
    }
    boundaries
}

/// One byte string, and what the locale has said about its characters.
///
/// For the caller that asks about the same offset more than once. A
/// backtracking matcher asks about every offset it can enter a
/// repetition at, once per way of entering it; a field split asks where
/// the character at an offset ends, and asks again when that character
/// turns out not to be a separator.
pub(crate) struct Characters<'a> {
    /// Readable, because a caller that has the table has no use for a
    /// second way to name the string and the locale it is about -- and
    /// because both are borrows the table only ever reads. Only
    /// `widths` is private, which is where the invariant is.
    pub(crate) locale: &'a nsh_platform::Locale,
    pub(crate) bytes: &'a [u8],
    /// One entry per byte position, learned in blocks: how wide the
    /// character beginning there is, or one where none begins.
    widths: Vec<u8>,
}

impl<'a> Characters<'a> {
    pub(crate) fn of(locale: &'a nsh_platform::Locale, bytes: &'a [u8]) -> Self {
        Self {
            locale,
            bytes,
            widths: Vec::new(),
        }
    }

    /// Where the character beginning at `at` runs out.
    ///
    /// A position at or past the string's end is its own end, so a
    /// caller stepping with this always terminates.
    pub(crate) fn end(&mut self, at: usize) -> usize {
        if at >= self.bytes.len() {
            return at;
        }
        self.learn(at);
        at + usize::from(self.widths[at]).max(1)
    }

    /// Where the character beginning at `at` runs out, seen from a
    /// caller that may read only `bytes[..limit]`.
    ///
    /// An extended glob's alternative is a slice of the pattern its
    /// group was read from, and indexes the whole pattern's table once
    /// shifted, so a character straddling the slice's end has to answer
    /// "one byte" against the slice while the table goes on holding the
    /// whole pattern's answer for that position. One byte is what the
    /// locale itself answers when it is shown the truncated string, so
    /// this is the same answer reached without asking twice.
    ///
    /// Separate from [`Self::end`] rather than the general case it calls,
    /// because every caller that reads a whole string would then pay a
    /// comparison per character to be told a bound it already knows:
    /// measured, folding the two cost the ERE engine 0.8% of its
    /// instructions on a restarting search.
    pub(crate) fn end_within(&mut self, at: usize, limit: usize) -> usize {
        if at >= limit {
            return at;
        }
        self.learn(at);
        let width = usize::from(self.widths[at]).max(1);
        if at + width <= limit {
            at + width
        } else {
            at + 1
        }
    }

    /// Make sure the locale has been asked about `at`.
    ///
    /// The block asked for doubles, so a walk of a whole string asks a
    /// logarithmic number of times while a walk that stops at its first
    /// character does not pay for the rest of the string. That second
    /// half is what the doubling is for: a search restarted at every
    /// offset of a long subject abandons most attempts at once.
    fn learn(&mut self, at: usize) {
        if at < self.widths.len() {
            return;
        }
        let known = self.widths.len();
        let want = (known * 2)
            .max(CHARACTER_BLOCK)
            .max(at + 1)
            .min(self.bytes.len());
        self.widths.extend(
            self.locale
                .character_widths(&self.bytes[known..], want - known),
        );
    }
}

/// How many byte positions the first block covers.
const CHARACTER_BLOCK: usize = 8;

#[cfg(test)]
mod tests {
    use super::*;

    /// The three entry points answer the same thing, and the memo does
    /// not change what it answers by having been filled in blocks.
    ///
    /// A caller that asks about position nine first must be told what
    /// one that walked there would have been told, or the memo has
    /// become a behaviour change; nine is past the first block, so it is
    /// the position where a wrong block policy would show.
    // [spec:nsh:req:shell-locale.operation-binding/test]
    #[test]
    fn every_way_of_asking_agrees_where_characters_end() {
        let locale = nsh_platform::Locale::new(b"C.UTF-8", &[]).expect("C.UTF-8 exists");
        /* Five two-byte characters, two ASCII, then a two-byte character
         * the string ends too soon to complete. */
        let bytes = b"\xc3\x8c\xc3\x8c\xc3\x8c\xc3\x8c\xc3\x8cab\xc3";
        let expected = [2, 4, 6, 8, 10, 11, 12, 13];

        let mut table = Characters::of(&locale, bytes);
        let mut walked = Vec::new();
        let mut at = 0;
        while at < bytes.len() {
            at = table.end(at);
            walked.push(at);
        }
        assert_eq!(walked, expected);
        assert_eq!(
            boundaries(&locale, bytes),
            [0].into_iter().chain(expected).collect::<Vec<_>>()
        );
        assert_eq!(width(&locale, bytes), 2);
        assert_eq!(width(&locale, &bytes[12..]), 1);

        let mut jumped = Characters::of(&locale, bytes);
        assert_eq!(jumped.end(9), 10);
        assert_eq!(jumped.end(0), 2);
        assert_eq!(jumped.end(bytes.len()), bytes.len());
        assert_eq!(jumped.end(bytes.len() + 4), bytes.len() + 4);

        /* A caller reading a slice is answered about the slice, while the
         * table goes on holding the whole string's answer: the character
         * at 8 is two bytes wide, and a caller that may read only nine of
         * them is told to step one -- which is what the locale itself
         * says when it is shown `bytes[..9]`. Asking again without a
         * limit still gets ten. */
        let mut sliced = Characters::of(&locale, bytes);
        assert_eq!(sliced.end_within(8, 9), 9);
        assert_eq!(sliced.end(8), 10);
        assert_eq!(sliced.end_within(8, bytes.len()), 10);
        assert_eq!(sliced.end_within(9, 9), 9);
        assert_eq!(width(&locale, &bytes[8..9]), 1);

        /* A single-byte charmap holds a character at every byte, and the
         * empty string holds none. */
        let c = nsh_platform::Locale::c().expect("the C locale exists");
        assert_eq!(boundaries(&c, bytes).len(), bytes.len() + 1);
        assert_eq!(boundaries(&c, b""), [0]);
        assert_eq!(width(&c, b""), 1);
    }
}
