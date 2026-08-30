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

use core::fmt;
use std::sync::Arc;

use bstr::{BStr, BString};

use crate::descriptors::LogicalDescriptor;
use crate::parser::SourceToken;
use crate::word::ParsedWord;

mod bash;
pub(crate) mod emit;
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
#[derive(Clone, PartialEq, Eq)]
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

/// The tokens a node was parsed from.
///
/// The reader's cuts are coarser than the grammar -- `echo $(true)` is
/// read as `echo`, a blank, `$(true` and `)` -- so what a node holds is a
/// run of the log rather than one token, and a parent's run contains its
/// children's. A node the shell built rather than parsed holds an empty
/// run, which is what [`SourceTokens::none`] says.
///
/// The bytes are shared rather than copied down the nesting, because
/// every level of a tree holds the run of every level under it and a
/// function definition is cloned for each call.
///
/// EQUALITY IS THE POINT. Comparing two trees as programs must ignore
/// their tokens, so this compares equal to everything and the derived
/// `PartialEq` of every node that holds one is unaffected by spelling.
/// Comparing as text is [`SourceTokens::same_text`], which is a named
/// operation precisely so that no `==` can be mistaken for it.
// [spec:nsh:req:idiom.canonical-tree+1]
// [spec:nsh:def:idiom.token-stream]
#[derive(Clone, Eq)]
pub struct SourceTokens(Arc<[SourceToken]>);

impl SourceTokens {
    /// Take a copy of the log run a node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn new(run: &[SourceToken]) -> SourceTokens {
        SourceTokens(Arc::from(run))
    }

    /// The empty run, for a node the shell built rather than parsed.
    ///
    /// [`spec:nsh:req:idiom.printable-ast+2`] makes this the one case a
    /// renderer has to spell a construct itself, so it is stated rather
    /// than left as a default that could be reached by forgetting.
    // [spec:nsh:req:idiom.printable-ast+2]
    pub(crate) fn none() -> SourceTokens {
        SourceTokens(Arc::from([] as [SourceToken; 0]))
    }

    /// The tokens themselves, in the order they were read.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn tokens(&self) -> &[SourceToken] {
        &self.0
    }

    /// Whether two runs are the same source text.
    ///
    /// The text question spelled out, because [`SourceTokens`] answers the
    /// program question -- always yes -- to `==`. A comparison that means
    /// text has to say so.
    // [spec:nsh:req:idiom.canonical-tree+1]
    #[cfg(test)]
    pub(crate) fn same_text(&self, other: &SourceTokens) -> bool {
        self.text() == other.text()
    }

    /// The bytes of the run, less the trivia it was reached through.
    ///
    /// A run begins behind the blanks, comments and line continuations in
    /// front of its first token, because that is what makes two nodes'
    /// runs meet rather than leave the space between them owned by
    /// nobody. A renderer that lays out its own whitespace wants what was
    /// written, not how it was reached.
    // [spec:nsh:req:idiom.printable-ast+2]
    pub(crate) fn written(&self) -> BString {
        let mut text = BString::from(Vec::new());
        let from = self
            .0
            .iter()
            .position(|token| !token.kind().is_trivia())
            .unwrap_or(self.0.len());
        for token in &self.0[from..] {
            text.extend_from_slice(token.text());
        }
        text
    }

    /// Whether the run is empty, which is what a node nothing read holds.
    ///
    /// Asked by the round-trip property, which is where the fuzz
    /// workspace links; the shell itself asks whether there is a run by
    /// trying to emit one.
    // [spec:nsh:req:idiom.printable-ast+2]
    #[cfg(any(feature = "fuzzing", test))]
    pub(crate) fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Whether this run already holds `other`'s tokens.
    ///
    /// A here-document's body is read at the newline that ends the
    /// redirection's line, and whether that newline falls inside the node
    /// being written decides whether its run already holds the body. A
    /// top-level `cat <<EOF` ends before that newline and does not;
    /// `{ cat <<EOF` reaches it inside the braces and does. Emission has
    /// to ask, because the answer is not a property of the redirection.
    // [spec:nsh:req:idiom.printable-ast+2]
    pub(crate) fn holds(&self, other: &SourceTokens) -> bool {
        !other.0.is_empty() && self.0.windows(other.0.len()).any(|run| run == &*other.0)
    }

    /// The bytes of the run, concatenated.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn text(&self) -> BString {
        let mut text = BString::from(Vec::new());
        for token in self.tokens() {
            text.extend_from_slice(token.text());
        }
        text
    }
}

