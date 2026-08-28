//! Writing a parsed tree back as the bytes it was read from.
//!
//! This is the whole renderer for anything the parser built. A node
//! carries the run of tokens it was read as, so writing one is emitting
//! that run: there is no table of bytes here, no quoting context, and no
//! second opinion about what a byte meant, because the reader's opinion
//! is what is being written out.
//!
//! It is a separate module from [`super::source`] on purpose. That one
//! spells constructs, which is the only thing left that can invent, and
//! the two should not be reachable from inside each other by accident.
//!
//! ONE EXCEPTION, AND IT IS STRUCTURAL. A here-document's body is read at
//! the newline that ends the redirection's line, not where the
//! redirection is. Whether that newline falls inside the node being
//! written is what decides whether its run already holds the body: a
//! top-level `cat <<EOF` ends before it and does not, while `{ cat <<EOF`
//! reaches it inside the braces and does. So the bodies are collected
//! from the subtree, and each is written after the line that promised it
//! only if the run does not already hold it -- which is also the order
//! the shell read them in.

use bstr::BString;

use super::{BashNode, HereDocument, Node, Redirection};

/// Write `node` as the source it was parsed from, every byte of it.
///
/// Including the trivia the node was reached through, because that is
/// part of what was read and a byte comparison against the source has to
/// account for it.
///
/// `None` when the node was built rather than read, which is the caller's
/// signal to spell it instead.
///
/// A renderer laying out its own whitespace wants the same bytes without
/// the trivia in front of them, and asks [`SourceTokens::written`] how
/// many of them that is rather than being given a second function here.
// [spec:nsh:req:idiom.printable-ast+2]
pub(crate) fn emitted(node: &Node) -> Option<BString> {
    let mut out = node.tokens().text();
    if out.is_empty() {
        return None;
    }
    let mut bodies = Vec::new();
    here_documents(node, &mut bodies);
    let run = node.tokens();
    for body in bodies {
        /* A body whose newline fell inside this node was read inside its
         * run and is already written. */
        if run.holds(&body.body.tokens) {
            continue;
        }
        if out.last() != Some(&b'\n') {
            out.push(b'\n');
        }
        out.extend_from_slice(&body.body.tokens.text());
    }
    Some(out)
}

/// Collect every here-document in `node`, in the order they were read.
///
/// Their bodies are the one part of a subtree that its own run may not
/// contain, so this is the only walk emission needs.
// [spec:nsh:req:idiom.printable-ast+2]
fn here_documents<'a>(node: &'a Node, into: &mut Vec<&'a HereDocument>) {
    for redirection in redirections(node) {
        if let Redirection::HereDocument(document) = redirection {
            into.push(document);
        }
    }
    for child in children(node) {
        here_documents(child, into);
    }
}

/// The first node whose run is not inside the run of the node above it.
///
/// Runs nest by construction, so this asks whether the marks into the log
/// agree with the tree they were taken for. A property over the bytes
/// cannot see that: the log can be complete while the indices into it are
/// wrong, which is how three defects reached a shipped renderer.
///
/// A here-document's body is exempt, because whether its run is inside
/// its command's depends on where the newline that ended the line fell.
// [spec:nsh:req:idiom.printable-ast+2]
#[cfg(feature = "fuzzing")]
pub(crate) fn misplaced_run(node: &Node) -> Option<(super::SourceTokens, super::SourceTokens)> {
    let run = node.tokens();
    for child in children(node) {
        let inside = child.tokens();
        if !run.is_empty() && !inside.is_empty() && !run.holds(inside) {
            return Some((run.clone(), inside.clone()));
        }
        if let Some(found) = misplaced_run(child) {
            return Some(found);
        }
    }
    None
}

/// The redirections attached to `node`, which only two shapes carry.
fn redirections(node: &Node) -> &[Redirection] {
    match node {
        Node::Command(command) => &command.redirections,
        Node::Redirect(wrapper)
        | Node::Background(wrapper)
        | Node::Subshell(wrapper)
        | Node::Group(wrapper) => &wrapper.redirections,
        _ => &[],
    }
}

/// The nodes directly under `node`, in the order they were read.
///
/// A word is a leaf here: whatever was written inside it, command
/// substitutions included, is in its own run.
// [spec:nsh:req:idiom.printable-ast+2]
fn children(node: &Node) -> Vec<&Node> {
    match node {
        Node::Command(command) => command
            .assignments
            .iter()
            .chain(&command.arguments)
            .collect(),
        Node::Pipeline(pipeline) => pipeline.commands.iter().collect(),
        Node::Redirect(wrapper)
        | Node::Background(wrapper)
        | Node::Subshell(wrapper)
        | Node::Group(wrapper) => vec![wrapper.command.as_ref()],
        Node::And(binary)
        | Node::Or(binary)
        | Node::Sequence(binary)
        | Node::While(binary)
        | Node::Until(binary) => vec![binary.left.as_ref(), binary.right.as_ref()],
        Node::If(command) => {
            let mut under = vec![command.condition.as_ref(), command.then_branch.as_ref()];
            under.extend(command.else_branch.as_deref());
            under
        }
        Node::For(command) | Node::Select(command) => {
            let mut under: Vec<&Node> = command.words.iter().collect();
            under.push(command.body.as_ref());
            under
        }
        Node::Timed(command) => command.command.as_deref().into_iter().collect(),
        Node::Case(command) => {
            let mut under = vec![command.word.as_ref()];
            for clause in &command.clauses {
                under.extend(clause.patterns.iter());
                under.extend(clause.body.as_deref());
            }
            under
        }
        Node::Function(definition) => vec![definition.body.as_ref()],
        Node::Not(negation) => vec![negation.command.as_ref()],
        Node::Bash(BashNode::ArithmeticFor(command)) => vec![command.body.as_ref()],
        Node::Bash(BashNode::Function(function)) => vec![function.body.as_ref()],
        Node::Bash(BashNode::ProcessSubstitution(substitution)) => {
            substitution.body.as_deref().into_iter().collect()
        }
        Node::Word(_)
        | Node::Bash(
            BashNode::Conditional(_)
            | BashNode::ArithmeticCommand(_)
            | BashNode::ArrayAssignment(_),
        ) => Vec::new(),
    }
}
