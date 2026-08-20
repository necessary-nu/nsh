//! `cd` and `chdir`.
//!
//! Port of `cdcmd` and its helpers from `src/cd.c`.
//!
//! What stays in `crate::cd` is the shell's idea of where it is --
//! `curdir`, `physdir` and the `setpwd` that maintains them. This module
//! is the command that moves it: the CDPATH search, the `-L`/`-P` option
//! scan, and the logical-path bookkeeping that `cd ..` needs and `chdir`
//! alone cannot do.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;
use nsh_platform::NativeStrExt as _;
use nsh_platform::ShellBytesExt as _;
use std::io::Write;

use crate::cd::{Pwd, cbytes, setpwd_inner};
use crate::error::{INTOFF, INTON};
use crate::eval::Flow;
use crate::options::Options;

const CD_PHYSICAL: c_int = 1;
const CD_PRINT: c_int = 2;
const CD_ERROR_IF_UNKNOWN: c_int = 4;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CdResult {
    Changed,
    ChangedPwdUnknown,
    Failed,
}

// [spec:dash:def:cd.cdopt-fn]
// [spec:dash:sem:cd.cdopt-fn]
// [spec:posix:syn:builtin.cd.syn]
// [spec:posix:req:builtin.cd.utility-syntax-guidelines]
// [spec:posix:req:builtin.cd.opt-l]
// [spec:posix:req:builtin.cd.opt-p]
// [spec:posix:req:builtin.cd.opt-l-p-last-wins]
// [spec:posix:req:builtin.cd.opt-e]
pub(crate) fn cdopt(sh: &mut crate::context::Shell, opts: &mut Options) -> Result<c_int, Error> {
    let mut flags: c_int = 0;
    let mut j: u8 = b'L';

    while let Some(i) = opts.next(sh, b"LPe")? {
        if i == b'e' {
            flags |= CD_ERROR_IF_UNKNOWN;
            continue;
        }
        if i != j {
            flags ^= CD_PHYSICAL;
            j = i;
        }
    }

    Ok(flags)
}

// [spec:dash:def:cd.cdcmd-fn]
// [spec:dash:sem:cd.cdcmd-fn]
// [spec:posix:req:builtin.cd.change-working-directory]
// [spec:posix:def:builtin.cd.curpath]
// [spec:posix:req:builtin.cd.step1-no-operand-no-home]
// [spec:posix:req:builtin.cd.step2-home-as-operand]
// [spec:posix:sem:builtin.cd.step3-absolute-operand]
// [spec:posix:sem:builtin.cd.step4-dot-or-dot-dot]
// [spec:posix:sem:builtin.cd.step5-cdpath-search]
// [spec:posix:sem:builtin.cd.step6-operand-as-curpath]
// [spec:posix:def:builtin.cd.operand-directory]
// [spec:posix:req:builtin.cd.operand-hyphen]
// [spec:posix:req:builtin.cd.env-cdpath]
// [spec:posix:def:builtin.cd.env-home]
// [spec:posix:req:builtin.cd.env-locale]
// [spec:posix:req:builtin.cd.env-nlspath]
// [spec:posix:req:builtin.cd.env-oldpwd]
// [spec:posix:req:builtin.cd.stdout-new-directory]
// [spec:posix:sem:builtin.cd.stdout-undeterminable-pathname]
// [spec:posix:req:builtin.cd.stdout-no-output]
// [spec:posix:req:builtin.cd.stderr]
// [spec:posix:req:builtin.cd.interfaces]
// [spec:posix:req:builtin.cd.exit-status]
// [spec:posix:req:builtin.cd.consequences-of-errors]
pub fn cdcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut flags: c_int;

    let mut opts = Options::new(args);
    flags = cdopt(sh, &mut opts)?;
    /* The operand outlives every reader below, which is what the C got
     * from `argv` living in `evalcommand`'s frame. */
    let operand = opts.operands().first().copied();
    // [spec:posix:req:builtin.cd.operand-empty-string]
    if operand.is_some_and(|directory| directory.is_empty()) {
        return Err(sh.sh_error_value(b"can't cd to an empty directory"));
    }
    let dest_value = match operand {
        None => crate::var::lookup_bytes(sh, BStr::new(b"HOME")).unwrap_or_default(),
        Some(d) if d == b"-" => {
            flags |= CD_PRINT;
            crate::var::lookup_bytes(sh, BStr::new(b"OLDPWD")).unwrap_or_default()
        }
        Some(d) => d.to_owned(),
    };
    let mut dest = dest_value.as_slice().as_bstr();
    let mut pwd_unknown = false;

    let step6 = nsh_platform::shell_path_is_absolute(dest)
        || dest == b"."
        || dest.starts_with(b"./")
        || dest == b".."
        || dest.starts_with(b"../");

    let mut out = false;
    if !step6 {
        if dest.is_empty() {
            dest = BStr::new(b".");
        }
        let path_value = crate::var::lookup_bytes(sh, BStr::new(b"CDPATH")).unwrap_or_default();
        let mut components =
            path_value.split(|byte| *byte == nsh_platform::search_path_separator());
        let mut path = crate::exec::PathCursor::literal(path_value.as_slice().as_bstr());
        while let Some(candidate) = crate::exec::padvance(&mut path, dest) {
            let component = components
                .next()
                .expect("PATH cursor and components advance together");
            let fullname = crate::mystring::cstr_prefix(&candidate.path);

            if fullname
                .try_to_path_buf()
                .is_ok_and(|path| nsh_platform::path_is_directory(&path))
            {
                if !component.is_empty() {
                    flags |= CD_PRINT;
                }
                /* docd: */
                match docd(sh, fullname, flags)? {
                    CdResult::Changed => {
                        out = true; /* goto out */
                        break;
                    }
                    CdResult::ChangedPwdUnknown => {
                        out = true;
                        pwd_unknown = true;
                        break;
                    }
                    CdResult::Failed => {}
                }
                /* goto err */
                let mut message = b"can't cd to ".to_vec();
                message.extend_from_slice(dest);
                return Err(sh.sh_error_value(&message));
            }
        }
    }

    if !out {
        /* step6: */
        /* docd: */
        match docd(sh, dest, flags)? {
            CdResult::Changed => {}
            CdResult::ChangedPwdUnknown => pwd_unknown = true,
            CdResult::Failed => {
                /* err: */
                let mut message = b"can't cd to ".to_vec();
                message.extend_from_slice(dest);
                return Err(sh.sh_error_value(&message));
            }
        }
    }

    /* out: */
    if (flags & CD_PRINT) != 0 {
        let mut d = cbytes(&sh.cwd.curdir);
        d.pop();
        d.push(b'\n');
        let _ = sh.io.stdout().write_all(&d);
    }
    let status =
        i32::from(pwd_unknown && (flags & CD_PHYSICAL) != 0 && (flags & CD_ERROR_IF_UNKNOWN) != 0);
    Ok(Flow::Done(status))
}

