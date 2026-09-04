//! What bytes spell, and what the character they spell is.
//!
//! Every conversion between a locale's charmap and a wide character
//! lives here: the incremental decoder a reader feeds one byte at a
//! time, the whole-character forms that answer under one thread-locale
//! selection, the two wide-character classifications, and the encoding
//! direction a `\u` escape needs. They are together because they share
//! the `unsafe extern "C"` block below -- the C library's restartable
//! conversion functions take no locale argument, so every one of them is
//! reached through [`Locale::with_selected`] rather than an `_l` variant.
//!
//! Single-byte classification and collation stay in the parent: they ask
//! `libc::isalpha` and `libc::strcoll`, name none of these declarations,
//! and are what the shell's pattern matcher reaches for.

use std::ffi::{CStr, CString};

use super::Locale;

#[cfg(not(any(target_vendor = "apple", target_env = "musl")))]
type MbState = libc::mbstate_t;

// Two libcs keep `mbstate_t` opaque, so the Rust bindings do not export the
// typedef and the state has to be carried as storage instead. Darwin's ABI
// defines a 128-byte union aligned for a 64-bit integer; musl's is a pair of
// `unsigned`, which is far smaller. One buffer covers both: the C side writes
// at most its own `sizeof`, so over-allocating is safe where under-allocating
// would not be, and the alignment here is at least either ABI's.
#[cfg(any(target_vendor = "apple", target_env = "musl"))]
#[repr(C, align(8))]
pub(crate) struct MbState([u8; 128]);

unsafe extern "C" {
    fn wctype(name: *const core::ffi::c_char) -> core::ffi::c_ulong;
    fn iswctype(wc: core::ffi::c_uint, desc: core::ffi::c_ulong) -> core::ffi::c_int;
    fn mbrtowc(
        wide: *mut i32,
        bytes: *const core::ffi::c_char,
        len: usize,
        state: *mut MbState,
    ) -> usize;
    fn mbrlen(bytes: *const core::ffi::c_char, len: usize, state: *mut MbState) -> usize;
    fn iswblank(wc: core::ffi::c_uint) -> core::ffi::c_int;
    fn iswspace(wc: core::ffi::c_uint) -> core::ffi::c_int;
    fn wcrtomb(bytes: *mut core::ffi::c_char, wide: i32, state: *mut MbState) -> usize;
    fn nl_langinfo(item: libc::nl_item) -> *const core::ffi::c_char;
}

// The destination `wcrtomb` is given.  C requires it to be at least
// `MB_LEN_MAX` bytes, which is 16 on glibc and smaller elsewhere, and the
// widest encoding any charmap glibc ships actually uses is six.  This is
// well over both, because over-allocating is safe where under-allocating
// would let the C library write past the end.
const ENCODED_CHARACTER_MAX: usize = 64;

/// Result of feeding one byte to a locale-bound incremental decoder.
pub enum LocaleDecode {
    Incomplete,
    Complete(i32),
    Invalid,
}

/// One character read from the front of a byte string, and what it cost
/// in bytes.
///
/// `Incomplete` says the bytes are a valid beginning that the string ends
/// too soon to finish: a caller holding more bytes should ask again with
/// them, and a caller holding none has found an invalid sequence. That
/// distinction is why this is not `Option`.
pub enum LocaleCharacter {
    Complete { wide: i32, width: usize },
    Incomplete,
    Invalid,
}

/// Incremental decoder for one character.
pub struct LocaleDecoder {
    state: MbState,
    locale: Locale,
}

impl LocaleDecoder {
    pub fn push(&mut self, byte: u8) -> LocaleDecode {
        self.locale.decode_byte(&mut self.state, byte)
    }
}

impl Locale {
    /// Start an incremental multibyte decoder bound to this locale.
    pub fn decoder(&self) -> LocaleDecoder {
        LocaleDecoder {
            // SAFETY: an all-zero `mbstate_t` is the initial conversion
            // state C requires of every `mbrtowc` sequence, and the type
            // holds no pointer or reference for which zero is invalid.
            state: unsafe { std::mem::zeroed() },
            locale: self.clone(),
        }
    }

