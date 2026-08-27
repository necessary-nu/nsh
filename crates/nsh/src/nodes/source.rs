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
    BashAssignmentOperator, BashConditionalExpr, BashFunctionStyle, BashNode, BashProcessDirection,
    BashProcessSubstitution, BinaryCommand, CaseCommand, CompoundCommand, DescriptorRedirection,
    DescriptorRedirectionOperator, DescriptorTarget, FileRedirection, FileRedirectionOperator,
    ForCommand, HereDocument, HereString, IfCommand, Node, Pipeline, Redirection, SimpleCommand,
    WordNode,
};
use crate::word::{
    ParameterExpansion, ParameterOperation, ParsedWord, QuoteBoundary, QuoteKind, WordPart,
};

/// Bash indents a printed body by four columns per level.
const STEP: usize = 4;

/// How a definition was introduced, which its printed form has to keep.
#[derive(Clone, Copy, Eq, PartialEq)]
enum DefinitionStyle {
    /// `name () compound`.
    Posix,
    /// `function name compound`.
    Keyword,
    /// `function name () compound`.
    KeywordParens,
}

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
    /// Inside the `[...]` of an assignment word, where the brackets make
    /// blanks and shell operators the subscript's own bytes and only the
    /// closing bracket ends it.
    Subscript,
    /// Inside a `${...}` operand that no quoting encloses. The braces make
    /// blanks and shell operators inert, so only the bytes that would end or
    /// reopen the expansion need protecting there.
    Parameter,
    /// Inside a `${...}` operand nested in a `"` this printer opened.
    DoubleParameter,
    /// A pattern or arithmetic `${...}` operand nested in a `"` this printer
    /// opened. The parser reads an apostrophe there as a quote even though
    /// the enclosing double quotes have already made it an ordinary byte.
    DoubleProtectedParameter,
    /// A `${...}` operand inside a `"` that will never expand -- because a
    /// NUL ended the parse inside it, or because the expansion around it is
    /// one the shell refuses. Its bytes are inert text, so the only one that
    /// still matters is the `}` that ends the expansion, and a toggle in it
    /// cannot be written back: the last one would leave the operand quoted at
    /// that `}`.
    DoubleInertParameter,
    /// Inside `$(( ))`.
    Arithmetic,
    /// A here-document body that still expands.
    HereDocument,
    /// A `${...}` operand inside an expanding here-document. Quotes are
    /// syntax here even though they are ordinary bytes in the surrounding body.
    HereDocumentParameter,
    /// A pattern or arithmetic `${...}` operand in an expanding here-document,
    /// where an apostrophe must be protected from the parameter grammar.
    HereDocumentProtectedParameter,
}

/// Print `name`'s definition the way `declare -f` does.
pub(crate) fn function_definition(
    locale: &nsh_platform::Locale,
    name: &BStr,
    body: &Node,
) -> BString {
    let mut printer = Printer::new(locale);
    push_function_name(&mut printer.out, name);
    printer.out.extend_from_slice(b" () \n{ ");
    printer.newline(STEP);
    // Bash writes one brace group here whatever the body was, so a body that
    // is already a group contributes its list rather than a second pair.
    let body = match body {
        Node::Group(group) if group.redirections.is_empty() => group.command.as_ref(),
        body => body,
    };
    printer.list(body, STEP);
    printer.newline(0);
    printer.out.push(b'}');
    printer.out
}

/// Print one parsed command tree as canonical shell source.
///
/// This is separate from [`function_definition`] because the fuzzing
/// round-trip needs to render an arbitrary top-level command without first
/// storing it as a function definition.
#[cfg(feature = "fuzzing")]
pub(crate) fn command(locale: &nsh_platform::Locale, node: &Node) -> BString {
    let mut printer = Printer::new(locale);
    printer.top_level_list(node, 0);
    printer.finish();
    printer.out
}

/// The output buffer and the here-document bodies owed to the next line.
struct Printer<'a> {
    /// Needed to spell a `$'...'` run back: which bytes are one character
    /// decides what stays literal and what becomes an octal escape.
    locale: &'a nsh_platform::Locale,
    out: BString,
    /// Bodies that must be written at column zero after the next newline,
    /// each already terminated by its own delimiter line.
    pending: Vec<BString>,
}