// [spec:dash:def:cd.docd-fn]
// [spec:dash:sem:cd.docd-fn]
// [spec:posix:sem:builtin.cd.step7-prefix-pwd]
// [spec:posix:req:builtin.cd.step10-chdir]
// [spec:posix:req:builtin.cd.step10-pwd-physical]
// [spec:posix:req:builtin.cd.oldpwd-set]
// [spec:posix:req:xcurel.change-cwd]
// [spec:nsh:req:idiom.platform-errors]
fn docd(sh: &mut Shell, dest: &BStr, flags: c_int) -> Result<CdResult, Error> {
    let mut logical = None;
    let err: c_int;

    /* `TRACE(("docd(sh, \"%s\", %d) called\n", dest, flags));` — `#ifdef DEBUG`
     * in `shell.h`, and the dash build does not define it. */

    INTOFF(sh);
    if (flags & CD_PHYSICAL) == 0 {
        logical = updatepwd(sh, dest);
    }
    /* `chdir(2)` either way -- std saves the `CString` and makes the same
     * call, and the result is folded back to the C's 0/-1 because `docd`
     * is a `chdir` return code to every one of its callers. */
    let target = logical
        .as_ref()
        .map(|dir| dir.as_slice().as_bstr())
        .unwrap_or(dest);
    err = match target
        .try_to_path_buf()
        .and_then(|path| nsh_platform::set_current_directory(&path))
    {
        Ok(()) => 0,
        // [spec:posix:req:builtin.cd.step9-path-max-relative]
        // `updatepwd` constructed this absolute path by prefixing PWD. If the
        // kernel rejects only that combined spelling as too long, POSIX says
        // the original short relative operand must still be used when that
        // conversion is possible.
        Err(error)
            if logical.is_some()
                && !nsh_platform::shell_path_is_absolute(dest)
                && nsh_platform::is_path_error(
                    &error,
                    nsh_platform::PathErrorKind::NameTooLong,
                ) =>
        {
            match dest
                .try_to_path_buf()
                .and_then(|path| nsh_platform::set_current_directory(&path))
            {
                Ok(()) => 0,
                Err(_) => -1,
            }
        }
        Err(_) => -1,
    };
    if err == 0 {
        /* The `?` returns between the INTOFF above and the INTON below,
         * leaking the interrupt counter exactly as the longjmp out of
         * `sh_error` did; see docs/errors-are-values.md 2.4. */
        match logical.as_ref() {
            Some(dir) => setpwd_inner(sh, Pwd::New(dir.as_slice().as_bstr()), 1)?,
            None => setpwd_inner(sh, Pwd::Unknown, 1)?,
        }
        crate::exec::hashcd(sh);
    }
    /* out: */
    INTON(sh);
    Ok(if err != 0 {
        CdResult::Failed
    } else if logical.is_none() && sh.cwd.curdir.is_none() {
        CdResult::ChangedPwdUnknown
    } else {
        CdResult::Changed
    })
}

