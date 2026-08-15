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
use core::ptr::{addr_of, addr_of_mut, null_mut};
use libc::{c_char, c_int};
use std::ffi::{CStr, OsStr};
use std::io::Write;
use std::os::unix::ffi::OsStrExt;

use crate::cd::{cbytes, setpwd};
use crate::error::{INTOFF, INTON};
use crate::eval::Flow;
use crate::mystring::{dotdir, homestr, nullstr};
use crate::options::Options;
use crate::var::bltinlookup;

const CD_PHYSICAL: c_int = 1;
const CD_PRINT: c_int = 2;

// [spec:dash:def:cd.cdopt-fn]
// [spec:dash:sem:cd.cdopt-fn]
pub(crate) unsafe fn cdopt(opts: &mut Options) -> Result<c_int, Error> {
    let mut flags: c_int = 0;
    let mut j: u8 = b'L';

    while let Some(i) = opts.next(b"LP")? {
        if i != j {
            flags ^= CD_PHYSICAL;
            j = i;
        }
    }

    Ok(flags)
}

// [spec:dash:def:cd.cdcmd-fn]
// [spec:dash:sem:cd.cdcmd-fn]
pub unsafe fn cdcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut dest: *const c_char;
    let mut path: *const c_char;
    let mut p: *const c_char;
    let mut c: c_char;
    let mut statb: libc::stat64 = core::mem::zeroed();
    let mut flags: c_int;
    let mut len: c_int;

    let mut opts = Options::new(args);
    flags = cdopt(&mut opts)?;
    /* The operand outlives every reader below, which is what the C got
     * from `argv` living in `evalcommand`'s frame. */
    let operand = opts.operands().first().map(|d| crate::shell::cstring(d));
    match &operand {
        None => dest = bltinlookup(sh, addr_of!(homestr) as *const c_char),
        Some(d) if d.as_bytes() == b"-" => {
            dest = bltinlookup(sh, b"OLDPWD\0".as_ptr() as *const c_char);
            flags |= CD_PRINT;
        }
        Some(d) => dest = d.as_ptr(),
    }
    if dest.is_null() {
        dest = addr_of!(nullstr) as *const c_char;
    }

    let mut step6 = false;
    if *dest == b'/' as c_char {
        step6 = true; /* goto step6 */
    } else if *dest == b'.' as c_char {
        c = *dest.offset(1);
        loop {
            /* dotdot: */
            if c == b'\0' as c_char || c == b'/' as c_char {
                step6 = true; /* goto step6 */
                break;
            }
            if c == b'.' as c_char {
                c = *dest.offset(2);
                if c != b'.' as c_char {
                    continue; /* goto dotdot */
                }
            }
            break;
        }
    }

    let mut out = false;
    /* The CDPATH candidate `docd` is handed, copied out of `padvance`'s
     * buffer.  Held across the whole loop rather than per iteration
     * because `p` still points into it after the `break`. */
    let mut keptbuf: Vec<u8> = Vec::new();
    if !step6 {
        if *dest == 0 {
            dest = addr_of!(dotdir) as *const c_char;
        }
        path = bltinlookup(sh, b"CDPATH\0".as_ptr() as *const c_char);
        loop {
            p = path;
            len = crate::exec::padvance_magic(&mut path, dest, 0);
            if len < 0 {
                break;
            }
            c = *p;
            /* `stalloc(len)` took the candidate the C had built in the
             * stack block; the copy is what takes it out of `padvance`'s
             * buffer, which the `docd` below can overwrite.  `len` is
             * `padvance`'s *allocation* size, one more than the string's
             * length when the PATH component is empty, so the buffer is
             * sized from it and the bytes are copied by hand. */
            let candidate = CStr::from_ptr(crate::exec::padvance_result()).to_bytes_with_nul();
            debug_assert!(candidate.len() <= len as usize);
            keptbuf.clear();
            keptbuf.resize(len as usize, 0);
            keptbuf[..candidate.len()].copy_from_slice(candidate);
            p = keptbuf.as_ptr() as *const c_char;

            if libc::stat64(p, &mut statb) >= 0 && (statb.st_mode & libc::S_IFMT) == libc::S_IFDIR {
                if c != 0 && c != b':' as c_char {
                    flags |= CD_PRINT;
                }
                /* docd: */
                if docd(sh, p, flags)? == 0 {
                    out = true; /* goto out */
                    break;
                }
                /* goto err */
                let mut message = b"can't cd to ".to_vec();
                message.extend_from_slice(CStr::from_ptr(dest).to_bytes());
                return Err(crate::error::sh_error_value(&message));
            }
        }
    }

    if !out {
        /* step6: */
        p = dest;
        /* docd: */
        if docd(sh, p, flags)? != 0 {
            /* err: */
            let mut message = b"can't cd to ".to_vec();
            message.extend_from_slice(CStr::from_ptr(dest).to_bytes());
            return Err(crate::error::sh_error_value(&message));
        }
    }

    /* out: */
    if (flags & CD_PRINT) != 0 {
        let mut d = cbytes(&*addr_of!(sh.cwd.curdir));
        d.pop();
        d.push(b'\n');
        let _ = (*crate::output::stdout()).write_all(&d);
    }
    Ok(Flow::Done(0))
}

