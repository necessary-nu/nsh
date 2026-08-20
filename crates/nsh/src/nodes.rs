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
//! owned Rust tree: children are `Option<Box<Node>>`, the `next`-linked
//! sibling lists are `Vec<Node>`, and the whole deep-copy apparatus is
//! a derived `Clone`.
//!
//! Each grammar form is a distinct [`Node`] variant with the payload that is
//! valid for that form. Consumers match variants directly; there are no
//! numeric tags, shared union arms, relabelled nodes, or wrong-arm accessors.
//! Two fields are still written at run time rather than at parse time:
//!
//!     `nfile.expfname` (the C marks it `temp`, so `copynode` never copied
//!     it) and `ndup.dupfd` are filled in by `expredir` on a tree that may
//!     be a shared function definition. They are `Cell`s.
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

use core::cell::{Cell, RefCell};
use std::sync::{Arc, Mutex, PoisonError};

use bstr::{BStr, BString};
use core::ffi::c_int;

use crate::word::ParsedWord;

mod bash;

pub(crate) use bash::{
    BashArithmeticCommand, BashArithmeticFor, BashArrayAssignment, BashArrayElement,
    BashArrayValue, BashAssignmentOperator, BashConditional, BashConditionalExpr, BashFunction,
    BashFunctionStyle, BashNode, BashProcessDirection, BashProcessSubstitution,
};

/// The slot a here-document body lands in.
///
/// A here-document is read *after* its redirection node is already buried in
/// a command: `parseredir` builds the node, and `parseheredoc` — which runs
/// at the next newline — supplies the text. The C did that with a back
/// pointer out of `struct heredoc` into the tree (`here->here->nhere.doc =
/// n`). A back pointer into an owned tree is the one thing Rust will not
/// give you, so the *slot* is shared instead: the node and the pending
/// `struct heredoc` hold one handle each. The body is filled exactly once,
/// then readers take an owned snapshot before an expansion can re-enter the
/// tree. The mutex makes sharing the delayed write safe without preventing a
/// complete [`Shell`](crate::context::Shell) from moving to another thread.
#[derive(Clone)]
pub struct HereDocumentBody(Arc<Mutex<Option<Node>>>);

impl HereDocumentBody {
    pub fn new() -> Self {
        Self(Arc::new(Mutex::new(None)))
    }

    pub fn fill(&self, node: Node) {
        let mut body = self.0.lock().unwrap_or_else(PoisonError::into_inner);
        assert!(body.is_none(), "a here-document body is filled only once");
        *body = Some(node);
    }

    pub fn snapshot(&self) -> Option<Node> {
        self.0
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }
}

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
/// The trailing NUL the parser wrote is *part of the value*; `as_bstr`
/// hides it from readers that operate on a length.
#[derive(Clone)]
pub struct NodeText(BString);

impl NodeText {
    /// Take ownership of a word's bytes. `text` ends in the NUL the parser
    /// wrote, which is part of the value.
    pub fn new(text: BString) -> NodeText {
        NodeText(text)
    }

    /* The `from_ptr` constructor is gone with the `strlen` it was built
     * around. Its one caller in the shell is `parser.rs` handing `for` an
     * implicit `"$@"`, and `dolatstr` is a fixed seven bytes ending in
     * the NUL, so the walk was answering a question the static had
     * already answered. */

    /// The text without its terminating NUL.
    pub fn as_bstr(&self) -> &BStr {
        BStr::new(&self.0[..self.0.len() - 1])
    }

    /// The text **with** its terminating NUL, which is what a reader that
    /// walks it as the C does needs: the terminator is the stop condition,
    /// and `argstr` counts it into the run it appends.
    pub fn as_cbytes(&self) -> &[u8] {
        &self.0
    }
}

/// A simple command and its lexical components.
// [spec:nsh:req:idiom.structural-ast]
#[derive(Clone)]
pub struct SimpleCommand {
    pub line: c_int,
    pub assignments: Vec<Node>,
    pub arguments: Vec<Node>,
    pub redirections: Vec<Node>,
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
    pub line: c_int,
    pub command: Option<Box<Node>>,
    pub redirections: Vec<Node>,
}

/// The two children of a binary grammar form.
#[derive(Clone)]
pub struct BinaryCommand {
    pub left: Option<Box<Node>>,
    pub right: Option<Box<Node>>,
}

/// An if command.
#[derive(Clone)]
pub struct IfCommand {
    pub condition: Option<Box<Node>>,
    pub then_branch: Option<Box<Node>>,
    pub else_branch: Option<Box<Node>>,
}

/// A for command.
#[derive(Clone)]
pub struct ForCommand {
    pub line: c_int,
    pub words: Vec<Node>,
    pub body: Option<Box<Node>>,
    pub variable: NodeText,
}

