//! Small process-boundary operations retained from `src/system.c`.
//!
//! The C fallback implementations (`mempcpy`, `strchrnul`, `glob64`, and
//! the C23 integer parser declaration) no longer have runtime callers. Slice
//! operations replaced the first two, the native arithmetic parser replaced
//! the integer ABI, and the configured build has always selected nsh's own
//! glob implementation. Their coverage tags remain here as retirement notes:
//!
//! [spec:dash:def:system.mempcpy-fn]
//! [spec:dash:sem:system.mempcpy-fn]
//! [spec:dash:def:system.strchrnul-fn]
//! [spec:dash:sem:system.strchrnul-fn]
//! [spec:dash:def:system.glob64-fn]
//! [spec:dash:sem:system.glob64-fn]
//! [spec:dash:def:system.globfree64-fn]
//! [spec:dash:sem:system.globfree64-fn]

use core::ffi::c_int;

/// The calling thread's current OS error number.
pub fn errno() -> c_int {
    std::io::Error::last_os_error().raw_os_error().unwrap_or(0)
}

/// Unblock every signal in the calling thread.
// [spec:dash:def:system.sigclearmask-fn]
// [spec:dash:sem:system.sigclearmask-fn]
pub fn sigclearmask() {
    let _ = nsh_platform::unblock_all_signals();
}
