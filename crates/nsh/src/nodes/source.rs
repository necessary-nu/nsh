//! Printing an owned function definition back as shell source.
//!
//! `declare -f`, `typeset -f` and `type -a` have to show a function's
//! body, and this is where that text comes from. Bash keeps every word's
//! original spelling and re-indents the grammar around it; the tree here
//! keeps structure rather than bytes, so the words are re-spelled from
//! their parts and the grammar is printed in Bash's canonical layout.
//!
//! Nothing in this module re-enters the parser.
//! [dec:nsh:safety-trumps-compatibility] is explicit that introspection
//! must not turn stored program text back into syntax: a renderer walks
//! the tree it was given and writes bytes, where a round trip through
//! the parser would instead give a caller a way to have the shell re-read
//! whatever a definition happened to contain. So this is a printer, and
//! only a printer.

use bstr::{BStr, BString};

use super::{
    BashArithmeticFor, BashArrayAssignment, BashArrayElement, BashArrayValue,
    BashAssignmentOperator, BashConditionalExpr, BashNode, BashProcessDirection,
    BashProcessSubstitution, BinaryCommand, CaseCommand, CompoundCommand, DescriptorRedirection,
    DescriptorRedirectionOperator, DescriptorTarget, FileRedirection, FileRedirectionOperator,
    ForCommand, HereDocument, IfCommand, Node, Pipeline, Redirection, SimpleCommand, WordNode,
};
use crate::word::{ParameterExpansion, ParameterOperation, ParsedWord, QuoteBoundary, WordPart};

/// Bash indents a printed body by four columns per level.
const STEP: usize = 4;

/// The base name a synthesised here-document delimiter starts from.
const HERE_DELIMITER: &[u8] = b"EOF";

/// Which quoting rules apply to the word being written.
///
/// A word is not one language: the operand of `${x:-...}` inside double
/// quotes may not open a quote of its own, an arithmetic expression is
/// already a quoting context, and a here-document body is protected by
/// its delimiter rather than by quotes.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum Quoting {
    /// An ordinary command word, free to open its own quotes.
    Word,
    /// Inside a `"` this printer opened.
    Double,
    /// Inside `$(( ))`.
    Arithmetic,
    /// A here-document body that still expands.
    HereDocument,
}

/// Print `name`'s definition the way `declare -f` does.
pub(crate) fn function_definition(name: &BStr, body: &Node) -> BString {
    let mut printer = Printer::new();
    printer.out.extend_from_slice(name);
    printer.out.extend_from_slice(b" () \n{ ");
    printer.newline(STEP);
    printer.list(body, STEP);
    printer.newline(0);
    printer.out.push(b'}');
    printer.out
}

/// The output buffer and the here-document bodies owed to the next line.
struct Printer {
    out: BString,
    /// Bodies that must be written at column zero after the next newline,
    /// each already terminated by its own delimiter line.
    pending: Vec<BString>,
}

impl Printer {
    const fn new() -> Self {
        Self {
            out: BString::new(Vec::new()),
            pending: Vec::new(),
        }
    }

    /// End the line, pay out any here-document bodies it promised, and
    /// indent the next one.
    fn newline(&mut self, indent: usize) {
        self.out.push(b'\n');
        for body in core::mem::take(&mut self.pending) {
            self.out.extend_from_slice(&body);
        }
        self.out.extend(core::iter::repeat_n(b' ', indent));
    }

    /// Whether the text so far ends in an `&`, which is its own
    /// separator and must not collect a `;` as well.
    fn ends_asynchronously(&self) -> bool {
        self.out.last() == Some(&b'&')
    }

    // -----------------------------------------------------------------
    // lists
    // -----------------------------------------------------------------

    /// Write a `;`-separated list, one command per line.
    fn list(&mut self, node: &Node, indent: usize) {
        let mut items = Vec::new();
        flatten(node, &mut items);
        let last = items.len().saturating_sub(1);
        for (position, item) in items.into_iter().enumerate() {
            self.command(item, indent);
            if position != last {
                if !self.ends_asynchronously() {
                    self.out.push(b';');
                }
                self.newline(indent);
            }
        }
    }

