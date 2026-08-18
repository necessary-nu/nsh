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
use std::ffi::OsStr;
use std::io::Write;
use std::os::unix::ffi::OsStrExt;

use crate::cd::{Pwd, cbytes, setpwd_inner};
use crate::error::{INTOFF, INTON};
use crate::eval::Flow;
use crate::options::Options;

const CD_PHYSICAL: c_int = 1;
const CD_PRINT: c_int = 2;

// [spec:dash:def:cd.cdopt-fn]
// [spec:dash:sem:cd.cdopt-fn]
// [spec:posix:syn:builtin.cd.syn]
// [spec:posix:req:builtin.cd.utility-syntax-guidelines]
// [spec:posix:req:builtin.cd.opt-l]
// [spec:posix:req:builtin.cd.opt-p]
// [spec:posix:req:builtin.cd.opt-l-p-last-wins]
pub(crate) fn cdopt(sh: &mut crate::context::Shell, opts: &mut Options) -> Result<c_int, Error> {
    let mut flags: c_int = 0;
    let mut j: u8 = b'L';

    while let Some(i) = opts.next(sh, b"LP")? {
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
    let dest_value = match operand {
        None => crate::var::lookup_bytes(sh, BStr::new(b"HOME")).unwrap_or_default(),
        Some(d) if d == b"-" => {
            flags |= CD_PRINT;
            crate::var::lookup_bytes(sh, BStr::new(b"OLDPWD")).unwrap_or_default()
        }
        Some(d) => d.to_owned(),
    };
    let mut dest = dest_value.as_slice().as_bstr();

    let step6 = dest.starts_with(b"/")
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
        let mut components = path_value.split(|byte| *byte == b':');
        let mut path = crate::exec::PathCursor::literal(path_value.as_slice().as_bstr());
        while let Some(candidate) = crate::exec::padvance(&mut path, dest) {
            let component = components.next().expect("PATH cursor and components advance together");
            let fullname = crate::mystring::cstr_prefix(&candidate.path);

            if std::fs::metadata(OsStr::from_bytes(fullname))
                .is_ok_and(|metadata| metadata.is_dir())
            {
                if !component.is_empty() {
                    flags |= CD_PRINT;
                }
                /* docd: */
                if docd(sh, fullname, flags)? == 0 {
                    out = true; /* goto out */
                    break;
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
        if docd(sh, dest, flags)? != 0 {
            /* err: */
            let mut message = b"can't cd to ".to_vec();
            message.extend_from_slice(dest);
            return Err(sh.sh_error_value(&message));
        }
    }

    /* out: */
    if (flags & CD_PRINT) != 0 {
        let mut d = cbytes(&sh.cwd.curdir);
        d.pop();
        d.push(b'\n');
        let _ = sh.io.stdout().write_all(&d);
    }
    Ok(Flow::Done(0))
}

// [spec:dash:def:cd.docd-fn]
// [spec:dash:sem:cd.docd-fn]
// [spec:posix:sem:builtin.cd.step7-prefix-pwd]
// [spec:posix:req:builtin.cd.step10-chdir]
// [spec:posix:req:builtin.cd.step10-pwd-physical]
// [spec:posix:req:builtin.cd.oldpwd-set]
fn docd(sh: &mut Shell, dest: &BStr, flags: c_int) -> Result<c_int, Error> {
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
    err = match std::env::set_current_dir(std::path::Path::new(OsStr::from_bytes(target))) {
        Ok(()) => 0,
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
    Ok(err)
}

// [spec:dash:def:cd.updatepwd-fn]
// [spec:dash:sem:cd.updatepwd-fn]
// [spec:posix:req:builtin.cd.step8-canonical-form-dot]
// [spec:posix:req:builtin.cd.step8-further-simplification]
// [spec:posix:req:builtin.cd.env-pwd]
fn updatepwd(sh: &mut Shell, dir: &BStr) -> Option<BString> {
    /* `lim` is `stackblock() + 1` in the C, re-read after `makestrspace`
     * because the block can move; against an owned buffer it is just an
     * index, and `new > lim` is a comparison of lengths. */
    let mut lim: usize;

    /* #ifdef __CYGWIN__ — not selected. */

    /* `sstrdup(dir)`.  The copy outlives the whole walk because the
     * components below borrow it while `new` grows. */
    let cdcompbuf = dir.to_vec();
    let mut new = BString::new(Vec::new());
    if !dir.starts_with(b"/") {
        let Some(cur) = &sh.cwd.curdir else {
            return None;
        };
        new.extend_from_slice(cur);
    }
    new.reserve(cdcompbuf.len() + 2);
    lim = 1;
    if !dir.starts_with(b"/") {
        /* `*(new - 1)` reads before the stack block when `curdir` is empty.
         * It cannot be — `curdir` is either `nullstr`, which returned above,
         * or a path `updatepwd` itself produced — so this only differs from
         * the C on a path the C reads out of bounds on. */
        if new.last() != Some(&b'/') {
            new.push(b'/');
        }
        if new.len() > lim && new[lim] == b'/' {
            lim += 1;
        }
    } else {
        new.push(b'/');
        if dir.get(1) == Some(&b'/') && dir.get(2) != Some(&b'/') {
            new.push(b'/');
            lim += 1;
        }
    }
    /* `strtok(cdcomppath, "/")` walked from just past the leading slashes the
     * arm above consumed; an empty field is exactly what `strtok` never
     * yields, so skipping them here would change nothing. */
    for p in cdcompbuf.split_str(b"/") {
        if p.is_empty() {
            continue;
        }
        if p == b".." {
            while new.len() > lim {
                new.pop();
                if new[new.len() - 1] == b'/' {
                    break;
                }
            }
        } else if p == b"." {
            /* nothing */
        } else {
            /* fall through / default: */
            new.extend_from_slice(p);
            new.push(b'/');
        }
    }
    if new.len() > lim {
        new.pop();
    }
    Some(new)
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
