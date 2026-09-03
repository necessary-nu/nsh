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
mod bash;
mod redirection;
mod word;

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

/// Print one word as it would be written in a command: the run it was
/// read from, or its parts spelled back when nothing read it.
///
/// What [`crate::script`] hands an embedder as a word's `source`, so that
/// what it reads back from the shell is the word it was shown. The same
/// entry the printer uses everywhere else, so an embedder is shown the
/// spelling `declare -f` would show.
pub(crate) fn word(locale: &nsh_platform::Locale, node: &WordNode) -> BString {
    let mut printer = Printer::new(locale);
    printer.word(node, 0);
    printer.out
}

/// Print one redirection as it would be written after a command, without
/// the blank that separates it from the command.
///
/// A here-document's body is owed to the next line and has no line to go
/// to here, so it is written straight after the operator instead.
pub(crate) fn redirection(locale: &nsh_platform::Locale, redirection: &Redirection) -> BString {
    let mut printer = Printer::new(locale);
    printer.redirections(core::slice::from_ref(redirection), 0);
    let mut out = printer.out;
    for body in printer.pending {
        out.push(b'\n');
        out.extend_from_slice(&body);
    }
    if out.first() == Some(&b' ') {
        out.remove(0);
    }
    out
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
            Node::For(command) => self.iteration(b"for ", command, indent),
            // `select` reprints as `select`, and is otherwise a `for`.
            Node::Select(command) => self.iteration(b"select ", command, indent),
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