    /// A list in a position where Bash terminates the last command too:
    /// the body of `if`, `while`, `until` and `for`.
    fn terminated_list(&mut self, node: &Node, indent: usize) {
        self.list(node, indent);
        if !self.ends_asynchronously() {
            self.out.push(b';');
        }
    }

    /// A list that a `{ ... }` has to hold together.
    fn brace_group(&mut self, node: &Node, indent: usize) {
        self.out.extend_from_slice(b"{ ");
        self.newline(indent + STEP);
        self.list(node, indent + STEP);
        self.newline(indent);
        self.out.push(b'}');
    }

    /// A redirected command, braced only when the operator would
    /// otherwise bind to the wrong part of it.
    fn grouped(&mut self, node: &Node, indent: usize) {
        if needs_braces(node) {
            self.brace_group(node, indent);
        } else {
            self.command(node, indent);
        }
    }

    // -----------------------------------------------------------------
    // commands
    // -----------------------------------------------------------------

    fn command(&mut self, node: &Node, indent: usize) {
        match node {
            Node::Command(command) => self.simple_command(command, indent),
            Node::Pipeline(pipeline) => self.pipeline(pipeline, indent),
            Node::Redirect(command) => {
                self.grouped(&command.command, indent);
                self.redirections(&command.redirections, indent);
            }
            Node::Background(command) => {
                self.command(&command.command, indent);
                self.redirections(&command.redirections, indent);
                self.out.extend_from_slice(b" &");
            }
            Node::Subshell(command) => self.subshell(command, indent),
            Node::And(command) => self.binary(command, b" && ", indent),
            Node::Or(command) => self.binary(command, b" || ", indent),
            Node::Sequence(_) => self.list(node, indent),
            Node::If(command) => self.if_command(command, indent),
            Node::While(command) => self.loop_command(b"while", command, indent),
            Node::Until(command) => self.loop_command(b"until", command, indent),
            Node::For(command) => self.for_command(command, indent),
            Node::Case(command) => self.case_command(command, indent),
            Node::Function(definition) => {
                self.nested_function(definition.name.as_bstr(), &definition.body, indent);
            }
            Node::Word(word) => self.word(word, indent),
            Node::Not(command) => {
                self.out.extend_from_slice(b"! ");
                self.command(&command.command, indent);
            }
            Node::Bash(command) => self.bash_command(command, indent),
        }
    }

    fn simple_command(&mut self, command: &SimpleCommand, indent: usize) {
        let mut written = false;
        for part in command.assignments.iter().chain(command.arguments.iter()) {
            if written {
                self.out.push(b' ');
            }
            written = true;
            self.command(part, indent);
        }
        self.redirections(&command.redirections, indent);
    }

    fn pipeline(&mut self, pipeline: &Pipeline, indent: usize) {
        for (position, command) in pipeline.commands.iter().enumerate() {
            if position > 0 {
                self.out.extend_from_slice(b" | ");
            }
            self.command(command, indent);
        }
        if pipeline.background {
            self.out.extend_from_slice(b" &");
        }
    }

    fn binary(&mut self, command: &BinaryCommand, operator: &[u8], indent: usize) {
        self.command(&command.left, indent);
        self.out.extend_from_slice(operator);
        self.command(&command.right, indent);
    }

    fn subshell(&mut self, command: &CompoundCommand, indent: usize) {
        self.out.extend_from_slice(b"( ");
        self.list(&command.command, indent);
        self.out.extend_from_slice(b" )");
        self.redirections(&command.redirections, indent);
    }

    fn if_command(&mut self, command: &IfCommand, indent: usize) {
        self.out.extend_from_slice(b"if ");
        self.list(&command.condition, indent);
        self.out.extend_from_slice(b"; then");
        self.newline(indent + STEP);
        self.terminated_list(&command.then_branch, indent + STEP);
        if let Some(branch) = &command.else_branch {
            self.newline(indent);
            self.out.extend_from_slice(b"else");
            self.newline(indent + STEP);
            self.terminated_list(branch, indent + STEP);
        }
        self.newline(indent);
        self.out.extend_from_slice(b"fi");
    }

