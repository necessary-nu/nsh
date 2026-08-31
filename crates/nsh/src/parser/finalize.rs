//! Finalization at parser boundaries.
//!
//! Here-document bodies are read at a grammar newline, after their parsed
//! redirections have been placed in the syntax tree. This module joins those
//! two source-ordered streams before any tree leaves the parser.

use std::collections::VecDeque;

use crate::context::Shell;
use crate::error::Error;
use crate::nodes::{Node, Redirection, WordNode};

use super::ParseResult;

// [spec:nsh:req:idiom.immutable-ast]
pub(super) fn parse_result(
    shell: &mut Shell,
    result: &mut ParseResult,
    bodies: Vec<WordNode>,
) -> Result<(), Error> {
    let mut bodies = VecDeque::from(bodies);
    if let ParseResult::Tree(Some(node)) = result {
        here_documents(node, &mut bodies).map_err(|()| {
            shell
                .diagnostics()
                .shell_error(b"parsed here-document redirection has no body")
        })?;
    }
    if bodies.is_empty() {
        Ok(())
    } else {
        Err(shell
            .diagnostics()
            .shell_error(b"parsed here-document body has no redirection"))
    }
}

pub(super) fn node(
    shell: &mut Shell,
    node: &mut Option<Node>,
    completed_at: usize,
) -> Result<(), Error> {
    let bodies = shell.input.completed_here_documents.split_off(completed_at);
    let mut result = ParseResult::Tree(node.take());
    parse_result(shell, &mut result, bodies)?;
    *node = result.into_node();
    Ok(())
}

/* A list's chain leans left and is as deep as the line is long, so walking
 * into it by recursion would spend a frame per element on a tree the parser
 * built without recursing at all. The spine is unwound here and its elements
 * are visited in source order, which is the order the bodies queued in. */
fn here_documents(node: &mut Node, bodies: &mut VecDeque<WordNode>) -> Result<(), ()> {
    let mut spine = vec![node];
    while let Some(element) = spine.pop() {
        match element.split_binary() {
            /* Left last, so it comes back off first: the bodies queued in
             * source order and are handed out in the order they are asked
             * for. */
            Ok((left, right)) => {
                spine.push(right);
                spine.push(left);
            }
            Err(element) => here_document(element, bodies)?,
        }
    }
    Ok(())
}

fn here_document(node: &mut Node, bodies: &mut VecDeque<WordNode>) -> Result<(), ()> {
    match node {
        Node::Command(command) => redirections(&mut command.redirections, bodies)?,
        Node::Pipeline(pipeline) => {
            for command in &mut pipeline.commands {
                here_documents(command, bodies)?;
            }
        }
        Node::Redirect(command)
        | Node::Background(command)
        | Node::Subshell(command)
        | Node::Group(command) => {
            here_documents(&mut command.command, bodies)?;
            redirections(&mut command.redirections, bodies)?;
        }
        // The caller unwinds a chain before visiting anything, so a binary
        // form never arrives here whole.
        Node::And(_) | Node::Or(_) | Node::Sequence(_) | Node::While(_) | Node::Until(_) => {}
        Node::If(command) => {
            here_documents(&mut command.condition, bodies)?;
            here_documents(&mut command.then_branch, bodies)?;
            if let Some(else_branch) = &mut command.else_branch {
                here_documents(else_branch, bodies)?;
            }
        }
        Node::For(command) | Node::Select(command) => here_documents(&mut command.body, bodies)?,
        Node::Timed(command) => {
            if let Some(inner) = command.command.as_mut() {
                here_documents(inner, bodies)?;
            }
        }
        Node::Case(command) => {
            for clause in &mut command.clauses {
                if let Some(body) = &mut clause.body {
                    here_documents(body, bodies)?;
                }
            }
        }
        Node::Function(function) => here_documents(&mut function.body, bodies)?,
        Node::Not(command) => here_documents(&mut command.command, bodies)?,
        Node::Bash(bash) => match bash {
            crate::nodes::BashNode::ArithmeticFor(command) => {
                here_documents(&mut command.body, bodies)?
            }
            crate::nodes::BashNode::Function(function) => {
                here_documents(&mut function.body, bodies)?
            }
            _ => {}
        },
        Node::Word(_) => {}
    }
    Ok(())
}

fn redirections(
    redirections: &mut [Redirection],
    bodies: &mut VecDeque<WordNode>,
) -> Result<(), ()> {
    for redirection in redirections {
        if let Redirection::HereDocument(document) = redirection {
            document.body = bodies.pop_front().ok_or(())?;
        }
    }
    Ok(())
}
