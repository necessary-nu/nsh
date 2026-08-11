//! Literal port of `src/cd.c` / `src/cd.h`.
//! Rules: `docs/spec/port/src/cd.md`.
//!
//! `__CYGWIN__` is not selected, so `updatepwd` does no path normalisation.
//! `getpwd` uses Rust's Unix OS-string cwd query, preserving the bytes that
//! the selected glibc `getcwd(0, 0)` path returned without its raw allocation.

use bstr::{BStr, BString, ByteSlice};
use core::ptr::{addr_of, addr_of_mut, null_mut};
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;

use crate::error::{INTOFF, INTON};
use crate::var::{VEXPORT, setvar};

/* The C's `nullstr` sentinel is `None`.  It is a sentinel, not an empty
 * path: `getpwd` never returns an empty string on success and `updatepwd`
 * never produces one, so no reachable value collides with it. */
pub(crate) static mut curdir: Option<BString> = None; /* current working directory */
pub(crate) static mut physdir: Option<BString> = None; /* physical working directory */

/// The bytes `setvar` and shell output want: a path with the terminator the C's
/// readers read up to, `nullstr`'s empty string when the sentinel is set.
pub(crate) unsafe fn cbytes(s: &Option<BString>) -> Vec<u8> {
    let mut v = match s {
        Some(b) => b.to_vec(),
        None => Vec::new(),
    };
    v.push(0);
    v
}

/*
 * Actually do the chdir.  We also call hashcd to let the routines in exec.c
 * know that the current directory has changed.
 */

/*
 * Update curdir (the name of the current directory) in response to a
 * cd command.
 */

/*
 * Find out what the current directory is. If we already know the current
 * directory, this routine returns immediately.
 */

// [spec:dash:def:cd.getpwd-fn]
// [spec:dash:sem:cd.getpwd-fn]
unsafe fn getpwd() -> Option<BString> {
    match std::env::current_dir() {
        Ok(dir) => return Some(BString::from(dir.as_os_str().as_bytes())),
        Err(err) => {
            /* `current_dir` is an OS query on Unix, so this is the errno
             * `getcwd` would have left for the C's `strerror(errno)` path. */
            let errno = err.raw_os_error().unwrap_or(libc::EIO);
            let mut message = b"getcwd() failed: ".to_vec();
            message.extend_from_slice(CStr::from_ptr(libc::strerror(errno)).to_bytes());
            crate::error::sh_warnx(&message);
        }
    }
    None
}

/// What `setpwd`'s `val` says, which the C encodes in two pointer
/// comparisons against a value the caller cannot construct any other way.
pub(crate) enum Pwd<'a> {
    /// `setpwd(NULL, …)` — ask the kernel, and take the answer for both
    /// `curdir` and `physdir`.
    Unknown,
    /// `setpwd(curdir, …)` — `pwdcmd`'s call.  Refresh `physdir`; `curdir`
    /// already holds the logical path and keeps it.
    Current,
    /// `setpwd(p, …)` — adopt `p` as the logical path.
    New(&'a BStr),
}

// [spec:dash:def:cd.setpwd-fn]
// [spec:dash:sem:cd.setpwd-fn]
pub unsafe fn setpwd(val: *const c_char, setold: c_int) {
    if val.is_null() {
        setpwd_inner(Pwd::Unknown, setold);
    } else {
        let bytes = core::slice::from_raw_parts(val as *const u8, libc::strlen(val));
        setpwd_inner(Pwd::New(BStr::new(bytes)), setold);
    }
}

pub(crate) unsafe fn setpwd_inner(val: Pwd, setold: c_int) {
    if setold != 0 {
        let old = cbytes(&*addr_of!(curdir));
        setvar(
            b"OLDPWD\0".as_ptr() as *const c_char,
            old.as_ptr() as *const c_char,
            VEXPORT,
        );
    }
    INTOFF();
    /* `free(physdir)` guarded by `physdir != oldcur`: the C's `curdir` and
     * `physdir` are one allocation after a `setpwd(NULL, …)`, and the guard
     * exists only to stop the double free.  Two owned copies say the same
     * thing without the alias. */
    physdir = None;
    match val {
        Pwd::Unknown | Pwd::Current => {
            let s = getpwd();
            if matches!(val, Pwd::Unknown) {
                curdir = s.clone();
            }
            physdir = s;
        }
        Pwd::New(v) => {
            curdir = Some(v.to_owned());
        }
    }
    let dir = cbytes(&*addr_of!(curdir));
    INTON();
    setvar(
        b"PWD\0".as_ptr() as *const c_char,
        dir.as_ptr() as *const c_char,
        VEXPORT,
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::ffi::OsString;
    use std::os::unix::ffi::OsStringExt;
    use std::path::PathBuf;

    struct CwdGuard {
        old: PathBuf,
        temporary: PathBuf,
    }

    impl Drop for CwdGuard {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.old).unwrap();
            match std::fs::remove_dir(&self.temporary) {
                Ok(()) => {}
                Err(err) if err.kind() == std::io::ErrorKind::NotFound => {}
                Err(err) => panic!("cannot remove cwd test directory: {err}"),
            }
        }
    }

    // [spec:dash:sem:cd.getpwd-fn/test]
    #[test]
    fn getpwd_preserves_non_utf8_path_bytes() {
        let _g = crate::testutil::lock();
        {
            let old = std::env::current_dir().unwrap();
            let mut component = format!("nsh-cd-test-{}-", std::process::id()).into_bytes();
            component.push(0xff);
            let temporary = std::env::temp_dir().join(OsString::from_vec(component));
            std::fs::create_dir(&temporary).unwrap();
            let _restore = CwdGuard {
                old,
                temporary: temporary.clone(),
            };
            std::env::set_current_dir(&temporary).unwrap();

            let got = unsafe { getpwd().unwrap() };
            assert_eq!(&got[..], temporary.as_os_str().as_bytes());
            assert!(!got.contains(&0));
        }

        {
            let old = std::env::current_dir().unwrap();
            let component = format!("nsh-cd-deleted-{}", std::process::id());
            let temporary = std::env::temp_dir().join(component);
            std::fs::create_dir(&temporary).unwrap();
            let _restore = CwdGuard {
                old,
                temporary: temporary.clone(),
            };
            std::env::set_current_dir(&temporary).unwrap();
            std::fs::remove_dir(&temporary).unwrap();

            assert!(unsafe { getpwd() }.is_none());
        }
    }
}
