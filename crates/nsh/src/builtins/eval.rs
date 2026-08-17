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

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;

use crate::eval::{EV_TESTED, Flow, evalstring};

// [spec:dash:def:eval.evalcmd-fn]
// [spec:dash:sem:eval.evalcmd-fn]
pub(crate) fn evalcmd(sh: &mut Shell, args: &[&BStr], flags: c_int) -> Result<Flow, Error> {
    /* `grabstackstr` kept the joined string alive until the enclosing mark
     * popped, which is past the `evalstring` that parses it. Owning it here
     * says the same thing, and it has to be a binding of this frame because
     * `setinputstring` reads through the pointer rather than copying. */
    let mut concat: BString = BString::new(Vec::new());

    if args.len() > 1 {
        let text: &BStr = if args.len() > 2 {
            for (i, word) in args[1..].iter().enumerate() {
                if i > 0 {
                    concat.push(b' ');
                }
                concat.extend_from_slice(word);
            }
            concat.as_bstr()
        } else {
            args[1]
        };
        return evalstring(sh, text, flags & EV_TESTED);
    }
    Ok(Flow::Done(0))
}