    fn loop_command(&mut self, keyword: &[u8], command: &BinaryCommand, indent: usize) {
        self.out.extend_from_slice(keyword);
        self.out.push(b' ');
        self.list(&command.left, indent);
        self.out.extend_from_slice(b"; do");
        self.newline(indent + STEP);
        self.terminated_list(&command.right, indent + STEP);
        self.newline(indent);
        self.out.extend_from_slice(b"done");
    }

    fn for_command(&mut self, command: &ForCommand, indent: usize) {
        self.out.extend_from_slice(b"for ");
        self.out.extend_from_slice(command.variable.as_bstr());
        if !command.words.is_empty() {
            self.out.extend_from_slice(b" in");
            for word in &command.words {
                self.out.push(b' ');
                self.command(word, indent);
            }
        }
        self.out.push(b';');
        self.newline(indent);
        self.out.extend_from_slice(b"do");
        self.newline(indent + STEP);
        self.terminated_list(&command.body, indent + STEP);
        self.newline(indent);
        self.out.extend_from_slice(b"done");
    }

    fn case_command(&mut self, command: &CaseCommand, indent: usize) {
        self.out.extend_from_slice(b"case ");
        self.command(&command.word, indent);
        self.out.extend_from_slice(b" in ");
        for clause in &command.clauses {
            self.newline(indent + STEP);
            for (position, pattern) in clause.patterns.iter().enumerate() {
                if position > 0 {
                    self.out.extend_from_slice(b" | ");
                }
                self.command(pattern, indent + STEP);
            }
            self.out.push(b')');
            self.newline(indent + STEP * 2);
            if let Some(body) = &clause.body {
                self.list(body, indent + STEP * 2);
            }
            self.newline(indent + STEP);
            self.out
                .extend_from_slice(if clause.fallthrough { b";&" } else { b";;" });
        }
        self.newline(indent);
        self.out.extend_from_slice(b"esac");
    }

    /// A definition nested inside another body. Bash prints the reserved
    /// word here even when the source did not use it.
    fn nested_function(&mut self, name: &BStr, body: &Node, indent: usize) {
        self.out.extend_from_slice(b"function ");
        self.out.extend_from_slice(name);
        self.out.extend_from_slice(b" () \n");
        self.out.extend(core::iter::repeat_n(b' ', indent));
        self.brace_group(body, indent);
    }

    fn bash_command(&mut self, node: &BashNode, indent: usize) {
        match node {
            BashNode::Conditional(command) => {
                self.out.extend_from_slice(b"[[ ");
                self.conditional(&command.expression, indent);
                self.out.extend_from_slice(b" ]]");
            }
            BashNode::ArithmeticCommand(command) => {
                self.out.extend_from_slice(b"(( ");
                self.out
                    .extend_from_slice(trimmed(command.expression.as_bstr()));
                self.out.extend_from_slice(b" ))");
            }
            BashNode::ArithmeticFor(command) => self.arithmetic_for(command, indent),
            BashNode::Function(function) => {
                self.nested_function(function.name.as_bstr(), &function.body, indent);
            }
            BashNode::ArrayAssignment(assignment) => self.array_assignment(assignment, indent),
            BashNode::ProcessSubstitution(substitution) => {
                self.process_substitution(substitution, indent);
            }
        }
    }

    fn arithmetic_for(&mut self, command: &BashArithmeticFor, indent: usize) {
        self.out.extend_from_slice(b"for ((");
        self.out.extend_from_slice(trimmed(command.init.as_bstr()));
        self.out.extend_from_slice(b"; ");
        self.out.extend_from_slice(trimmed(command.test.as_bstr()));
        self.out.extend_from_slice(b"; ");
        self.out
            .extend_from_slice(trimmed(command.update.as_bstr()));
        self.out.extend_from_slice(b"))");
        self.newline(indent);
        self.out.extend_from_slice(b"do");
        self.newline(indent + STEP);
        self.terminated_list(&command.body, indent + STEP);
        self.newline(indent);
        self.out.extend_from_slice(b"done");
    }

