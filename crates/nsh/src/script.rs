//! A script as the shell reads it, for an embedder that has to reason about
//! one without running it.
//!
//! [`Shell::run`] parses and executes; nothing on the surface parsed
//! without executing, and an embedder that wanted to know what a line
//! *would* do -- which commands it names, in what order, under what
//! condition -- had to read the bytes itself with a second grammar that
//! could only disagree with this one. A build tool composing recursive
//! invocations out of recipe lines is that embedder.
//!
//! What is exposed is a PROJECTION of the parse tree and not the tree. The
//! tree ([`crate::nodes`]) is the evaluator's working state: thirty-odd
//! types carrying descriptor slots, here-document bodies and the Bash
//! forms, every one of them free to change with the evaluator. The
//! projection keeps the grammar -- which command runs after which, and
//! under what -- and the one fact about a word that decides its meaning
//! without running it: whether each piece of it is quoted. Every type here
//! is `#[non_exhaustive]`, so it can grow a variant or a field without
//! breaking the embedder that matches on it, and what it does not say it
//! says as [`Command::Other`] or [`Piece::Expansion`] rather than by
//! guessing.
//!
//! Reading never executes: no command substitution runs, no word expands,
//! no variable is read or written. A [`Reader`] holds one shell built for
//! the purpose, whose streams are the null device so that a syntax error
//! -- which the shell reports as well as returns -- can never reach the
//! host's own stderr. [dec:nsh:public-surface] closed the surface around
//! `Shell`; this is the one addition since, and it is sized like the
//! original: what an embedder writes, and nothing the shell keeps.
//!
//! [`Shell::run`]: crate::Shell::run

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::nodes::{self, Node, SourceTokens, WordNode};
use crate::parser::ParseResult;
use crate::word::{ParameterOperation, ParsedWord, WordPart};

/// Reads scripts without running them.
///
/// Build one and keep it: it owns a shell, and building a shell costs a
/// locale, a variable table and three descriptors, none of which a
/// reading touches.
pub struct Reader {
    shell: Shell,
}

impl Reader {
    /// A reader of POSIX shell syntax.
    ///
    /// # Errors
    ///
    /// When the shell it holds cannot be built, which is when the process
    /// cannot open the null device its diagnostics go to.
    // [spec:nsh:req:embedding-safety.reading-without-running]
    pub fn new() -> Result<Reader, Error> {
        Reader::in_dialect(false)
    }

    /// A reader of Bash syntax, as [`crate::Shell`] reads it with the
    /// `bash` option on.
    ///
    /// The dialect decides the grammar, so it has to be the caller's to
    /// choose: `[[ -f x ]]` is a command in one and a word in the other,
    /// and a reader fixed to POSIX would answer a Bash embedder about a
    /// language it is not running.
    ///
    /// # Errors
    ///
    /// As [`Reader::new`].
    // [spec:nsh:req:embedding-safety.reading-without-running]
    pub fn bash() -> Result<Reader, Error> {
        Reader::in_dialect(true)
    }

    fn in_dialect(bash: bool) -> Result<Reader, Error> {
        let streams = crate::streams::Streams::discarding().map_err(|error| {
            Error::other(
                0,
                2,
                format!("cannot open the reader's streams: {error}").as_bytes(),
            )
        })?;
        let shell = Shell::builder()
            .streams(streams)
            .option(BStr::new(b"bash"), bash)
            .build()?;
        Ok(Reader { shell })
    }

