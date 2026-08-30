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
    ForCommand, HereDocument, HereString, IfCommand, Node, Pipeline, Redirection,
    RedirectionDescriptor, SimpleCommand, WordNode,
};
use crate::word::{ParameterExpansion, ParameterOperation, ParsedWord, WordPart};

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

/// Spell a whole tree the shell built, as the fallback would.
///
/// Separate from [`function_definition`] because what it renders is not a
/// definition and has no frame around it. Its one caller is the property
/// that holds the fallback to structure: a node with no bytes cannot be
/// compared against any, so it is compared against the node its spelling
/// parses back to.
#[cfg(test)]
pub(crate) fn command(locale: &nsh_platform::Locale, node: &Node) -> BString {
    let mut printer = Printer::new(locale);
    printer.top_level_list(node, 0);
    printer.finish();
    printer.out
}

/// Spell a whole tree from its structure, ignoring every run in it.
///
/// The same printer as [`command`], with the three places that replay a
/// node's source bytes switched off. That is deliberate reuse rather than
/// convenience: deciding how to spell something is the one job this file
/// has, and a second speller written to disagree with it would be
/// measuring itself. What it returns is therefore a *different* spelling
/// of the same program -- the parser was handed one, this is the other --
/// which is what makes an equivalence class derivable instead of listed.
// [spec:nsh:req:idiom.canonical-tree+1]
#[cfg(any(feature = "fuzzing", test))]
pub(crate) fn respelled(locale: &nsh_platform::Locale, node: &Node) -> BString {
    let mut printer = Printer::new(locale);
    printer.ignore_runs = true;
    /* The semicolon list, not the one-per-line list: what goes in is one
     * parse unit and what comes out has to be one too, or the comparison
     * would be against a different number of programs. */
    printer.list(node, 0);
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
    /// Spell from structure even where a run is available.
    ///
    /// Replaying a run is how this printer keeps what the operator wrote.
    /// Switching it off is how [`respelled`] gets a second spelling of one
    /// program without a second thing that knows how to spell.
    // [spec:nsh:req:idiom.canonical-tree+1]
    ignore_runs: bool,
}

impl<'a> Printer<'a> {
    const fn new(locale: &'a nsh_platform::Locale) -> Self {
        Self {
            locale,
            out: BString::new(Vec::new()),
            pending: Vec::new(),
            ignore_runs: false,
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
    #[cfg(any(feature = "fuzzing", test))]
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
    #[cfg(test)]
    fn top_level_list(&mut self, node: &Node, indent: usize) {
        self.list_with_separator(node, indent, false);
    }

    fn list_with_separator(&mut self, node: &Node, indent: usize, semicolon: bool) {
        let mut items = Vec::new();
        flatten(node, &mut items);
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
        /* Anything the parser built is written as the bytes it was read
         * from. What follows this is the fallback, and it only ever sees
         * nodes the shell synthesized. */
        // [spec:nsh:req:idiom.printable-ast+2]
        if let Some(source) = super::emit::emitted(node).filter(|_| !self.ignore_runs) {
            /* This renderer wrote the indent itself, so the blank the
             * statement was reached through is not its to write again. */
            let run = node.tokens();
            let reached_through = run.text().len().saturating_sub(run.written().len());
            self.out
                .extend_from_slice(&source[reached_through.min(source.len())..]);
            return;
        }
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

    /// Spell a definition the shell built, the way it was introduced.
    ///
    /// The three spellings are three trees, so a renderer that picks one
    /// hands the next parse a different definition than it was given. Only
    /// reached for a definition nothing read, since one that was read is
    /// written as its own bytes.
    // [spec:nsh:req:idiom.printable-ast+2]
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
                // it. [spec:nsh:req:idiom.printable-ast+2]
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
                Redirection::HereString(here) => self.here_string(here, indent),
            }
        }
    }

    /// `<<< word`, which unlike a here-document carries its whole body in
    /// the word and so needs no queueing to the end of the line.
    fn here_string(&mut self, redirection: &HereString, indent: usize) {
        self.descriptor_prefix(&redirection.descriptor, 0);
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
        self.descriptor_prefix(&redirection.descriptor, default);
        self.out.extend_from_slice(operator);
        self.out.push(b' ');
        self.word(&redirection.target, indent);
    }

