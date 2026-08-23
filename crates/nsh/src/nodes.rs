//! The shell's parse tree.
//!
//! `src/nodes.c` and `src/nodes.h` are generated at build time by
//! `src/mknodes.c` from `src/nodetypes` and `src/nodes.c.pat`, and the first
//! port of this module was a transcription of that output: a `#[repr(C)]
//! union node` whose arms shared an `int type` first member, a `nodesize[]`
//! table indexed by that member, and a `copyfunc` that measured a tree with
//! `calcsize` and then laid a copy of it out inside one `ckmalloc`'d block,
//! refcounted through `struct funcnode { int count; union node n; }`.
//!
//! Every part of that existed because the tree lived in the stack allocator
//! and C has neither destructors nor a discriminated union.
//! [dec:nsh:owned-data] takes the allocator away, so what is here now is an
//! owned Rust tree: required children are `Box<Node>`, optional grammar
//! branches use `Option<Box<Node>>`, the `next`-linked sibling lists are
//! `Vec<Node>`, and the whole deep-copy apparatus is a derived `Clone`.
//!
//! Each grammar form is a distinct [`Node`] variant with the payload that is
//! valid for that form. Consumers match variants directly; there are no
//! numeric tags, shared union arms, relabelled nodes, or wrong-arm accessors.
//! Redirections likewise store only parsed syntax. Here-document bodies are
//! attached before parsing returns, while expanded paths and resolved
//! descriptor operands live in evaluation-local values. A parsed function
//! can consequently be cloned, cached, and evaluated without mutation.
//!
//! Parsed arguments own a structural [`crate::word::ParsedWord`]. Function
//! names and the few remaining raw grammar fields use [`NodeText`] while the
//! AST itself is being typed; both own their bytes rather than borrowing the
//! parser's scratch storage.
//!
//! `src/mknodes.c` still generates the `nodes.c`/`nodes.h` that the C
//! reference is built from, but nothing it emits — `nodesize[]`, `calcsize`,
//! `copynode` — has a counterpart here, so the port of that generator went
//! with the layout it described.

use bstr::{BStr, BString};

use crate::descriptors::LogicalDescriptor;
use crate::word::ParsedWord;

mod bash;
pub(crate) mod source;

pub(crate) use bash::{
    BashArithmeticCommand, BashArithmeticFor, BashArrayAssignment, BashArrayElement,
    BashArrayValue, BashAssignmentOperator, BashConditional, BashConditionalExpr, BashFunction,
    BashFunctionStyle, BashNode, BashProcessDirection, BashProcessSubstitution,
};

/// The text of a word, a `for` variable or a function name.
///
/// The C stores a bare `char *` into the stack allocator, which works only
/// because the tree and the text die together at the `popstackmark` that ends
/// the command. `copyfunc` is where they do not: a function definition
/// outlives the mark its text was parsed under, so `copynode` copied the
/// *bytes* — `funcstringsize` measured them and `nodesavestr` wrote them.
///
/// Here the bytes are owned from the moment the parser produces them, so both
/// cases are the same case and `Clone` is derived.
///
#[derive(Clone)]
pub struct NodeText(BString);

impl NodeText {
    /// Take ownership of a grammar field's shell bytes.
    pub fn new(text: BString) -> NodeText {
        NodeText(text)
    }

    /* The `from_ptr` constructor is gone with the `strlen` it was built
     * around. Its one caller in the shell is `parser.rs` handing `for` an
     * implicit `"$@"`, and `dolatstr` is a fixed seven bytes ending in
     * the NUL, so the walk was answering a question the static had
     * already answered. */

    /// Borrow the stored bytes.
    pub fn as_bstr(&self) -> &BStr {
        BStr::new(&self.0)
    }
}

impl From<&[u8]> for NodeText {
    fn from(bytes: &[u8]) -> Self {
        Self(BString::from(bytes))
    }
}

impl From<&BStr> for NodeText {
    fn from(bytes: &BStr) -> Self {
        Self(bytes.to_owned())
    }
}

/// A simple command and its lexical components.
// [spec:nsh:req:idiom.structural-ast]
#[derive(Clone)]
pub struct SimpleCommand {
    pub line: i32,
    pub assignments: Vec<Node>,
    pub arguments: Vec<Node>,
    pub redirections: Vec<Redirection>,
}

/// A pipeline of commands.
#[derive(Clone)]
pub struct Pipeline {
    pub background: bool,
    pub commands: Vec<Node>,
}

/// A command wrapped by redirection, background execution, or a subshell.
#[derive(Clone)]
pub struct CompoundCommand {
    pub line: i32,
    pub command: Box<Node>,
    pub redirections: Vec<Redirection>,
}

/// The two children of a binary grammar form.
#[derive(Clone)]
pub struct BinaryCommand {
    pub left: Box<Node>,
    pub right: Box<Node>,
}

/// An if command.
#[derive(Clone)]
pub struct IfCommand {
    pub condition: Box<Node>,
    pub then_branch: Box<Node>,
    pub else_branch: Option<Box<Node>>,
}

/// A for command.
#[derive(Clone)]
pub struct ForCommand {
    pub line: i32,
    pub words: Vec<Node>,
    pub body: Box<Node>,
    pub variable: NodeText,
}

