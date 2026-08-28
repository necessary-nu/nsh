//! What a node's tokens are, and what comparing two nodes does not see.
//!
//! Two properties divide [`spec:nsh:req:idiom.canonical-tree+1`] between
//! them. A node carries the run of source it was parsed from, so the tree
//! records what was written; and comparing two nodes as programs ignores
//! that run, so what was written does not decide what the program is.
//! Either one alone is satisfiable by a tree that is wrong in the other
//! direction, which is why both are here.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::nodes::{Node, SourceTokens};
use crate::parser::ParseResult;

fn shell(bash: bool) -> Shell {
    Shell::builder()
        .streams(crate::Streams::capture().expect("captured streams"))
        .option(BStr::new(b"bash"), bash)
        .build()
        .expect("shell")
}

/// Parse `source` as one command unit and hand back the node.
// [spec:nsh:req:idiom.canonical-tree+1/test]
fn parse(source: &[u8]) -> Node {
    let mut shell = shell(false);
    crate::resource::with_resources(&mut shell, |shell, _resources| {
        crate::input::set_input_string(shell, BStr::new(source));
        match crate::parser::parse_command(shell, false) {
            Ok(ParseResult::Tree(Some(node))) => node,
            _ => panic!("{:?} did not parse to one command", BStr::new(source)),
        }
    })
}

/// The run a node holds.
///
/// The variants the sources below produce, and no others: a test that
/// grew a shape this does not name should say so rather than be widened
/// into a second copy of the tree walk.
// [spec:nsh:def:idiom.token-stream/test]
fn run_of(node: &Node) -> &SourceTokens {
    match node {
        Node::Command(command) => &command.tokens,
        Node::Pipeline(pipeline) => &pipeline.tokens,
        Node::Redirect(wrapper)
        | Node::Background(wrapper)
        | Node::Subshell(wrapper)
        | Node::Group(wrapper) => &wrapper.tokens,
        Node::And(binary)
        | Node::Or(binary)
        | Node::Sequence(binary)
        | Node::While(binary)
        | Node::Until(binary) => &binary.tokens,
        Node::If(command) => &command.tokens,
        Node::For(command) | Node::Select(command) => &command.tokens,
        Node::Timed(command) => &command.tokens,
        Node::Case(command) => &command.tokens,
        Node::Function(definition) => &definition.tokens,
        Node::Word(word) => &word.tokens,
        Node::Not(negation) => &negation.tokens,
        Node::Bash(_) => panic!("no source here parses to a Bash node"),
    }
}

/// Sources that spell one program two ways.
///
/// Each pair is a distinction [`dec:nsh:no-equivalent-forms`] names as
/// one the structure does not carry: whitespace, a comment, the default
/// descriptor of a redirection, a separator written as a newline, the
/// optional `(` of a case pattern, and the `in "$@"` a bare `for`
/// implies.
// [spec:nsh:req:idiom.canonical-tree+1/test]
const EQUIVALENT: &[(&[u8], &[u8])] = &[
    (b"echo a", b"echo    a"),
    (b"if a; then b; fi", b"if a # what it tests\nthen b; fi"),
    (b"cat >f", b"cat 1>f"),
    (b"if true; then :; fi", b"if true\nthen\n:\nfi"),
    (b"a && b", b"a &&\nb"),
    (b"case x in y) ;; esac", b"case x in (y) ;; esac"),
    (b"for a; do :; done", b"for a in \"$@\"; do :; done"),
    (b"a; b", b"a \\\n; b"),
];

#[test]
// [spec:nsh:req:idiom.canonical-tree+1/test]
fn two_spellings_of_one_program_are_one_node() {
    for (left, right) in EQUIVALENT {
        let (parsed_left, parsed_right) = (parse(left), parse(right));
        assert!(
            parsed_left == parsed_right,
            "{:?} and {:?} are one program and parsed to two nodes",
            BStr::new(left),
            BStr::new(right)
        );
        assert!(
            !run_of(&parsed_left).same_text(run_of(&parsed_right)),
            "{:?} and {:?} were written differently and kept one run",
            BStr::new(left),
            BStr::new(right)
        );
    }
}

