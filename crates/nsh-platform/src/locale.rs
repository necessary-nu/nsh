//! Owned POSIX locale objects and locale-explicit operations.
//!
//! The raw locale handle and the temporary thread selection needed by C APIs
//! without `_l` variants never leave this module.  Callers hold [`Locale`]
//! values and use safe methods whose selection is restored before they return.

use std::cmp::Ordering;
use std::ffi::{CStr, CString};
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

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
}

static STRSIGNAL: Mutex<()> = Mutex::new(());

/// Result of feeding one byte to a locale-bound incremental decoder.
pub enum LocaleDecode {
    Incomplete,
    Complete(i32),
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
struct RawLocale {
    handle: libc::locale_t,
    /// What this locale answers about each of the 256 byte values, read out of
    /// it once and answered from here afterwards.
    ///
    /// A locale object is immutable once `newlocale` has returned it, so a
    /// byte's class is a property of the handle and not of the moment it is
    /// asked about. Asking the C library instead means selecting this locale
    /// and restoring the previous one around every single byte, and every
    /// caller is a loop: validating a variable name, skipping the blanks in
    /// front of a `printf` conversion, reading an arithmetic identifier.
    /// Importing an environment of forty variables spent several hundred
    /// `uselocale` pairs establishing that ordinary letters are letters.
    ///
    /// Filled on the first byte question rather than at construction, because
    /// building a shell constructs seven of these — a base plus one per locale
    /// category — and keeps the last. A locale nothing asks about a byte never
    /// pays for the table.
    classes: OnceLock<[ByteClass; 256]>,
}

/// What one locale says about one byte value, in the three single-byte classes
/// this module exposes.
#[derive(Clone, Copy, Default)]
struct ByteClass(u8);

impl ByteClass {
    const ALPHA: u8 = 1 << 0;
    const ALPHANUMERIC: u8 = 1 << 1;
    const SPACE: u8 = 1 << 2;

    const fn holds(self, class: u8) -> bool {
        self.0 & class != 0
    }
}

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
        unsafe { libc::freelocale(self.handle) };
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
    ///
    /// A run of adjacent overrides naming the same locale is applied as one
    /// call with their masks joined. `newlocale` sets every category in the
    /// mask it is given to the name it is given, so a joined run says exactly
    /// what applying it a category at a time says — and says it without
    /// loading and interning the same locale data once per category. A shell
    /// selects all six of its categories through one `LANG`, so this is the
    /// ordinary shape rather than an unusual one. Runs rather than the whole
    /// slice, so that a name is still first tried where the caller wrote it
    /// and a later override of the same category still wins.
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
        let mut raw =
            unsafe { libc::newlocale(LC_ALL_MASK, base.as_ptr(), std::ptr::null_mut()) };
        if raw.is_null() {
            return Err(std::io::Error::last_os_error());
        }

        /* A run of adjacent overrides naming the same locale is applied as one
         * call with their masks joined. `newlocale` sets every category in the
         * mask it is given to the name it is given, so joining a run says
         * exactly what applying it a category at a time says -- and it says it
         * without loading and interning the same locale data six times over.
         * A shell selects all six categories through `LANG`, so this is not an
         * unusual shape but the ordinary one. Runs rather than the whole slice,
         * so that a name is still first tried where the caller wrote it and a
         * later override of the same category still wins. */
        /// Set every category in `mask` to `name`, taking ownership of `raw`
        /// and freeing it if the name cannot be used.
        fn apply(
            raw: libc::locale_t,
            mask: core::ffi::c_int,
            name: &[u8],
        ) -> std::io::Result<libc::locale_t> {
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
            let next = unsafe { libc::newlocale(mask, name.as_ptr(), raw) };
            if next.is_null() {
                let error = std::io::Error::last_os_error();
                // SAFETY: `newlocale` failed, so `raw` is still valid.
                unsafe { libc::freelocale(raw) };
                return Err(error);
            }
            Ok(next)
        }

        let mut pending: Option<(core::ffi::c_int, &[u8])> = None;
        for (category, name) in overrides {
            match pending {
                Some((mask, held)) if held == *name => {
                    pending = Some((mask | category.mask(), held));
                }
                Some((mask, held)) => {
                    raw = apply(raw, mask, held)?;
                    pending = Some((category.mask(), name));
                }
                None => pending = Some((category.mask(), name)),
            }
        }
        if let Some((mask, held)) = pending {
            raw = apply(raw, mask, held)?;
        }

