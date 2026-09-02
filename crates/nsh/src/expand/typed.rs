//! Expansion over the structural word representation.
//!
//! Each output byte carries the two facts needed by later phases: whether
//! quoting protects it from pattern syntax, and whether it came from an
//! unquoted expansion and is therefore eligible for IFS splitting.  Field
//! boundaries are values too, most notably for `"$@"`; no sentinel bytes are
//! inserted into the shell data.

// [spec:nsh:req:idiom.operation-modes]
use bstr::{BStr, BString, ByteSlice};
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};

use super::{ExpandedField, ExpandedFields, ExpansionMode};
use crate::characters::boundaries as character_boundaries;
use crate::context::Shell;
use crate::error::Error;
use crate::nodes::Node;
use crate::options::{Dialect, OPTION_SPECS, ShellOption};
// [spec:nsh:def:idiom.shell-options]
use crate::pattern::Pattern;
use crate::variables::value::VariableValue;
use crate::word::{ParameterExpansion, ParameterOperation, ParsedWord, WordPart};

mod bash;
mod brace;
mod field;
mod pathname;
mod split;

use field::Field;
#[cfg(test)]
use field::FieldRegion;

#[derive(Clone, Debug)]
struct Expansion {
    fields: Vec<Field>,
}

impl Expansion {
    /// One empty field that survives field splitting.
    ///
    /// `""` writes a field the shell must keep even though it has no
    /// bytes. An expansion that produces *no* fields is a different
    /// thing and must stay distinguishable from this one.
    // [spec:posix:req:expand.quote-removal]
    fn anchored_empty() -> Self {
        let mut field = Field::default();
        field.anchor_empty();
        Self {
            fields: vec![field],
        }
    }

    fn none() -> Self {
        Self { fields: Vec::new() }
    }

    fn one(field: Field) -> Self {
        Self {
            fields: vec![field],
        }
    }

    /// Concatenate a shell expansion. The first resulting field joins the
    /// current last field and any additional fields retain their boundary.
    /// This is the prefix/suffix rule for `"pre$@post"` expressed directly.
    fn append(&mut self, mut other: Self) {
        if other.fields.is_empty() {
            return;
        }
        if self.fields.is_empty() {
            self.fields = other.fields;
            return;
        }
        let first = other.fields.remove(0);
        self.fields
            .last_mut()
            .expect("nonempty expansion")
            .append(first);
        self.fields.append(&mut other.fields);
    }

    fn preserve_empty(&mut self) {
        for field in &mut self.fields {
            field.anchor_empty();
        }
    }

    fn collapse(mut self) -> Field {
        if self.fields.is_empty() {
            return Field::default();
        }
        let mut first = self.fields.remove(0);
        for field in self.fields {
            first.append(field);
        }
        first
    }
}

#[derive(Clone, Copy)]
struct Context {
    quoted: bool,
    full: bool,
    operand: bool,
    pattern: bool,
    tilde_at_start: bool,
    tilde_after_equal: bool,
    tilde_after_colon: bool,
}

impl Context {
    fn top(mode: ExpansionMode) -> Self {
        Self {
            quoted: mode.contains(ExpansionMode::QUOTED),
            full: mode.contains(ExpansionMode::SPLIT),
            operand: false,
            pattern: false,
            tilde_at_start: mode.contains(ExpansionMode::TILDE),
            tilde_after_equal: mode.contains(ExpansionMode::ASSIGNMENT_TILDE),
            tilde_after_colon: mode
                .intersects(ExpansionMode::ASSIGNMENT_TILDE | ExpansionMode::COLON_TILDE),
        }
    }

    fn quoted(self) -> Self {
        Self {
            quoted: true,
            tilde_at_start: false,
            tilde_after_equal: false,
            tilde_after_colon: false,
            ..self
        }
    }

    fn operand(self) -> Self {
        Self {
            operand: true,
            tilde_at_start: true,
            tilde_after_equal: false,
            // An operand remains part of the surrounding assignment word;
            // POSIX.1-2024 therefore keeps `:`-separated tilde prefixes
            // active inside `${parameter-word}`.
            tilde_after_colon: self.tilde_after_colon,
            ..self
        }
    }

