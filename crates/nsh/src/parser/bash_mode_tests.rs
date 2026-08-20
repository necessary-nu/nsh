// [spec:nsh:req:idiom.operation-modes]
use super::{ParseResult, parse_command};
use crate::context::Shell;
use bstr::BStr;

// [spec:nsh:req:compat.bash.parse-boundary/test]
#[test]
fn dialect_changes_apply_next_parse_unit() {
    let mut shell = Shell::builder().build().unwrap();
    let parse_tree = |shell: &mut Shell| match parse_command(shell, false).unwrap() {
        ParseResult::Tree(Some(tree)) => tree,
        ParseResult::Tree(None) => panic!("expected a command, found a blank unit"),
        ParseResult::Eof => panic!("expected a command, found EOF"),
    };
    crate::input::set_input_string(
        &mut shell,
        BStr::new(b"first=posix\nkept() { second=bash; }\nkept\n"),
    );

    let first = parse_tree(&mut shell);
    assert_eq!(shell.input.parse_dialect(), crate::options::Dialect::Posix);
    crate::options::set_option_by_name(&mut shell, BStr::new(b"bash"), true).unwrap();
    assert_eq!(shell.input.parse_dialect(), crate::options::Dialect::Posix);
    assert!(matches!(
        crate::evaluation::evaluate_tree(
            &mut shell,
            Some(&first),
            crate::evaluation::EvaluationContext::DEFAULT
        )
        .unwrap(),
        crate::evaluation::Flow::Done(crate::status::ExitStatus::SUCCESS)
    ));

    let second = parse_tree(&mut shell);
    assert_eq!(shell.input.parse_dialect(), crate::options::Dialect::Bash);
    crate::options::set_option_by_name(&mut shell, BStr::new(b"bash"), false).unwrap();
    assert_eq!(shell.input.parse_dialect(), crate::options::Dialect::Bash);
    assert!(matches!(
        crate::evaluation::evaluate_tree(
            &mut shell,
            Some(&second),
            crate::evaluation::EvaluationContext::DEFAULT
        )
        .unwrap(),
        crate::evaluation::Flow::Done(crate::status::ExitStatus::SUCCESS)
    ));

    let third = parse_tree(&mut shell);
    assert_eq!(shell.input.parse_dialect(), crate::options::Dialect::Posix);
    assert!(matches!(
        crate::evaluation::evaluate_tree(
            &mut shell,
            Some(&third),
            crate::evaluation::EvaluationContext::DEFAULT
        )
        .unwrap(),
        crate::evaluation::Flow::Done(crate::status::ExitStatus::SUCCESS)
    ));
    assert_eq!(
        crate::variables::lookup_bytes(&mut shell, BStr::new(b"second"))
            .map(|value| value.to_vec()),
        Some(b"bash".to_vec())
    );
}