    /// Read `source` as the shell would, and say what it holds.
    ///
    /// One [`Command`] per top-level line, blank lines skipped. The whole
    /// source is read before anything is returned, so a syntax error on
    /// its last line is an error and not a shorter script.
    ///
    /// # Errors
    ///
    /// The shell's own syntax error, with its message. Nothing has been
    /// written to the host: the diagnostic the parser reports on its way
    /// out went to the reader's null stderr.
    ///
    /// The parse is `parse_command`, the one [`crate::Shell::run`] uses,
    /// so what a caller is told is what the shell would act on rather
    /// than a second reading that could drift from it.
    // [spec:nsh:req:embedding-safety.reading-without-running]
    pub fn read(&mut self, source: &BStr) -> Result<Script, Error> {
        let shell = &mut self.shell;
        crate::resource::with_resources(shell, |shell, _resources| {
            crate::input::set_input_string(shell, source);
            let mut commands = Vec::new();
            loop {
                match crate::parser::parse_command(shell, false)? {
                    ParseResult::Eof => break,
                    ParseResult::Tree(Some(node)) => {
                        commands.push(command(&shell.locale, &node));
                    }
                    ParseResult::Tree(None) => {}
                }
            }
            Ok(Script { commands })
        })
    }
}

/// What a source held: its top-level commands, in order.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Script {
    /// One per top-level line the source wrote.
    pub commands: Vec<Command>,
}

/// One command, in the grammar's own shape.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Command {
    /// Words, assignments and redirections: the shape that names a program.
    Simple(SimpleCommand),
    /// Two or more commands joined by `|`, or one command negated by `!`.
    Pipeline {
        /// Whether a leading `!` inverts the status.
        negated: bool,
        /// The commands, in pipe order.
        commands: Vec<Command>,
    },
    /// `left && right`: the right runs only if the left succeeded.
    And(Box<Command>, Box<Command>),
    /// `left || right`: the right runs only if the left failed.
    Or(Box<Command>, Box<Command>),
    /// `left; right`: the right runs whatever the left did.
    Sequence(Box<Command>, Box<Command>),
    /// `command &`: started, and not waited for.
    Background(Box<Command>),
    /// `( command )`: run in a shell of its own, whose state dies with it.
    Subshell(Box<Command>),
    /// A compound command with redirections on it: `{ …; } > log`,
    /// `( … ) 2>&1`, `for … done < list`.
    Redirected {
        /// The command the redirections apply to.
        command: Box<Command>,
        /// What they redirect.
        redirections: Vec<Redirection>,
    },
    /// `if condition; then …; [else …;] fi`. An `elif` is an `if` in the
    /// `else` branch.
    If {
        /// The list whose status is tested.
        condition: Box<Command>,
        /// What runs when the condition succeeds.
        then: Box<Command>,
        /// What runs when it fails, if anything was written.
        otherwise: Option<Box<Command>>,
    },
    /// `for variable in words; do body; done`. A `for` written without
    /// `in` iterates `"$@"`, and says so in its words.
    For {
        /// The name bound on each iteration.
        variable: BString,
        /// The words iterated, before expansion.
        words: Vec<Word>,
        /// What runs once per field the words expand to.
        body: Box<Command>,
    },
    /// `while condition; do body; done`.
    While {
        /// The list run before each iteration.
        condition: Box<Command>,
        /// What runs while it succeeds.
        body: Box<Command>,
    },
    /// `until condition; do body; done`.
    Until {
        /// The list run before each iteration.
        condition: Box<Command>,
        /// What runs until it succeeds.
        body: Box<Command>,
    },
    /// `case word in pattern) …;; esac`.
    Case {
        /// The word matched against each clause's patterns.
        word: Word,
        /// The clauses, in the order they are tried.
        clauses: Vec<CaseClause>,
    },
    /// `name() body`.
    Function {
        /// The function's name.
        name: BString,
        /// Its body.
        body: Box<Command>,
    },
    /// A form this projection does not describe: Bash's `[[ ]]`, `(( ))`,
    /// arrays and process substitutions, `time`, and `select`.
    Other,
}

/// Words, assignments and redirections.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct SimpleCommand {
    /// The `name=value` words in front of the command, in order.
    pub assignments: Vec<Assignment>,
    /// The command and its arguments, before expansion. Empty for a line
    /// that is assignments alone.
    pub words: Vec<Word>,
    /// The command's redirections, in order.
    pub redirections: Vec<Redirection>,
}

/// One `name=value` word.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Assignment {
    /// The variable's name.
    pub name: BString,
    /// What it is assigned, before expansion: the value is expanded as a
    /// word is, save that it is never split into fields.
    pub value: Word,
}