// [spec:dash:def:cd.docd-fn]
// [spec:dash:sem:cd.docd-fn]
unsafe fn docd(sh: &mut Shell, mut dest: *const c_char, flags: c_int) -> Result<c_int, Error> {
    let mut dir: *const c_char = null_mut();
    let err: c_int;

    /* `TRACE(("docd(sh, \"%s\", %d) called\n", dest, flags));` — `#ifdef DEBUG`
     * in `shell.h`, and the dash build does not define it. */

    INTOFF();
    if (flags & CD_PHYSICAL) == 0 {
        dir = updatepwd(sh, dest);
        if !dir.is_null() {
            dest = dir;
        }
    }
    /* `chdir(2)` either way -- std saves the `CString` and makes the same
     * call, and the result is folded back to the C's 0/-1 because `docd`
     * is a `chdir` return code to every one of its callers. */
    err = match std::env::set_current_dir(std::path::Path::new(OsStr::from_bytes(
        CStr::from_ptr(dest).to_bytes(),
    ))) {
        Ok(()) => 0,
        Err(_) => -1,
    };
    if err == 0 {
        /* The `?` returns between the INTOFF above and the INTON below,
         * leaking the interrupt counter exactly as the longjmp out of
         * `sh_error` did; see docs/errors-are-values.md 2.4. */
        setpwd(sh, dir, 1)?;
        crate::exec::hashcd(sh);
    }
    /* out: */
    INTON();
    Ok(err)
}

/// [`updatepwd`]'s result, which the C left in the stack block for its one
/// caller to read before the next `cd`.
static mut pwdbuf: BString = BString::new(Vec::new());

// [spec:dash:def:cd.updatepwd-fn]
// [spec:dash:sem:cd.updatepwd-fn]
unsafe fn updatepwd(sh: &mut Shell, dir: *const c_char) -> *const c_char {
    /* `lim` is `stackblock() + 1` in the C, re-read after `makestrspace`
     * because the block can move; against an owned buffer it is just an
     * index, and `new > lim` is a comparison of lengths. */
    let mut lim: usize;

    /* #ifdef __CYGWIN__ — not selected. */

    /* `sstrdup(dir)`.  The copy outlives the whole walk because the
     * components below borrow it while `new` grows. */
    let cdcompbuf: Vec<u8> = CStr::from_ptr(dir).to_bytes().to_vec();
    let new = &mut *addr_of_mut!(pwdbuf);
    new.clear();
    if *dir != b'/' as c_char {
        let Some(cur) = &*addr_of!(sh.cwd.curdir) else {
            return null_mut();
        };
        new.extend_from_slice(cur);
    }
    new.reserve(cdcompbuf.len() + 2);
    lim = 1;
    if *dir != b'/' as c_char {
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
        if *dir.offset(1) == b'/' as c_char && *dir.offset(2) != b'/' as c_char {
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
    /* `*new = '\0'` — the C writes the terminator at the cursor without
     * advancing it, and the caller reads the block as a C string. */
    new.push(0);
    new.as_ptr() as *const c_char
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
        unsafe { cdopt(&mut scan) }.unwrap()
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
        assert_eq!(unsafe { cdopt(&mut scan) }.unwrap(), CD_PHYSICAL);
        assert_eq!(scan.operands(), [BStr::new("dir")]);
    }
}