    fn pattern_operand(self) -> Self {
        Self {
            quoted: false,
            full: false,
            operand: false,
            pattern: true,
            tilde_at_start: true,
            tilde_after_equal: false,
            tilde_after_colon: false,
        }
    }

    /// The context a `${name/pattern/replacement}` replacement expands in.
    ///
    /// Bash takes the surrounding double quotes off before expanding a
    /// replacement, so `"${v/b/~}"` expands the tilde the outer quoting
    /// would otherwise have made literal, and `$@` joins the way an
    /// unsplit context joins rather than producing fields. What is left
    /// on the bytes is the quoting the replacement's own source wrote,
    /// which is the bit `&` reads to decide whether it names the match.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    fn replacement_operand(self) -> Self {
        Self {
            quoted: false,
            full: false,
            operand: true,
            pattern: false,
            tilde_at_start: true,
            tilde_after_equal: false,
            tilde_after_colon: false,
        }
    }

    fn protects(self) -> bool {
        self.quoted
    }

    fn splits(self) -> bool {
        self.full && !self.quoted && !self.pattern
    }

    fn literal_splits(self) -> bool {
        self.operand && self.splits()
    }
}

// [spec:nsh:sem:idiom.typed-expansion]
// [spec:nsh:req:idiom.parser-control-flow]
// [spec:dash:sem:expand.argstr-fn]
pub(super) fn expand_argument(
    shell: &mut Shell,
    word: &ParsedWord,
    output: Option<&mut ExpandedFields>,
    mode: ExpansionMode,
) -> Result<(), Error> {
    let context = Context::top(mode);
    let Some(output) = output else {
        let field = expand_parts(shell, word.parts(), context)?.collapse();
        shell.expand.buffer.clear();
        shell.expand.buffer.extend_from_slice(&field.bytes);
        return Ok(());
    };

    // Brace expansion precedes every other expansion and is the only one
    // that turns one word into several before they are expanded at all.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    let braced = if context.full {
        brace::expand(shell, word)?
    } else {
        Vec::new()
    };
    if braced.is_empty() {
        return expand_word(shell, word, output, context);
    }
    for word in &braced {
        expand_word(shell, word, output, context)?;
    }
    Ok(())
}

fn expand_word(
    shell: &mut Shell,
    word: &ParsedWord,
    output: &mut ExpandedFields,
    context: Context,
) -> Result<(), Error> {
    let expanded = expand_parts(shell, word.parts(), context)?;
    let fields = if context.full {
        let mut split = split::fields(shell, expanded.fields);
        if !shell.options.enabled(ShellOption::NoGlob) {
            let settings = pathname::settings(shell);
            split = pathname::expand(shell, split, &settings)?;
        }
        split
    } else {
        vec![expanded.collapse()]
    };
    output.fields.extend(fields.into_iter().map(into_field));
    Ok(())
}

/// Expand one word in a pattern position, retaining its quote bits.
///
/// `case` and `[[ ]]` ask the same question of a word, and the answer is
/// the pattern rather than a match: the regular-expression operator needs
/// the same bits to decide which bytes the shell already made literal.
// [spec:nsh:sem:idiom.typed-expansion]
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
pub(super) fn conditional_pattern_of(
    shell: &mut Shell,
    word: &ParsedWord,
) -> Result<Pattern, Error> {
    let mut options = bash::match_options(shell);
    options.extended = shell.options.dialect() == Dialect::Bash;
    Ok(pattern_field(shell, word)?.pattern(options))
}

pub(super) fn pattern_of(shell: &mut Shell, word: &ParsedWord) -> Result<Pattern, Error> {
    let options = bash::match_options(shell);
    Ok(pattern_field(shell, word)?.pattern(options))
}