// [spec:dash:def:cd.updatepwd-fn]
// [spec:dash:sem:cd.updatepwd-fn]
// [spec:posix:req:builtin.cd.step8-canonical-form-dot]
// [spec:posix:req:builtin.cd.step8-further-simplification]
// [spec:posix:req:builtin.cd.env-pwd]
fn updatepwd(sh: &mut Shell, dir: &BStr) -> Option<BString> {
    let current = sh
        .cwd
        .curdir
        .as_ref()
        .and_then(|path| path.try_to_path_buf().ok());
    let directory = dir.try_to_path_buf().ok()?;
    nsh_platform::logical_path(current.as_deref(), &directory)
        .map(|path| BString::from(path.to_shell_bytes()))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    struct RestoreCwd(PathBuf);

    impl Drop for RestoreCwd {
        fn drop(&mut self) {
            std::env::set_current_dir(&self.0).unwrap();
        }
    }

    /// `-L` and `-P` are a toggle rather than two flags: the C tracks
    /// which it saw last and flips only when the next one differs, so a
    /// repeat is not a flip and the pair cancels.
    fn opts(words: &[&[u8]]) -> c_int {
        let args: Vec<&BStr> = words.iter().map(|w| BStr::new(*w)).collect();
        let mut scan = Options::new(&args);
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        cdopt(&mut owned, &mut scan).unwrap()
    }

    #[test]
    fn no_option_is_logical() {
        assert_eq!(opts(&[b"cd"]), 0);
        assert_eq!(opts(&[b"cd", b"/tmp"]), 0);
    }

    #[test]
    fn physical_and_logical_toggle() {
        assert_eq!(opts(&[b"cd", b"-P"]), CD_PHYSICAL);
        assert_eq!(opts(&[b"cd", b"-L"]), 0);
        assert_eq!(opts(&[b"cd", b"-P", b"-L"]), 0);
        assert_eq!(opts(&[b"cd", b"-L", b"-P"]), CD_PHYSICAL);
    }

    // [spec:posix:req:builtin.cd.opt-e/test]
    #[test]
    fn error_if_pwd_unknown_is_independent() {
        assert_eq!(opts(&[b"cd", b"-e"]), CD_ERROR_IF_UNKNOWN);
        assert_eq!(opts(&[b"cd", b"-Pe"]), CD_PHYSICAL | CD_ERROR_IF_UNKNOWN);
        assert_eq!(opts(&[b"cd", b"-eP", b"-L"]), CD_ERROR_IF_UNKNOWN);
    }

    // [spec:posix:req:builtin.cd.opt-e/test]
    #[test]
    fn physical_e_reports_unknown_pwd() {
        if !nsh_platform::can_unlink_current_directory() {
            return;
        }
        let _guard = crate::testutil::lock();
        let old = std::env::current_dir().unwrap();
        let path = std::env::temp_dir().join(format!(
            "nsh-cd-e-{}-{}",
            std::process::id(),
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap()
                .as_nanos()
        ));
        std::fs::create_dir(&path).unwrap();
        std::env::set_current_dir(&path).unwrap();
        let _restore = RestoreCwd(old);
        std::fs::remove_dir(&path).unwrap();

        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        assert_eq!(
            cdcmd(
                &mut shell,
                &[BStr::new(b"cd"), BStr::new(b"-e"), BStr::new(b".")]
            )
            .unwrap(),
            Flow::Done(0)
        );
        assert_eq!(
            cdcmd(
                &mut shell,
                &[BStr::new(b"cd"), BStr::new(b"-Pe"), BStr::new(b".")]
            )
            .unwrap(),
            Flow::Done(1)
        );
    }

    /// A repeat is not a flip, whether clustered or spread.
    #[test]
    fn a_repeat_is_not_a_flip() {
        assert_eq!(opts(&[b"cd", b"-PP"]), CD_PHYSICAL);
        assert_eq!(opts(&[b"cd", b"-P", b"-P"]), CD_PHYSICAL);
        assert_eq!(opts(&[b"cd", b"-LL"]), 0);
    }

    #[test]
    fn the_scan_stops_at_the_operand() {
        let args = [BStr::new("cd"), BStr::new("-P"), BStr::new("dir")];
        let mut scan = Options::new(&args);
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        assert_eq!(cdopt(&mut owned, &mut scan).unwrap(), CD_PHYSICAL);
        assert_eq!(scan.operands(), [BStr::new("dir")]);
    }

    // [spec:posix:req:builtin.cd.operand-empty-string/test]
    #[test]
    fn empty_operand_is_an_error() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let error = cdcmd(&mut shell, &[BStr::new(b"cd"), BStr::new(b"")])
            .expect_err("an explicit empty operand must not mean the current directory");
        assert_eq!(
            error.message(),
            BStr::new(b"can't cd to an empty directory")
        );
    }
}