/// One clause of a `case`.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct CaseClause {
    /// The patterns any one of which selects this clause.
    pub patterns: Vec<Word>,
    /// What runs when it is selected, if anything was written.
    pub body: Option<Box<Command>>,
    /// Whether the clause ends in `;&`, falling through to the next.
    pub fallthrough: bool,
}

/// One word, before expansion.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Word {
    /// The word's pieces, in order. `''` and `""` are one quoted literal
    /// holding no bytes, which is what tells them from a word that holds
    /// nothing at all.
    pub pieces: Vec<Piece>,
    /// The word spelled back as shell source, with every piece protected
    /// exactly as far as it was: what to hand a shell to get this word
    /// again.
    pub source: BString,
}

/// One piece of a word.
///
/// `quoted` is the one fact expansion turns on. A quoted piece is text and
/// nothing else; an unquoted literal may be a pattern or a `~`, and an
/// unquoted expansion is split into fields and matched against the disk.
///
/// Between the variant and that flag, two spellings the shell runs
/// differently read differently here: `'$x'` is a quoted [`Piece::Literal`]
/// where `"$x"` is a quoted [`Piece::Parameter`], and `$x` is the same
/// parameter unquoted. Which quote was written is not a distinction the
/// shell draws, and is in [`Word::source`] for a caller that wants the
/// spelling back rather than the meaning.
// [spec:nsh:req:embedding-safety.reading-without-running]
#[derive(Clone, Debug)]
#[non_exhaustive]
pub enum Piece {
    /// Bytes written as themselves.
    Literal {
        /// The bytes.
        bytes: BString,
        /// Whether quotes or a backslash protect them.
        quoted: bool,
    },
    /// A plain `$name` or `${name}`: the parameter's value and nothing
    /// done to it.
    Parameter {
        /// The parameter's name — a variable, a positional or a special
        /// parameter such as `@` or `?`.
        name: BString,
        /// Whether double quotes protect the value from splitting.
        quoted: bool,
    },
    /// Any other `${…}` form: a default, an alternate, a prefix or suffix
    /// removed, a length, an indirection.
    Expansion {
        /// Whether double quotes protect the value from splitting.
        quoted: bool,
    },
    /// `$(command)` or `` `command` ``: what it expands to is known only
    /// by running it.
    CommandSubstitution,
    /// `$((expression))`.
    Arithmetic,
}

/// One redirection, as source.
#[derive(Clone, Debug)]
#[non_exhaustive]
pub struct Redirection {
    /// The redirection spelled back as shell source: `2>&1`, `> log`.
    pub source: BString,
}

// ---------------------------------------------------------------------
// the projection
// ---------------------------------------------------------------------