fn pattern_field(shell: &mut Shell, word: &ParsedWord) -> Result<Field, Error> {
    let context = Context {
        quoted: false,
        full: false,
        operand: false,
        pattern: true,
        tilde_at_start: true,
        tilde_after_equal: false,
        tilde_after_colon: false,
    };
    Ok(expand_parts(shell, word.parts(), context)?.collapse())
}

// Quote protection is metadata during expansion and is discarded only when
// the final owned field is materialized; no escape marker bytes are removed.
// [spec:dash:sem:expand.rmescapes-fn]
// [spec:posix:req:expand.quote-removal]
// [spec:posix:sem:expand.quote-removal-quoting-remembered]
// [spec:posix:syn:pattern.backslash-escape-with-shell-quoting]
// [spec:posix:syn:pattern.backslash-escape-without-shell-quoting]
// [spec:posix:req:pattern.escaping-follows-quoting-rules]
// [spec:posix:syn:pattern.trailing-backslash-unspecified]
// [spec:posix:req:pattern.quote-to-match-literally]
fn into_field(field: Field) -> ExpandedField {
    ExpandedField { text: field.bytes }
}

// The former helpers allocated fixed C buffers and threaded cursors through
// them. Structural words, owned fields, and Rust's integer formatting now
// implement the same observable expansion behavior without those mechanisms.
// [spec:dash:sem:expand.expari-fn]
// [spec:dash:sem:expand.expcmd-fn]
// [spec:dash:sem:shell.max-int-length-fn]
// [spec:posix:req:expand.arith-token-expansion]
fn expand_parts(
    shell: &mut Shell,
    parts: &[WordPart],
    context: Context,
) -> Result<Expansion, Error> {
    /* No field is seeded. A part that expands to nothing -- `"$@"` with
     * no positional parameters, `"${a[@]}"` over an empty array -- has to
     * be able to leave the word with no fields at all, and a seeded field
     * would silently turn that into one empty field. */
    // [spec:posix:req:param.special-at-no-positional]
    let mut result = Expansion::none();
    let mut at = 0;
    let mut tilde = if context.tilde_at_start && !context.quoted {
        /* Where `:` separates tilde prefixes, it ends the first one too:
         * an assignment operand -- `${x-~:~}` inside one, or the value of
         * a `[k]=~:~` element, whose `name[k]=` the parser already took
         * off -- expands every colon-separated segment. */
        if context.tilde_after_colon {
            TildePosition::Assignment
        } else {
            TildePosition::WordStart
        }
    } else {
        TildePosition::None
    };
    let mut assignment_equal_available = context.tilde_after_equal;

    while at < parts.len() {
        match &parts[at] {
            WordPart::Text {
                bytes,
                quoted: false,
            } => {
                append_literal(
                    shell,
                    &mut result,
                    bytes,
                    context,
                    at + 1 < parts.len(),
                    &mut tilde,
                    &mut assignment_equal_available,
                );
            }
            /* A quoted run is data: it protects, it does not split, and
             * `''` is written text that keeps its empty field where an
             * expansion yielding nothing does not. Which quote made it so
             * is the node's run and not the tree's business. */
            // [spec:nsh:req:idiom.canonical-tree+1]
            WordPart::Text {
                bytes,
                quoted: true,
            } => {
                let mut quoted = if bytes.is_empty() {
                    Expansion::anchored_empty()
                } else {
                    let mut inner = Expansion::none();
                    let mut no_tilde = TildePosition::None;
                    let mut no_assignment = false;
                    append_literal(
                        shell,
                        &mut inner,
                        bytes,
                        context.quoted(),
                        at + 1 < parts.len(),
                        &mut no_tilde,
                        &mut no_assignment,
                    );
                    inner
                };
                quoted.preserve_empty();
                result.append(quoted);
                tilde = TildePosition::None;
            }
            WordPart::Parameter(parameter) => {
                let inner = quoting(context, parameter.quoted);
                let mut expanded = expand_parameter(shell, parameter, inner)?;
                if parameter.quoted {
                    expanded.preserve_empty();
                }
                result.append(expanded);
                tilde = TildePosition::None;
            }
            WordPart::Command { command, quoted } => {
                /* `$(list)` contributes the bytes the list wrote, and they
                 * are the list's data: unquoted, they split and they glob.
                 * Bash's `<(list)` and `>(list)` occupy the same lexical
                 * position and the same word part, but contribute a name the
                 * shell chose for a pipe the list is still using. That name
                 * is not data, so an `IFS` containing `/` leaves it whole. */
                // [spec:nsh:req:compat.bash.process-substitution]
                let inner = quoting(context, *quoted);
                let field = match command.as_deref() {
                    Some(Node::Bash(crate::nodes::BashNode::ProcessSubstitution(substitution))) => {
                        let name = crate::evaluation::bash_process_substitution::substitute(
                            shell,
                            substitution,
                        )?;
                        Field::from_bytes(&name, true, false, inner.quoted)
                    }
                    command => {
                        let bytes = command_substitution(shell, command)?;
                        Field::from_bytes(&bytes, inner.protects(), inner.splits(), inner.quoted)
                    }
                };
                let mut expanded = Expansion::one(field);
                if *quoted {
                    expanded.preserve_empty();
                }
                result.append(expanded);
                tilde = TildePosition::None;
            }
            WordPart::Arithmetic { expression, quoted } => {
                let arithmetic_context = Context {
                    quoted: false,
                    full: false,
                    operand: false,
                    pattern: false,
                    tilde_at_start: false,
                    tilde_after_equal: false,
                    tilde_after_colon: false,
                };
                let inner = quoting(context, *quoted);
                let expression =
                    expand_parts(shell, expression.parts(), arithmetic_context)?.collapse();
                let number = crate::arithmetic::evaluate(shell, expression.bytes.as_bstr())?;
                let rendered = number.to_string();
                let mut expanded = Expansion::one(Field::from_bytes(
                    rendered.as_bytes(),
                    inner.protects(),
                    inner.splits(),
                    inner.quoted,
                ));
                if *quoted {
                    expanded.preserve_empty();
                }
                result.append(expanded);
                tilde = TildePosition::None;
            }
        }
        at += 1;
    }
    Ok(result)
}