/// A case command.
#[derive(Clone)]
pub struct CaseCommand {
    pub line: c_int,
    pub word: Option<Box<Node>>,
    pub clauses: Vec<Node>,
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
    pub line: c_int,
    pub name: NodeText,
    pub body: Option<Box<Node>>,
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

/// A redirection whose operand names a file.
pub struct FileRedirection {
    pub operator: FileRedirectionOperator,
    pub descriptor: c_int,
    pub target: Option<Box<Node>>,
    /// actual file name — the C's `temp` field: written by `expredir` before
    /// every use and never copied by `copynode`, so it is interior-mutable
    /// here rather than part of the tree's value.
    ///
    /// The C stores `fn.list->text`, a pointer into the region that stays
    /// valid until `evalcommand`'s `popstackmark`. The node owns the bytes
    /// instead, which is the same lifetime said without the allocator:
    /// `redirect` runs while the node is alive, and nothing between
    /// `expredir` and it can free the word. `None` is the C's null — the
    /// value the field has before `expredir` has ever written it, which is
    /// not the same as an empty file name (`> ""` is a real redirection).
    pub expanded_target: RefCell<Option<BString>>,
}

impl FileRedirection {
    /// The expanded redirection target as owned shell bytes, without its
    /// storage terminator. Ownership lets opening the path re-enter shell
    /// code without retaining a `RefCell` borrow or a raw pointer.
    pub fn expanded_filename(&self) -> BString {
        let mut name = self
            .expanded_target
            .borrow()
            .as_ref()
            .expect("expredir fills every file redirection target")
            .clone();
        debug_assert_eq!(name.last(), Some(&0), "expfname is a C string");
        name.pop();
        name
    }
}

impl Clone for FileRedirection {
    /// `expfname` is the C's `temp` field: `copynode` skips it, so a copied
    /// node inherits whatever was in the block. `expredir` writes it before
    /// every use, so what it starts as does not matter; null is the value an
    /// owned node can state.
    fn clone(&self) -> FileRedirection {
        FileRedirection {
            operator: self.operator,
            descriptor: self.descriptor,
            target: self.target.clone(),
            expanded_target: RefCell::new(None),
        }
    }
}

/// The side of a descriptor-duplication redirection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorRedirectionOperator {
    Input,
    Output,
}

/// A redirection whose operand names another shell descriptor.
#[derive(Clone)]
pub struct DescriptorRedirection {
    pub operator: DescriptorRedirectionOperator,
    pub descriptor: c_int,
    /// file descriptor to duplicate — rewritten by `expredir`/`fixredir`
    /// each time the redirection is performed, so interior-mutable.
    pub dupfd: Cell<c_int>,
    /// file name if `fd>&$var`; `fixredir` clears it at parse time.
    pub variable_target: RefCell<Option<Box<Node>>>,
}

/// A here-document redirection.
pub struct HereDocument {
    pub descriptor: c_int,
    pub expand: bool,
    pub body: HereDocumentBody,
}

impl Clone for HereDocument {
    /// `new->nhere.doc = copynode(n->nhere.doc)`. The slot is shared with a
    /// `struct heredoc` only so `parseheredoc` can reach it; a *copy* of the
    /// node needs its own body, not a second handle on this one — otherwise
    /// the copy would keep pointing at text in the stack allocator.
    fn clone(&self) -> HereDocument {
        let body = HereDocumentBody::new();
        if let Some(node) = self.body.snapshot() {
            body.fill(node);
        }
        HereDocument {
            descriptor: self.descriptor,
            expand: self.expand,
            body,
        }
    }
}

/// A negated command.
#[derive(Clone)]
pub struct NegatedCommand {
    pub command: Option<Box<Node>>,
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
    CaseClause(CaseClause),
    Function(FunctionDefinition),
    Word(WordNode),
    FileRedirection(FileRedirection),
    DescriptorRedirection(DescriptorRedirection),
    HereDocument(HereDocument),
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
        let t = NodeText::new(BString::from(vec![0xff, 0]));
        assert_eq!(t.as_bstr(), &[0xff]);
        assert!(core::str::from_utf8(t.as_bstr()).is_err());
        assert_eq!(t.as_cbytes(), &[0xff, 0]);
    }

    #[test]
    fn node_text_keeps_its_terminator() {
        // `as_bstr` stops one short of the end, so a value built from
        // bytes that already carry their NUL reads back without it -- and
        // the storage still carries one.
        let src = BString::from(vec![b'n', b'a', b'm', b'e', 0]);
        let t = NodeText::new(BString::from(&src[..]));
        assert_eq!(t.as_bstr(), &src[..4]);
        assert_eq!(t.as_cbytes()[4], 0);
    }

    #[test]
    fn cloning_a_node_copies_the_bytes_rather_than_sharing_them() {
        // `copyfunc` is why this matters: a function definition outlives
        // the text it was parsed from, so `copynode` called `nodesavestr`.
        let n = Node::Word(WordNode {
            word: ParsedWord::literal(BString::from("$x")),
        });
        let copy = n.clone();
        let (Node::Word(copy), Node::Word(original)) = (&copy, &n) else {
            unreachable!()
        };
        assert_eq!(copy.word.as_bstr(), original.word.as_bstr());
        assert_ne!(copy.word.parts().as_ptr(), original.word.parts().as_ptr());
    }

    #[test]
    fn a_word_may_contain_a_nul_the_terminator_does_not_hide() {
        // A raw NUL byte reaches the parser from the input, so the value is
        // its bytes and not what a C reader makes of them.
        let t = NodeText::new(BString::from(vec![b'a', 0, b'b', 0]));
        assert_eq!(t.as_bstr(), b"a\0b".as_slice());
        assert_eq!(t.as_cbytes().iter().position(|byte| *byte == 0), Some(1));
    }

    #[test]
    // [spec:nsh:req:idiom.structural-ast/test]
    fn grammar_forms_are_distinct_variants() {
        let child = BinaryCommand {
            left: None,
            right: None,
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
