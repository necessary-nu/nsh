// [spec:nsh:req:idiom.operation-modes]
use super::{ParseResult, parsecmd};
use crate::context::Shell;
use bstr::BStr;

// [spec:nsh:req:compat.bash.parse-boundary/test]
#[test]
fn dialect_changes_apply_next_parse_unit() {
    let mut sh = Shell::builder().build().unwrap();
    let parse_tree = |sh: &mut Shell| match parsecmd(sh, 0).unwrap() {
        ParseResult::Tree(Some(tree)) => tree,
        ParseResult::Tree(None) => panic!("expected a command, found a blank unit"),
        ParseResult::Eof => panic!("expected a command, found EOF"),
    };
    crate::input::setinputstring(
        &mut sh,
        BStr::new(b"first=posix\nkept() { second=bash; }\nkept\n"),
    );

    let first = parse_tree(&mut sh);
    assert_eq!(sh.input.parse_dialect(), crate::options::Dialect::Posix);
    crate::options::set_option_by_name(&mut sh, BStr::new(b"bash"), true).unwrap();
    assert_eq!(sh.input.parse_dialect(), crate::options::Dialect::Posix);
    assert!(matches!(
        crate::eval::evaltree(&mut sh, Some(&first), crate::eval::EvalContext::DEFAULT).unwrap(),
        crate::eval::Flow::Done(crate::status::ExitStatus::SUCCESS)
    ));

    let second = parse_tree(&mut sh);
    assert_eq!(sh.input.parse_dialect(), crate::options::Dialect::Bash);
    crate::options::set_option_by_name(&mut sh, BStr::new(b"bash"), false).unwrap();
    assert_eq!(sh.input.parse_dialect(), crate::options::Dialect::Bash);
    assert!(matches!(
        crate::eval::evaltree(&mut sh, Some(&second), crate::eval::EvalContext::DEFAULT).unwrap(),
        crate::eval::Flow::Done(crate::status::ExitStatus::SUCCESS)
    ));

    let third = parse_tree(&mut sh);
    assert_eq!(sh.input.parse_dialect(), crate::options::Dialect::Posix);
    assert!(matches!(
        crate::eval::evaltree(&mut sh, Some(&third), crate::eval::EvalContext::DEFAULT).unwrap(),
        crate::eval::Flow::Done(crate::status::ExitStatus::SUCCESS)
    ));
    assert_eq!(
        crate::var::lookup_bytes(&mut sh, BStr::new(b"second")).map(|value| value.to_vec()),
        Some(b"bash".to_vec())
    );
}
