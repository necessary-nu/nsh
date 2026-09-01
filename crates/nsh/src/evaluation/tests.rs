//! `Flow`, and the propagation operator that carries it.
//!
//! What these pin is not the shape of the enum but the two claims the
//! conversion rests on: that `flow!` *returns* rather than falling
//! through, which is what makes it the literal stand-in for a longjmp
//! past this frame; and that an explicit exit carries its selected
//! status while EXEND does not. The behaviour is pinned end to end in
//! `tests/errors_are_values.rs`.

use super::*;

// [spec:nsh:req:idiom.immutable-ast/test]
#[test]
fn redirection_expansion_stays_evaluation_local() {
    let _guard = crate::test_support::lock();
    let mut shell = crate::context::Shell::builder().build().unwrap();
    crate::input::set_input_string(&mut shell, BStr::new(b": >\"$target\"\n"));
    let tree = match crate::parser::parse_command(&mut shell, false).unwrap() {
        crate::parser::ParseResult::Tree(Some(tree)) => tree,
        _ => panic!("expected a command"),
    };
    let Node::Command(command) = &tree else {
        panic!("expected a simple command");
    };
    let Redirection::File(parsed) = &command.redirections[0] else {
        panic!("expected a file redirection");
    };
    let parsed_spelling = parsed.target.word.as_bstr().to_owned();

    crate::variables::set_bytes(
        &mut shell,
        BStr::new(b"target"),
        Some(BStr::new(b"one")),
        VariableAttributes::NONE,
    )
    .unwrap();
    let first = expand_redirections(&mut shell, &command.redirections).unwrap();
    assert!(matches!(
        &first[0],
        ExpandedRedirection::File { target, .. } if target == BStr::new(b"one")
    ));
    drop(first);

    crate::variables::set_bytes(
        &mut shell,
        BStr::new(b"target"),
        Some(BStr::new(b"two")),
        VariableAttributes::NONE,
    )
    .unwrap();
    let second = expand_redirections(&mut shell, &command.redirections).unwrap();
    assert!(matches!(
        &second[0],
        ExpandedRedirection::File { target, .. } if target == BStr::new(b"two")
    ));
    assert_eq!(parsed.target.word.as_bstr(), parsed_spelling.as_bstr());
}

/// `flow!` on a finished evaluation yields the status and carries on.
// [spec:dash:sem:eval.evaltree-fn/test]
#[test]
fn flow_yields_a_status() {
    fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
        let status = flow!(inner);
        Ok(Flow::Done(ExitStatus::from_code(
            i32::from(status.code()) + 100,
        )))
    }
    let got = body(Ok(Flow::Done((7).into())));
    assert_eq!(got.unwrap(), Flow::Done((107).into()));
}

/// …and on an exit it returns, so nothing after it runs. That is the
/// whole of what the C got from jumping past the frame, and getting
/// it wrong would run epilogues the unwind skipped.
// [spec:dash:sem:eval.evaltree-fn/test]
#[test]
fn flow_returns_an_exit() {
    fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
        let _status = flow!(inner);
        panic!("flow! must not fall through on an exit");
    }
    let got = body(Ok(Flow::exit(9)));
    assert_eq!(
        got.unwrap(),
        Flow::Exit {
            status: Some(ExitStatus::from_code(9))
        }
    );
}

/// A diagnostic still propagates through it, because the `?` is
/// inside: `flow!` adds an arm, it does not replace one.
// [spec:dash:sem:eval.evaltree-fn/test]
#[test]
fn flow_still_propagates_an_error() {
    fn body(inner: Result<Flow, Error>) -> Result<Flow, Error> {
        let _status = flow!(inner);
        panic!("flow! must not fall through on an error");
    }
    let error = Error::Other {
        line: 3,
        status: ExitStatus::ERROR,
        message: bstr::BString::from(&b"nope"[..]),
    };
    let got = body(Err(error));
    assert_eq!(got.unwrap_err().message(), "nope");
}

/// EXEXIT owns the selected status while EXEND uses the status already
/// on the shell.
// [spec:dash:sem:init.exitreset-fn/test]
// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn explicit_exit_carries_status() {
    assert_eq!(
        Flow::exit(9),
        Flow::Exit {
            status: Some(ExitStatus::from_code(9))
        }
    );
    assert_eq!(Flow::END, Flow::Exit { status: None });
    assert_ne!(Flow::exit(9), Flow::END);
}

/// The catch frame applies any selected status before cleanup. Reset
/// therefore cannot overwrite the status chosen by either exit path.
// [spec:dash:sem:init.exitreset-fn/test]
// [spec:nsh:req:compat.smoosh.trap-status/test]
#[test]
fn exitreset_preserves_status() {
    let _guard = crate::test_support::lock();
    let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
    let shell = &mut owned;

    shell.status = ExitStatus::from_code(9);
    shell.evaluation.loop_depth = 3;
    shell.evaluation.expanding_trace_prompt = true;
    shell.clear_evaluation_resources();
    assert_eq!(shell.status, ExitStatus::from_code(9));
    assert_eq!(shell.evaluation.loop_depth, 0);
    assert!(!shell.evaluation.expanding_trace_prompt);
}