    fn conditional(&mut self, expression: &BashConditionalExpr, indent: usize) {
        match expression {
            BashConditionalExpr::Empty => {}
            BashConditionalExpr::Word(word) => self.word(word, indent),
            BashConditionalExpr::Unary { operator, operand } => {
                self.out.extend_from_slice(operator.as_bstr());
                self.out.push(b' ');
                self.word(operand, indent);
            }
            BashConditionalExpr::Binary {
                left,
                operator,
                right,
            } => {
                self.word(left, indent);
                self.out.push(b' ');
                self.out.extend_from_slice(operator.as_bstr());
                self.out.push(b' ');
                self.word(right, indent);
            }
            BashConditionalExpr::Not(inner) => {
                self.out.extend_from_slice(b"! ");
                self.conditional(inner, indent);
            }
            BashConditionalExpr::And(left, right) => {
                self.conditional(left, indent);
                self.out.extend_from_slice(b" && ");
                self.conditional(right, indent);
            }
            BashConditionalExpr::Or(left, right) => {
                self.conditional(left, indent);
                self.out.extend_from_slice(b" || ");
                self.conditional(right, indent);
            }
            BashConditionalExpr::Group(inner) => {
                self.out.extend_from_slice(b"( ");
                self.conditional(inner, indent);
                self.out.extend_from_slice(b" )");
            }
        }
    }

    fn array_assignment(&mut self, assignment: &BashArrayAssignment, indent: usize) {
        self.out.extend_from_slice(assignment.name.as_bstr());
        if let Some(subscript) = &assignment.subscript {
            self.out.push(b'[');
            self.word(subscript, indent);
            self.out.push(b']');
        }
        self.out
            .extend_from_slice(operator_text(assignment.operator));
        match &assignment.value {
            BashArrayValue::Word(word) => self.word(word, indent),
            BashArrayValue::Compound(elements) => {
                self.out.push(b'(');
                for (position, element) in elements.iter().enumerate() {
                    if position > 0 {
                        self.out.push(b' ');
                    }
                    self.array_element(element, indent);
                }
                self.out.push(b')');
            }
        }
    }

    fn array_element(&mut self, element: &BashArrayElement, indent: usize) {
        if let Some(subscript) = &element.subscript {
            self.out.push(b'[');
            self.word(subscript, indent);
            self.out.push(b']');
            self.out.extend_from_slice(operator_text(element.operator));
        }
        self.word(&element.value, indent);
    }

    fn process_substitution(&mut self, node: &BashProcessSubstitution, indent: usize) {
        self.out.extend_from_slice(match node.direction {
            BashProcessDirection::Input => b"<(",
            BashProcessDirection::Output => b">(",
        });
        if let Some(body) = &node.body {
            self.list(body, indent);
        }
        self.out.push(b')');
    }

    // -----------------------------------------------------------------
    // redirections
    // -----------------------------------------------------------------

    fn redirections(&mut self, redirections: &[Redirection], indent: usize) {
        for redirection in redirections {
            self.out.push(b' ');
            match redirection {
                Redirection::File(file) => self.file_redirection(file, indent),
                Redirection::Descriptor(descriptor) => self.descriptor_redirection(descriptor),
                Redirection::HereDocument(document) => self.here_document(document, indent),
            }
        }
    }

    fn file_redirection(&mut self, redirection: &FileRedirection, indent: usize) {
        let (operator, default): (&[u8], usize) = match redirection.operator {
            FileRedirectionOperator::Write => (b">", 1),
            FileRedirectionOperator::Clobber => (b">|", 1),
            FileRedirectionOperator::Read => (b"<", 0),
            FileRedirectionOperator::ReadWrite => (b"<>", 0),
            FileRedirectionOperator::Append => (b">>", 1),
        };
        self.descriptor_prefix(redirection.descriptor.index(), default);
        self.out.extend_from_slice(operator);
        self.out.push(b' ');
        self.word(&redirection.target, indent);
    }

