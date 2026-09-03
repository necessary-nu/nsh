//! Owned POSIX locale objects and locale-explicit operations.
//!
//! The raw locale handle and the temporary thread selection needed by C APIs
//! without `_l` variants never leave this module.  Callers hold [`Locale`]
//! values and use safe methods whose selection is restored before they return.

use std::cmp::Ordering;
use std::ffi::{CStr, CString};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex};

use crate::Signal;

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

static STRSIGNAL: Mutex<()> = Mutex::new(());

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

/// One POSIX locale category that a shell variable can select.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum LocaleCategory {
    Collate,
    Ctype,
    Messages,
    Monetary,
    Numeric,
    Time,
}

/// Every category this shell manages, as one mask.
///
/// glibc and Darwin export `LC_ALL_MASK`; musl's Rust bindings do not. It is
/// derived here from the per-category masks rather than transcribed, so the
/// value cannot disagree with the categories actually used below -- and musl
/// defines no locale category outside this set, so the derived mask is its
/// `LC_ALL_MASK`.
#[cfg(target_env = "musl")]
const LC_ALL_MASK: core::ffi::c_int = libc::LC_COLLATE_MASK
    | libc::LC_CTYPE_MASK
    | libc::LC_MESSAGES_MASK
    | libc::LC_MONETARY_MASK
    | libc::LC_NUMERIC_MASK
    | libc::LC_TIME_MASK;

#[cfg(not(target_env = "musl"))]
const LC_ALL_MASK: core::ffi::c_int = libc::LC_ALL_MASK;

impl LocaleCategory {
    fn mask(self) -> core::ffi::c_int {
        match self {
            Self::Collate => libc::LC_COLLATE_MASK,
            Self::Ctype => libc::LC_CTYPE_MASK,
            Self::Messages => libc::LC_MESSAGES_MASK,
            Self::Monetary => libc::LC_MONETARY_MASK,
            Self::Numeric => libc::LC_NUMERIC_MASK,
            Self::Time => libc::LC_TIME_MASK,
        }
    }
}

// [spec:nsh:def:shell-locale.owned-locale]
/// An immutable, reference-counted POSIX locale object.
///
/// Clones share one C-library handle.  The last clone frees it, so an
/// incremental decoder can keep the locale alive independently of the shell
/// that created it.
#[derive(Clone)]
pub struct Locale(Arc<RawLocale>);

// [spec:nsh:req:shell-locale.handle-lifetime]
struct RawLocale(libc::locale_t);

// SAFETY: a successful `newlocale` handle is immutable after construction.
// POSIX permits the same locale object to be passed to locale-taking APIs and
// selected independently by multiple threads.  Destruction is serialized by
// `Arc` and happens only after the last operation has released its clone.
unsafe impl Send for RawLocale {}
// SAFETY: see the `Send` argument above; selection itself is thread-local.
unsafe impl Sync for RawLocale {}

impl Drop for RawLocale {
    fn drop(&mut self) {
        // SAFETY: this is the one owning handle returned by `newlocale`, and
        // `Arc` proves that no user remains when this destructor runs.
        unsafe { libc::freelocale(self.0) };
    }
}

// The lifetime prevents the selected locale from being freed, and `Rc` makes
// the guard neither Send nor Sync: restoration must run on the selecting
// thread.
struct LocaleGuard<'a> {
    previous: libc::locale_t,
    _locale: PhantomData<&'a RawLocale>,
    _not_send: PhantomData<Rc<()>>,
}

impl Drop for LocaleGuard<'_> {
    fn drop(&mut self) {
        // SAFETY: `previous` was returned by `uselocale` on this same thread,
        // and the `Rc` marker prevents this guard from crossing threads.
        let restored = unsafe { libc::uselocale(self.previous) };
        debug_assert!(!restored.is_null(), "restoring a valid locale failed");
    }
}