/// A case command.
#[derive(Clone)]
pub struct CaseCommand {
    pub line: i32,
    pub word: Box<Node>,
    pub clauses: Vec<CaseClause>,
}

/// One case clause.
#[derive(Clone)]
pub struct CaseClause {
    pub patterns: Vec<Node>,
    pub body: Option<Box<Node>>,
    pub fallthrough: bool,
}

/// A shell function definition.
#[derive(Clone)]
pub struct FunctionDefinition {
    pub line: i32,
    pub name: NodeText,
    pub body: Box<Node>,
}

/// A parsed word stored in the syntax tree.
#[derive(Clone)]
pub struct WordNode {
    pub word: ParsedWord,
}

/// A file-opening redirection operator.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FileRedirectionOperator {
    Write,
    Clobber,
    Read,
    ReadWrite,
    Append,
}

/// A parsed redirection. Redirections are syntax attached to commands, not
/// commands themselves, so they do not inhabit [`Node`].
// [spec:nsh:req:idiom.immutable-ast]
#[derive(Clone)]
pub enum Redirection {
    File(FileRedirection),
    Descriptor(DescriptorRedirection),
    HereDocument(HereDocument),
}

/// A redirection whose operand names a file.
// [spec:nsh:def:idiom.logical-descriptors]
#[derive(Clone)]
pub struct FileRedirection {
    pub operator: FileRedirectionOperator,
    pub descriptor: LogicalDescriptor,
    pub target: WordNode,
}

/// The side of a descriptor-duplication redirection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorRedirectionOperator {
    Input,
    Output,
}

/// The parsed operand of `<&` or `>&`.
#[derive(Clone)]
pub enum DescriptorTarget {
    Number(LogicalDescriptor),
    Close,
    Word(WordNode),
}

/// A redirection whose operand names another shell descriptor.
#[derive(Clone)]
pub struct DescriptorRedirection {
    pub operator: DescriptorRedirectionOperator,
    pub descriptor: LogicalDescriptor,
    pub target: DescriptorTarget,
}

/// A here-document redirection.
#[derive(Clone)]
pub struct HereDocument {
    pub descriptor: LogicalDescriptor,
    pub expand: bool,
    pub body: WordNode,
}

/// A negated command.
#[derive(Clone)]
pub struct NegatedCommand {
    pub command: Box<Node>,
}

/// The shell syntax tree.
// [spec:nsh:req:idiom.structural-ast]
#[derive(Clone)]
pub enum Node {
    Command(SimpleCommand),
    Pipeline(Pipeline),
    Redirect(CompoundCommand),
    Background(CompoundCommand),
    Subshell(CompoundCommand),
    And(BinaryCommand),
    Or(BinaryCommand),
    Sequence(BinaryCommand),
    If(IfCommand),
    While(BinaryCommand),
    Until(BinaryCommand),
    For(ForCommand),
    Case(CaseCommand),
    Function(FunctionDefinition),
    Word(WordNode),
    Not(NegatedCommand),
    Bash(BashNode),
}

// ---------------------------------------------------------------------
// Shell names are bytes rather than UTF-8 text.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn node_text_preserves_non_utf8_names() {
        let t = NodeText::new(BString::from(vec![0xff]));
        assert_eq!(t.as_bstr(), &[0xff]);
        assert!(core::str::from_utf8(t.as_bstr()).is_err());
    }

    #[test]
    fn node_text_keeps_complete_bytes() {
        let src = BString::from(vec![b'n', b'a', b'm', b'e']);
        let t = NodeText::new(BString::from(&src[..]));
        assert_eq!(t.as_bstr(), src);
    }

    #[test]
    fn cloning_a_node_copies_the_bytes_rather_than_sharing_them() {
        // `copyfunc` is why this matters: a function definition outlives
        // the text it was parsed from, so `copynode` called `nodesavestr`.
        let node = Node::Word(WordNode {
            word: ParsedWord::literal(BString::from("$x")),
        });
        let copy = node.clone();
        let (Node::Word(copy), Node::Word(original)) = (&copy, &node) else {
            unreachable!()
        };
        assert_eq!(copy.word.as_bstr(), original.word.as_bstr());
        assert_ne!(copy.word.parts().as_ptr(), original.word.parts().as_ptr());
    }

    #[test]
    fn node_text_preserves_embedded_nul() {
        let t = NodeText::new(BString::from(vec![b'a', 0, b'b']));
        assert_eq!(t.as_bstr(), b"a\0b".as_slice());
        assert_eq!(t.as_bstr().iter().position(|byte| *byte == 0), Some(1));
    }

    #[test]
    // [spec:nsh:req:idiom.structural-ast/test]
    fn grammar_forms_are_distinct_variants() {
        let command = || {
            Box::new(Node::Word(WordNode {
                word: ParsedWord::literal(BString::from("command")),
            }))
        };
        let child = BinaryCommand {
            left: command(),
            right: command(),
        };
        assert!(matches!(Node::And(child.clone()), Node::And(_)));
        assert!(matches!(Node::Or(child.clone()), Node::Or(_)));
        assert!(matches!(Node::Sequence(child), Node::Sequence(_)));
        assert_ne!(
            FileRedirectionOperator::Write,
            FileRedirectionOperator::Append
        );
    }
}
