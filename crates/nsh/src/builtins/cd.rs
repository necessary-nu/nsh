//! `cd` and `chdir`.
//!
//! Port of `cdcmd` and its helpers from `src/cd.c`.
//!
//! What stays in `crate::working_directory` is the shell's idea of where it is --
//! `curdir`, `physdir` and the `setpwd` that maintains them. This module
//! is the command that moves it: the CDPATH search, the `-L`/`-P` option
//! scan, and the logical-path bookkeeping that `cd ..` needs and `chdir`
//! alone cannot do.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString, ByteSlice};
use nsh_platform::NativeStrExt as _;
use nsh_platform::ShellBytesExt as _;

use crate::evaluation::Flow;
use crate::options::Options;
use crate::output::OutputDestination;
use crate::working_directory::{DirectoryUpdate, update_current_directory};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct CdOptions {
    pub(crate) physical: bool,
    print: bool,
    error_if_unknown: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CdResult {
    Changed,
    ChangedPwdUnknown,
    Failed,
}

// [spec:dash:sem:cd.cdopt-fn]
// [spec:posix:syn:builtin.cd.syn]
// [spec:posix:req:builtin.cd.utility-syntax-guidelines]
// [spec:posix:req:builtin.cd.opt-l]
// [spec:posix:req:builtin.cd.opt-p]
// [spec:posix:req:builtin.cd.opt-l-p-last-wins]
// [spec:posix:req:builtin.cd.opt-e]
pub(crate) fn parse_cd_options(
    shell: &mut crate::context::Shell,
    option_scan: &mut Options,
) -> Result<CdOptions, Error> {
    let mut options = CdOptions::default();

    while let Some(option) = option_scan.next(&mut shell.diagnostics(), b"LPe")? {
        match option {
            b'e' => options.error_if_unknown = true,
            b'P' => options.physical = true,
            b'L' => options.physical = false,
            _ => unreachable!("the option scanner returns only accepted letters"),
        }
    }

    Ok(options)
}

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
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut option_scan = Options::new(args);
    let mut options = parse_cd_options(shell, &mut option_scan)?;
    /* The operand outlives every reader below, which is what the C got
     * from `argv` living in `evalcommand`'s frame. */
    let operand = option_scan.operands().first().copied();
    // [spec:posix:req:builtin.cd.operand-empty-string]
    if operand.is_some_and(|directory| directory.is_empty()) {
        return Err(shell
            .diagnostics()
            .shell_error(b"can't cd to an empty directory"));
    }
    let dest_value = match operand {
        None => crate::variables::lookup_bytes(shell, BStr::new(b"HOME")).unwrap_or_default(),
        Some(d) if d == b"-" => {
            options.print = true;
            crate::variables::lookup_bytes(shell, BStr::new(b"OLDPWD")).unwrap_or_default()
        }
        Some(d) => d.to_owned(),
    };
    let mut dest = dest_value.as_slice().as_bstr();
    let mut working_directory_unknown = false;

    let step6 = nsh_platform::shell_path_is_absolute(dest)
        || dest == b"."
        || dest.starts_with(b"./")
        || dest == b".."
        || dest.starts_with(b"../");

    let mut changed_via_search_path = false;
    if !step6 {
        if dest.is_empty() {
            dest = BStr::new(b".");
        }
        let path_value =
            crate::variables::lookup_bytes(shell, BStr::new(b"CDPATH")).unwrap_or_default();
        let mut components =
            path_value.split(|byte| *byte == nsh_platform::search_path_separator());
        let mut path = crate::execution::PathCursor::literal(path_value.as_slice().as_bstr());
        while let Some(candidate) = path.advance(dest) {
            let component = components
                .next()
                .expect("PATH cursor and components advance together");
            let full_path = candidate.path.as_bstr();

            if full_path
                .try_to_path_buf()
                .is_ok_and(|path| nsh_platform::path_is_directory(&path))
            {
                if !component.is_empty() {
                    options.print = true;
                }
                /* docd: */
                match change_directory(shell, full_path, options)? {
                    CdResult::Changed => {
                        changed_via_search_path = true;
                        break;
                    }
                    CdResult::ChangedPwdUnknown => {
                        changed_via_search_path = true;
                        working_directory_unknown = true;
                        break;
                    }
                    CdResult::Failed => {}
                }
                /* goto err */
                let mut message = b"can't cd to ".to_vec();
                message.extend_from_slice(dest);
                return Err(shell.diagnostics().shell_error(&message));
            }
        }
    }

    if !changed_via_search_path {
        /* step6: */
        /* docd: */
        match change_directory(shell, dest, options)? {
            CdResult::Changed => {}
            CdResult::ChangedPwdUnknown => working_directory_unknown = true,
            CdResult::Failed => {
                /* err: */
                let mut message = b"can't cd to ".to_vec();
                message.extend_from_slice(dest);
                return Err(shell.diagnostics().shell_error(&message));
            }
        }
    }

    /* out: */
    if options.print {
        let mut directory = shell.working_directory.logical.clone().unwrap_or_default();
        directory.push(b'\n');
        shell.write_output(OutputDestination::Stdout, &directory)?;
    }
    let status =
        i32::from(working_directory_unknown && options.physical && options.error_if_unknown);
    Ok(Flow::Done((status).into()))
}