// [spec:posix:def:expand.tilde-prefix]
// [spec:posix:def:expand.tilde-prefix-in-assignment]
#[derive(Clone, Copy, Eq, PartialEq)]
enum TildePosition {
    None,
    WordStart,
    Assignment,
}

/// The context a part expands in, given whether the source quoted it.
///
/// Quoting used to be a pair of parts around a region and is a flag on
/// the part now, so the region walk is a question asked once per part.
// [spec:nsh:req:idiom.canonical-tree+1]
fn quoting(context: Context, quoted: bool) -> Context {
    if quoted { context.quoted() } else { context }
}

// [spec:posix:sem:expand.tilde-no-further-expansion]
// [spec:posix:req:expand.tilde-result-quoted]
// [spec:dash:sem:expand.chtodest-fn]
// [spec:dash:sem:expand.mbtodest-fn]
// [spec:dash:sem:expand.memtodest-fn]
// [spec:dash:sem:expand.strtodest-fn]
fn append_literal(
    shell: &mut Shell,
    result: &mut Expansion,
    bytes: &[u8],
    context: Context,
    has_following_parts: bool,
    tilde: &mut TildePosition,
    assignment_equal_available: &mut bool,
) {
    let mut at = 0;
    while at < bytes.len() {
        if *tilde != TildePosition::None && bytes[at] == b'~' && !context.quoted {
            let end = bytes[at + 1..]
                .iter()
                .position(|byte| {
                    *byte == b'/' || (*byte == b':' && *tilde == TildePosition::Assignment)
                })
                .map_or(bytes.len(), |offset| at + 1 + offset);
            if !(end == bytes.len() && has_following_parts)
                && let Some(home) = tilde_home(shell, &bytes[at + 1..end])
            {
                result.append(Expansion::one(Field::from_bytes(&home, true, false, false)));
                at = end;
                *tilde = TildePosition::None;
                continue;
            }
        }

        /* The loop above is a state machine over three bytes and nothing
         * else reads a literal one at a time, so the bytes between them
         * are one contribution rather than one each. The field is the
         * same either way -- adjacent regions of equal quoting merge, and
         * an anchor inside a run sits on a byte quoting made unsplittable,
         * which is a byte splitting never asks about -- but a byte at a
         * time buys a field, four allocations and an anchor per byte of
         * the word, and the anchors then made appending quadratic. */
        let end = if TILDE_SIGNIFICANT.contains(&bytes[at]) {
            at + 1
        } else {
            bytes[at + 1..]
                .iter()
                .position(|byte| TILDE_SIGNIFICANT.contains(byte))
                .map_or(bytes.len(), |offset| at + 1 + offset)
        };
        result.append(Expansion::one(Field::from_bytes(
            &bytes[at..end],
            context.protects(),
            context.literal_splits(),
            context.quoted,
        )));
        let byte = bytes[end - 1];
        *tilde = if byte == b'=' && *assignment_equal_available {
            *assignment_equal_available = false;
            TildePosition::Assignment
        } else if byte == b':' && context.tilde_after_colon {
            TildePosition::Assignment
        } else {
            TildePosition::None
        };
        at = end;
    }
}