fn command(locale: &nsh_platform::Locale, node: &Node) -> Command {
    match node {
        Node::Command(simple) => Command::Simple(simple_command(locale, simple)),
        Node::Pipeline(pipeline) => {
            let commands = Command::Pipeline {
                negated: false,
                commands: pipeline
                    .commands
                    .iter()
                    .map(|node| command(locale, node))
                    .collect(),
            };
            if pipeline.background {
                Command::Background(Box::new(commands))
            } else {
                commands
            }
        }
        Node::Redirect(compound) => redirected(
            locale,
            command(locale, &compound.command),
            &compound.redirections,
        ),
        /* A brace group runs its list in this shell, so what it holds is
         * what runs and the projection keeps the list. The braces decide
         * what a redirection or an `&` after them attaches to, and that is
         * what `Redirected` and `Background` are already saying: the list
         * is inside them where a bare `;` chain would not be. */
        Node::Group(group) => {
            redirected(locale, command(locale, &group.command), &group.redirections)
        }
        Node::Background(compound) => Command::Background(Box::new(redirected(
            locale,
            command(locale, &compound.command),
            &compound.redirections,
        ))),
        Node::Subshell(compound) => redirected(
            locale,
            Command::Subshell(Box::new(command(locale, &compound.command))),
            &compound.redirections,
        ),
        Node::And(binary) => Command::And(
            Box::new(command(locale, &binary.left)),
            Box::new(command(locale, &binary.right)),
        ),
        Node::Or(binary) => Command::Or(
            Box::new(command(locale, &binary.left)),
            Box::new(command(locale, &binary.right)),
        ),
        Node::Sequence(binary) => Command::Sequence(
            Box::new(command(locale, &binary.left)),
            Box::new(command(locale, &binary.right)),
        ),
        Node::If(conditional) => Command::If {
            condition: Box::new(command(locale, &conditional.condition)),
            then: Box::new(command(locale, &conditional.then_branch)),
            otherwise: conditional
                .else_branch
                .as_deref()
                .map(|branch| Box::new(command(locale, branch))),
        },
        Node::While(binary) => Command::While {
            condition: Box::new(command(locale, &binary.left)),
            body: Box::new(command(locale, &binary.right)),
        },
        Node::Until(binary) => Command::Until {
            condition: Box::new(command(locale, &binary.left)),
            body: Box::new(command(locale, &binary.right)),
        },
        Node::For(iteration) => Command::For {
            variable: iteration.variable.as_bstr().to_owned(),
            words: iteration
                .words
                .iter()
                .filter_map(|node| word_of(locale, node))
                .collect(),
            body: Box::new(command(locale, &iteration.body)),
        },
        Node::Case(selection) => Command::Case {
            word: word_of(locale, &selection.word).unwrap_or_else(empty_word),
            clauses: selection
                .clauses
                .iter()
                .map(|clause| CaseClause {
                    patterns: clause
                        .patterns
                        .iter()
                        .filter_map(|node| word_of(locale, node))
                        .collect(),
                    body: clause
                        .body
                        .as_deref()
                        .map(|body| Box::new(command(locale, body))),
                    fallthrough: clause.fallthrough,
                })
                .collect(),
        },
        Node::Function(definition) => Command::Function {
            name: definition.name.as_bstr().to_owned(),
            body: Box::new(command(locale, &definition.body)),
        },
        Node::Not(negated) => {
            let (negated, commands) = match command(locale, &negated.command) {
                Command::Pipeline { negated, commands } => (!negated, commands),
                other => (true, vec![other]),
            };
            Command::Pipeline { negated, commands }
        }
        Node::Select(_) | Node::Timed(_) | Node::Word(_) | Node::Bash(_) => Command::Other,
    }
}

/// A command with its redirections on it, or the command alone when there
/// are none: the grammar attaches an empty list to every compound.
fn redirected(
    locale: &nsh_platform::Locale,
    command: Command,
    redirections: &[nodes::Redirection],
) -> Command {
    if redirections.is_empty() {
        return command;
    }
    Command::Redirected {
        command: Box::new(command),
        redirections: redirections
            .iter()
            .map(|node| redirection(locale, node))
            .collect(),
    }
}

fn redirection(locale: &nsh_platform::Locale, redirection: &nodes::Redirection) -> Redirection {
    Redirection {
        source: nodes::source::redirection(locale, redirection),
    }
}

fn simple_command(locale: &nsh_platform::Locale, simple: &nodes::SimpleCommand) -> SimpleCommand {
    SimpleCommand {
        assignments: simple
            .assignments
            .iter()
            .filter_map(|node| match node {
                Node::Word(word) => Some(assignment(locale, word)),
                _ => None,
            })
            .collect(),
        words: simple
            .arguments
            .iter()
            .filter_map(|node| word_of(locale, node))
            .collect(),
        redirections: simple
            .redirections
            .iter()
            .map(|node| redirection(locale, node))
            .collect(),
    }
}

/// The word a grammar slot holds, when the slot holds a word. The slots
/// this is asked of hold words in every POSIX form; a Bash array node in
/// one is not a word and is left out.
fn word_of(locale: &nsh_platform::Locale, node: &Node) -> Option<Word> {
    match node {
        Node::Word(word) => Some(word_from_node(locale, word)),
        _ => None,
    }
}

