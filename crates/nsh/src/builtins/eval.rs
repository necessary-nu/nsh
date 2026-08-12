//! `eval`.
//!
//! Port of `evalcmd` from `src/eval.c`. It is the builtin with the
//! special entry point -- the table's row carries no function pointer,
//! because this one also needs the evaluation flags its caller was given.
//!
//! It re-enters evaluation, which is the constraint that decides the
//! builtin signature: the words it joins must not borrow from the shell,
//! or handing the shell back would not compile. See
//! `[dec:nsh:public-surface]`.

use crate::error::Error;
use bstr::{BStr, BString};
use libc::{c_char, c_int};
use std::ffi::CString;

use crate::eval::{EV_TESTED, evalstring};

// [spec:dash:def:eval.evalcmd-fn]
// [spec:dash:sem:eval.evalcmd-fn]
pub(crate) unsafe fn evalcmd(args: &[&BStr], flags: c_int) -> Result<c_int, Error> {
    /* `grabstackstr` kept the joined string alive until the enclosing mark
     * popped, which is past the `evalstring` that parses it. Owning it here
     * says the same thing, and it has to be a binding of this frame because
     * `setinputstring` reads through the pointer rather than copying. */
    let mut concat: BString = BString::new(Vec::new());

    if args.len() > 1 {
        let single: CString;
        let p: *mut c_char = if args.len() > 2 {
            for (i, word) in args[1..].iter().enumerate() {
                if i > 0 {
                    concat.push(b' ');
                }
                concat.extend_from_slice(crate::shell::cstring(word).as_bytes());
            }
            concat.push(0);
            concat.as_mut_ptr() as *mut c_char
        } else {
            single = crate::shell::cstring(args[1]);
            single.as_ptr() as *mut c_char
        };
        return Ok(evalstring(p, flags & EV_TESTED));
    }
    Ok(0)
}