/// The literal bytes the tilde rules can tell apart.
///
/// `~` opens a prefix, and `=` and `:` are the separators after which
/// another may open. Every other byte closes any prefix under way and is
/// otherwise indistinguishable from its neighbours.
const TILDE_SIGNIFICANT: &[u8] = b"~=:";

// [spec:posix:req:expand.tilde-home]
// [spec:posix:req:expand.tilde-login-name]
// [spec:posix:req:expand.tilde-replacement-pathname]
// [spec:dash:sem:expand.exptilde-fn]
fn tilde_home(shell: &mut Shell, user: &[u8]) -> Option<Vec<u8>> {
    if user.is_empty() {
        crate::variables::lookup_bytes(shell, BStr::new(b"HOME")).map(|home| home.to_vec())
    } else {
        let user = user.try_to_os_string().ok()?;
        nsh_platform::named_user_home(&user).map(|home| home.to_shell_bytes())
    }
}

mod parameter;
use parameter::{
    Value, expand_parameter, first_ifs_character, join_parameters, split_subscript, value_bytes,
    value_expansion,
};

fn character_count(locale: &nsh_platform::Locale, bytes: &[u8]) -> usize {
    character_boundaries(locale, bytes).len().saturating_sub(1)
}

// [spec:posix:req:expand.cmdsub-semantics]
// [spec:posix:req:expand.cmdsub-no-reexpansion]
// [spec:dash:sem:expand.expbackq-fn]
fn command_substitution(shell: &mut Shell, command: Option<&Node>) -> Result<BString, Error> {
    // `$(<file)` never reaches a child: it is a file read wearing command
    // syntax.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    let mut output = match bash::file_substitution(shell, command)? {
        Some(content) => content,
        None => run_command_substitution(shell, command)?,
    };
    while output.last() == Some(&b'\n') {
        output.pop();
    }
    Ok(output)
}

fn run_command_substitution(shell: &mut Shell, command: Option<&Node>) -> Result<BString, Error> {
    let mut result = crate::evaluation::CommandSubstitution {
        descriptor: None,
        job_id: None,
    };
    let output = crate::error::with_interrupts_deferred(shell, |shell| {
        let mut output = BString::new(Vec::new());
        let mut buffer = [0u8; 128];

        crate::evaluation::evaluate_command_substitution(shell, command, &mut result)?;
        while let Some(descriptor) = result.descriptor.as_ref() {
            let count = loop {
                match nsh_platform::read_once(descriptor, &mut buffer) {
                    Ok(count) => break count,
                    Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                    Err(_) => break 0,
                }
            };
            if count == 0 {
                break;
            }
            output.extend(buffer[..count].iter().copied().filter(|byte| *byte != 0));
        }
        if result.descriptor.take().is_some() {
            shell.evaluation.command_substitution_status =
                crate::jobs::wait_for_job(shell, result.job_id)?;
        }
        Ok::<_, Error>(output)
    })?;

    if let Some(error) = crate::error::poll_interrupt(shell.interrupt_context()) {
        return Err(error);
    }
    Ok(output)
}