    pub(crate) fn decode_byte(&self, state: &mut MbState, byte: u8) -> LocaleDecode {
        self.with_selected(|| {
            crate::work::record_character_queries(1);
            let mut wide = 0_i32;
            // SAFETY: the byte and conversion records are live for the call;
            // `mbrtowc` retains no pointers.
            let result = unsafe { mbrtowc(&mut wide, (&byte as *const u8).cast(), 1, state) };
            if result == usize::MAX - 1 {
                LocaleDecode::Incomplete
            } else if result == 1 {
                LocaleDecode::Complete(wide)
            } else {
                LocaleDecode::Invalid
            }
        })
    }

    /// Decode the character beginning at the start of `bytes` under one
    /// thread-locale selection.
    ///
    /// The same answer the incremental decoder reaches by being fed the
    /// same bytes one at a time -- `mbstate_t` is what makes those two
    /// equivalent -- for one selection rather than one per byte. A caller
    /// that cannot offer the whole character at once, because offering it
    /// would mean blocking on a read, still wants [`LocaleDecoder`].
    pub fn decode_prefix(&self, bytes: &[u8]) -> LocaleCharacter {
        if bytes.is_empty() {
            return LocaleCharacter::Incomplete;
        }
        self.with_selected(|| {
            crate::work::record_character_queries(1);
            let mut wide = 0_i32;
            // SAFETY: the conversion is bounded by the slice and both the
            // wide character and the state are initialized local storage;
            // `mbrtowc` retains no pointer past the call.
            let consumed = unsafe {
                let mut state = std::mem::zeroed();
                mbrtowc(&mut wide, bytes.as_ptr().cast(), bytes.len(), &mut state)
            };
            if consumed == usize::MAX - 1 {
                LocaleCharacter::Incomplete
            } else if consumed == usize::MAX {
                LocaleCharacter::Invalid
            } else {
                // A null character reports zero bytes consumed and is one
                // byte wide, which is what a stepping caller has to move.
                LocaleCharacter::Complete {
                    wide,
                    width: consumed.max(1),
                }
            }
        })
    }

    pub fn wide_is_blank(&self, wide: i32) -> bool {
        self.with_selected(|| {
            crate::work::record_character_queries(1);
            // SAFETY: every value is accepted as `wint_t`; invalid values do
            // not match.
            unsafe { iswblank(wide as core::ffi::c_uint) != 0 }
        })
    }

    pub fn wide_is_space(&self, wide: i32) -> bool {
        self.with_selected(|| {
            crate::work::record_character_queries(1);
            // SAFETY: every value is accepted as `wint_t`; invalid values do
            // not match.
            unsafe { iswspace(wide as core::ffi::c_uint) != 0 }
        })
    }

    /// Whether this locale's charmap is UTF-8.
    ///
    /// Asked by name rather than by probing an encoding, because the caller
    /// wants the charmap's identity and not one of its spellings: UTF-8 is
    /// the charmap whose encoding a shell writes itself instead of asking
    /// the C library for it, so that the range the original UTF-8 defined
    /// stays reachable on a C library that has since narrowed to
    /// Unicode's.
    pub(crate) fn charmap_is_utf8(&self) -> bool {
        self.with_selected(|| {
            // SAFETY: `nl_langinfo` returns a terminated string owned by the
            // C library, valid until the locale is changed -- which cannot
            // happen before the comparison below, on this thread.
            let name = unsafe { nl_langinfo(libc::CODESET) };
            if name.is_null() {
                return false;
            }
            // SAFETY: see above; the pointer is non-null and terminated.
            let name = unsafe { CStr::from_ptr(name) };
            let name = name.to_bytes();
            name.eq_ignore_ascii_case(b"UTF-8") || name.eq_ignore_ascii_case(b"UTF8")
        })
    }

