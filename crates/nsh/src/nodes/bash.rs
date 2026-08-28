//! Owned syntax nodes for the Bash compatibility dialect.
//!
//! These nodes extend the one shell tree; they are not a parallel parser
//! representation.  Each variant records the grammar boundary that later
//! runtime work consumes, while embedded words use [`WordNode`].

#![expect(
    dead_code,
    reason = "Bash syntax is parsed ahead of the paused evaluator implementation"
)]

use super::{Node, NodeText, SourceLine, SourceTokens, WordNode};

/// Bash-only syntax in the shell's owned parse tree.
// [spec:nsh:req:idiom.structural-ast]
// [spec:nsh:req:compat.bash.parser-ast]
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum BashNode {
    Conditional(Box<BashConditional>),
    ArithmeticCommand(BashArithmeticCommand),
    ArithmeticFor(Box<BashArithmeticFor>),
    Function(BashFunction),
    ArrayAssignment(Box<BashArrayAssignment>),
    ProcessSubstitution(BashProcessSubstitution),
}

impl BashNode {
    /// Give a Bash node the run of tokens it was parsed from.
    ///
    /// The counterpart of [`super::Node::with_tokens`], reached the same
    /// way and for the same reason.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) fn set_tokens(&mut self, tokens: SourceTokens) {
        match self {
            BashNode::Conditional(node) => node.tokens = tokens,
            BashNode::ArithmeticCommand(node) => node.tokens = tokens,
            BashNode::ArithmeticFor(node) => node.tokens = tokens,
            BashNode::Function(node) => node.tokens = tokens,
            BashNode::ArrayAssignment(node) => node.tokens = tokens,
            BashNode::ProcessSubstitution(node) => node.tokens = tokens,
        }
    }
}

/// A `[[ expression ]]` command.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BashConditional {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) tokens: SourceTokens,
    pub(crate) line: SourceLine,
    pub(crate) expression: BashConditionalExpr,
}

/// The precedence-bearing expression inside `[[ ... ]]`.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum BashConditionalExpr {
    Empty,
    Word(WordNode),
    Unary {
        operator: NodeText,
        operand: WordNode,
    },
    Binary {
        left: WordNode,
        operator: NodeText,
        right: WordNode,
    },
    Not(Box<BashConditionalExpr>),
    And(Box<BashConditionalExpr>, Box<BashConditionalExpr>),
    Or(Box<BashConditionalExpr>, Box<BashConditionalExpr>),
    Group(Box<BashConditionalExpr>),
}

/// A `(( expression ))` command.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BashArithmeticCommand {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) tokens: SourceTokens,
    pub(crate) line: SourceLine,
    pub(crate) expression: NodeText,
}

/// A `for (( init; test; update )); do ...; done` command.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BashArithmeticFor {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) tokens: SourceTokens,
    pub(crate) line: SourceLine,
    pub(crate) init: NodeText,
    pub(crate) test: NodeText,
    pub(crate) update: NodeText,
    pub(crate) body: Box<Node>,
}

/// Which Bash spelling introduced a function definition.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BashFunctionStyle {
    Function,
    FunctionParens,
}

/// A function introduced by Bash's `function` reserved word.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BashFunction {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) tokens: SourceTokens,
    pub(crate) line: SourceLine,
    pub(crate) name: NodeText,
    pub(crate) style: BashFunctionStyle,
    pub(crate) body: Box<Node>,
}

/// Assignment operator used by a structural array assignment.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BashAssignmentOperator {
    Set,
    Append,
}

/// An indexed or compound array assignment.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BashArrayAssignment {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) tokens: SourceTokens,
    pub(crate) name: NodeText,
    pub(crate) subscript: Option<WordNode>,
    pub(crate) operator: BashAssignmentOperator,
    pub(crate) value: BashArrayValue,
}

/// The right-hand side of an array assignment.
#[derive(Clone, PartialEq, Eq)]
pub(crate) enum BashArrayValue {
    Word(WordNode),
    Compound(Vec<BashArrayElement>),
}

/// One word in a compound array assignment.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BashArrayElement {
    pub(crate) subscript: Option<WordNode>,
    pub(crate) operator: BashAssignmentOperator,
    pub(crate) value: WordNode,
}

/// Whether a process substitution feeds or consumes a path.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum BashProcessDirection {
    Input,
    Output,
}

/// An owned `<(list)` or `>(list)` embedded in a word.
#[derive(Clone, PartialEq, Eq)]
pub(crate) struct BashProcessSubstitution {
    /// The tokens this node was parsed from.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) tokens: SourceTokens,
    pub(crate) direction: BashProcessDirection,
    pub(crate) body: Option<Box<Node>>,
}