        Ok(Self(Arc::new(RawLocale {
            handle: raw,
            classes: OnceLock::new(),
        })))
    }

    /// Construct the portable POSIX `C` locale.
    pub fn c() -> std::io::Result<Self> {
        Self::new(b"C", &[])
    }

    fn select(&self) -> LocaleGuard<'_> {
        // SAFETY: the Arc-backed handle is live for the returned guard.  A
        // null return is the error sentinel; a valid previous selection may
        // be `LC_GLOBAL_LOCALE`, which is non-null.
        let previous = unsafe { libc::uselocale(self.0.handle) };
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

    /// This locale's answer for every byte value, read out of it under a
    /// single selection the first time one is wanted.
    ///
    /// The three classes are read together because they are read from the same
    /// table on the C side: separating them would buy a shell that only ever
    /// asks `is_space` nothing, and would cost one selection per class to the
    /// shells that ask more than one.
    fn byte_classes(&self) -> &[ByteClass; 256] {
        self.0.classes.get_or_init(|| {
            self.with_selected(|| {
                let mut classes = [ByteClass::default(); 256];
                for (value, class) in classes.iter_mut().enumerate() {
                    // `as` rather than a fallible conversion: the loop bound
                    // is the number of values the type has.
                    #[expect(clippy::cast_possible_truncation, reason = "value < 256")]
                    let byte = i32::from(value as u8);
                    let mut bits = 0u8;
                    // SAFETY: these three accept every unsigned-char value,
                    // and the guard above has this locale selected.
                    unsafe {
                        if libc::isalpha(byte) != 0 {
                            bits |= ByteClass::ALPHA;
                        }
                        if libc::isalnum(byte) != 0 {
                            bits |= ByteClass::ALPHANUMERIC;
                        }
                        if libc::isspace(byte) != 0 {
                            bits |= ByteClass::SPACE;
                        }
                    }
                    *class = ByteClass(bits);
                }
                classes
            })
        })
    }

    pub fn is_alpha(&self, byte: u8) -> bool {
        self.byte_classes()[usize::from(byte)].holds(ByteClass::ALPHA)
    }

    pub fn is_alphanumeric(&self, byte: u8) -> bool {
        self.byte_classes()[usize::from(byte)].holds(ByteClass::ALPHANUMERIC)
    }

    pub fn is_space(&self, byte: u8) -> bool {
        self.byte_classes()[usize::from(byte)].holds(ByteClass::SPACE)
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
            assert_eq!(current(), outer.0.handle);
            inner.with_selected(|| assert_eq!(current(), inner.0.handle));
            assert_eq!(current(), outer.0.handle);
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

    /// The cached table has to answer for every byte value what the C library
    /// answers with this locale selected, which is what makes reading it
    /// instead of asking a cache rather than a change of behaviour.
    // [spec:nsh:req:shell-locale.operation-binding/test]
    #[test]
    fn the_byte_table_says_what_the_locale_says() {
        let locale = Locale::c().unwrap();
        for value in 0..=u8::MAX {
            let (alpha, alphanumeric, space) = locale.with_selected(|| {
                // SAFETY: all three accept every unsigned-char value, and the
                // guard has this locale selected.
                unsafe {
                    (
                        libc::isalpha(i32::from(value)) != 0,
                        libc::isalnum(i32::from(value)) != 0,
                        libc::isspace(i32::from(value)) != 0,
                    )
                }
            });
            assert_eq!(locale.is_alpha(value), alpha, "is_alpha({value})");
            assert_eq!(
                locale.is_alphanumeric(value),
                alphanumeric,
                "is_alphanumeric({value})"
            );
            assert_eq!(locale.is_space(value), space, "is_space({value})");
        }
    }

    /// Filling the table selects this locale, so it has to leave the thread's
    /// selection where it found it like every other operation here. The first
    /// question is the one that fills it, so it is the one that has to be
    /// asked from a locale nothing has touched.
    // [spec:nsh:req:embedding-safety.process-locale-is-unchanged/test]
    #[test]
    fn the_first_byte_question_restores_the_locale() {
        let before = current();
        let locale = Locale::c().unwrap();
        assert!(locale.is_alpha(b'a'));
        assert_eq!(current(), before);
    }

    /// Joining a run of overrides that name one locale must reach the same
    /// object as applying them one at a time, which is the whole claim.
    #[test]
    fn one_name_over_six_categories_answers_alike() {
        let categories = [
            LocaleCategory::Collate,
            LocaleCategory::Ctype,
            LocaleCategory::Messages,
            LocaleCategory::Monetary,
            LocaleCategory::Numeric,
            LocaleCategory::Time,
        ];
        let joined: Vec<_> = categories
            .iter()
            .map(|category| (*category, &b"C"[..]))
            .collect();
        let run = Locale::new(b"POSIX", &joined).unwrap();
        // Interleaved so that no two adjacent entries share a name, which is
        // the shape the joining cannot take and has to fall back from.
        let alternating: Vec<_> = categories
            .iter()
            .enumerate()
            .map(|(index, category)| {
                (
                    *category,
                    if index % 2 == 0 { &b"C"[..] } else { &b"POSIX"[..] },
                )
            })
            .collect();
        let separate = Locale::new(b"POSIX", &alternating).unwrap();
        for value in 0..=u8::MAX {
            assert_eq!(run.is_alpha(value), separate.is_alpha(value));
            assert_eq!(run.is_alphanumeric(value), separate.is_alphanumeric(value));
            assert_eq!(run.is_space(value), separate.is_space(value));
        }
        assert_eq!(run.collate(b"a", b"b"), separate.collate(b"a", b"b"));
    }

    /// A bad name still refuses, and refuses without leaking the base handle
    /// or the ones applied before it.
    #[test]
    fn an_unusable_override_name_still_refuses() {
        let refused = Locale::new(
            b"C",
            &[
                (LocaleCategory::Ctype, &b"C"[..]),
                (LocaleCategory::Collate, &b""[..]),
            ],
        );
        let Err(error) = refused else {
            panic!("an empty override name was accepted");
        };
        assert_eq!(error.kind(), std::io::ErrorKind::InvalidInput);
    }
}