    /// The bytes this locale's charmap spells character number `value` with,
    /// or `None` when the charmap has no spelling for it.
    ///
    /// This is the direction [`Self::decode_exact`] does not go, and the one
    /// a shell needs to write a `\u` escape out: the escape names a
    /// character and the charmap decides which bytes stand for it, or that
    /// none do.
    ///
    /// A value above `i32::MAX` is refused here rather than handed on,
    /// because `wchar_t` is signed and passing one through would ask the C
    /// library about a different character than the caller named.
    pub(crate) fn encode_character(&self, value: u32) -> Option<Vec<u8>> {
        let wide = i32::try_from(value).ok()?;
        self.with_selected(|| {
            crate::work::record_character_queries(1);
            let mut encoded = [0_u8; ENCODED_CHARACTER_MAX];
            // SAFETY: the destination is `MB_LEN_MAX` bytes or more of live
            // local storage, which is the whole of `wcrtomb`'s contract for
            // it, and the conversion state is initialized here; `wcrtomb`
            // retains neither pointer.
            let written = unsafe {
                let mut state = std::mem::zeroed();
                wcrtomb(encoded.as_mut_ptr().cast(), wide, &mut state)
            };
            (written != usize::MAX).then(|| encoded[..written.min(encoded.len())].to_vec())
        })
    }

    pub fn multibyte_len(&self, bytes: &[u8]) -> Option<usize> {
        self.with_selected(|| {
            crate::work::record_character_queries(1);
            // SAFETY: the conversion is bounded by the input slice and uses
            // initialized local state.
            let mut state = unsafe { std::mem::zeroed() };
            let length = unsafe { mbrlen(bytes.as_ptr().cast(), bytes.len(), &mut state) };
            if length == usize::MAX || length == usize::MAX - 1 {
                None
            } else {
                Some(length)
            }
        })
    }