fn empty_word() -> Word {
    Word {
        pieces: Vec::new(),
        source: BString::from("''"),
    }
}

/// A `name=value` word split at its `=`. The grammar admitted it as an
/// assignment only because the name is unquoted literal bytes, so the
/// first run holds the whole name.
fn assignment(locale: &nsh_platform::Locale, node: &WordNode) -> Assignment {
    let parts = node.word.parts();
    let Some((
        WordPart::Text {
            bytes: first,
            quoted: false,
        },
        rest,
    )) = parts.split_first()
    else {
        return Assignment {
            name: BString::default(),
            value: word_from_node(locale, node),
        };
    };
    let at = first
        .iter()
        .position(|byte| *byte == b'=')
        .unwrap_or(first.len());
    let name = BString::from(&first[..at]);
    let value_head = &first[(at + 1).min(first.len())..];
    /* The value is the rest of the first run, followed by the rest of the
     * parts. It is a word the shell cut rather than one it read, so it
     * carries no run of its own and its `source` is spelled from those
     * parts. */
    let mut value_parts = Vec::with_capacity(rest.len() + 1);
    if !value_head.is_empty() {
        value_parts.push(WordPart::Text {
            bytes: BString::from(value_head),
            quoted: false,
        });
    }
    value_parts.extend(rest.iter().cloned());
    let value = WordNode {
        tokens: SourceTokens::none(),
        word: ParsedWord::from_parts(value_parts),
    };
    Assignment {
        name,
        value: word_from_node(locale, &value),
    }
}

fn word_from_node(locale: &nsh_platform::Locale, node: &WordNode) -> Word {
    Word {
        pieces: pieces(&node.word),
        source: nodes::source::word(locale, node),
    }
}