    fn descriptor_redirection(&mut self, redirection: &DescriptorRedirection) {
        let (operator, default): (&[u8], usize) = match redirection.operator {
            DescriptorRedirectionOperator::Input => (b"<&", 0),
            DescriptorRedirectionOperator::Output => (b">&", 1),
        };
        self.descriptor_prefix(&redirection.descriptor, default);
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
    /// The number, or `{name}`, before a redirection operator.
    ///
    /// A fixed slot the operator would have taken anyway is left unwritten,
    /// which is why the default comes in. `{name}` is never the default and
    /// is always written: it is the request to allocate.
    // [spec:nsh:req:compat.bash.parser-ast]
    fn descriptor_prefix(&mut self, descriptor: &RedirectionDescriptor, default: usize) {
        if descriptor
            .fixed()
            .is_some_and(|fixed| fixed.index() == default)
        {
            return;
        }
        self.out.extend_from_slice(&descriptor.text());
    }

    /// Write `<<DELIM` and queue the body for the end of the line.
    ///
    /// The tree keeps the body but not the delimiter the source spelled,
    /// so one is chosen that no line of the body can be mistaken for.
    fn here_document(&mut self, document: &HereDocument, indent: usize) {
        /* The body's run is the body and the delimiter line that ended
         * it, read together at the newline after this redirection. When
         * there is one it is the whole document, terminator included, and
         * the delimiter below is the one the source wrote. */
        // [spec:nsh:req:idiom.printable-ast+2]
        let read = document.body.tokens.text();
        if !self.ignore_runs && !read.is_empty() && !document.delimiter.as_bstr().is_empty() {
            self.descriptor_prefix(&document.descriptor, 0);
            self.out.extend_from_slice(b"<<");
            if document.expand {
                self.out.extend_from_slice(document.delimiter.as_bstr());
            } else {
                self.out.push(b'\'');
                self.out.extend_from_slice(document.delimiter.as_bstr());
                self.out.push(b'\'');
            }
            self.pending.push(read);
            return;
        }
        let mut body = Self::new(self.locale);
        body.ignore_runs = self.ignore_runs;
        body.spelled_body(&document.body.word, document.expand, indent);
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

        self.descriptor_prefix(&document.descriptor, 0);
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

    /// Write a word as the source it was read from.
    ///
    /// A parsed word carries its own run, so nothing here decides how to
    /// spell one: the bytes that were read are the bytes that go back.
    /// This is where the printer kept its second copy of the grammar --
    /// eleven quoting contexts, nine byte-sets, and a per-byte opinion
    /// about when a `$` opens an expansion -- all of it re-deriving what
    /// the reader had already answered.
    // [spec:nsh:req:idiom.printable-ast+2]
    fn word(&mut self, word: &WordNode, indent: usize) {
        let run = word.tokens.written();
        if self.ignore_runs || run.is_empty() {
            self.spelled(&word.word, indent);
        } else {
            self.out.extend_from_slice(&run);
        }
    }

    /// Spell a word the shell built rather than read.
    ///
    /// The obligation is only that what it writes parses back to a
    /// structurally equal word, so it picks one rule and applies it
    /// everywhere: an inert run goes inside single quotes, an ordinary
    /// run goes as it is, and an expansion keeps the quoting it was
    /// under. Choosing per byte is what the deleted grammar was for, and
    /// choosing at all is only sound because the bytes here were never
    /// written by anyone.
    // [spec:nsh:req:idiom.printable-ast+2]
    fn spelled(&mut self, word: &ParsedWord, indent: usize) {
        if word.parts().is_empty() {
            self.out.extend_from_slice(b"''");
            return;
        }
        for part in word.parts() {
            if let WordPart::Text { bytes, quoted } = part {
                if *quoted {
                    push_single_quoted(&mut self.out, bytes);
                } else {
                    self.out.extend_from_slice(bytes);
                }
                continue;
            }
            if part.quoted() {
                self.out.push(b'"');
            }
            match part {
                WordPart::Parameter(parameter) => self.spelled_parameter(parameter, indent),
                WordPart::Command { command, .. } => {
                    self.command_substitution(command.as_deref(), indent);
                }
                WordPart::Arithmetic { expression, .. } => {
                    self.out.extend_from_slice(b"$((");
                    self.spelled(expression, indent);
                    self.out.extend_from_slice(b"))");
                }
                WordPart::Text { .. } => unreachable!("a text run was written above"),
            }
            if part.quoted() {
                self.out.push(b'"');
            }
        }
    }

    /// Spell a word as the body of a here-document.
    ///
    /// A body is not a shell word and cannot be spelled as one. Nothing
    /// quotes there: a `'` is a `'`, so the single quotes [`spelled`] puts
    /// around an inert run would be two more bytes of body, and the `"` it
    /// puts around an expansion likewise. What makes a run inert here is
    /// the delimiter, and that is already decided by `expand`.
    ///
    /// So the two cases are spelled by what the delimiter says. A body
    /// that does not expand is written exactly as it is, because its
    /// delimiter is quoted and every byte is already data. A body that
    /// does expand writes its expansions bare and backslash-escapes the
    /// three bytes that would otherwise start one.
    // [spec:nsh:req:idiom.canonical-tree+1]
    fn spelled_body(&mut self, word: &ParsedWord, expand: bool, indent: usize) {
        for part in word.parts() {
            match part {
                WordPart::Text { bytes, .. } => {
                    if expand {
                        for byte in bytes.iter() {
                            if matches!(byte, b'\\' | b'$' | b'`') {
                                self.out.push(b'\\');
                            }
                            self.out.push(*byte);
                        }
                    } else {
                        self.out.extend_from_slice(bytes);
                    }
                }
                WordPart::Parameter(parameter) => self.spelled_parameter(parameter, indent),
                WordPart::Command { command, .. } => {
                    self.command_substitution(command.as_deref(), indent);
                }
                WordPart::Arithmetic { expression, .. } => {
                    self.out.extend_from_slice(b"$((");
                    self.spelled(expression, indent);
                    self.out.extend_from_slice(b"))");
                }
            }
        }
    }

    /// Spell an expansion from its fields, for a word nothing read.
    ///
    /// An expansion the shell refused never reaches here: the parser is
    /// the only thing that builds one, and a parsed word is written as
    /// its run.
    // [spec:nsh:req:idiom.printable-ast+2]
    fn spelled_parameter(&mut self, parameter: &ParameterExpansion, indent: usize) {
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
        /* An operand that is there and empty is spelled by the operator
         * alone: `${a-}` is the empty operand, and the `''` that
         * [`spelled`] writes for a word with no parts would be two bytes
         * of operand rather than none. */
        // [spec:nsh:req:idiom.canonical-tree+1]
        if let Some(operand) = &parameter.operand {
            if !operand.parts().is_empty() {
                self.spelled(operand, indent);
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

fn reserved_command_word(node: &Node) -> Option<&[u8]> {
    let Node::Word(word) = node else {
        return None;
    };
    let [
        WordPart::Text {
            bytes,
            quoted: false,
        },
    ] = word.word.parts()
    else {
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

// ---------------------------------------------------------------------
// A node the shell built has no bytes to be equal to.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;
    use crate::nodes::{SimpleCommand, SourceLine, SourceTokens, WordNode};
    use crate::word::{ParsedWord, WordUnit};

    /// What the fallback spells has to parse back to the same node.
    ///
    /// This is the seam, and this is the whole of what is asked on this
    /// side of it. `nodes/emit.rs` writes what was read and is held to
    /// bytes, because there are bytes to be held to. Here there are none,
    /// so the obligation is the older one -- a structurally equal node --
    /// and it is the only place left where a renderer decides anything.
    ///
    /// A word with no parts at all is left out, and is the one thing this
    /// cannot satisfy: no source reads back as a word holding nothing, so
    /// the fallback writes `''` and gets back a word holding one empty
    /// inert run. The shell never builds one as an argument --
    /// `ParsedWord::new()` is the placeholder a here-document body carries
    /// until its body replaces it.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn a_built_node_spells_itself_back_into_itself() {
        let word = |word| {
            Node::Word(WordNode {
                tokens: SourceTokens::none(),
                word,
            })
        };
        let inert: Vec<WordUnit> = b"a b'c"
            .iter()
            .map(|byte| WordUnit::Literal {
                byte: *byte,
                quoted: true,
            })
            .collect();
        let built = Node::Command(Box::new(SimpleCommand {
            tokens: SourceTokens::none(),
            line: SourceLine::new(1),
            assignments: Vec::new(),
            arguments: vec![
                word(ParsedWord::literal("echo")),
                word(ParsedWord::from_units(&inert)),
                word(ParsedWord::quoted_parameter("@")),
            ],
            redirections: Vec::new(),
        }));

        let mut shell = crate::Shell::builder()
            .streams(crate::Streams::capture().expect("captured streams"))
            .build()
            .expect("shell");
        let spelled = command(&shell.locale, &built);
        let reparsed = crate::resource::with_resources(&mut shell, |shell, _resources| {
            crate::input::set_input_string(shell, BStr::new(&spelled));
            match crate::parser::parse_command(shell, false) {
                Ok(crate::parser::ParseResult::Tree(Some(node))) => node,
                _ => panic!("{spelled:?} did not parse back"),
            }
        });
        assert!(reparsed == built, "{spelled:?} parsed back to another node");
    }
}
