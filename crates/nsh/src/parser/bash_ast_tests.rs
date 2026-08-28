use bstr::BStr;

use super::{ParseResult, parse_command};
use crate::context::Shell;
use crate::error::Error;
use crate::nodes::{
    BashArrayValue, BashAssignmentOperator, BashConditionalExpr, BashFunctionStyle, BashNode,
    BashProcessDirection, Node, SimpleCommand, WordNode,
};
use crate::word::WordPart;

fn parse(source: &[u8], bash: bool) -> Result<Node, Error> {
    let mut shell = Shell::builder()
        .streams(crate::streams::Streams::capture().unwrap())
        .build()
        .unwrap();
    if bash {
        crate::options::set_option_by_name(&mut shell, BStr::new(b"bash"), true).unwrap();
    }
    crate::input::set_input_string(&mut shell, BStr::new(source));
    match parse_command(&mut shell, false)? {
        ParseResult::Tree(Some(tree)) => Ok(tree),
        ParseResult::Tree(None) => panic!("expected a command, found a blank parse unit"),
        ParseResult::Eof => panic!("expected a command, found EOF"),
    }
}

// [spec:nsh:req:idiom.structural-ast/test]
fn command(node: &Node) -> &SimpleCommand {
    let Node::Command(command) = node else {
        panic!("expected a simple command")
    };
    command
}

fn word(node: &Node) -> &WordNode {
    let Node::Word(word) = node else {
        panic!("expected a word")
    };
    word
}

// [spec:nsh:req:compat.bash.parser-ast/test]
#[test]
fn conditional_has_owned_precedence_tree() {
    let tree = parse(b"[[ x == y && ! -z x ]]\n", true).unwrap();
    let Node::Bash(BashNode::Conditional(conditional)) = tree else {
        panic!("[[ must be a Bash conditional node");
    };
    let BashConditionalExpr::And(left, right) = conditional.expression else {
        panic!("&& must be represented structurally");
    };
    assert!(matches!(*left, BashConditionalExpr::Binary { .. }));
    assert!(matches!(
        *right,
        BashConditionalExpr::Not(inner)
            if matches!(*inner, BashConditionalExpr::Unary { .. })
    ));

    let baseline = parse(b"[[ x ]]\n", false).unwrap();
    assert!(matches!(baseline, Node::Command(_)));
    assert_eq!(
        word(&command(&baseline).arguments[0]).word.as_bstr(),
        BStr::new(b"[[")
    );
}

#[test]
fn arithmetic_forms_have_distinct_nodes() {
    let command = parse(b"((i += 1))\n", true).unwrap();
    let Node::Bash(BashNode::ArithmeticCommand(command)) = command else {
        panic!("(( expression )) must be an arithmetic-command node");
    };
    assert_eq!(command.expression.as_bstr(), BStr::new(b"i += 1"));
    assert!(!matches!(
        parse(b"((i += 1))\n", false).unwrap(),
        Node::Bash(_)
    ));

    let loop_node = parse(b"for ((i=0; i<3; i++)); do :; done\n", true).unwrap();
    let Node::Bash(BashNode::ArithmeticFor(loop_node)) = loop_node else {
        panic!("arithmetic for must have its own node");
    };
    assert_eq!(loop_node.init.as_bstr(), BStr::new(b"i=0"));
    assert_eq!(loop_node.test.as_bstr(), BStr::new(b" i<3"));
    assert_eq!(loop_node.update.as_bstr(), BStr::new(b" i++"));
    assert!(matches!(loop_node.body.as_ref(), Node::Command(_)));
}