/// A run shows as the source it stands for.
///
/// The derived form would print the cut points, which are the reader's
/// business and not a reader-of-a-dump's; what a node was parsed from is
/// the text.
// [spec:nsh:def:idiom.token-stream]
impl fmt::Debug for SourceTokens {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(formatter, "{:?}", self.text())
    }
}

/// Two nodes that differ only in how they were spelled are one program.
///
/// The same shape as [`SourceLine`]'s, and for the same reason: a fact
/// the tree records but that identity does not depend on. The warning is
/// the same too -- an equality that ignores a field is invisible at the
/// call site -- which is why the text comparison has a name.
// [spec:nsh:req:idiom.canonical-tree+1]
impl PartialEq for SourceTokens {
    fn eq(&self, _: &SourceTokens) -> bool {
        true
    }
}

/// The input line a grammar form was parsed on.
///
/// A position is provenance, not identity: two forms that differ only in
/// where they were read are the same form, and printing a tree relocates
/// every line in it. So all positions compare equal, and code that really
/// wants the number asks for it with [`SourceLine::get`]. That is what lets
/// a node the shell built be compared with the one its spelling parses
/// back to, which is what `[spec:nsh:req:idiom.printable-ast+2]` asks of
/// the fallback.
// [spec:nsh:req:idiom.printable-ast+2]
#[derive(Clone, Copy, Debug, Eq)]
pub struct SourceLine(i32);

impl SourceLine {
    pub const fn new(line: i32) -> SourceLine {
        SourceLine(line)
    }

    pub const fn get(self) -> i32 {
        self.0
    }
}

impl PartialEq for SourceLine {
    fn eq(&self, _: &SourceLine) -> bool {
        true
    }
}

impl From<i32> for SourceLine {
    fn from(line: i32) -> Self {
        SourceLine(line)
    }
}

/// A simple command and its lexical components.
// [spec:nsh:req:idiom.structural-ast]
#[derive(Clone, PartialEq, Eq)]
pub struct SimpleCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub line: SourceLine,
    pub assignments: Vec<Node>,
    pub arguments: Vec<Node>,
    pub redirections: Vec<Redirection>,
}

/// A pipeline of commands.
#[derive(Clone, PartialEq, Eq)]
pub struct Pipeline {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub background: bool,
    pub commands: Vec<Node>,
}

/// A command wrapped by redirection, background execution, or a subshell.
#[derive(Clone, PartialEq, Eq)]
pub struct CompoundCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub line: SourceLine,
    pub command: Box<Node>,
    pub redirections: Vec<Redirection>,
}

/// The two children of a binary grammar form.
#[derive(Clone, PartialEq, Eq)]
pub struct BinaryCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub left: Box<Node>,
    pub right: Box<Node>,
}

/// An if command.
#[derive(Clone, PartialEq, Eq)]
pub struct IfCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub condition: Box<Node>,
    pub then_branch: Box<Node>,
    pub else_branch: Option<Box<Node>>,
}

/// A for command.
#[derive(Clone, PartialEq, Eq)]
pub struct ForCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub line: SourceLine,
    pub words: Vec<Node>,
    pub body: Box<Node>,
    pub variable: NodeText,
}

/// A pipeline the shell reports the duration of.
#[derive(Clone, PartialEq, Eq)]
pub struct TimedCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub line: SourceLine,
    /// `time -p`, which reports seconds to two places instead of Bash's
    /// `real\t0m0.000s`.
    pub posix_format: bool,
    /// A bare `time` has nothing to time and reports zeros.
    pub command: Option<Box<Node>>,
}