    /// How wide the character beginning at each of the first `offsets`
    /// byte positions of `bytes` is, in bytes.
    ///
    /// One entry per byte position, because `pattern.rs` and `regex.rs`
    /// index the answer by byte position.  A position where no character
    /// begins -- the interior of a wider one, an invalid sequence, one
    /// the string ends too soon to complete, or the null character -- is
    /// one byte wide, which is what a caller has to step over to make
    /// progress.
    ///
    /// **`bytes` must begin a character.** The C library is asked only
    /// about positions the walk reaches by stepping from position zero;
    /// the interior positions it steps over are filled in without
    /// asking, because a walk cannot arrive at one and no caller in this
    /// tree indexes the table anywhere else.  Asking about them cost one
    /// `mbrlen` per byte of a value rather than one per character.
    /// `[spec:nsh:req:cost.only-the-work-the-command-needs]` is that
    /// difference.
    ///
    /// The answer therefore runs on past `offsets` to the end of the
    /// character straddling it, never stopping mid-character: the
    /// vector's length is itself a boundary, which is what lets a caller
    /// learning a long string in blocks hand the next block a start that
    /// begins a character.
    ///
    /// A run of positions is answered together because `mbrlen` has no
    /// locale-taking form: every answer needs the thread locale selected
    /// and restored, and one selection covers the whole run.
    // [spec:nsh:req:cost.only-the-work-the-command-needs]
    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged]
    // [spec:nsh:req:shell-locale.operation-binding]
    pub fn character_widths(&self, bytes: &[u8], offsets: usize) -> Vec<u8> {
        let offsets = offsets.min(bytes.len());
        self.with_selected(|| {
            let mut widths: Vec<u8> = Vec::with_capacity(offsets);
            while widths.len() < offsets {
                let at = widths.len();
                crate::work::record_character_queries(1);
                // SAFETY: each conversion is bounded by the bytes that
                // remain after `at` and uses initialized local state.
                let width = unsafe {
                    let mut state = std::mem::zeroed();
                    mbrlen(bytes[at..].as_ptr().cast(), bytes.len() - at, &mut state)
                };
                let width = u8::try_from(width)
                    .ok()
                    .filter(|width| *width > 0)
                    .unwrap_or(1);
                widths.push(width);
                /* `mbrlen` never reports more bytes than it was given,
                 * so the interior never runs past the string. */
                widths.resize(at + usize::from(width), 1);
            }
            widths
        })
    }

    pub fn decode_exact(&self, bytes: &[u8], expected_len: usize) -> Option<i32> {
        if expected_len > bytes.len() {
            return None;
        }
        self.with_selected(|| {
            crate::work::record_character_queries(1);
            // SAFETY: the byte count is bounded by the slice and both output
            // records are initialized local storage.
            let mut state = unsafe { std::mem::zeroed() };
            let mut wide = 0_i32;
            let converted =
                unsafe { mbrtowc(&mut wide, bytes.as_ptr().cast(), expected_len, &mut state) };
            (converted == expected_len).then_some(wide)
        })
    }

    pub fn wide_class_matches(
        &self,
        name: &[u8],
        bytes: &[u8],
        expected_len: usize,
    ) -> Option<bool> {
        let name = CString::new(name).ok()?;
        self.with_selected(|| {
            crate::work::record_character_queries(1);
            // SAFETY: all pointers name initialized, bounded storage and the
            // class name is terminated.
            unsafe {
                let class = wctype(name.as_ptr());
                if class == 0 {
                    return None;
                }
                let mut wide = 0_i32;
                let mut state = std::mem::zeroed();
                let converted = mbrtowc(
                    &mut wide,
                    bytes.as_ptr().cast(),
                    expected_len.min(bytes.len()),
                    &mut state,
                );
                Some(converted == expected_len && iswctype(wide as core::ffi::c_uint, class) != 0)
            }
        })
    }

    pub fn wide_chars(&self, bytes: &[u8]) -> (usize, Vec<i32>) {
        if bytes.is_empty() {
            return (0, Vec::new());
        }
        self.with_selected(|| {
            // SAFETY: every conversion is bounded by the remaining slice and
            // writes only initialized local storage.
            unsafe {
                crate::work::record_character_queries(1);
                let mut first_state = std::mem::zeroed();
                let mut first = 0_i32;
                let first_len = mbrtowc(
                    &mut first,
                    bytes.as_ptr().cast(),
                    bytes.len(),
                    &mut first_state,
                );
                let first_len = if first_len == usize::MAX || first_len == usize::MAX - 1 {
                    1
                } else {
                    first_len
                };

                let mut decoded = vec![0_i32; bytes.len() + 1];
                let mut state = std::mem::zeroed();
                let mut offset = 0;
                let mut output = 0;
                while offset < bytes.len() {
                    crate::work::record_character_queries(1);
                    let mut wide = 0_i32;
                    let count = mbrtowc(
                        &mut wide,
                        bytes[offset..].as_ptr().cast(),
                        bytes.len() - offset,
                        &mut state,
                    );
                    if count == usize::MAX || count == usize::MAX - 1 || count == 0 {
                        break;
                    }
                    decoded[output] = wide;
                    output += 1;
                    offset += count;
                }
                (first_len, decoded)
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CharacterEncoding;
    use crate::unix::locale::tests::{current, utf8};

    /// Three charmaps give three different answers for one character.
    ///
    /// U+00CC is the case that separates them: ASCII cannot write it at
    /// all, ISO-8859-1 writes it as the single byte 0xCC, and UTF-8 is
    /// named rather than encoded because its original range is wider than
    /// the one `wcrtomb` will now produce. The single-byte charmap is
    /// generated rather than installed, and it is a third of what this test
    /// measures, so its absence is reported instead of skipped.
    // [spec:nsh:req:shell-locale.operation-binding/test]
    #[test]
    fn a_charmap_answers_for_its_own_characters() {
        let c = Locale::c().unwrap();
        assert!(matches!(
            c.character_encoding(0xcc),
            CharacterEncoding::Unrepresentable
        ));
        assert!(matches!(
            c.character_encoding(0x20ac),
            CharacterEncoding::Unrepresentable
        ));
        /* ASCII is in every charmap, including the narrowest. */
        assert!(
            matches!(c.character_encoding(0x41), CharacterEncoding::Bytes(bytes) if bytes == b"A")
        );

        assert!(matches!(
            utf8().character_encoding(0xcc),
            CharacterEncoding::Utf8
        ));

        let latin1 = Locale::new(b"en_US.ISO-8859-1", &[]).unwrap_or_else(|error| {
            panic!(
                "en_US.ISO-8859-1 is required by this test and could not be opened: {error}\n\
                 build it and name it to the run:\n\
                 \x20   export LOCPATH=$(tests/build-locales.sh)"
            )
        });
        assert!(matches!(
            latin1.character_encoding(0xcc),
            CharacterEncoding::Bytes(bytes) if bytes == [0xcc]
        ));
        assert!(matches!(
            latin1.character_encoding(0x100),
            CharacterEncoding::Unrepresentable
        ));
    }

    /// `decode_prefix` answers what the incremental decoder answers, and
    /// says which of the two "no character" cases it found.
    ///
    /// `Incomplete` and `Invalid` are different instructions to the
    /// caller -- fetch more bytes, or step over one -- and conflating
    /// them is what `Option` would have done. The truncated sequence and
    /// the lone continuation byte are the two that separate them, and the
    /// same bytes in a single-byte charmap are neither: there every byte
    /// begins a character one byte wide.
    // [spec:nsh:req:shell-locale.operation-binding/test]
    #[test]
    fn a_prefix_decodes_or_says_why_not() {
        let utf8 = utf8();
        assert!(matches!(
            utf8.decode_prefix(&[0xc3, 0x8c, b'a']),
            LocaleCharacter::Complete {
                wide: 0xcc,
                width: 2
            }
        ));
        assert!(matches!(
            utf8.decode_prefix(b"az"),
            LocaleCharacter::Complete {
                wide: 0x61,
                width: 1
            }
        ));
        /* Valid so far, and the string ends too soon to finish it. */
        assert!(matches!(
            utf8.decode_prefix(&[0xc3]),
            LocaleCharacter::Incomplete
        ));
        assert!(matches!(
            utf8.decode_prefix(&[0xe2, 0x82]),
            LocaleCharacter::Incomplete
        ));
        /* A continuation byte begins nothing, however many follow it. */
        assert!(matches!(
            utf8.decode_prefix(&[0x8c, 0x8c]),
            LocaleCharacter::Invalid
        ));
        /* Nothing to read is not an answer about a character. */
        assert!(matches!(
            utf8.decode_prefix(&[]),
            LocaleCharacter::Incomplete
        ));

        /* The null character consumes a byte even though `mbrtowc`
         * reports zero, because one byte is what a caller has to step. */
        assert!(matches!(
            utf8.decode_prefix(&[0, b'a']),
            LocaleCharacter::Complete { wide: 0, width: 1 }
        ));

        /* The C locale is ASCII and nothing else, which is easy to get
         * wrong in the other direction: it does not call a high byte a
         * one-byte character, it refuses it. `character_widths` is what
         * turns that refusal into "step one byte" for a walking caller;
         * `decode_prefix` reports what the charmap said. */
        let c = Locale::c().unwrap();
        for bytes in [&[0xc3_u8][..], &[0x8c, 0x8c][..], &[0xe2, 0x82][..]] {
            assert!(matches!(c.decode_prefix(bytes), LocaleCharacter::Invalid));
        }

        /* A single-byte charmap is where every byte does begin a
         * character: the answer follows the charmap, not the bytes. */
        let latin1 = Locale::new(b"en_US.ISO-8859-1", &[]).unwrap_or_else(|error| {
            panic!(
                "en_US.ISO-8859-1 is required by this test and could not be opened: {error}\n\
                 build it and name it to the run:\n\
                 \x20   export LOCPATH=$(tests/build-locales.sh)"
            )
        });
        for bytes in [&[0xcc_u8][..], &[0xc3][..], &[0x8c, 0x8c][..]] {
            assert!(matches!(
                latin1.decode_prefix(bytes),
                LocaleCharacter::Complete { width: 1, .. }
            ));
        }

        let before = current();
        let _ = utf8.decode_prefix(&[0xc3, 0x8c]);
        assert_eq!(
            current(),
            before,
            "decode_prefix left the thread locale moved"
        );
    }

    /// A value no `wchar_t` can hold is refused rather than wrapped.
    ///
    /// `wchar_t` is signed, so handing the C library a value above
    /// `i32::MAX` would ask it about a different character than the caller
    /// named.
    // [spec:nsh:req:shell-locale.operation-binding/test]
    #[test]
    fn a_value_beyond_wchar_is_refused() {
        let utf8 = utf8();
        assert!(utf8.encode_character(0x8000_0000).is_none());
        assert!(utf8.encode_character(0xffff_ffff).is_none());
        assert!(utf8.encode_character(0x7fff_ffff).is_some());
    }

    /// Asking a charmap a question leaves the thread's own locale alone.
    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged/test]
    #[test]
    fn encoding_preserves_thread_locale() {
        let before = current();
        let locale = Locale::c().unwrap();
        let _ = locale.character_encoding(0xcc);
        let _ = locale.charmap_is_utf8();
        assert_eq!(current(), before);
    }

    /// One entry per byte position, one byte wherever no character
    /// begins, and a run that stops only on a boundary.
    ///
    /// The three ways a position can hold no character are each here,
    /// because a caller reading this table steps by what it says and
    /// would loop forever on a zero: the second byte of a two-byte
    /// character, a byte no character may start with, and a start byte
    /// the string ends too soon to complete. All three answer one, which
    /// is what stepping over them costs.
    ///
    /// The short run is the same test rather than a second one, and it
    /// is where the two-byte character straddles the end of what was
    /// asked about: a caller learning widths in blocks hands the next
    /// block a start, so a run that stopped at the two positions asked
    /// for would hand it the interior of a character. Three entries,
    /// agreeing with the full table, is the answer that cannot.
    // [spec:nsh:req:shell-locale.operation-binding/test]
    #[test]
    fn character_widths_stop_only_where_a_character_does() {
        /* U+00CC, then a lone continuation byte, then a truncated
         * two-byte start. */
        let bytes = [b'a', 0xc3, 0x8c, 0x8c, b'z', 0xc3];
        let utf8 = utf8();
        assert_eq!(
            utf8.character_widths(&bytes, bytes.len()),
            [1, 2, 1, 1, 1, 1]
        );
        assert_eq!(utf8.character_widths(&bytes, 0), []);
        assert_eq!(utf8.character_widths(&bytes, 2), [1, 2, 1]);
        assert_eq!(utf8.character_widths(&bytes, 99), [1, 2, 1, 1, 1, 1]);
        assert_eq!(utf8.character_widths(&[], 4), []);

        /* A single-byte charmap has no character wider than its bytes,
         * and the C locale's own NUL is a character one byte wide. */
        let c = Locale::c().unwrap();
        assert_eq!(c.character_widths(&bytes, bytes.len()), [1; 6]);
        assert_eq!(c.character_widths(&[0, b'a', 0], 3), [1, 1, 1]);
    }

    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged/test]
    #[test]
    fn character_widths_preserve_thread_locale() {
        let before = current();
        let _ = utf8().character_widths(&[0xc3, 0x8c, b'a'], 3);
        assert_eq!(current(), before);
    }

    // [spec:nsh:req:shell-locale.handle-lifetime/test]
    #[test]
    fn decoder_keeps_locale_handle_alive() {
        let mut decoder = Locale::c().unwrap().decoder();
        assert!(matches!(
            decoder.push(b'A'),
            LocaleDecode::Complete(value) if value == i32::from(b'A')
        ));
    }
}
