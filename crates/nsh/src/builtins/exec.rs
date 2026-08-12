//! `exec`.
//!
//! Port of `execcmd` from `src/eval.c`. With an operand it replaces the
//! shell's process image and never returns; with none it is the
//! redirection-only form, and the redirections have already been made
//! permanent by the time it runs.
//!
//! This is the builtin `[dec:nsh:public-surface]` singles out: an
//! embedded shell cannot survive it, so the API gates it behind a `Host`
//! method a frontend grants and an ordinary embedder refuses.

use crate::error::Error;
use core::ptr::null_mut;
use std::ffi::CString;

use bstr::BStr;
use libc::{c_char, c_int};

use crate::exec::shellexec;

// [spec:dash:def:eval.execcmd-fn]
// [spec:dash:sem:eval.execcmd-fn]
pub unsafe fn execcmd(args: &[&BStr]) -> Result<c_int, Error> {
    if args.len() > 1 {
        crate::options::optlist[crate::options::iflag] = 0; /* exit on error */
        crate::options::optlist[crate::options::mflag] = 0;
        crate::options::optschanged();
        crate::input::flush_input();
        /* `execve` wants the array back, so this is where it is built --
         * once, for the one builtin that replaces the process, instead of
         * for every builtin that does not. `shellexec` writes `argv[-1]`
         * when it retries a script through the shell, so the spare slot
         * `evalcommand` reserves is reserved here too.
         *
         * `shellexec` does not return: it either replaces the image or
         * raises, so neither the words nor the array outlive their use. */
        let words: Vec<CString> = args[1..].iter().map(|a| crate::shell::cstring(a)).collect();
        let mut argv: Vec<*mut c_char> = Vec::with_capacity(words.len() + 2);
        argv.push(null_mut());
        argv.extend(words.iter().map(|w| w.as_ptr() as *mut c_char));
        argv.push(null_mut());
        shellexec(argv.as_mut_ptr().add(1), crate::var::pathval(), 0);
    }
    Ok(0)
}