impl<'a> Printer<'a> {
    const fn new(locale: &'a nsh_platform::Locale) -> Self {
        Self {
            locale,
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

    /// Pay any here-document bodies owed by the final command.
    #[cfg(feature = "fuzzing")]
    fn finish(&mut self) {
        if !self.pending.is_empty() {
            self.newline(0);
        }
    }

    // -----------------------------------------------------------------
    // lists
    // -----------------------------------------------------------------

    /// Write a `;`-separated list, one command per line.
    fn list(&mut self, node: &Node, indent: usize) {
        self.list_with_separator(node, indent, true);
    }

    /// Write a top-level list, one command per line.
    #[cfg(feature = "fuzzing")]
    fn top_level_list(&mut self, node: &Node, indent: usize) {
        self.list_with_separator(node, indent, false);
    }

    fn list_with_separator(&mut self, node: &Node, indent: usize, semicolon: bool) {
        let mut items = Vec::new();
        flatten(node, &mut items);
        items.retain(|item| !empty_simple_command(item));
        let last = items.len().saturating_sub(1);
        for (position, item) in items.into_iter().enumerate() {
            self.command(item, indent);
            if position != last {
                if semicolon && !self.ends_asynchronously() {
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

    // -----------------------------------------------------------------
    // commands
    // -----------------------------------------------------------------

    fn command(&mut self, node: &Node, indent: usize) {
        match node {
            Node::Command(command) => self.simple_command(command, indent),
            Node::Pipeline(pipeline) => self.pipeline(pipeline, indent),
            Node::Redirect(command) => {
                self.command(&command.command, indent);
                self.redirections(&command.redirections, indent);
            }
            Node::Background(command) => {
                if needs_background_braces(&command.command) {
                    self.brace_group(&command.command, indent);
                } else {
                    self.command(&command.command, indent);
                }
                self.redirections(&command.redirections, indent);
                self.out.extend_from_slice(b" &");
            }
            Node::Subshell(command) => self.subshell(command, indent),
            Node::Group(command) => {
                self.brace_group(&command.command, indent);
                self.redirections(&command.redirections, indent);
            }
            Node::And(command) => self.binary(command, b" && ", indent),
            Node::Or(command) => self.binary(command, b" || ", indent),
            Node::Sequence(_) => self.list(node, indent),
            Node::If(command) => self.if_command(command, indent),
            Node::While(command) => self.loop_command(b"while", command, indent),
            Node::Until(command) => self.loop_command(b"until", command, indent),
            Node::For(command) => self.for_command(command, indent),
            Node::Select(command) => self.select_command(command, indent),
            Node::Timed(command) => {
                self.out.extend_from_slice(b"time ");
                if command.posix_format {
                    self.out.extend_from_slice(b"-p ");
                }
                if let Some(inner) = &command.command {
                    self.command(inner, indent);
                }
            }
            Node::Case(command) => self.case_command(command, indent),
            Node::Function(definition) => {
                self.nested_function(
                    definition.name.as_bstr(),
                    &definition.body,
                    indent,
                    DefinitionStyle::Posix,
                );
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
        for (position, part) in command
            .assignments
            .iter()
            .chain(command.arguments.iter())
            .enumerate()
        {
            let start = self.out.len();
            if position == command.assignments.len()
                && let Some(word) = reserved_command_word(part)
            {
                push_single_quoted(&mut self.out, word);
            } else {
                self.command(part, indent);
            }
            if self.out.len() > start {
                if written {
                    self.out.insert(start, b' ');
                }
                written = true;
            }
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

    /// `select` reprints as `select`, and is otherwise a `for`.
    fn select_command(&mut self, command: &ForCommand, indent: usize) {
        self.iteration(b"select ", command, indent);
    }

    fn for_command(&mut self, command: &ForCommand, indent: usize) {
        self.iteration(b"for ", command, indent);
    }

    fn iteration(&mut self, keyword: &[u8], command: &ForCommand, indent: usize) {
        self.out.extend_from_slice(keyword);
        self.out.extend_from_slice(command.variable.as_bstr());
        if command.words.is_empty() {
            self.out.extend_from_slice(b" in \"$@\"");
        } else {
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
    /// Print a definition the way it was introduced.
    ///
    /// The three spellings are three trees, so a renderer that picks one
    /// hands the next parse a different definition than it was given. This is
    /// the one place that differs from [`function_definition`], which answers
    /// for `declare -f` and normalises the way Bash's does.
    // [spec:nsh:req:idiom.printable-ast]
    fn nested_function(&mut self, name: &BStr, body: &Node, indent: usize, style: DefinitionStyle) {
        if style != DefinitionStyle::Posix {
            self.out.extend_from_slice(b"function ");
        }
        push_function_name(&mut self.out, name);
        if style != DefinitionStyle::Keyword {
            self.out.extend_from_slice(b" ()");
        }
        self.out.extend_from_slice(b" \n");
        self.out.extend(core::iter::repeat_n(b' ', indent));
        // The braces belong to the body when the body is a group, and a body
        // that is some other compound has none to print. `function_definition`
        // does normalise, the way Bash's `declare -f` does.
        match body {
            Node::Group(group) if group.redirections.is_empty() => {
                self.brace_group(&group.command, indent);
            }
            body => self.command(body, indent),
        }
    }

    fn bash_command(&mut self, node: &BashNode, indent: usize) {
        match node {
            BashNode::Conditional(command) => {
                self.out.extend_from_slice(b"[[ ");
                self.conditional(&command.expression, indent);
                self.out.extend_from_slice(b" ]]");
            }
            BashNode::ArithmeticCommand(command) => {
                // No padding: the expression the tree holds is the one the
                // source wrote, and a space added here comes back as part of
                // it. [spec:nsh:req:idiom.printable-ast]
                let expression = command.expression.as_bstr();
                if expression.is_empty() {
                    self.out.extend_from_slice(b"(())");
                } else {
                    self.out.extend_from_slice(b"((");
                    self.out.extend_from_slice(expression);
                    self.out.extend_from_slice(b"))");
                }
            }
            BashNode::ArithmeticFor(command) => self.arithmetic_for(command, indent),
            BashNode::Function(function) => {
                let style = match function.style {
                    BashFunctionStyle::Function => DefinitionStyle::Keyword,
                    BashFunctionStyle::FunctionParens => DefinitionStyle::KeywordParens,
                };
                self.nested_function(function.name.as_bstr(), &function.body, indent, style);
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
            self.parsed_word(&subscript.word, Quoting::Subscript, indent);
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
            self.parsed_word(&subscript.word, Quoting::Subscript, indent);
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
                Redirection::HereString(here) => self.here_string(here, indent),
            }
        }
    }

    /// `<<< word`, which unlike a here-document carries its whole body in
    /// the word and so needs no queueing to the end of the line.
    fn here_string(&mut self, redirection: &HereString, indent: usize) {
        self.descriptor_prefix(redirection.descriptor.index(), 0);
        self.out.extend_from_slice(b"<<< ");
        self.word(&redirection.word, indent);
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
        let mut body = Self::new(self.locale);
        if document.expand {
            body.parsed_word(&document.body.word, Quoting::HereDocument, indent);
        } else {
            body.out.extend_from_slice(document.body.word.as_bstr());
        }
        let mut body = body.out;
        if !body.is_empty() && body.last() != Some(&b'\n') {
            body.push(b'\n');
        }
        // The source's own delimiter, unless the body holds a line spelling
        // it -- which only happens when the input ended before the terminator
        // did, and then any delimiter is a guess.
        let spelled = document.delimiter.as_bstr();
        let delimiter = if spelled.is_empty()
            || body
                .split(|byte| *byte == b'\n')
                .any(|line| line == spelled)
        {
            unused_delimiter(&body)
        } else {
            BString::from(spelled)
        };

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
        if unterminated_array_word(&word.word) {
            push_single_quoted(&mut self.out, word.word.as_bstr());
        } else {
            self.parsed_word(&word.word, Quoting::Word, indent);
        }
    }

    fn parsed_word(&mut self, word: &ParsedWord, quoting: Quoting, indent: usize) {
        let parts = word.parts();
        if parts.is_empty() {
            if quoting == Quoting::Word {
                self.out.extend_from_slice(b"''");
            }
            return;
        }
        self.parsed_parts(parts, quoting, indent);
    }

    fn parsed_parts(&mut self, parts: &[WordPart], quoting: Quoting, indent: usize) {
        let mut at = 0;
        while at < parts.len() {
            if let WordPart::Quote(QuoteBoundary::Open(kind)) = parts[at] {
                let end = closing_quote(parts, at + 1);
                let region = &parts[at + 1..end];
                match quoting {
                    Quoting::Word | Quoting::Parameter | Quoting::Subscript => {
                        self.quoted_region(region, kind, indent);
                    }
                    Quoting::DoubleParameter
                    | Quoting::DoubleProtectedParameter
                    | Quoting::DoubleInertParameter => {
                        self.double_parameter_quoted_region(region, kind, quoting, indent);
                    }
                    Quoting::Arithmetic => self.arithmetic_quoted_region(region, indent),
                    Quoting::HereDocumentParameter => {
                        self.here_document_parameter_quoted_region(region, indent);
                    }
                    Quoting::Double
                    | Quoting::HereDocument
                    | Quoting::HereDocumentProtectedParameter => {}
                }
                if matches!(
                    quoting,
                    Quoting::Word
                        | Quoting::Parameter
                        | Quoting::Subscript
                        | Quoting::DoubleParameter
                        | Quoting::DoubleProtectedParameter
                        | Quoting::DoubleInertParameter
                        | Quoting::Arithmetic
                        | Quoting::HereDocumentParameter
                ) {
                    at = end.saturating_add(1);
                    continue;
                }
            }
            self.part(&parts[at], parts.get(at + 1), quoting, indent);
            at += 1;
        }
    }

    /// Preserve an operand quote that protects a `}` from ending `${...}`.
    ///
    /// That `}` is the only thing left for it to protect: the `"` this
    /// printer opened already covers the rest, and a quote reopened inside
    /// that `"` does not come back as one. A pattern operand keeps its
    /// quotes regardless, because there the parser reads them as syntax.
    fn double_parameter_quoted_region(
        &mut self,
        region: &[WordPart],
        kind: QuoteKind,
        quoting: Quoting,
        indent: usize,
    ) {
        let quoted =
            quoting == Quoting::DoubleProtectedParameter || parts_contain_literal(region, b'}');
        if !quoted {
            for (at, part) in region.iter().enumerate() {
                self.part(part, region.get(at + 1), quoting, indent);
            }
            return;
        }
        // An apostrophe is a quote to the parser only where the operand is a
        // pattern; anywhere else it is a byte, and a run written with one has
        // to come back inside the quotes that will still be read as quotes.
        let mark = if kind == QuoteKind::Single && quoting == Quoting::DoubleProtectedParameter {
            b'\''
        } else {
            b'"'
        };
        self.out.push(mark);
        for (at, part) in region.iter().enumerate() {
            self.part(part, region.get(at + 1), Quoting::Double, indent);
        }
        self.out.push(mark);
    }

    fn here_document_parameter_quoted_region(&mut self, region: &[WordPart], indent: usize) {
        for (at, part) in region.iter().enumerate() {
            self.part(
                part,
                region.get(at + 1),
                Quoting::HereDocumentProtectedParameter,
                indent,
            );
        }
    }

    /// Bash discards quote bytes before evaluating arithmetic.
    fn arithmetic_quoted_region(&mut self, region: &[WordPart], indent: usize) {
        for (at, part) in region.iter().enumerate() {
            match part {
                WordPart::Literal(bytes) => self.out.extend(
                    bytes
                        .iter()
                        .copied()
                        .filter(|byte| !matches!(byte, b'\'' | b'"')),
                ),
                WordPart::Escaped(b'\'' | b'"')
                | WordPart::Protected(b'\'' | b'"')
                | WordPart::Quote(_) => {}
                _ => self.part(part, region.get(at + 1), Quoting::Arithmetic, indent),
            }
        }
    }

    /// Write one part in whichever quoting the caller has already opened.
    ///
    /// `next` is the part that will follow it, which decides whether a
    /// parameter may be written as `$name` or needs `${name}`.
    fn part(&mut self, part: &WordPart, next: Option<&WordPart>, quoting: Quoting, indent: usize) {
        match part {
            WordPart::Literal(bytes) => self.literal(bytes, next, quoting),
            WordPart::Multibyte { bytes, escaped } => {
                if *escaped && quoting == Quoting::Word {
                    push_single_quoted(&mut self.out, bytes);
                } else {
                    self.literal(bytes, next, quoting);
                }
            }
            // The mark says the source spelled this byte with a backslash,
            // so writing the backslash back is the whole rule. Arithmetic is
            // the exception: Bash discards quote bytes before evaluating, so
            // one written there would become part of the expression.
            // [spec:nsh:req:idiom.printable-ast]
            WordPart::Escaped(byte) => {
                if quoting != Quoting::Arithmetic {
                    self.out.push(b'\\');
                }
                self.out.push(*byte);
            }
            WordPart::Protected(byte) => self.protected(*byte, next, quoting),
            // A `"` inside a `${...}` operand toggles the quoting the word
            // arrived in, and the parser records the toggle rather than a
            // region. Dropping it silently reopens the parameter grammar to
            // whatever the operand was protecting -- a `}` above all.
            WordPart::Quote(_)
                if matches!(
                    quoting,
                    Quoting::DoubleParameter | Quoting::DoubleProtectedParameter
                ) =>
            {
                self.out.push(b'"');
            }
            WordPart::Quote(_) => {}
            WordPart::Parameter(parameter) => self.parameter(parameter, next, quoting, indent),
            WordPart::Command(command) => self.command_substitution(command.as_deref(), indent),
            // `$(( ))` ends on a matching `))`, so an expression whose own
            // parentheses do not balance cannot be written inside one. Bash's
            // older `$[ ]` ends on the bracket and can, and the tree does not
            // record which of the two the source wrote, so either spells the
            // same expansion. Writing `$((0))` there spelled a different one.
            // [spec:nsh:req:idiom.printable-ast]
            WordPart::Arithmetic(expression) => {
                let parenthesised = arithmetic_delimiters_balanced(expression.as_bstr());
                self.out
                    .extend_from_slice(if parenthesised { b"$((" } else { b"$[" });
                self.parsed_word(expression, Quoting::Arithmetic, indent);
                self.out
                    .extend_from_slice(if parenthesised { b"))" } else { b"]" });
            }
        }
    }

    /// Bytes the source left unprotected, or protected by the enclosing
    /// quoting this printer has already opened.
    fn literal(&mut self, bytes: &[u8], next: Option<&WordPart>, quoting: Quoting) {
        let protected: &[u8] = match quoting {
            Quoting::Word => {
                for (at, &byte) in bytes.iter().enumerate() {
                    if byte == b'$' && !opens_expansion(bytes.get(at + 1).copied(), next) {
                        self.out.push(byte);
                        continue;
                    }
                    // A `#` opens a comment only where a word begins, which
                    // is a question about what was written last.
                    // [spec:nsh:req:idiom.printable-ast]
                    let begins_word = self.out.last().is_none_or(|byte| {
                        matches!(byte, b' ' | b'\t' | b'\n' | b';' | b'&' | b'|' | b'(')
                    });
                    if byte == b'#' && !begins_word {
                        self.out.push(byte);
                        continue;
                    }
                    if matches!(
                        byte,
                        b' ' | b'\t'
                            | b'"'
                            | b'#'
                            | b'$'
                            | b'&'
                            | b'\''
                            | b'('
                            | b')'
                            | b';'
                            | b'<'
                            | b'>'
                            | b'\\'
                            | b'`'
                            | b'|'
                    ) {
                        self.out.push(b'\\');
                    } else if byte == b'\n' {
                        push_single_quoted(&mut self.out, &[byte]);
                        continue;
                    }
                    self.out.push(byte);
                }
                return;
            }
            Quoting::Arithmetic => {
                self.out.extend_from_slice(bytes);
                return;
            }
            Quoting::Double => b"\"\\$`",
            Quoting::Parameter => b"'\"\\$`}",
            Quoting::Subscript => b"'\"\\$`]",
            Quoting::DoubleParameter => b"\"\\$`",
            Quoting::DoubleInertParameter => b"\"\\$`}",
            Quoting::DoubleProtectedParameter => b"'\"\\$`",
            Quoting::HereDocument => b"\\$`",
            Quoting::HereDocumentParameter => b"\"\\$`}",
            Quoting::HereDocumentProtectedParameter => b"'\"\\$`}",
        };
        for (at, &byte) in bytes.iter().enumerate() {
            if protected.contains(&byte)
                && (byte != b'$' || opens_expansion(bytes.get(at + 1).copied(), next))
            {
                self.out.push(b'\\');
            }
            self.out.push(byte);
        }
    }

    /// One byte the source protected with a backslash.
    ///
    /// `next` decides the backslash's own case: inside quotes it protects
    /// itself only when the byte after it would otherwise read as an escape,
    /// and protecting it anyway spells the same bytes with an extra part.
    // [spec:nsh:req:idiom.printable-ast]
    /// One byte the quoting protected, written as the byte it is.
    ///
    /// No backslash put it there, so writing one spells a word the source did
    /// not -- unless the enclosing quoting would read the byte as something
    /// other than itself, which is the one thing a backslash is for. The
    /// backslash is that byte's own case: inside quotes it protects the next
    /// one, so it needs protecting exactly when something follows that it
    /// would otherwise take.
    // [spec:nsh:req:idiom.printable-ast]
    fn protected(&mut self, byte: u8, next: Option<&WordPart>, quoting: Quoting) {
        let protected: &[u8] = match quoting {
            // No quoting encloses the byte here, so only its own backslash
            // keeps it from ending the word or being read as syntax -- a
            // trailing one above all, which would take the newline that ends
            // the printed command.
            Quoting::Word | Quoting::Parameter | Quoting::Subscript => {
                self.out.push(b'\\');
                self.out.push(byte);
                return;
            }
            Quoting::Arithmetic => b"",
            Quoting::Double | Quoting::DoubleParameter => b"\"\\$`",
            Quoting::DoubleInertParameter => b"\"\\$`}",
            Quoting::DoubleProtectedParameter => b"'\"\\$`",
            Quoting::HereDocument => b"\\$`",
            Quoting::HereDocumentParameter | Quoting::HereDocumentProtectedParameter => b"'\"\\$`}",
        };
        if protected.contains(&byte) && (byte != b'\\' || takes_next(next, protected)) {
            self.out.push(b'\\');
        }
        self.out.push(byte);
    }

    /// One quoted run, in the quote the source opened it with.
    ///
    /// `'a'` and `"a"` protect the same byte, so which one was written is not
    /// recoverable from the region and the parser records it instead. A run
    /// that expands can only be spelled with double quotes whatever the
    /// source said, and one written `$'...'` is reproduced from bytes whose
    /// escapes are already decoded, so both come back as the plainer form.
    // [spec:nsh:req:idiom.printable-ast]
    fn quoted_region(&mut self, region: &[WordPart], kind: QuoteKind, indent: usize) {
        let expands = region.iter().any(|part| {
            matches!(
                part,
                WordPart::Parameter(_) | WordPart::Command(_) | WordPart::Arithmetic(_)
            )
        });
        if kind == QuoteKind::DollarSingle && !expands {
            let mut bytes = BString::new(Vec::new());
            for part in region {
                match part {
                    WordPart::Literal(text) | WordPart::Multibyte { bytes: text, .. } => {
                        bytes.extend_from_slice(text);
                    }
                    WordPart::Escaped(byte) | WordPart::Protected(byte) => bytes.push(*byte),
                    _ => {}
                }
            }
            let quoted =
                crate::escape::bash::ansi_c_quote(self.locale, BStr::new(bytes.as_slice()));
            self.out.extend_from_slice(&quoted);
            return;
        }
        if kind == QuoteKind::DollarDouble {
            self.out.push(b'$');
        }
        if expands || matches!(kind, QuoteKind::Double | QuoteKind::DollarDouble) {
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
                WordPart::Escaped(byte) | WordPart::Protected(byte) => bytes.push(*byte),
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
        if parameter.operation == ParameterOperation::Invalid {
            // The expansion fails, but it fails on bytes the source wrote and
            // the tree still holds. Printing `${}` in their place spelled a
            // different failure and lost whatever the braces were around.
            // [spec:nsh:req:idiom.printable-ast]
            self.out.extend_from_slice(b"${");
            if parameter.indirect {
                self.out.push(b'!');
            }
            self.out.extend_from_slice(&parameter.invalid_marker);
            self.out.extend_from_slice(&parameter.name);
            self.out.extend_from_slice(&parameter.invalid_prefix);
            if let Some(operand) = parameter.operand.as_ref() {
                let inert = match quoting {
                    Quoting::Word | Quoting::Parameter | Quoting::Subscript => Quoting::Parameter,
                    Quoting::Double
                    | Quoting::DoubleParameter
                    | Quoting::DoubleProtectedParameter
                    | Quoting::DoubleInertParameter => Quoting::DoubleInertParameter,
                    quoting => quoting,
                };
                self.parsed_parts(operand.parts(), inert, indent);
            }
            self.out.push(b'}');
            return;
        }
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
        if let Some(operand) = parameter.operand.as_ref() {
            if !operand.is_empty() {
                // Inside a `"` this printer opened, the operand is already
                // protected and may not open a quote of its own.
                let inner = match quoting {
                    Quoting::Word | Quoting::Parameter | Quoting::Subscript => Quoting::Parameter,
                    Quoting::Double
                    | Quoting::DoubleParameter
                    | Quoting::DoubleProtectedParameter
                    | Quoting::DoubleInertParameter => {
                        if !operand_quoting_closed(operand) {
                            Quoting::DoubleInertParameter
                        } else if operand_needs_apostrophe_protection(parameter.operation) {
                            Quoting::DoubleProtectedParameter
                        } else {
                            Quoting::DoubleParameter
                        }
                    }
                    Quoting::Arithmetic => Quoting::Arithmetic,
                    Quoting::HereDocument
                    | Quoting::HereDocumentParameter
                    | Quoting::HereDocumentProtectedParameter => {
                        if operand_needs_apostrophe_protection(parameter.operation) {
                            Quoting::HereDocumentProtectedParameter
                        } else {
                            Quoting::HereDocumentParameter
                        }
                    }
                };
                self.parsed_parts(operand.parts(), inner, indent);
            }
        }
        self.out.push(b'}');
    }

    fn command_substitution(&mut self, node: Option<&Node>, indent: usize) {
        if let Some(Node::Bash(BashNode::ProcessSubstitution(substitution))) = node {
            self.process_substitution(substitution, indent);
            return;
        }
        let outer_pending = core::mem::take(&mut self.pending);
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
        if !self.pending.is_empty() {
            self.newline(indent);
        }
        self.out.push(b')');
        self.pending = outer_pending;
    }
}

fn empty_simple_command(node: &Node) -> bool {
    let Node::Command(command) = node else {
        return false;
    };
    command.assignments.is_empty()
        && command.redirections.is_empty()
        && command.arguments.iter().all(|node| {
            let Node::Word(word) = node else {
                return false;
            };
            !word.word.parts().is_empty()
                && word
                    .word
                    .parts()
                    .iter()
                    .all(|part| matches!(part, WordPart::Quote(QuoteBoundary::Close)))
        })
}

/// Keep a parsed function name one shell word when its bytes include syntax.
fn push_function_name(out: &mut BString, name: &BStr) {
    let needs_quotes = name.is_empty()
        || name.iter().any(|byte| {
            byte.is_ascii_whitespace()
                || matches!(
                    byte,
                    b'!' | b'"'
                        | b'#'
                        | b'$'
                        | b'&'
                        | b'\''
                        | b'('
                        | b')'
                        | b';'
                        | b'<'
                        | b'>'
                        | b'\\'
                        | b'`'
                        | b'|'
                )
        });
    if needs_quotes {
        push_single_quoted(out, name);
    } else {
        out.extend_from_slice(name);
    }
}

/// A NUL can end parsing after an assignment-shaped word has opened a
/// subscript. Quote that inert prefix so printing it cannot reopen the grammar.
///
/// An expansion inside the prefix does not rescue it: the word was cut short
/// before its `]`, so nothing in it will ever expand, and printing it as
/// written asks the next parse for a bracket the source never had.
fn unterminated_array_word(word: &ParsedWord) -> bool {
    let bytes = word.as_bstr();
    let Some(open) = bytes.iter().position(|byte| *byte == b'[') else {
        return false;
    };
    if open == 0
        || !bytes[..open]
            .iter()
            .all(|byte| byte.is_ascii_alphanumeric() || *byte == b'_')
    {
        return false;
    }

    let mut bracket_depth = 0isize;
    let mut quote_depth = 0usize;
    for part in word.parts() {
        match part {
            WordPart::Quote(QuoteBoundary::Open(..)) => quote_depth += 1,
            WordPart::Quote(QuoteBoundary::Close) => {
                quote_depth = quote_depth.saturating_sub(1);
            }
            WordPart::Literal(bytes) if quote_depth == 0 => {
                for byte in bytes.iter() {
                    match *byte {
                        b'[' => bracket_depth += 1,
                        b']' => bracket_depth -= 1,
                        _ => {}
                    }
                }
            }
            _ => {}
        }
    }
    bracket_depth != 0
}

/// Whether a `$` written before these would start an expansion.
///
/// A `$` that starts nothing is an ordinary byte, and protecting it anyway
/// spells the same byte with a part the source never wrote. `after` is the
/// byte following it inside the same literal run, and `next` the part that
/// follows the run when there is no such byte.
// [spec:nsh:req:idiom.printable-ast]
fn opens_expansion(after: Option<u8>, next: Option<&WordPart>) -> bool {
    if let Some(byte) = after {
        return matches!(
            byte,
            b'{' | b'(' | b'_' | b'@' | b'*' | b'#' | b'?' | b'-' | b'$' | b'!'
        ) || byte.is_ascii_alphanumeric();
    }
    match next {
        Some(WordPart::Parameter(_) | WordPart::Command(_) | WordPart::Arithmetic(_)) => true,
        // `$'...'` and `$"..."` are expansions of their own, so a `$` written
        // against the quote that opens one has to be held off it.
        Some(WordPart::Quote(QuoteBoundary::Open(_))) => true,
        Some(WordPart::Quote(QuoteBoundary::Close)) | None => false,
        Some(WordPart::Literal(bytes) | WordPart::Multibyte { bytes, .. }) => {
            opens_expansion(bytes.first().copied(), None)
        }
        // An escape carries its own backslash, and a `$` against a backslash
        // starts nothing.
        Some(WordPart::Escaped(_)) => false,
        Some(WordPart::Protected(byte)) => opens_expansion(Some(*byte), None),
    }
}

/// Whether a backslash written here would take the part after it.
///
/// Inside quotes a backslash protects only the few bytes the quoting reads,
/// so before anything else it is data and needs no backslash of its own.
// [spec:nsh:req:idiom.printable-ast]
fn takes_next(next: Option<&WordPart>, protected: &[u8]) -> bool {
    match next {
        Some(WordPart::Literal(bytes) | WordPart::Multibyte { bytes, .. }) => {
            bytes.first().is_none_or(|byte| protected.contains(byte))
        }
        Some(WordPart::Protected(byte)) => protected.contains(byte),
        Some(WordPart::Escaped(_)) => true,
        _ => false,
    }
}

/// Whether every quote the operand opened was closed again.
///
/// A NUL can end the parse inside an operand, leaving one toggle unmatched.
/// The bytes it was protecting are still the operand's, but the toggle cannot
/// be written back: it would leave the operand quoted at the `}` that ends it.
// [spec:nsh:req:idiom.printable-ast]
fn operand_quoting_closed(operand: &ParsedWord) -> bool {
    operand
        .parts()
        .iter()
        .filter(|part| matches!(part, WordPart::Quote(_)))
        .count()
        % 2
        == 0
}

/// Whether the parser reads this operation's operand as a pattern, where an
/// apostrophe is a quote rather than one of the operand's own bytes.
const fn operand_needs_apostrophe_protection(operation: ParameterOperation) -> bool {
    matches!(
        operation,
        ParameterOperation::Substring
            | ParameterOperation::RemoveSmallestSuffix
            | ParameterOperation::RemoveLargestSuffix
            | ParameterOperation::RemoveSmallestPrefix
            | ParameterOperation::RemoveLargestPrefix
            | ParameterOperation::SubstituteFirst
            | ParameterOperation::SubstituteAll
            | ParameterOperation::UpperFirst
            | ParameterOperation::UpperAll
            | ParameterOperation::LowerFirst
            | ParameterOperation::LowerAll
            | ParameterOperation::Transform
    )
}

fn reserved_command_word(node: &Node) -> Option<&[u8]> {
    let Node::Word(word) = node else {
        return None;
    };
    let [WordPart::Literal(bytes)] = word.word.parts() else {
        return None;
    };
    matches!(
        bytes.as_slice(),
        b"!" | b"case"
            | b"do"
            | b"done"
            | b"elif"
            | b"else"
            | b"esac"
            | b"fi"
            | b"for"
            | b"if"
            | b"in"
            | b"select"
            | b"then"
            | b"time"
            | b"until"
            | b"while"
            | b"{"
            | b"}"
    )
    .then_some(bytes.as_slice())
}

fn arithmetic_delimiters_balanced(expression: &[u8]) -> bool {
    let mut parentheses = 0usize;
    let mut brackets = 0usize;
    for &byte in expression {
        match byte {
            b'(' => parentheses += 1,
            b')' if parentheses == 0 => return false,
            b')' => parentheses -= 1,
            b'[' => brackets += 1,
            b']' if brackets == 0 => return false,
            b']' => brackets -= 1,
            _ => {}
        }
    }
    parentheses == 0 && brackets == 0
}

fn parts_contain_literal(parts: &[WordPart], needle: u8) -> bool {
    parts.iter().any(|part| match part {
        WordPart::Literal(bytes) | WordPart::Multibyte { bytes, .. } => bytes.contains(&needle),
        WordPart::Parameter(parameter) => parameter
            .operand
            .as_deref()
            .is_some_and(|operand| parts_contain_literal(operand.parts(), needle)),
        WordPart::Arithmetic(expression) => parts_contain_literal(expression.parts(), needle),
        WordPart::Escaped(byte) | WordPart::Protected(byte) => *byte == needle,
        WordPart::Quote(_) | WordPart::Command(_) => false,
    })
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

/// Whether `&` would attach only to the final command unless braces
/// hold the list together.
const fn needs_background_braces(node: &Node) -> bool {
    matches!(node, Node::Sequence(_))
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
    if bytes.is_empty() {
        out.extend_from_slice(b"''");
        return;
    }
    for (position, chunk) in bytes.split(|byte| *byte == b'\'').enumerate() {
        if position > 0 {
            out.extend_from_slice(b"\\'");
        }
        if !chunk.is_empty() {
            out.push(b'\'');
            out.extend_from_slice(chunk);
            out.push(b'\'');
        }
    }
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
