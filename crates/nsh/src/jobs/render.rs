//! Writing a command back out for the job table.
//!
//! A second printer, and deliberately not the one in `nodes::source`:
//! that one reproduces the source byte-exactly because
//! `[dec:nsh:no-equivalent-forms]` requires it, and this one produces the
//! short single-line description `jobs` prints, where losing the original
//! spelling is the point.

use super::*;

/*
 * Return a string identifying a command (to be printed by the
 * jobs command).
 */

// [spec:dash:sem:jobs.commandtext-fn]
// [spec:posix:req:builtin.jobs.stdout-default-format]
// [spec:nsh:sem:idiom.specified-defects+1]
pub(super) fn render_command(node: &Node) -> BString {
    let mut text = BString::new(Vec::new());
    render_node(Some(node), &mut text);
    text
}

// [spec:dash:sem:jobs.cmdtxt-fn]
// [spec:nsh:req:idiom.structural-ast]
fn render_node(node: Option<&Node>, text: &mut BString) {
    let Some(node) = node else { return };
    match node {
        Node::Sequence(binary) => render_binary_command(binary, b"; ", text),
        Node::And(binary) => render_binary_command(binary, b" && ", text),
        Node::Or(binary) => render_binary_command(binary, b" || ", text),
        Node::Redirect(command) => {
            render_node(Some(command.command.as_ref()), text);
            render_redirections(&command.redirections, text);
        }
        Node::Background(command) => {
            render_node(Some(command.command.as_ref()), text);
        }
        Node::Not(command) => {
            push_command_text(b"!", text);
            render_node(Some(command.command.as_ref()), text);
        }
        Node::If(command) => {
            push_command_text(b"if ", text);
            render_node(Some(command.condition.as_ref()), text);
            push_command_text(b"; then ", text);
            render_node(Some(command.then_branch.as_ref()), text);
            if command.else_branch.is_some() {
                push_command_text(b"; else ", text);
                render_node(command.else_branch.as_deref(), text);
            }
            push_command_text(b"; fi", text);
        }
        Node::Subshell(command) => {
            push_command_text(b"(", text);
            render_node(Some(command.command.as_ref()), text);
            push_command_text(b")", text);
            render_redirections(&command.redirections, text);
        }
        Node::Group(command) => {
            push_command_text(b"{ ", text);
            render_node(Some(command.command.as_ref()), text);
            push_command_text(b"; }", text);
            render_redirections(&command.redirections, text);
        }
        Node::While(command) | Node::Until(command) => {
            push_command_text(
                if matches!(node, Node::While(_)) {
                    b"while "
                } else {
                    b"until "
                },
                text,
            );
            render_node(Some(command.left.as_ref()), text);
            push_command_text(b"; do ", text);
            render_node(Some(command.right.as_ref()), text);
            push_command_text(b"; done", text);
        }
        Node::Timed(command) => {
            push_command_text(b"time ", text);
            if command.posix_format {
                push_command_text(b"-p ", text);
            }
            render_node(command.command.as_deref(), text);
        }
        Node::Select(command) => {
            push_command_text(b"select ", text);
            push_command_text(command.variable.as_bstr(), text);
            push_command_text(b" in ", text);
            render_command_list(&command.words, true, text);
            push_command_text(b"; do ", text);
            render_node(Some(command.body.as_ref()), text);
            push_command_text(b"; done", text);
        }
        Node::For(command) => {
            push_command_text(b"for ", text);
            push_command_text(command.variable.as_bstr(), text);
            push_command_text(b" in ", text);
            render_command_list(&command.words, true, text);
            push_command_text(b"; do ", text);
            render_node(Some(command.body.as_ref()), text);
            push_command_text(b"; done", text);
        }
        Node::Function(function) => {
            push_command_text(function.name.as_bstr(), text);
            push_command_text(b"() { ... }", text);
        }
        Node::Command(command) => {
            render_command_list(&command.assignments, true, text);
            if !command.assignments.is_empty() && !command.arguments.is_empty() {
                push_command_text(b" ", text);
            }
            render_command_list(&command.arguments, true, text);
            render_redirections(&command.redirections, text);
        }
        Node::Word(word) => word.word.render(text),
        Node::Case(command) => {
            push_command_text(b"case ", text);
            render_node(Some(command.word.as_ref()), text);
            push_command_text(b" in ", text);
            for clause in &command.clauses {
                for (index, pattern) in clause.patterns.iter().enumerate() {
                    if index != 0 {
                        push_command_text(b"|", text);
                    }
                    render_node(Some(pattern), text);
                }
                push_command_text(b") ", text);
                render_node(clause.body.as_deref(), text);
                push_command_text(if clause.fallthrough { b";& " } else { b";; " }, text);
            }
            push_command_text(b"esac", text);
        }
        Node::Pipeline(pipeline) => {
            for (index, command) in pipeline.commands.iter().enumerate() {
                if index != 0 {
                    push_command_text(b" | ", text);
                }
                render_node(Some(command), text);
            }
        }
        Node::Bash(_) => push_command_text(b"<bash syntax>", text),
    }
}