/// A case command.
#[derive(Clone, PartialEq, Eq)]
pub struct CaseCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub line: SourceLine,
    pub word: Box<Node>,
    pub clauses: Vec<CaseClause>,
}

/// One case clause.
#[derive(Clone, PartialEq, Eq)]
pub struct CaseClause {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub patterns: Vec<Node>,
    pub body: Option<Box<Node>>,
    pub fallthrough: bool,
}

/// A shell function definition.
#[derive(Clone, PartialEq, Eq)]
pub struct FunctionDefinition {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub line: SourceLine,
    pub name: NodeText,
    pub body: Box<Node>,
}

/// A parsed word stored in the syntax tree.
#[derive(Clone, PartialEq, Eq)]
pub struct WordNode {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
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

/// Which shell descriptor a redirection acts on.
///
/// `2>file` names slot 2 in the source and that is the whole answer. Bash's
/// `{name}>file` names no slot: the shell allocates a free one above the
/// range it inherited and stores that number in `name`, so a script can
/// hold a descriptor open without picking a number that might collide with
/// one it wrote by hand. Which slot that is depends on what is open when
/// the redirection is applied, so it is not a number the parser can know --
/// which is why this is syntax and not a `LogicalDescriptor`.
// [spec:nsh:req:compat.bash.parser-ast]
#[derive(Clone, PartialEq, Eq)]
pub enum RedirectionDescriptor {
    /// A number the source wrote, or the operator's own default.
    Fixed(LogicalDescriptor),
    /// `{name}`: allocate a slot, and assign its number to this name.
    Allocated(NodeText),
}

impl RedirectionDescriptor {
    /// The slot this names outright, if it names one.
    pub fn fixed(&self) -> Option<LogicalDescriptor> {
        match self {
            Self::Fixed(descriptor) => Some(*descriptor),
            Self::Allocated(_) => None,
        }
    }

    /// How the source spells this, for whatever reprints the command.
    pub fn text(&self) -> Vec<u8> {
        match self {
            Self::Fixed(descriptor) => descriptor.digits(),
            Self::Allocated(name) => {
                let mut text = vec![b'{'];
                text.extend_from_slice(name.as_bstr());
                text.push(b'}');
                text
            }
        }
    }
}

/// A parsed redirection. Redirections are syntax attached to commands, not
/// commands themselves, so they do not inhabit [`Node`].
// [spec:nsh:req:idiom.immutable-ast]
#[derive(Clone, PartialEq, Eq)]
pub enum Redirection {
    File(FileRedirection),
    Descriptor(DescriptorRedirection),
    HereDocument(HereDocument),
    HereString(HereString),
}

/// A redirection whose operand names a file.
// [spec:nsh:def:idiom.logical-descriptors]
#[derive(Clone, PartialEq, Eq)]
pub struct FileRedirection {
    pub operator: FileRedirectionOperator,
    pub descriptor: RedirectionDescriptor,
    pub target: WordNode,
}

/// The side of a descriptor-duplication redirection.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DescriptorRedirectionOperator {
    Input,
    Output,
}

/// The parsed operand of `<&` or `>&`.
#[derive(Clone, PartialEq, Eq)]
pub enum DescriptorTarget {
    Number(LogicalDescriptor),
    Close,
    Word(WordNode),
}

/// A redirection whose operand names another shell descriptor.
#[derive(Clone, PartialEq, Eq)]
pub struct DescriptorRedirection {
    pub operator: DescriptorRedirectionOperator,
    pub descriptor: RedirectionDescriptor,
    pub target: DescriptorTarget,
}

/// A here-document redirection.
#[derive(Clone, PartialEq, Eq)]
pub struct HereDocument {
    pub descriptor: RedirectionDescriptor,
    pub expand: bool,
    pub body: WordNode,
    /// The word that ends the body.
    ///
    /// The parser has already found the end by the time this node exists,
    /// and a here-document that was read is written as the run holding its
    /// body and its terminator line together. What reads this is the
    /// fallback, which has to name a delimiter for a document nothing
    /// wrote.
    // [spec:nsh:req:idiom.printable-ast+2]
    pub delimiter: NodeText,
}