impl Locale {
    /// Construct a locale from an explicit base name and category overrides.
    ///
    /// Empty names are rejected rather than interpreted as requests to read
    /// the process environment.  Overrides are applied in slice order.
    pub fn new(base: &[u8], overrides: &[(LocaleCategory, &[u8])]) -> std::io::Result<Self> {
        fn explicit_name(name: &[u8]) -> std::io::Result<CString> {
            if name.is_empty() {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "a locale name cannot be empty",
                ));
            }
            CString::new(name).map_err(|_| {
                std::io::Error::new(
                    std::io::ErrorKind::InvalidInput,
                    "a locale name cannot contain NUL",
                )
            })
        }

        let base = explicit_name(base)?;
        // SAFETY: the name is live and terminated, the mask is supplied by
        // libc, and a null base requests a new independently owned object.
        let mut raw = unsafe { libc::newlocale(LC_ALL_MASK, base.as_ptr(), std::ptr::null_mut()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        for (category, name) in overrides {
            let name = match explicit_name(name) {
                Ok(name) => name,
                Err(error) => {
                    // SAFETY: a failed validation has not passed `raw` back
                    // to libc, so it remains the one owned live handle.
                    unsafe { libc::freelocale(raw) };
                    return Err(error);
                }
            };
            // SAFETY: `raw` is a live modifiable locale object and `name` is
            // terminated.  On success only the returned handle may be used;
            // on failure POSIX leaves the base handle valid.
            let next = unsafe { libc::newlocale(category.mask(), name.as_ptr(), raw) };
            if next.is_null() {
                let error = std::io::Error::last_os_error();
                // SAFETY: `newlocale` failed, so `raw` is still valid.
                unsafe { libc::freelocale(raw) };
                return Err(error);
            }
            raw = next;
        }

        Ok(Self(Arc::new(RawLocale(raw))))
    }

    /// Construct the portable POSIX `C` locale.
    pub fn c() -> std::io::Result<Self> {
        Self::new(b"C", &[])
    }

    fn select(&self) -> LocaleGuard<'_> {
        // SAFETY: the Arc-backed handle is live for the returned guard.  A
        // null return is the error sentinel; a valid previous selection may
        // be `LC_GLOBAL_LOCALE`, which is non-null.
        let previous = unsafe { libc::uselocale(self.0.0) };
        assert!(!previous.is_null(), "selecting an owned locale failed");
        LocaleGuard {
            previous,
            _locale: PhantomData,
            _not_send: PhantomData,
        }
    }

    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged]
    // [spec:nsh:req:shell-locale.operation-binding]
    pub(crate) fn with_selected<T>(&self, operation: impl FnOnce() -> T) -> T {
        let guard = self.select();
        let result = operation();
        drop(guard);
        result
    }

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

    pub fn is_alpha(&self, byte: u8) -> bool {
        self.with_selected(|| {
            // SAFETY: `isalpha` accepts every unsigned-char value.
            unsafe { libc::isalpha(byte.into()) != 0 }
        })
    }

    pub fn is_alphanumeric(&self, byte: u8) -> bool {
        self.with_selected(|| {
            // SAFETY: `isalnum` accepts every unsigned-char value.
            unsafe { libc::isalnum(byte.into()) != 0 }
        })
    }

    pub fn is_space(&self, byte: u8) -> bool {
        self.with_selected(|| {
            // SAFETY: `isspace` accepts every unsigned-char value.
            unsafe { libc::isspace(byte.into()) != 0 }
        })
    }

    pub fn wide_is_blank(&self, wide: i32) -> bool {
        self.with_selected(|| {
            // SAFETY: every value is accepted as `wint_t`; invalid values do
            // not match.
            unsafe { iswblank(wide as core::ffi::c_uint) != 0 }
        })
    }

    pub fn wide_is_space(&self, wide: i32) -> bool {
        self.with_selected(|| {
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
    /// One entry per byte position rather than per character, because a
    /// caller stepping through a string does not know where the next
    /// character starts until it has asked.  A position where no
    /// character begins -- an invalid sequence, one the string ends too
    /// soon to complete, or the null character -- is one byte wide, which
    /// is what such a caller has to step over to make progress.
    ///
    /// A run of positions is answered together because `mbrlen` has no
    /// locale-taking form: every answer needs the thread locale selected
    /// and restored, and one selection covers the whole run.
    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged]
    // [spec:nsh:req:shell-locale.operation-binding]
    pub fn character_widths(&self, bytes: &[u8], offsets: usize) -> Vec<u8> {
        let offsets = offsets.min(bytes.len());
        self.with_selected(|| {
            (0..offsets)
                .map(|at| {
                    // SAFETY: each conversion is bounded by the bytes that
                    // remain after `at` and uses initialized local state.
                    let width = unsafe {
                        let mut state = std::mem::zeroed();
                        mbrlen(bytes[at..].as_ptr().cast(), bytes.len() - at, &mut state)
                    };
                    u8::try_from(width)
                        .ok()
                        .filter(|width| *width > 0)
                        .unwrap_or(1)
                })
                .collect()
        })
    }

    pub fn decode_exact(&self, bytes: &[u8], expected_len: usize) -> Option<i32> {
        if expected_len > bytes.len() {
            return None;
        }
        self.with_selected(|| {
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

    pub fn collate(&self, left: &[u8], right: &[u8]) -> Ordering {
        fn c_string(bytes: &[u8]) -> CString {
            let visible = bytes.split(|&byte| byte == 0).next().unwrap_or_default();
            CString::new(visible).expect("the C-visible prefix contains no NUL")
        }

        let left = c_string(left);
        let right = c_string(right);
        self.with_selected(|| {
            // SAFETY: both strings are live and terminated for the call.
            unsafe { libc::strcoll(left.as_ptr(), right.as_ptr()).cmp(&0) }
        })
    }

    /// Ask the selected locale whether one collating bracket member matches.
    ///
    /// This is intentionally narrower than a second shell pattern matcher:
    /// the platform owns the locale database needed to resolve collating
    /// symbols and primary equivalence classes, while `nsh` owns all pattern
    /// traversal and shell quoting.
    pub fn collating_bracket_matches(&self, pattern: &[u8], subject: &[u8]) -> bool {
        let Ok(pattern) = CString::new(pattern) else {
            return false;
        };
        let Ok(subject) = CString::new(subject) else {
            return false;
        };
        self.with_selected(|| {
            // SAFETY: both inputs are live, terminated strings and fnmatch
            // retains neither pointer.
            unsafe { libc::fnmatch(pattern.as_ptr(), subject.as_ptr(), 0) == 0 }
        })
    }

    // [spec:nsh:req:idiom.platform-errors]
    pub fn error_message(&self, error: &std::io::Error) -> String {
        self.with_selected(|| {
            let Some(code) = error.raw_os_error() else {
                return error.to_string();
            };
            let rendered = error.to_string();
            let suffix = format!(" (os error {code})");
            rendered
                .strip_suffix(&suffix)
                .unwrap_or(&rendered)
                .to_owned()
        })
    }

    fn error_message_code(&self, code: i32) -> String {
        self.error_message(&std::io::Error::from_raw_os_error(code))
    }

    pub fn range_error_message(&self) -> String {
        self.error_message_code(libc::ERANGE)
    }

    pub fn signal_description(&self, signal: Signal) -> Vec<u8> {
        // glibc documents `strsignal` as MT-Unsafe because it may return a
        // shared buffer.  Serialize only that call and its immediate copy;
        // locale selection remains independent and thread-local.
        let _lock = STRSIGNAL
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        self.with_selected(|| {
            // SAFETY: `strsignal` accepts an integer signal number.  A
            // non-null result is a process-owned terminated string, copied
            // while the mutex is still held.
            let description = unsafe { libc::strsignal(signal.number()) };
            if description.is_null() {
                signal.number().to_string().into_bytes()
            } else {
                unsafe { CStr::from_ptr(description) }.to_bytes().to_vec()
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::CharacterEncoding;
    use std::panic::{AssertUnwindSafe, catch_unwind};

    fn current() -> libc::locale_t {
        // SAFETY: a null argument queries the current thread selection.
        unsafe { libc::uselocale(std::ptr::null_mut()) }
    }

    #[test]
    fn locale_is_send_and_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<Locale>();
    }

    // [spec:nsh:def:shell-locale.owned-locale/test]
    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged/test]
    #[test]
    fn construction_preserves_thread_locale() {
        let before = current();
        let _locale = Locale::c().unwrap();
        assert_eq!(current(), before);
    }

    /// A locale whose charmap is UTF-8, and a failure where there is none.
    ///
    /// `en_US.UTF-8` is not redundant with `C.UTF-8` here: setting `LOCPATH`
    /// stops glibc consulting the system locale archive, so a host that
    /// keeps its UTF-8 locales only in that archive has none until the
    /// generated one under `LOCPATH` answers to the third name.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure]
    fn utf8() -> Locale {
        [b"C.UTF-8".as_slice(), b"C.utf8", b"en_US.UTF-8"]
            .into_iter()
            .find_map(|name| Locale::new(name, &[]).ok())
            .filter(|locale| matches!(locale.character_encoding(0xcc), CharacterEncoding::Utf8))
            .expect("no UTF-8 charmap: tried C.UTF-8, C.utf8 and en_US.UTF-8")
    }

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
    /// begins, and only for the run of positions asked about.
    ///
    /// The three ways a position can hold no character are each here,
    /// because a caller reading this table steps by what it says and
    /// would loop forever on a zero: the second byte of a two-byte
    /// character, a byte no character may start with, and a start byte
    /// the string ends too soon to complete. All three answer one, which
    /// is what stepping over them costs.
    ///
    /// The short run is the same test rather than a second one: a caller
    /// learning widths in blocks asks about a prefix of the positions
    /// while passing the bytes that follow them, so that a character at
    /// the last position it asks about is measured whole, and the answer
    /// has to be the one it would have got asking about all of them.
    // [spec:nsh:req:shell-locale.operation-binding/test]
    #[test]
    fn character_widths_answer_every_byte_position() {
        /* U+00CC, then a lone continuation byte, then a truncated
         * two-byte start. */
        let bytes = [b'a', 0xc3, 0x8c, 0x8c, b'z', 0xc3];
        let utf8 = utf8();
        assert_eq!(
            utf8.character_widths(&bytes, bytes.len()),
            [1, 2, 1, 1, 1, 1]
        );
        assert_eq!(utf8.character_widths(&bytes, 0), []);
        assert_eq!(utf8.character_widths(&bytes, 2), [1, 2]);
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

    #[test]
    fn locale_collating_members_match() {
        let locale = Locale::c().unwrap();
        assert!(locale.collating_bracket_matches(b"[[.-.]]", b"-"));
        assert!(locale.collating_bracket_matches(b"[[=a=]]", b"a"));
        assert!(!locale.collating_bracket_matches(b"[[.zz.]]", b"zz"));
    }

    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged/test]
    #[test]
    fn nested_selections_restore_in_stack_order() {
        let before = current();
        let outer = Locale::c().unwrap();
        let inner = Locale::c().unwrap();
        outer.with_selected(|| {
            assert_eq!(current(), outer.0.0);
            inner.with_selected(|| assert_eq!(current(), inner.0.0));
            assert_eq!(current(), outer.0.0);
        });
        assert_eq!(current(), before);
    }

    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged/test]
    #[test]
    fn selection_is_restored_while_unwinding() {
        let before = current();
        let locale = Locale::c().unwrap();
        let panic = catch_unwind(AssertUnwindSafe(|| {
            locale.with_selected(|| panic!("probe"));
        }));
        assert!(panic.is_err());
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
