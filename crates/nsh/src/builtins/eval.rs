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

// [spec:nsh:req:idiom.operation-modes]
use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::{EvaluationContext, Flow, evaluate_string};
use bstr::{BStr, BString, ByteSlice};

// [spec:dash:sem:eval.evalcmd-fn]
// [spec:posix:syn:builtin.eval.syn]
// [spec:posix:req:builtin.eval.construct-and-execute]
// [spec:posix:req:builtin.eval.stderr]
// [spec:posix:req:builtin.eval.interfaces]
// [spec:posix:req:builtin.eval.exit-status]
pub(crate) fn evaluate_arguments(
    shell: &mut Shell,
    args: &[&BStr],
    context: EvaluationContext,
) -> Result<Flow, Error> {
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
        return evaluate_string(shell, text, context.tested_only());
    }
    Ok(Flow::Done((0).into()))
}
