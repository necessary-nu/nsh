//! Literal port of `src/cd.c` / `src/cd.h`.
//! Rules: `docs/spec/port/src/cd.md`.
//!
//! `__CYGWIN__` is not selected, so `updatepwd` does no path normalisation.
//! `getpwd` uses Rust's Unix OS-string cwd query, preserving the bytes that
//! the selected glibc `getcwd(0, 0)` path returned without its raw allocation.

use crate::error::Error;
use bstr::{BStr, BString};
use nsh_platform::NativeStrExt as _;

use crate::variables::{VariableAttributes, set_bytes};

/* The C's `nullstr` sentinel is `None`.  It is a sentinel, not an empty
 * path: `getpwd` never returns an empty string on success and `updatepwd`
 * never produces one, so no reachable value collides with it. */
/// Where the shell thinks it is.
///
/// `docs/api-design.md` §5 does not list these, and it should: they are
/// the C's own `curdir`/`physdir`, the logical-versus-physical `$PWD`
/// pair, and two shells in different directories cannot share them.
/// Recorded as a correction to §5 on this node's log.
pub struct WorkingDirectoryState {
    /// `curdir` — the logical working directory, as `cd` computed it.
    pub(crate) logical: Option<BString>,
    /// `physdir` — the physical one, after symlinks.
    pub(crate) physical: Option<BString>,
}

impl WorkingDirectoryState {
    pub(crate) const fn new() -> Self {
        WorkingDirectoryState {
            logical: None,
            physical: None,
        }
    }
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

// [spec:dash:sem:cd.getpwd-fn]
fn current_directory() -> std::io::Result<BString> {
    nsh_platform::current_directory().map(|dir| BString::from(dir.to_shell_bytes()))
}

/// What `setpwd`'s `val` says, which the C encodes in two pointer
/// comparisons against a value the caller cannot construct any other way.
pub(crate) enum DirectoryUpdate<'a> {
    /// `setpwd(NULL, …)` — ask the kernel, and take the answer for both
    /// `curdir` and `physdir`.
    Unknown,
    /// `setpwd(curdir, …)` — `pwdcmd`'s call.  Refresh `physdir`; `curdir`
    /// already holds the logical path and keeps it.
    Current,
    /// `setpwd(p, …)` — adopt `p` as the logical path.
    New(&'a BStr),
}

// [spec:dash:sem:cd.setpwd-fn]
// [spec:posix:req:param.pwd]
// [spec:posix:req:param.pwd-assignment]
pub(crate) fn update_current_directory(
    shell: &mut crate::context::Shell,
    val: DirectoryUpdate,
    set_old: bool,
) -> Result<(), Error> {
    if set_old {
        let old = shell.working_directory.logical.clone().unwrap_or_default();
        set_bytes(
            shell,
            BStr::new("OLDPWD"),
            Some(BStr::new(&old)),
            VariableAttributes::EXPORTED,
        )?;
    }
    let dir = crate::error::with_interrupts_deferred(shell, |shell| {
        /* `free(physdir)` guarded by `physdir != oldcur`: the C's `curdir` and
         * `physdir` are one allocation after a `setpwd(NULL, …)`, and the guard
         * exists only to stop the double free. Two owned copies say the same
         * thing without the alias. */
        shell.working_directory.physical = None;
        match val {
            DirectoryUpdate::Unknown | DirectoryUpdate::Current => {
                let current = match current_directory() {
                    Ok(current) => Some(current),
                    Err(error) => {
                        let mut message = b"getcwd() failed: ".to_vec();
                        message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
                        shell.diagnostics().shell_warning(&message);
                        None
                    }
                };
                if matches!(val, DirectoryUpdate::Unknown) {
                    shell.working_directory.logical = current.clone();
                }
                shell.working_directory.physical = current;
            }
            DirectoryUpdate::New(path) => {
                shell.working_directory.logical = Some(path.to_owned());
            }
        }
        shell.working_directory.logical.clone().unwrap_or_default()
    });
    set_bytes(
        shell,
        BStr::new("PWD"),
        Some(BStr::new(&dir)),
        VariableAttributes::EXPORTED,
    )?;
    /* `$DIRSTACK`'s first entry is the current directory, so a `cd`
     * moves the stack even though it never touches `pushd`'s half. */
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    crate::variables::special::publish_directory_stack(shell);
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use nsh_platform::ShellBytesExt as _;
    use std::path::PathBuf;

    struct WorkingDirectoryGuard {
        old: PathBuf,
        temporary: PathBuf,
    }

    impl Drop for WorkingDirectoryGuard {
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
        let _g = crate::test_support::lock();
        {
            let old = std::env::current_dir().unwrap();
            let mut component = format!("nsh-cd-test-{}-", std::process::id()).into_bytes();
            #[cfg(unix)]
            component.push(0xff);
            #[cfg(windows)]
            component.extend_from_slice(&[0xed, 0xa0, 0x80]);
            let temporary = std::env::temp_dir().join(component.try_to_os_string().unwrap());
            std::fs::create_dir(&temporary).unwrap();
            let _restore = WorkingDirectoryGuard {
                old,
                temporary: temporary.clone(),
            };
            std::env::set_current_dir(&temporary).unwrap();

            let got = current_directory().unwrap();
            assert_eq!(&got[..], temporary.to_shell_bytes());
            assert!(!got.contains(&0));
        }

        if !nsh_platform::can_unlink_current_directory() {
            return;
        }

        {
            let old = std::env::current_dir().unwrap();
            let component = format!("nsh-cd-deleted-{}", std::process::id());
            let temporary = std::env::temp_dir().join(component);
            std::fs::create_dir(&temporary).unwrap();
            let _restore = WorkingDirectoryGuard {
                old,
                temporary: temporary.clone(),
            };
            std::env::set_current_dir(&temporary).unwrap();
            std::fs::remove_dir(&temporary).unwrap();

            assert!(current_directory().is_err());
        }
    }
}