/// One piece per structural part.
///
/// The tree merges adjacent runs of one inertness, so a part is already
/// the longest run of bytes that were protected the same way, and there
/// is nothing left here to join. Quoting is read off each part rather
/// than tracked across the word: a run says whether its bytes are data,
/// and an expansion says whether its result splits and globs.
fn pieces(word: &ParsedWord) -> Vec<Piece> {
    word.parts()
        .iter()
        .map(|part| match part {
            WordPart::Text { bytes, quoted } => Piece::Literal {
                bytes: bytes.clone(),
                quoted: *quoted,
            },
            WordPart::Parameter(parameter) => {
                let plain = parameter.operation == ParameterOperation::Value
                    && !parameter.indirect
                    && !parameter.colon
                    && parameter.operand.is_none();
                if plain {
                    Piece::Parameter {
                        name: parameter.name.clone(),
                        quoted: parameter.quoted,
                    }
                } else {
                    Piece::Expansion {
                        quoted: parameter.quoted,
                    }
                }
            }
            WordPart::Command { .. } => Piece::CommandSubstitution,
            WordPart::Arithmetic { .. } => Piece::Arithmetic,
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn read(source: &[u8]) -> Script {
        Reader::new()
            .expect("a reader")
            .read(BStr::new(source))
            .expect("a script")
    }

    fn one(source: &[u8]) -> Command {
        let mut script = read(source);
        assert_eq!(script.commands.len(), 1, "{source:?}");
        script.commands.remove(0)
    }

    fn literal(piece: &Piece) -> (&BStr, bool) {
        match piece {
            Piece::Literal { bytes, quoted } => (bytes.as_ref(), *quoted),
            other => panic!("not a literal: {other:?}"),
        }
    }

    /// A quote that closed over nothing is still something an embedder
    /// has to see: `x=''` is one empty field where `x=` inside a `for`
    /// list is none, and a projection that dropped the run could not tell
    /// a recipe's `test "$v" = ''` from `test "$v" =`.
    // [spec:nsh:req:idiom.canonical-tree+1/test]
    #[test]
    fn a_quote_that_wrote_nothing_is_a_piece() {
        let Command::Simple(simple) = one(b"echo '' a''b") else {
            panic!("not simple");
        };
        assert_eq!(literal(&simple.words[1].pieces[0]), (BStr::new(b""), true));
        assert_eq!(simple.words[1].pieces.len(), 1);
        assert_eq!(simple.words[1].source, "''");
        let split = &simple.words[2].pieces;
        assert_eq!(literal(&split[0]), (BStr::new(b"a"), false));
        assert_eq!(literal(&split[1]), (BStr::new(b""), true));
        assert_eq!(literal(&split[2]), (BStr::new(b"b"), false));
    }

    /// A brace group holds a list rather than a shell, so the projection
    /// keeps the list -- and the braces still show, because what they
    /// decide is what the redirection or the `&&` is applied to.
    #[test]
    fn a_brace_group_keeps_what_the_braces_decided() {
        let Command::Redirected {
            command,
            redirections,
        } = one(b"{ a; b; } > log")
        else {
            panic!("not redirected");
        };
        assert!(matches!(*command, Command::Sequence(_, _)));
        assert_eq!(redirections[0].source, "> log");
        // `{ a; b; } && c` runs `c` after both, `a; b && c` after `b`.
        assert!(matches!(
            one(b"{ a; b; } && c"),
            Command::And(left, _) if matches!(*left, Command::Sequence(_, _))
        ));
        assert!(matches!(
            one(b"a; b && c"),
            Command::Sequence(_, right) if matches!(*right, Command::And(_, _))
        ));
    }

    #[test]
    fn a_simple_command_is_its_words() {
        let Command::Simple(simple) = one(b"make -C sub all") else {
            panic!("not simple");
        };
        let words = simple
            .words
            .iter()
            .map(|word| word.source.clone())
            .collect::<Vec<_>>();
        assert_eq!(words, ["make", "-C", "sub", "all"]);
        assert!(simple.assignments.is_empty());
        assert!(simple.redirections.is_empty());
    }

    #[test]
    fn quoting_is_read_into_each_piece() {
        let Command::Simple(simple) = one(b"echo a'b c'\"$x\"$y\\ d") else {
            panic!("not simple");
        };
        let pieces = &simple.words[1].pieces;
        assert_eq!(literal(&pieces[0]), (BStr::new(b"a"), false));
        assert_eq!(literal(&pieces[1]), (BStr::new(b"b c"), true));
        assert!(matches!(&pieces[2], Piece::Parameter { name, quoted: true } if name == "x"));
        assert!(matches!(&pieces[3], Piece::Parameter { name, quoted: false } if name == "y"));
        // The escaped blank and the `d` after it are both text, and one
        // was protected where the other was not.
        assert_eq!(literal(&pieces[4]), (BStr::new(b" "), true));
        assert_eq!(literal(&pieces[5]), (BStr::new(b"d"), false));
    }

    #[test]
    fn a_words_source_reads_back_as_itself() {
        let Command::Simple(simple) = one(b"echo 'it''s' \"$v\" $(pwd) plain") else {
            panic!("not simple");
        };
        let sources = simple
            .words
            .iter()
            .map(|word| word.source.clone())
            .collect::<Vec<_>>();
        // Two quoted regions side by side, spelled back as the two they are.
        assert_eq!(sources[1], "'it''s'");
        assert_eq!(sources[2], "\"$v\"");
        assert_eq!(sources[3], "$(pwd)");
        assert_eq!(sources[4], "plain");
    }

    #[test]
    fn an_assignment_is_split_at_its_equals() {
        let Command::Simple(simple) = one(b"subdirs='Src Doc' V=a$b") else {
            panic!("not simple");
        };
        assert!(simple.words.is_empty());
        assert_eq!(simple.assignments[0].name, "subdirs");
        assert_eq!(
            literal(&simple.assignments[0].value.pieces[0]),
            (BStr::new(b"Src Doc"), true)
        );
        assert_eq!(simple.assignments[1].name, "V");
        assert_eq!(
            literal(&simple.assignments[1].value.pieces[0]),
            (BStr::new(b"a"), false)
        );
        assert!(matches!(
            &simple.assignments[1].value.pieces[1],
            Piece::Parameter { name, quoted: false } if name == "b"
        ));
    }

    #[test]
    fn expansions_that_run_something_say_so() {
        let Command::Simple(simple) = one(b"x `date` $((1+2)) ${v:-d} ${#v}") else {
            panic!("not simple");
        };
        assert!(matches!(
            simple.words[1].pieces[0],
            Piece::CommandSubstitution
        ));
        assert!(matches!(simple.words[2].pieces[0], Piece::Arithmetic));
        assert!(matches!(
            simple.words[3].pieces[0],
            Piece::Expansion { quoted: false }
        ));
        assert!(matches!(
            simple.words[4].pieces[0],
            Piece::Expansion { quoted: false }
        ));
    }

    #[test]
    fn the_grammar_keeps_its_shape() {
        let source = b"for d in Src Doc; do (cd $d && make $@) || exit 1; done";
        let Command::For {
            variable,
            words,
            body,
        } = one(source)
        else {
            panic!("not a for");
        };
        assert_eq!(variable, "d");
        assert_eq!(words.len(), 2);
        let Command::Or(left, right) = *body else {
            panic!("the body is not an alternation: {body:?}");
        };
        let Command::Subshell(inner) = *left else {
            panic!("the left is not a subshell: {left:?}");
        };
        assert!(matches!(*inner, Command::And(_, _)));
        let Command::Simple(exit) = *right else {
            panic!("the right is not simple: {right:?}");
        };
        assert_eq!(exit.words[0].source, "exit");
    }

    #[test]
    fn redirections_and_negation_and_background_are_kept() {
        assert!(matches!(
            one(b"make > log"),
            Command::Simple(SimpleCommand { redirections, .. }) if redirections.len() == 1
        ));
        assert!(matches!(
            one(b"{ make; } 2>&1"),
            Command::Redirected { redirections, .. } if redirections[0].source == "2>&1"
        ));
        assert!(matches!(
            one(b"! make | tee log"),
            Command::Pipeline { negated: true, commands } if commands.len() == 2
        ));
        assert!(matches!(one(b"make &"), Command::Background(_)));
        assert!(matches!(one(b"(make) &"), Command::Background(_)));
    }

    #[test]
    fn conditionals_loops_and_cases_are_kept() {
        assert!(matches!(
            one(b"if test -n \"$v\"; then make; elif true; then :; else make b; fi"),
            Command::If {
                otherwise: Some(_),
                ..
            }
        ));
        assert!(matches!(
            one(b"while true; do make; done"),
            Command::While { .. }
        ));
        assert!(matches!(
            one(b"until true; do make; done"),
            Command::Until { .. }
        ));
        let Command::Case { word, clauses } = one(b"case $x in a|b) make;; *) ;; esac") else {
            panic!("not a case");
        };
        assert!(matches!(word.pieces[0], Piece::Parameter { .. }));
        assert_eq!(clauses.len(), 2);
        assert_eq!(clauses[0].patterns.len(), 2);
        assert!(clauses[1].body.is_none());
        assert!(matches!(one(b"f() { make; }"), Command::Function { name, .. } if name == "f"));
    }

    #[test]
    fn a_for_without_in_iterates_the_positional_parameters() {
        let Command::For { words, .. } = one(b"for x; do :; done") else {
            panic!("not a for");
        };
        assert!(
            matches!(&words[0].pieces[..], [Piece::Parameter { name, quoted: true }] if name == "@")
        );
    }

    #[test]
    fn several_lines_are_several_commands() {
        let script = read(b"a=1\n\nmake\n");
        assert_eq!(script.commands.len(), 2);
    }

    #[test]
    fn a_syntax_error_is_returned_not_written() {
        let mut reader = Reader::new().expect("a reader");
        let error = reader
            .read(BStr::new(b"for d in a b; do make"))
            .expect_err("an unterminated loop");
        assert!(
            error.message().starts_with(b"Syntax error"),
            "{}",
            error.message()
        );
        // The reader is still a reader afterwards.
        assert!(reader.read(BStr::new(b"make")).is_ok());
    }

    #[test]
    fn a_reader_reads_the_same_way_twice() {
        let mut reader = Reader::new().expect("a reader");
        let source = BStr::new(b"x=1; for d in $x; do make; done");
        let first = format!("{:?}", reader.read(source).expect("first").commands);
        let second = format!("{:?}", reader.read(source).expect("second").commands);
        assert_eq!(first, second);
    }

    #[test]
    fn bash_forms_are_other_under_posix() {
        // `[[` is not a reserved word to a POSIX reader: it is a command
        // called `[[`, and this reader says so rather than guessing.
        assert!(matches!(one(b"[[ -n x ]]"), Command::Simple(_)));
        assert!(matches!(one(b"time make"), Command::Other));
    }

    /// A Bash reader is asked about Bash, not about POSIX.
    ///
    /// The same bytes are a different program in the two dialects, so a
    /// reader that could only be POSIX would answer a Bash embedder
    /// confidently and wrongly. `[[ -n x ]]` is the shortest pair: a
    /// simple command named `[[` in one dialect, a conditional this
    /// projection does not describe in the other.
    // [spec:nsh:req:embedding-safety.reading-without-running/test]
    #[test]
    fn a_reader_reads_the_dialect_it_was_given() {
        let posix = Reader::new()
            .expect("a reader")
            .read(BStr::new(b"[[ -n x ]]"))
            .expect("a script");
        let bash = Reader::bash()
            .expect("a reader")
            .read(BStr::new(b"[[ -n x ]]"))
            .expect("a script");
        assert!(matches!(posix.commands[0], Command::Simple(_)));
        assert!(matches!(bash.commands[0], Command::Other));
    }

    /// Run a script in an ordinary shell and give back what it wrote.
    fn ran(source: &[u8]) -> BString {
        let mut shell = Shell::builder()
            .streams(crate::streams::Streams::capture().expect("captured streams"))
            .build()
            .expect("a shell");
        shell.run(BStr::new(source)).expect("a run");
        shell.take_captured_stdout().expect("the output")
    }

    /// A pair the shell runs differently has to read differently.
    ///
    /// Nothing in this repository consumes the projection, so a
    /// distinction it drops is invisible here unless something goes
    /// looking. This is what looks.
    ///
    /// The pairs are not held to a recorded expectation. Each side is
    /// RUN, in this shell, and the two outputs must differ before the
    /// reading is asked about at all -- so the projection is measured
    /// against the shell's own behaviour rather than against itself, and
    /// a pair that has stopped being a pair is reported rather than
    /// quietly passed.
    // [spec:nsh:req:embedding-safety.reading-without-running/test]
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn a_pair_the_shell_runs_differently_reads_differently() {
        let _guard = crate::test_support::lock();
        for (left, right) in [
            /* Splitting: one field or two. */
            (&b"printf '%s|' 'a b'"[..], &b"printf '%s|' a b"[..]),
            /* A quoted expansion against the same bytes as data. */
            (b"x=y; printf '%s|' \"$x\"", b"x=y; printf '%s|' '$x'"),
            /* A substitution against its own spelling. */
            (b"printf '%s|' \"$(echo h)\"", b"printf '%s|' '$(echo h)'"),
            /* Arithmetic against its own spelling. */
            (b"printf '%s|' $((1+1))", b"printf '%s|' '$((1+1))'"),
            /* An empty quoted run is a field; nothing is not. */
            (b"printf '%s|' '' x", b"printf '%s|' x"),
            /* The operator between two commands. */
            (b"false && printf 'x|'", b"false; printf 'x|'"),
        ] {
            assert_ne!(
                ran(left),
                ran(right),
                "{:?} and {:?} no longer run differently, so this pair \
                 measures nothing and has to be replaced",
                BStr::new(left),
                BStr::new(right),
            );
            assert_ne!(
                format!("{:?}", read(left).commands),
                format!("{:?}", read(right).commands),
                "{:?} and {:?} run differently and read the same, so the \
                 reading lost a distinction the shell draws",
                BStr::new(left),
                BStr::new(right),
            );
        }
    }
}
