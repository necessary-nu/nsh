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

    /* Bash's `eval` ends its (empty) option list at `--`, and `ble.sh` and
     * other Bash code write `eval -- "$script"` for exactly that reason.
     * POSIX gives `eval` no options, so with the dialect off `--` stays an
     * ordinary word and joins the text, which is what dash does. */
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    let words = match args {
        [_, separator, rest @ ..]
            if shell.options.dialect() == crate::options::Dialect::Bash
                && separator.as_bytes() == b"--" =>
        {
            rest
        }
        [_, rest @ ..] => rest,
        [] => &[],
    };

    if !words.is_empty() {
        let text: &BStr = if words.len() > 1 {
            for (i, word) in words.iter().enumerate() {
                if i > 0 {
                    concat.push(b' ');
                }
                concat.extend_from_slice(word);
            }
            concat.as_bstr()
        } else {
            words[0]
        };
        return evaluate_string(shell, text, context.tested_only());
    }
    Ok(Flow::Done((0).into()))
}