#[test]
fn bash_function_retains_owned_body() {
    let function = parse(b"function bash-name() { :; }\n", true).unwrap();
    let cloned = function.clone();
    let Node::Bash(BashNode::Function(function)) = cloned else {
        panic!("function reserved-word form must have its own node");
    };
    assert_eq!(function.name.as_bstr(), BStr::new(b"bash-name"));
    assert_eq!(function.style, BashFunctionStyle::FunctionParens);
    // The braces are the body's own node now, because they decide what a
    // redirection or a `&` after them attaches to.
    let Node::Group(group) = function.body.as_ref() else {
        panic!("a braced body keeps its braces");
    };
    assert!(matches!(group.command.as_ref(), Node::Command(_)));

    let bare = parse(b"function slash/name { :; }\n", true).unwrap();
    let Node::Bash(BashNode::Function(bare)) = bare else {
        panic!("bare function reserved-word form must have its own node");
    };
    assert_eq!(bare.name.as_bstr(), BStr::new(b"slash/name"));
    assert_eq!(bare.style, BashFunctionStyle::Function);
    let Node::Group(bare_body) = bare.body.as_ref() else {
        panic!("a braced body keeps its braces");
    };
    assert!(matches!(bare_body.command.as_ref(), Node::Command(_)));

    let baseline = parse(b"function bash-name\n", false).unwrap();
    assert!(matches!(baseline, Node::Command(_)));
}

#[test]
fn array_assignments_are_structural() {
    let indexed = parse(b"a[2]+=x\n", true).unwrap();
    let Node::Bash(BashNode::ArrayAssignment(indexed)) = &command(&indexed).assignments[0] else {
        panic!("indexed assignment must be structural");
    };
    assert_eq!(indexed.name.as_bstr(), BStr::new(b"a"));
    assert_eq!(
        indexed.subscript.as_ref().unwrap().word.as_bstr(),
        BStr::new(b"2")
    );
    assert_eq!(indexed.operator, BashAssignmentOperator::Append);
    let BashArrayValue::Word(value) = &indexed.value else {
        panic!("simple indexed assignment must retain a word value");
    };
    assert_eq!(value.word.as_bstr(), BStr::new(b"x"));

    let compound = parse(b"a=(zero [2]=two)\n", true).unwrap();
    let Node::Bash(BashNode::ArrayAssignment(compound)) = &command(&compound).assignments[0] else {
        panic!("compound assignment must be structural");
    };
    let BashArrayValue::Compound(elements) = &compound.value else {
        panic!("compound assignment needs element structure");
    };
    assert_eq!(elements.len(), 2);
    assert!(elements[0].subscript.is_none());
    assert_eq!(
        elements[1].subscript.as_ref().unwrap().word.as_bstr(),
        BStr::new(b"2")
    );
    assert_eq!(elements[1].value.word.as_bstr(), BStr::new(b"two"));

    let declaration = parse(b"declare -a a=(x)\n", true).unwrap();
    assert!(matches!(
        command(&declaration).arguments[2],
        Node::Bash(BashNode::ArrayAssignment(_))
    ));

    assert!(parse(b"a=(zero)\n", false).is_err());
}

#[test]
fn array_parameter_subscript_is_dialect_gated() {
    let tree = parse(b"echo ${a[1]}\n", true).unwrap();
    assert_eq!(command(&tree).arguments.len(), 2);
    let baseline = parse(b"echo ${a[1]}\n", false).unwrap();
    let parameter_name = |node: &Node| {
        word(node).word.parts().iter().find_map(|part| {
            let WordPart::Parameter(parameter) = part else {
                return None;
            };
            Some(parameter.name.clone())
        })
    };
    assert_ne!(
        parameter_name(&command(&tree).arguments[1]),
        parameter_name(&command(&baseline).arguments[1]),
    );
}

#[test]
// [spec:nsh:def:idiom.word-ir/test]
fn process_substitutions_own_their_commands() {
    let tree = parse(b"echo <(printf x) >(cat)\n", true).unwrap();
    let args = &command(&tree).arguments;
    assert_eq!(args.len(), 3);

    for (argument, expected) in [
        (&args[1], BashProcessDirection::Input),
        (&args[2], BashProcessDirection::Output),
    ] {
        let substitution = word(argument)
            .word
            .parts()
            .iter()
            .find_map(|part| match part {
                WordPart::Command {
                    command: Some(command),
                    ..
                } => Some(command.as_ref()),
                _ => None,
            })
            .unwrap();
        let Node::Bash(BashNode::ProcessSubstitution(substitution)) = substitution else {
            panic!("process substitution must not masquerade as command substitution");
        };
        assert_eq!(substitution.direction, expected);
        assert!(substitution.body.is_some());
    }

    assert!(parse(b"echo <(printf x)\n", false).is_err());
}