    fn descriptor_redirection(&mut self, redirection: &DescriptorRedirection) {
        let (operator, default): (&[u8], usize) = match redirection.operator {
            DescriptorRedirectionOperator::Input => (b"<&", 0),
            DescriptorRedirectionOperator::Output => (b">&", 1),
        };
        self.descriptor_prefix(redirection.descriptor.index(), default);
        self.out.extend_from_slice(operator);
        match &redirection.target {
            DescriptorTarget::Number(number) => {
                self.out
                    .extend_from_slice(number.index().to_string().as_bytes());
            }
            DescriptorTarget::Close => self.out.push(b'-'),
            DescriptorTarget::Word(word) => self.word(word, 0),
        }
    }

    /// Write the descriptor number, unless the operator already implies it.
    fn descriptor_prefix(&mut self, descriptor: usize, default: usize) {
        if descriptor != default {
            self.out
                .extend_from_slice(descriptor.to_string().as_bytes());
        }
    }

    /// Write `<<DELIM` and queue the body for the end of the line.
    ///
    /// The tree keeps the body but not the delimiter the source spelled,
    /// so one is chosen that no line of the body can be mistaken for.
    fn here_document(&mut self, document: &HereDocument, indent: usize) {
        let mut body = Self::new();
        if document.expand {
            body.parsed_word(&document.body.word, Quoting::HereDocument, indent);
        } else {
            body.out.extend_from_slice(document.body.word.as_bstr());
        }
        let mut body = body.out;
        if !body.is_empty() && body.last() != Some(&b'\n') {
            body.push(b'\n');
        }
        let delimiter = unused_delimiter(&body);

        self.descriptor_prefix(document.descriptor.index(), 0);
        self.out.extend_from_slice(b"<<");
        if document.expand {
            self.out.extend_from_slice(&delimiter);
        } else {
            self.out.push(b'\'');
            self.out.extend_from_slice(&delimiter);
            self.out.push(b'\'');
        }

        body.extend_from_slice(&delimiter);
        body.push(b'\n');
        self.pending.push(body);
    }

    // -----------------------------------------------------------------
    // words
    // -----------------------------------------------------------------

    fn word(&mut self, word: &WordNode, indent: usize) {
        self.parsed_word(&word.word, Quoting::Word, indent);
    }

    fn parsed_word(&mut self, word: &ParsedWord, quoting: Quoting, indent: usize) {
        let parts = word.parts();
        if parts.is_empty() {
            if quoting == Quoting::Word {
                self.out.extend_from_slice(b"''");
            }
            return;
        }
        let mut at = 0;
        while at < parts.len() {
            if quoting == Quoting::Word && matches!(parts[at], WordPart::Quote(QuoteBoundary::Open))
            {
                let end = closing_quote(parts, at + 1);
                self.quoted_region(&parts[at + 1..end], indent);
                at = end.saturating_add(1);
                continue;
            }
            self.part(&parts[at], parts.get(at + 1), quoting, indent);
            at += 1;
        }
    }

    /// Write one part in whichever quoting the caller has already opened.
    ///
    /// `next` is the part that will follow it, which decides whether a
    /// parameter may be written as `$name` or needs `${name}`.
    fn part(&mut self, part: &WordPart, next: Option<&WordPart>, quoting: Quoting, indent: usize) {
        match part {
            WordPart::Literal(bytes) => self.literal(bytes, quoting),
            WordPart::Multibyte { bytes, escaped } => {
                if *escaped && quoting == Quoting::Word {
                    push_single_quoted(&mut self.out, bytes);
                } else {
                    self.literal(bytes, quoting);
                }
            }
            WordPart::Escaped(byte) => self.escaped(*byte, quoting),
            WordPart::Quote(_) => {}
            WordPart::Parameter(parameter) => self.parameter(parameter, next, quoting, indent),
            WordPart::Command(command) => self.command_substitution(command.as_deref(), indent),
            WordPart::Arithmetic(expression) => {
                self.out.extend_from_slice(b"$((");
                self.parsed_word(expression, Quoting::Arithmetic, indent);
                self.out.extend_from_slice(b"))");
            }
        }
    }