/// A Bash here-string redirection.
///
/// `<<< word` is a here-document whose body is one ordinary word rather
/// than lines of input: the word is expanded once, a newline is appended,
/// and the result is read from the descriptor.
// [spec:nsh:req:compat.bash.expansion-globbing]
#[derive(Clone, PartialEq, Eq)]
pub struct HereString {
    pub descriptor: RedirectionDescriptor,
    pub word: WordNode,
}

/// A negated command.
#[derive(Clone, PartialEq, Eq)]
pub struct NegatedCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub tokens: SourceTokens,
    pub command: Box<Node>,
}

/// The shell syntax tree.
// [spec:nsh:req:idiom.structural-ast]
#[derive(Clone, PartialEq, Eq)]
pub enum Node {
    Command(Box<SimpleCommand>),
    Pipeline(Pipeline),
    Redirect(CompoundCommand),
    Background(CompoundCommand),
    Subshell(CompoundCommand),
    /// `{ list; }` -- a list run in this shell, held together by braces.
    ///
    /// The braces are not decoration: they decide what a redirection or a
    /// `&` after them attaches to, and a list that lost them is a different
    /// program from the one that was written.
    // [spec:nsh:req:idiom.printable-ast+2]
    Group(CompoundCommand),
    And(BinaryCommand),
    Or(BinaryCommand),
    Sequence(BinaryCommand),
    If(IfCommand),
    While(BinaryCommand),
    Until(BinaryCommand),
    For(Box<ForCommand>),
    /// `select name in words; do list; done` -- Bash's menu loop, which
    /// reuses [`ForCommand`] because the syntax is `for`'s exactly.
    Select(Box<ForCommand>),
    /// `time [-p] pipeline` -- a reserved word, so it can prefix a
    /// built-in or a function, which an external `time` cannot.
    Timed(TimedCommand),
    Case(CaseCommand),
    Function(FunctionDefinition),
    Word(WordNode),
    Not(NegatedCommand),
    Bash(BashNode),
}

impl Node {
    /// The run of tokens this node was parsed from.
    ///
    /// Empty for a node the shell built rather than read, which is the
    /// one case a renderer has to spell for itself.
    // [spec:nsh:req:idiom.printable-ast+2]
    pub(crate) fn tokens(&self) -> &SourceTokens {
        match self {
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
            Node::Bash(node) => node.tokens(),
        }
    }

    /// Give a node the run of tokens it was parsed from.
    ///
    /// A compound form ends at a closing token its own branch of the
    /// parser does not read -- `done`, `fi`, `esac`, `)` are all consumed
    /// by the dispatch that called it -- so the run cannot be taken where
    /// the node is built without stopping one token short. It is taken
    /// where the construct actually ends and handed back here.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn with_tokens(mut self, tokens: SourceTokens) -> Node {
        match &mut self {
            Node::Command(command) => command.tokens = tokens,
            Node::Pipeline(pipeline) => pipeline.tokens = tokens,
            Node::Redirect(wrapper)
            | Node::Background(wrapper)
            | Node::Subshell(wrapper)
            | Node::Group(wrapper) => wrapper.tokens = tokens,
            Node::And(binary)
            | Node::Or(binary)
            | Node::Sequence(binary)
            | Node::While(binary)
            | Node::Until(binary) => binary.tokens = tokens,
            Node::If(command) => command.tokens = tokens,
            Node::For(command) | Node::Select(command) => command.tokens = tokens,
            Node::Timed(command) => command.tokens = tokens,
            Node::Case(command) => command.tokens = tokens,
            Node::Function(definition) => definition.tokens = tokens,
            Node::Word(word) => word.tokens = tokens,
            Node::Not(negation) => negation.tokens = tokens,
            Node::Bash(node) => node.set_tokens(tokens),
        }
        self
    }
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
            tokens: SourceTokens::none(),
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
                tokens: SourceTokens::none(),
                word: ParsedWord::literal(BString::from("command")),
            }))
        };
        let child = BinaryCommand {
            tokens: SourceTokens::none(),
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
