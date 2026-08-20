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

#[cfg(not(target_vendor = "apple"))]
type MbState = libc::mbstate_t;

// Darwin's libc keeps `mbstate_t` opaque and its Rust libc bindings do not
// export the typedef. The system ABI defines it as a 128-byte union aligned
// for a 64-bit integer.
#[cfg(target_vendor = "apple")]
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
        let mut raw =
            unsafe { libc::newlocale(libc::LC_ALL_MASK, base.as_ptr(), std::ptr::null_mut()) };
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

    pub fn signal_description(&self, signal: i32) -> Vec<u8> {
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
            let description = unsafe { libc::strsignal(signal) };
            if description.is_null() {
                signal.to_string().into_bytes()
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