fn effective_ifs(shell: &Shell) -> &[u8] {
    &shell.ifs.bytes
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:sem:idiom.typed-expansion/test]
    // [spec:nsh:def:idiom.variable-expansion-state/test]
    #[test]
    fn typed_field_masks() {
        let field = Field::from_bytes(b"a*b", true, false, true);
        assert_eq!(field.bytes, BString::from("a*b"));
        assert_eq!(
            field.regions,
            vec![FieldRegion {
                start: 0,
                end: 3,
                quoted: true,
                splittable: false,
            }]
        );
        assert_eq!(field.empty_anchors, vec![3]);

        let empty = Field::from_bytes(b"", true, false, true);
        assert_eq!(empty.empty_anchors, vec![0]);

        let indexed = Value::Variable(VariableValue::empty(
            crate::variables::value::VariableKind::Indexed,
        ));
        let associative = Value::Variable(VariableValue::empty(
            crate::variables::value::VariableKind::Associative,
        ));

        assert!(!indexed.is_unset());
        let shell = Shell::builder().build().unwrap();
        assert!(indexed.is_empty(&shell, Context::top(ExpansionMode::SPLIT)));
        assert!(matches!(
            indexed,
            Value::Variable(VariableValue::Indexed(_))
        ));
        assert!(matches!(
            associative,
            Value::Variable(VariableValue::Associative(_))
        ));
    }

    #[test]
    fn expanded_backslash_quotes_pattern_byte() {
        let locale = nsh_platform::Locale::c().unwrap();
        let pattern = Field::from_bytes(b"\\*", false, false, false)
            .pattern(crate::pattern::PatternOptions::NONE);

        assert!(pattern.matches(&locale, b"*"));
        assert!(!pattern.matches(&locale, b"anything"));
    }

    #[test]
    fn field_metadata_stays_sparse() {
        let mut field = Field::from_bytes(&vec![b'x'; 131_072], false, true, false);
        assert_eq!(field.regions.len(), 1);

        field.append(Field::from_bytes(b"quoted", true, false, false));
        field.append(Field::from_bytes(b"-tail", true, false, false));
        assert_eq!(field.regions.len(), 2);

        let slice = field.slice(131_068..131_080);
        assert_eq!(slice.bytes, BString::from("xxxxquoted-t"));
        assert_eq!(
            slice.regions,
            [
                FieldRegion {
                    start: 0,
                    end: 4,
                    quoted: false,
                    splittable: true,
                },
                FieldRegion {
                    start: 4,
                    end: 12,
                    quoted: true,
                    splittable: false,
                },
            ]
        );

        /* Sparse is also what a long word costs. A quoted run appended a
         * byte at a time left an anchor at every byte boundary and then
         * rescanned every anchor already recorded on the next append, so
         * reading a 200,000-byte word took seconds where dash takes
         * milliseconds. The metadata is the shape that gives that away
         * and a clock on a shared machine is not: what a run records must
         * not grow with how long the run is. */
        let mut shell = Shell::builder().build().unwrap();
        let mut recorded = |length: usize| {
            let mut expansion = Expansion::none();
            let mut tilde = TildePosition::None;
            let mut assignment_equal_available = false;
            append_literal(
                &mut shell,
                &mut expansion,
                &vec![b'a'; length],
                Context::top(ExpansionMode::SPLIT).quoted(),
                false,
                &mut tilde,
                &mut assignment_equal_available,
            );
            let run = expansion.collapse();
            (run.bytes.len(), run.regions.len(), run.empty_anchors)
        };

        assert_eq!(recorded(16), (16, 1, vec![16]));
        assert_eq!(recorded(200_000), (200_000, 1, vec![200_000]));
    }
}
