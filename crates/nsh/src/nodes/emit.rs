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

/// Write `node` as the source it was parsed from.
///
/// `None` when the node was built rather than read, which is the caller's
/// signal to spell it instead.
// [spec:nsh:req:idiom.printable-ast+2]
pub(crate) fn emitted(node: &Node) -> Option<BString> {
    let mut out = node.tokens().written();
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
/// Their bodies are the one part of a subtree that its own run does not
/// contain, so this is the only walk emission needs.
// [spec:nsh:req:idiom.printable-ast+2]
fn here_documents<'a>(node: &'a Node, into: &mut Vec<&'a HereDocument>) {
    match node {
        Node::Command(command) => {
            for argument in command.assignments.iter().chain(&command.arguments) {
                here_documents(argument, into);
            }
            redirections(&command.redirections, into);
        }
        Node::Pipeline(pipeline) => {
            for command in &pipeline.commands {
                here_documents(command, into);
            }
        }
        Node::Redirect(wrapper)
        | Node::Background(wrapper)
        | Node::Subshell(wrapper)
        | Node::Group(wrapper) => {
            here_documents(&wrapper.command, into);
            redirections(&wrapper.redirections, into);
        }
        Node::And(binary)
        | Node::Or(binary)
        | Node::Sequence(binary)
        | Node::While(binary)
        | Node::Until(binary) => {
            here_documents(&binary.left, into);
            here_documents(&binary.right, into);
        }
        Node::If(command) => {
            here_documents(&command.condition, into);
            here_documents(&command.then_branch, into);
            if let Some(branch) = &command.else_branch {
                here_documents(branch, into);
            }
        }
        Node::For(command) | Node::Select(command) => {
            for word in &command.words {
                here_documents(word, into);
            }
            here_documents(&command.body, into);
        }
        Node::Timed(command) => {
            if let Some(command) = &command.command {
                here_documents(command, into);
            }
        }
        Node::Case(command) => {
            here_documents(&command.word, into);
            for clause in &command.clauses {
                for pattern in &clause.patterns {
                    here_documents(pattern, into);
                }
                if let Some(body) = &clause.body {
                    here_documents(body, into);
                }
            }
        }
        Node::Function(definition) => here_documents(&definition.body, into),
        Node::Not(negation) => here_documents(&negation.command, into),
        Node::Bash(BashNode::ArithmeticFor(command)) => here_documents(&command.body, into),
        Node::Bash(BashNode::Function(function)) => here_documents(&function.body, into),
        Node::Bash(BashNode::ProcessSubstitution(substitution)) => {
            if let Some(body) = &substitution.body {
                here_documents(body, into);
            }
        }
        /* A word's own run holds whatever was written inside it, command
         * substitutions included, so there is nothing under one to
         * collect. The remaining Bash nodes hold no redirection. */
        Node::Word(_)
        | Node::Bash(
            BashNode::Conditional(_)
            | BashNode::ArithmeticCommand(_)
            | BashNode::ArrayAssignment(_),
        ) => {}
    }
}

fn redirections<'a>(list: &'a [Redirection], into: &mut Vec<&'a HereDocument>) {
    for redirection in list {
        if let Redirection::HereDocument(document) = redirection {
            into.push(document);
        }
    }
}