    /// Bytes the source left unprotected, or protected by the enclosing
    /// quoting this printer has already opened.
    fn literal(&mut self, bytes: &[u8], quoting: Quoting) {
        let protected: &[u8] = match quoting {
            Quoting::Word | Quoting::Arithmetic => {
                self.out.extend_from_slice(bytes);
                return;
            }
            Quoting::Double => b"\"\\$`",
            Quoting::HereDocument => b"\\$`",
        };
        for &byte in bytes {
            if protected.contains(&byte) {
                self.out.push(b'\\');
            }
            self.out.push(byte);
        }
    }

    /// One byte the source protected with a backslash.
    fn escaped(&mut self, byte: u8, quoting: Quoting) {
        let protected: &[u8] = match quoting {
            Quoting::Word => {
                push_single_quoted(&mut self.out, &[byte]);
                return;
            }
            Quoting::Arithmetic => b"",
            Quoting::Double => b"\"\\$`",
            Quoting::HereDocument => b"\\$`",
        };
        if quoting == Quoting::Arithmetic || protected.contains(&byte) {
            self.out.push(b'\\');
        }
        self.out.push(byte);
    }

    /// One `'...'` or `"..."` run, chosen by what the region holds.
    ///
    /// A region with nothing to expand can be reproduced byte for byte
    /// inside single quotes; one that expands has to keep the expansion
    /// live, so it goes inside double quotes with the four bytes that
    /// mean something there escaped.
    fn quoted_region(&mut self, region: &[WordPart], indent: usize) {
        let expands = region.iter().any(|part| {
            matches!(
                part,
                WordPart::Parameter(_) | WordPart::Command(_) | WordPart::Arithmetic(_)
            )
        });
        if expands {
            self.out.push(b'"');
            for (at, part) in region.iter().enumerate() {
                self.part(part, region.get(at + 1), Quoting::Double, indent);
            }
            self.out.push(b'"');
            return;
        }
        let mut bytes = BString::new(Vec::new());
        for part in region {
            match part {
                WordPart::Literal(text) | WordPart::Multibyte { bytes: text, .. } => {
                    bytes.extend_from_slice(text);
                }
                WordPart::Escaped(byte) => bytes.push(*byte),
                WordPart::Quote(_)
                | WordPart::Parameter(_)
                | WordPart::Command(_)
                | WordPart::Arithmetic(_) => {}
            }
        }
        push_single_quoted(&mut self.out, &bytes);
    }

    fn parameter(
        &mut self,
        parameter: &ParameterExpansion,
        next: Option<&WordPart>,
        quoting: Quoting,
        indent: usize,
    ) {
        if bare_parameter(parameter, next) {
            self.out.push(b'$');
            self.out.extend_from_slice(&parameter.name);
            return;
        }
        self.out.extend_from_slice(b"${");
        if parameter.operation == ParameterOperation::Length {
            self.out.push(b'#');
        }
        if parameter.indirect {
            self.out.push(b'!');
        }
        self.out.extend_from_slice(&parameter.name);
        if parameter.colon {
            self.out.push(b':');
        }
        self.out.extend_from_slice(parameter.operation.operator());
        if let Some(operand) = parameter.operand.as_ref().filter(|word| !word.is_empty()) {
            // Inside a `"` this printer opened, the operand is already
            // protected and may not open a quote of its own.
            let inner = if quoting == Quoting::Double {
                Quoting::Double
            } else {
                Quoting::Word
            };
            self.parsed_word(operand, inner, indent);
        }
        self.out.push(b'}');
    }

    fn command_substitution(&mut self, node: Option<&Node>, indent: usize) {
        if let Some(Node::Bash(BashNode::ProcessSubstitution(substitution))) = node {
            self.process_substitution(substitution, indent);
            return;
        }
        let start = self.out.len();
        self.out.extend_from_slice(b"$(");
        if let Some(node) = node {
            self.list(node, indent);
        }
        // `$((` would read as an arithmetic expansion, so a leading
        // subshell needs a blank between the two parentheses.
        if self.out.get(start + 2) == Some(&b'(') {
            self.out.insert(start + 2, b' ');
        }
        self.out.push(b')');
    }
}