fn render_binary_command(
    command: &crate::nodes::BinaryCommand,
    separator: &[u8],
    text: &mut BString,
) {
    render_node(Some(command.left.as_ref()), text);
    push_command_text(separator, text);
    render_node(Some(command.right.as_ref()), text);
}

// [spec:dash:sem:jobs.cmdlist-fn]
fn render_command_list(nodes: &[Node], space_between: bool, text: &mut BString) {
    for (index, node) in nodes.iter().enumerate() {
        if !space_between {
            push_command_text(b" ", text);
        }
        render_node(Some(node), text);
        if space_between && index + 1 < nodes.len() {
            push_command_text(b" ", text);
        }
    }
}

fn render_redirections(redirections: &[Redirection], text: &mut BString) {
    for redirection in redirections {
        push_command_text(b" ", text);
        match redirection {
            Redirection::File(redirection) if redirection.with_stderr => {
                /* `&>` and `&>>` name no slot: the ampersand is where the
                 * number would go, so the descriptor the parser stored is
                 * the operator's own default and printing it would spell a
                 * form Bash does not have. */
                // [spec:nsh:req:compat.bash.expansion-globbing]
                push_command_text(
                    match redirection.operator {
                        FileRedirectionOperator::Append => b"&>>",
                        _ => b"&>",
                    },
                    text,
                );
                redirection.target.word.render(text);
            }
            Redirection::File(redirection) => {
                push_command_text(&redirection.descriptor.text(), text);
                push_command_text(
                    match redirection.operator {
                        FileRedirectionOperator::Write => b">",
                        FileRedirectionOperator::Clobber => b">|",
                        FileRedirectionOperator::Read => b"<",
                        FileRedirectionOperator::ReadWrite => b"<>",
                        FileRedirectionOperator::Append => b">>",
                    },
                    text,
                );
                redirection.target.word.render(text);
            }
            Redirection::Descriptor(redirection) => {
                push_command_text(&redirection.descriptor.text(), text);
                push_command_text(
                    match redirection.operator {
                        DescriptorRedirectionOperator::Input => b"<&",
                        DescriptorRedirectionOperator::Output => b">&",
                    },
                    text,
                );
                match &redirection.target {
                    DescriptorTarget::Number(descriptor) => {
                        push_command_text(&descriptor.digits(), text)
                    }
                    DescriptorTarget::Close => push_command_text(b"-", text),
                    DescriptorTarget::Word(word) => word.word.render(text),
                }
            }
            Redirection::HereDocument(_) => push_command_text(b"<<...", text),
            Redirection::HereString(here) => {
                push_command_text(&here.descriptor.text(), text);
                push_command_text(b"<<<", text);
                here.word.word.render(text);
            }
        }
    }
}

// [spec:dash:sem:jobs.cmdputs-fn]
fn push_command_text(s: &[u8], text: &mut BString) {
    for &byte in s {
        if matches!(byte, b'\'' | b'\\' | b'"' | b'$') {
            text.push(b'\\');
        }
        text.push(byte);
    }
    /* The C leaves an unadvanced `*nextc = '\0'` for `commandtext` to
     * read as the end of the text. The length is that. */
}

// [spec:dash:sem:jobs.showpipe-fn]
pub(crate) fn write_pipeline(
    shell: &mut crate::context::Shell,
    job_id: JobId,
    destination: OutputDestination,
) -> Result<(), Error> {
    let process_count: usize = shell.jobs[job_id].processes.len();

    for process_index in 1..process_count {
        shell.write_output(destination, b" | ")?;
        write_command_text(shell, job_id, process_index, destination)?;
    }
    shell.write_output(destination, b"\n")?;
    shell.flush_output()
}