/// The equality above has to be able to fail, or it says nothing.
///
/// A run that compares equal to everything is one line away from a node
/// that does too, and the pairs above would pass just as happily.
// [spec:nsh:req:idiom.canonical-tree+1/test]
#[test]
fn programs_that_differ_are_not_equal() {
    let differing: &[(&[u8], &[u8])] = &[
        (b"echo a", b"echo b"),
        (b"a && b", b"a || b"),
        (b"a; b", b"a | b"),
        (b"cat >f", b"cat >>f"),
        (b"cat >f", b"cat 2>f"),
        (b"if a; then b; fi", b"if a; then b; else c; fi"),
        (b"case x in y) ;; esac", b"case x in y) ;& esac"),
        (b"while a; do b; done", b"until a; do b; done"),
    ];
    for (left, right) in differing {
        assert!(
            parse(left) != parse(right),
            "{:?} and {:?} are different programs and parsed equal",
            BStr::new(left),
            BStr::new(right)
        );
    }
}

/// Every shape of node keeps the source it was read from.
///
/// The run is taken where the construct ends rather than where the node
/// is built, so a form whose closing token the dispatch reads -- `fi`,
/// `done`, `esac`, `)` -- keeps it. It stops there too: a blank or a
/// comment after the last token of a command is read on the way to
/// whatever follows and belongs to that, so none of these sources ends
/// in trivia.
// [spec:nsh:def:idiom.token-stream/test]
#[test]
fn a_node_holds_the_source_it_came_from() {
    let sources: &[&[u8]] = &[
        b"echo a b",
        b"if a # a comment inside a construct\nthen b; fi",
        b"cat <f >g 2>&1",
        b"a; b; c",
        b"a && b || c",
        b"a | b | c",
        b"! a | b",
        b"a &",
        b"(a; b)",
        b"{ a; } >f",
        b"if a; then b; elif c; then d; else e; fi",
        b"while a; do b; done",
        b"until a; do b; done",
        b"for x in 1 2 3; do echo $x; done",
        b"case $x in a|b) c ;; *) d ;; esac",
        b"f() { echo hi; }",
        b"time -p a | b",
        b"echo $(true) `false` \"$x\" 'y'",
    ];
    for source in sources {
        let parsed = parse(source);
        assert_eq!(
            run_of(&parsed).text(),
            BString::from(*source),
            "the run of {:?} is not the source it was read from",
            BStr::new(source)
        );
    }
}

/// A node the shell built rather than read holds no run.
///
/// `for a; do` has no `in` list in the source, so the `"$@"` the grammar
/// supplies is the one word in that tree a renderer has to spell itself.
// [spec:nsh:req:idiom.printable-ast+2/test]
#[test]
fn a_node_the_grammar_supplied_holds_no_run() {
    let Node::For(loop_command) = parse(b"for a; do :; done") else {
        panic!("a for command")
    };
    let [word] = loop_command.words.as_slice() else {
        panic!("one implied word")
    };
    assert!(run_of(word).text().is_empty());

    let Node::For(written) = parse(b"for a in \"$@\"; do :; done") else {
        panic!("a for command")
    };
    let [word] = written.words.as_slice() else {
        panic!("one written word")
    };
    assert_eq!(run_of(word).text(), BString::from(b" \"$@\"".as_slice()));
}

/// A here-document's body is not next to the redirection that named it.
///
/// The body is read at the newline that ends the command's line, so the
/// command's own run stops at the redirection operator and the bytes of
/// the document hang off the body word instead. A renderer that assumed
/// one contiguous span per node would lose them.
// [spec:nsh:def:idiom.token-stream/test]
#[test]
fn a_here_document_body_hangs_off_its_word() {
    let parsed = parse(b"cat <<EOF\nbody\nEOF\n");
    assert_eq!(
        run_of(&parsed).text(),
        BString::from(b"cat <<EOF".as_slice())
    );
    let Node::Command(command) = &parsed else {
        panic!("a simple command")
    };
    let [crate::nodes::Redirection::HereDocument(here)] = command.redirections.as_slice() else {
        panic!("one here-document")
    };
    assert_eq!(
        here.body.tokens.text(),
        BString::from(b"body\nEOF\n".as_slice())
    );
}
