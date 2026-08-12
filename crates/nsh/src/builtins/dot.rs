//! `.`, the dot builtin.
//!
//! Port of `dotcmd` and `find_dot_file` from `src/main.c`.
//!
//! It re-enters evaluation by pushing the named file onto the input
//! stack and running the command loop over it, so like `eval` it depends
//! on its words not borrowing from the shell.

use crate::error::Error;
use bstr::{BStr, BString};
use core::ptr::addr_of;
use libc::{c_char, c_int};
use std::ffi::CStr;

use crate::shellmain::cmdloop;

// [spec:dash:def:main.find-dot-file-fn]
// [spec:dash:sem:main.find-dot-file-fn]
/// The C returns a `stalloc`'d copy of the candidate — "This will be
/// freed by the caller", meaning `dotcmd`'s enclosing `popstackmark`.
/// The caller owns the buffer and this fills it, so the copy lasts
/// exactly as long as the frame that asked for it.
unsafe fn find_dot_file(basename: *mut c_char, out: &mut Vec<u8>) -> *mut c_char {
    let mut fullname: *mut c_char;
    let mut path: *const c_char = crate::var::pathval();
    let mut statb: libc::stat64 = core::mem::zeroed();
    let mut len: c_int;

    /* don't try this for absolute or relative paths */
    if CStr::from_ptr(basename).to_bytes().contains(&b'/') {
        return basename;
    }

    loop {
        len = crate::exec::padvance(&mut path, basename);
        if len < 0 {
            break;
        }
        fullname = crate::exec::padvance_result();
        if (crate::exec::pathopt.is_null() || *crate::exec::pathopt == b'f' as c_char)
            && libc::stat64(fullname, &mut statb) == 0
            && (statb.st_mode & libc::S_IFMT) == libc::S_IFREG
        {
            /* This will be freed by the caller. */
            /* `len` is `padvance`'s *allocation* size, one more than the
             * string's length when the PATH component is empty, so the
             * buffer is sized from it and the bytes copied by hand. */
            let candidate = CStr::from_ptr(fullname).to_bytes_with_nul();
            debug_assert!(len > 0);
            debug_assert!(candidate.len() <= len as usize);
            out.clear();
            out.resize(len as usize, 0);
            out[..candidate.len()].copy_from_slice(candidate);
            return out.as_mut_ptr() as *mut c_char;
        }
    }

    /* not found in the PATH */
    let mut message = Vec::new();
    message.extend_from_slice(CStr::from_ptr(basename).to_bytes());
    message.extend_from_slice(b": not found");
    crate::error::sh_error(&message);
    /* NOTREACHED */
}

// [spec:dash:def:main.dotcmd-fn]
// [spec:dash:sem:main.dotcmd-fn]
pub unsafe fn dotcmd(args: &[&BStr]) -> Result<c_int, Error> {
    let mut status: c_int = 0;

    let mut opts = crate::options::Options::new(args);
    opts.next(b"");

    if let Some(name) = opts.operands().first() {
        let mut dotfile: Vec<u8> = Vec::new();
        let name = crate::shell::cstring(name);
        let fullname = find_dot_file(name.as_ptr() as *mut c_char, &mut dotfile);

        crate::input::setinputfile(fullname, crate::input::INPUT_PUSH_FILE);
        /* `evalbltin`'s epilogue reads `commandname` after this returns —
         * `flushall(); if (outerr(out1)) sh_warnx("%s: I/O error",
         * commandname);` — and the C is safe there only because the block
         * is `stalloc`'d and the enclosing mark has not popped yet.
         *
         * Now that `commandname` owns its bytes there is nothing to keep
         * alive: what the epilogue reads is a copy, so the buffer this
         * frame allocated can be freed with the frame like any other
         * local, and the static slot that used to hold it is gone. */
        crate::eval::commandname = Some(BString::from(CStr::from_ptr(fullname).to_bytes()));
        status = cmdloop(0);
        crate::input::popfile();
    }

    Ok(status)
}