/// Whether `$name` says what `${name}` would.
///
/// Only a plain value expansion can drop its braces, and only when the
/// byte that follows cannot be read as more of the name. The lookahead
/// is deliberately pessimistic about anything above ASCII, because
/// whether such a byte continues a name is the locale's business and
/// this printer has no locale.
fn bare_parameter(parameter: &ParameterExpansion, next: Option<&WordPart>) -> bool {
    if parameter.indirect
        || parameter.colon
        || parameter.operand.is_some()
        || parameter.operation != ParameterOperation::Value
    {
        return false;
    }
    match parameter.name.as_slice() {
        // A special parameter, or one positional digit: neither can
        // swallow the byte after it.
        [byte] if !byte.is_ascii_alphabetic() && *byte != b'_' => true,
        name if name
            .first()
            .is_some_and(|byte| byte.is_ascii_alphabetic() || *byte == b'_')
            && name
                .iter()
                .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_') =>
        {
            !starts_name_byte(next)
        }
        _ => false,
    }
}

/// Whether the next part could continue a name written without braces.
fn starts_name_byte(next: Option<&WordPart>) -> bool {
    let bytes = match next {
        Some(WordPart::Literal(bytes) | WordPart::Multibyte { bytes, .. }) => bytes.as_slice(),
        _ => return false,
    };
    bytes
        .first()
        .is_some_and(|byte| byte.is_ascii_alphanumeric() || *byte == b'_' || *byte >= 0x80)
}

/// A stored expression without the blanks the grammar left around it.
fn trimmed(expression: &[u8]) -> &[u8] {
    let start = expression
        .iter()
        .position(|byte| !byte.is_ascii_whitespace())
        .unwrap_or(expression.len());
    let end = expression
        .iter()
        .rposition(|byte| !byte.is_ascii_whitespace())
        .map_or(start, |at| at + 1);
    &expression[start..end]
}

/// Collect a left-nested `;` chain into the order it was written in.
fn flatten<'a>(node: &'a Node, items: &mut Vec<&'a Node>) {
    if let Node::Sequence(sequence) = node {
        flatten(&sequence.left, items);
        flatten(&sequence.right, items);
    } else {
        items.push(node);
    }
}

/// Whether a redirection would bind to the wrong part of this command
/// unless braces hold it together.
const fn needs_braces(node: &Node) -> bool {
    matches!(
        node,
        Node::Sequence(_)
            | Node::And(_)
            | Node::Or(_)
            | Node::Pipeline(_)
            | Node::Background(_)
            | Node::Not(_)
    )
}

const fn operator_text(operator: BashAssignmentOperator) -> &'static [u8] {
    match operator {
        BashAssignmentOperator::Set => b"=",
        BashAssignmentOperator::Append => b"+=",
    }
}

/// The index of the boundary that closes a quoted run, or the end.
fn closing_quote(parts: &[WordPart], from: usize) -> usize {
    parts[from..]
        .iter()
        .position(|part| matches!(part, WordPart::Quote(QuoteBoundary::Close)))
        .map_or(parts.len(), |offset| from + offset)
}

/// Wrap bytes so that every one of them survives as data.
///
/// Single quotes protect everything except themselves, so the only case
/// to break out of is the quote itself.
fn push_single_quoted(out: &mut BString, bytes: &[u8]) {
    out.push(b'\'');
    for &byte in bytes {
        if byte == b'\'' {
            out.extend_from_slice(b"'\\''");
        } else {
            out.push(byte);
        }
    }
    out.push(b'\'');
}

/// A here-document delimiter that no line of `body` can be mistaken for.
fn unused_delimiter(body: &[u8]) -> BString {
    let mut suffix = 0u32;
    loop {
        let mut candidate = BString::from(HERE_DELIMITER);
        if suffix > 0 {
            candidate.extend_from_slice(suffix.to_string().as_bytes());
        }
        let taken = body
            .split(|byte| *byte == b'\n')
            .any(|line| line == candidate.as_slice());
        if !taken {
            return candidate;
        }
        suffix += 1;
    }
}