// [spec:dash:sem:cd.docd-fn]
// [spec:posix:sem:builtin.cd.step7-prefix-pwd]
// [spec:posix:req:builtin.cd.step10-chdir]
// [spec:posix:req:builtin.cd.step10-pwd-physical]
// [spec:posix:req:builtin.cd.oldpwd-set]
// [spec:posix:req:xcurel.change-cwd]
// [spec:nsh:req:idiom.platform-errors]
fn change_directory(shell: &mut Shell, dest: &BStr, options: CdOptions) -> Result<CdResult, Error> {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let logical = if !options.physical {
            update_logical_directory(shell, dest)
        } else {
            None
        };
        /* Both logical and physical resolution cross the same typed platform
         * boundary. The result is still folded into the translated return
         * code until the ABI-scalar cleanup types this function. */
        let target = logical
            .as_ref()
            .map(|dir| dir.as_slice().as_bstr())
            .unwrap_or(dest);
        let changed = match target
            .try_to_path_buf()
            .and_then(|path| nsh_platform::set_current_directory(&path))
        {
            Ok(()) => true,
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
                    Ok(()) => true,
                    Err(_) => false,
                }
            }
            Err(_) => false,
        };
        if changed {
            match logical.as_ref() {
                Some(dir) => update_current_directory(
                    shell,
                    DirectoryUpdate::New(dir.as_slice().as_bstr()),
                    true,
                )?,
                None => update_current_directory(shell, DirectoryUpdate::Unknown, true)?,
            }
            crate::execution::invalidate_cache_after_directory_change(shell);
        }
        Ok(if !changed {
            CdResult::Failed
        } else if logical.is_none() && shell.working_directory.logical.is_none() {
            CdResult::ChangedPwdUnknown
        } else {
            CdResult::Changed
        })
    })
}

// [spec:dash:sem:cd.updatepwd-fn]
// [spec:posix:req:builtin.cd.step8-canonical-form-dot]
// [spec:posix:req:builtin.cd.step8-further-simplification]
// [spec:posix:req:builtin.cd.env-pwd]
fn update_logical_directory(shell: &mut Shell, dir: &BStr) -> Option<BString> {
    let current = shell
        .working_directory
        .logical
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
    fn option_scan(words: &[&[u8]]) -> CdOptions {
        let args: Vec<&BStr> = words.iter().map(|w| BStr::new(*w)).collect();
        let mut scan = Options::new(&args);
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        parse_cd_options(&mut owned, &mut scan).unwrap()
    }

    #[test]
    fn no_option_is_logical() {
        assert_eq!(option_scan(&[b"cd"]), CdOptions::default());
        assert_eq!(option_scan(&[b"cd", b"/tmp"]), CdOptions::default());
    }

    #[test]
    fn physical_and_logical_toggle() {
        assert!(option_scan(&[b"cd", b"-P"]).physical);
        assert!(!option_scan(&[b"cd", b"-L"]).physical);
        assert!(!option_scan(&[b"cd", b"-P", b"-L"]).physical);
        assert!(option_scan(&[b"cd", b"-L", b"-P"]).physical);
    }

    // [spec:posix:req:builtin.cd.opt-e/test]
    #[test]
    fn error_if_pwd_unknown_is_independent() {
        assert!(option_scan(&[b"cd", b"-e"]).error_if_unknown);
        let physical_error = option_scan(&[b"cd", b"-Pe"]);
        assert!(physical_error.physical && physical_error.error_if_unknown);
        let logical_error = option_scan(&[b"cd", b"-eP", b"-L"]);
        assert!(!logical_error.physical && logical_error.error_if_unknown);
    }

    // [spec:posix:req:builtin.cd.opt-e/test]
    #[test]
    fn physical_e_reports_unknown_pwd() {
        if !nsh_platform::can_unlink_current_directory() {
            return;
        }
        let _guard = crate::test_support::lock();
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
            run(
                &mut shell,
                &[BStr::new(b"cd"), BStr::new(b"-e"), BStr::new(b".")]
            )
            .unwrap(),
            Flow::Done((0).into())
        );
        assert_eq!(
            run(
                &mut shell,
                &[BStr::new(b"cd"), BStr::new(b"-Pe"), BStr::new(b".")]
            )
            .unwrap(),
            Flow::Done((1).into())
        );
    }

    /// A repeat is not a flip, whether clustered or spread.
    #[test]
    fn a_repeat_is_not_a_flip() {
        assert!(option_scan(&[b"cd", b"-PP"]).physical);
        assert!(option_scan(&[b"cd", b"-P", b"-P"]).physical);
        assert!(!option_scan(&[b"cd", b"-LL"]).physical);
    }

    #[test]
    fn the_scan_stops_at_the_operand() {
        let args = [BStr::new("cd"), BStr::new("-P"), BStr::new("dir")];
        let mut scan = Options::new(&args);
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        assert!(parse_cd_options(&mut owned, &mut scan).unwrap().physical);
        assert_eq!(scan.operands(), [BStr::new("dir")]);
    }

    // [spec:posix:req:builtin.cd.operand-empty-string/test]
    #[test]
    fn empty_operand_is_an_error() {
        let mut shell = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let error = run(&mut shell, &[BStr::new(b"cd"), BStr::new(b"")])
            .expect_err("an explicit empty operand must not mean the current directory");
        assert_eq!(
            error.message(),
            BStr::new(b"can't cd to an empty directory")
        );
    }
}
