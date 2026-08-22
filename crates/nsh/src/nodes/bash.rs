//! Owned syntax nodes for the Bash compatibility dialect.
//!
//! These nodes extend the one shell tree; they are not a parallel parser
//! representation.  Each variant records the grammar boundary that later
//! runtime work consumes, while embedded words use [`WordNode`].

#![expect(
    dead_code,
    reason = "Bash syntax is parsed ahead of the paused evaluator implementation"
)]

use super::{Node, NodeText, WordNode};

/// Bash-only syntax in the shell's owned parse tree.
// [spec:nsh:req:idiom.structural-ast]
// [spec:nsh:req:compat.bash.parser-ast]
#[derive(Clone)]
pub(crate) enum BashNode {
    Conditional(BashConditional),
    ArithmeticCommand(BashArithmeticCommand),
    ArithmeticFor(BashArithmeticFor),
    Function(BashFunction),
    ArrayAssignment(BashArrayAssignment),
    ProcessSubstitution(BashProcessSubstitution),
}

/// A `[[ expression ]]` command.
#[derive(Clone)]
pub(crate) struct BashConditional {
    pub(crate) line: i32,
    pub(crate) expression: BashConditionalExpr,
}

/// The precedence-bearing expression inside `[[ ... ]]`.
#[derive(Clone)]
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
#[derive(Clone)]
pub(crate) struct BashArithmeticCommand {
    pub(crate) line: i32,
    pub(crate) expression: NodeText,
}

/// A `for (( init; test; update )); do ...; done` command.
#[derive(Clone)]
pub(crate) struct BashArithmeticFor {
    pub(crate) line: i32,
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
#[derive(Clone)]
pub(crate) struct BashFunction {
    pub(crate) line: i32,
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
#[derive(Clone)]
pub(crate) struct BashArrayAssignment {
    pub(crate) name: NodeText,
    pub(crate) subscript: Option<WordNode>,
    pub(crate) operator: BashAssignmentOperator,
    pub(crate) value: BashArrayValue,
}

/// The right-hand side of an array assignment.
#[derive(Clone)]
pub(crate) enum BashArrayValue {
    Word(WordNode),
    Compound(Vec<BashArrayElement>),
}

/// One word in a compound array assignment.
#[derive(Clone)]
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
#[derive(Clone)]
pub(crate) struct BashProcessSubstitution {
    pub(crate) direction: BashProcessDirection,
    pub(crate) body: Option<Box<Node>>,
}
