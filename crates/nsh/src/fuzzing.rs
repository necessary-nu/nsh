//! The deliberately narrow interface used by the separate cargo-fuzz
//! workspace.
//!
//! This module is public only with the `fuzzing` feature. It provides an
//! opaque parse-and-print operation so the fuzzer can hold the renderer to
//! the bytes it was given, without exposing the AST or parser as library
//! API.

use bstr::{BStr, BString};

use crate::context::Shell;

/// What comparing a printed program against its source found.
///
/// A verdict rather than a pair of texts: the property is what the fuzz
/// workspace needs, and the syntax tree stays inside the crate that owns
/// it.
// [spec:nsh:req:idiom.printable-ast+2]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum RoundTrip {
    /// The source did not parse, so there was nothing to print. An
    /// ordinary answer to arbitrary bytes, and not a finding.
    NotParsed,
    /// An alias replaced text before the parser saw it, so the bytes it
    /// read are not the bytes that were written. Carved out by the rule
    /// rather than papered over.
    Aliased,
    /// Every byte came back.
    Exact,
    /// A byte did not, and `at` is the first offset where the printed
    /// program and the source disagree. An offset reduces far better than
    /// two whole texts.
    Differed { at: usize, printed: BString },
    /// The bytes came back and the marks into them did not agree with the
    /// tree: a node's run is not inside the run of the node above it.
    ///
    /// Separate from `Differed` because it is a different failure and was
    /// invisible to every property this replaces. Concatenating a tree's
    /// bytes only ever reads the outermost run, so a child whose run is
    /// empty, truncated, or pointing at someone else's tokens passes a
    /// byte comparison and breaks anything that renders a subtree --
    /// which is what `declare -f` does.
    Misplaced { outer: BString, inner: BString },
}

/// Whether printing `source`'s program gives `source` back, byte for byte.
///
/// [`spec:nsh:req:idiom.printable-ast+2`] made checkable. The comparison
/// is against the input itself and not against anything derived from the
/// tree, which is what makes it able to fail: a run taken from the wrong
/// place produces bytes that are not the ones that were read there.
///
/// Measured per parse unit, because a unit is what the parser consumes
/// and what the input frame's own cursor can be asked about independently
/// of the tree. A unit that produced no command contributes its own bytes
/// and is required to be nothing but trivia -- otherwise a parser that
/// silently dropped a command would satisfy this by having nothing to
/// print.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure]
// [spec:nsh:req:idiom.printable-ast+2]
pub fn round_trips_byte_exactly(shell: &mut Shell, source: &BStr) -> RoundTrip {
    crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string(shell, source);
        let mut printed = BString::new(Vec::new());
        let mut consumed = 0usize;
        loop {
            let outcome = crate::parser::parse_command(shell, false);
            if shell.input.tokens.expanded_alias() {
                return RoundTrip::Aliased;
            }
            let frame = crate::input::current_input_frame(&mut shell.input);
            let reached = frame
                .position
                .saturating_sub(frame.unread_count)
                .min(source.len());
            let unit = &source[consumed.min(reached)..reached];
            consumed = reached;
            match outcome {
                Err(_) => return RoundTrip::NotParsed,
                /* End of input still consumed whatever was in front of it
                 * -- a comment on the last line, blanks after the last
                 * command -- and those bytes are in no node, because
                 * trivia goes to whatever follows it and here nothing
                 * does. They are the unit's, so the unit contributes
                 * them. */
                Ok(crate::parser::ParseResult::Eof) => {
                    if !is_only_trivia(unit) {
                        return RoundTrip::Differed {
                            at: printed.len(),
                            printed,
                        };
                    }
                    printed.extend_from_slice(unit);
                    break;
                }
                Ok(crate::parser::ParseResult::Tree(None)) => {
                    if !is_only_trivia(unit) {
                        return RoundTrip::Differed {
                            at: printed.len(),
                            printed,
                        };
                    }
                    printed.extend_from_slice(unit);
                }
                Ok(crate::parser::ParseResult::Tree(Some(node))) => {
                    if let Some((outer, inner)) = crate::nodes::emit::misplaced_run(&node) {
                        return RoundTrip::Misplaced {
                            outer: outer.text(),
                            inner: inner.text(),
                        };
                    }
                    match crate::nodes::emit::emitted(&node) {
                        Some(bytes) => printed.extend_from_slice(&bytes),
                        /* A unit the parser produced always has bytes; a
                         * node without them was built rather than read,
                         * and that path is the fallback's. */
                        None => {
                            return RoundTrip::Differed {
                                at: printed.len(),
                                printed,
                            };
                        }
                    }
                }
            }
        }
        /* The reader can stop short of the end when the last thing in
         * the file is trivia with no newline after it: end of input is
         * not a token, so nothing moves the cursor past a trailing
         * comment. Those bytes belong to the program and to no unit, and
         * they still have to be trivia -- a truncation leaves something
         * else there and is caught below. */
        if consumed < source.len() {
            let tail = &source[consumed..];
            if !is_only_trivia(tail) {
                return RoundTrip::Differed {
                    at: printed.len(),
                    printed,
                };
            }
            printed.extend_from_slice(tail);
        }
        if printed == source {
            return RoundTrip::Exact;
        }
        let at = printed
            .iter()
            .zip(source.iter())
            .position(|(written, read)| written != read)
            .unwrap_or_else(|| printed.len().min(source.len()));
        RoundTrip::Differed { at, printed }
    })
}

/// Whether a parse unit that produced no command read nothing but layout.
///
/// Blank lines and comments are what a unit with no command is made of.
/// Anything else means a command went missing, and the byte comparison
/// would not notice, because a unit with no node contributes its own
/// bytes.
// [spec:nsh:req:idiom.printable-ast+2]
fn is_only_trivia(unit: &[u8]) -> bool {
    let mut at = 0;
    while at < unit.len() {
        match unit[at] {
            b' ' | b'\t' | b'\n' | b'\r' => at += 1,
            b'\\' if unit.get(at + 1) == Some(&b'\n') => at += 2,
            b'#' => {
                at += unit[at..]
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .unwrap_or(unit.len() - at);
            }
            _ => return false,
        }
    }
    true
}

/// The commands of a sequence, in order, with the sequence itself gone.
///
/// A `;` and a newline between two commands are one program spelled two
/// ways, and only one of them nests. Flattening is what lets the
/// comparison ignore that without ignoring anything else.
// [spec:nsh:req:idiom.canonical-tree+1]
#[cfg(any(feature = "fuzzing", test))]
fn flattened<'tree>(node: &'tree crate::nodes::Node, out: &mut Vec<&'tree crate::nodes::Node>) {
    if let crate::nodes::Node::Sequence(list) = node {
        flattened(&list.left, out);
        flattened(&list.right, out);
        return;
    }
    out.push(node);
}

/// What comparing a program against a respelling of itself found.
///
/// Distinct from [`RoundTrip`] because it is a different question about a
/// different thing. That one compares *text*: it emits the runs a node
/// kept and requires the source back byte for byte. This one compares
/// *programs*, with the runs deliberately set aside -- and a tree holding
/// two shapes for one program passes the first and fails this, because
/// both shapes carry their own tokens and both print back exactly.
// [spec:nsh:req:idiom.canonical-tree+1]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Canonicity {
    /// The source did not parse, so there was no tree to respell. Not a
    /// finding.
    NotParsed,
    /// An alias replaced text before the parser saw it. The bytes read
    /// are not the bytes written, so the two spellings are not a pair.
    Aliased,
    /// Both spellings built one program.
    OneTree,
    /// They did not: the same program, written two ways, parsed to two
    /// structures. This is the finding the node exists for.
    TwoTrees { respelled: BString },
    /// The respelling could not be read back at all, which is the
    /// speller's defect rather than the parser's, and is reported apart
    /// from `TwoTrees` so a sweep does not confuse the two.
    Unreadable { respelled: BString },
}

/// Whether two spellings of one program build one tree.
///
/// [`spec:nsh:req:idiom.canonical-tree+1`] made checkable, and the
/// mechanical half of it: the equivalence class is *derived* rather than
/// listed. The source is one spelling; `nodes::source::respelled` renders
/// the parsed tree from its structure, ignoring every run in it, which is
/// the other. Requiring the two to parse equal is canonicity, and it
/// reaches classes no hand-written list contains -- which is the whole
/// reason to prefer it, twice over, after a corpus rather than a property
/// turned out to be the limit in two consecutive nodes.
///
/// The comparison ignores runs and positions, because `SourceTokens` and
/// `SourceLine` compare equal unconditionally by design. That is what
/// makes this a question about programs; `same_text` is the named
/// operation for asking about bytes, and this must not use it.
///
/// WHAT A FAILURE MEANS. Either the parser built two structures for one
/// program, or the speller wrote a different program. The property cannot
/// tell them apart and does not pretend to: it hands back the respelling
/// so the reduction can.
// [spec:nsh:req:idiom.canonical-tree+1]
pub fn builds_one_tree_per_program(shell: &mut Shell, source: &BStr) -> Canonicity {
    let read = crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string(shell, source);
        let mut units = Vec::new();
        loop {
            let outcome = crate::parser::parse_command(shell, false);
            if shell.input.tokens.expanded_alias() {
                return Err(Canonicity::Aliased);
            }
            match outcome {
                Err(_) => return Err(Canonicity::NotParsed),
                Ok(crate::parser::ParseResult::Eof) => break,
                Ok(crate::parser::ParseResult::Tree(None)) => {}
                Ok(crate::parser::ParseResult::Tree(Some(node))) => {
                    let text = crate::nodes::source::respelled(&shell.locale, &node);
                    units.push((node, text));
                }
            }
        }
        Ok(units)
    });
    let units = match read {
        Ok(units) => units,
        Err(verdict) => return verdict,
    };
    for (node, respelled) in units {
        let again = crate::resource::with_resources(shell, |shell, _resources| {
            crate::input::set_input_string(shell, BStr::new(respelled.as_slice()));
            let mut trees = Vec::new();
            loop {
                let outcome = crate::parser::parse_command(shell, false);
                if shell.input.tokens.expanded_alias() {
                    return Err(Canonicity::Aliased);
                }
                match outcome {
                    Err(_) => return Ok(None),
                    Ok(crate::parser::ParseResult::Eof) => break,
                    Ok(crate::parser::ParseResult::Tree(None)) => {}
                    Ok(crate::parser::ParseResult::Tree(Some(tree))) => trees.push(tree),
                }
            }
            Ok(Some(trees))
        });
        match again {
            Err(verdict) => return verdict,
            Ok(None) => return Canonicity::Unreadable { respelled },
            /* Compared as a flat run of commands, because how a program
             * is divided into parse units is spelling too: `a; b` is one
             * unit holding a sequence, and the same program written on
             * two lines is two units holding one command each. Requiring
             * the same division would fail them for being spelled
             * differently, which is the opposite of the question. */
            Ok(Some(trees)) => {
                let mut want = Vec::new();
                flattened(&node, &mut want);
                let mut got = Vec::new();
                for tree in &trees {
                    flattened(tree, &mut got);
                }
                if want != got {
                    return Canonicity::TwoTrees { respelled };
                }
            }
        }
    }
    Canonicity::OneTree
}

/// Where the pinned Bash the differential targets must be judged against
/// is recorded, and how to tell whether a given binary is that one.
///
/// [`dec:nsh:differential-is-the-oracle`] only means something if the
/// oracle is the Bash the repository pins. `calibrate-bash-5-3-oracle`
/// pinned 5.3 and recorded its identity beside the survey corpus, and the
/// fuzz targets were asking `/usr/bin/bash` instead -- 5.2.37 on the
/// machine this was found on. Nothing had gone wrong yet, which is luck
/// about which cases the fuzzer reached rather than a property of the
/// arrangement.
// [spec:nsh:req:compat.bash.reference-profile]
pub mod reference {
    use std::path::Path;

    /// The calibration record the survey already keeps, read rather than
    /// copied: one pin, not a second answer to the same question.
    const CASES: &str = concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/../../tests/surveys/oils/BASH_REFERENCE_CASES.json"
    );

    /// The version string `calibrate-bash-5-3-oracle` pinned.
    ///
    /// Read out of the calibration record with a string search rather
    /// than a JSON parser, because this is the one field wanted and the
    /// fuzz workspace has no serde. A malformed record is an error, not a
    /// default: a missing pin must not silently become "any Bash".
    // [spec:nsh:req:compat.bash.reference-profile]
    pub fn pinned_version() -> Result<String, String> {
        let text = std::fs::read_to_string(CASES)
            .map_err(|error| format!("cannot read the Bash calibration record {CASES}: {error}"))?;
        let key = "\"oracle_version\"";
        let at = text
            .find(key)
            .ok_or_else(|| format!("{CASES} records no oracle_version"))?;
        let rest = &text[at + key.len()..];
        let open = rest
            .find('"')
            .ok_or_else(|| format!("{CASES} has a malformed oracle_version"))?;
        let tail = &rest[open + 1..];
        let close = tail
            .find('"')
            .ok_or_else(|| format!("{CASES} has an unterminated oracle_version"))?;
        Ok(tail[..close].to_owned())
    }

    /// Whether `shell` is the pinned Bash, by asking it.
    ///
    /// The survey pins the oracle by digest as well, which is the
    /// stronger check and the one it uses. This asks the version instead,
    /// because it is what separates the two builds that were actually
    /// confused -- 5.2.37 from 5.3.15 -- and because computing a digest
    /// here would mean a hashing dependency in a workspace whose whole
    /// point is that nothing in it ships.
    ///
    /// Every failure is an error and none is a default. A reference that
    /// cannot be run is the one case that must never quietly become
    /// "no comparison to make".
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure]
    // [spec:nsh:req:compat.bash.reference-profile]
    pub fn verify(shell: &Path) -> Result<(), String> {
        let pinned = pinned_version()?;
        let output = std::process::Command::new(shell)
            .arg("--version")
            .output()
            .map_err(|error| {
                format!(
                    "cannot run the pinned Bash at {}: {error}. \
                     The fuzz containment mounts an empty tmpfs over /tmp, so an oracle \
                     kept there is invisible to the targets; keep it outside /tmp and \
                     name it with NSH_FUZZ_BASH.",
                    shell.display()
                )
            })?;
        let reported = String::from_utf8_lossy(&output.stdout);
        let first = reported.lines().next().unwrap_or_default();
        if first.contains(&pinned) {
            return Ok(());
        }
        Err(format!(
            "{} reports {first:?}, which is not the pinned Bash {pinned:?}. \
             Build it with `nsh-survey build-bash-reference` and name it with NSH_FUZZ_BASH.",
            shell.display()
        ))
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::Streams;

    fn shell() -> Shell {
        let streams = Streams::capture().expect("captured streams");
        Shell::builder()
            .streams(streams)
            .option(BStr::new(b"bash"), true)
            .build()
            .expect("shell")
    }

    /// Assert each source prints as the text beside it.
    /// Assert each source comes back byte for byte.
    ///
    /// There used to be a second column saying what the printer chose to
    /// write. There is no choice now, so the answer is the source and a
    /// column repeating it would be a second opinion about the one thing
    /// this property exists to remove.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    fn assert_prints_itself(shell: &mut Shell, sources: &[&[u8]]) {
        for source in sources {
            assert_eq!(
                round_trips_byte_exactly(shell, BStr::new(source)),
                RoundTrip::Exact,
                "{:?}",
                BStr::new(source),
            );
        }
    }

    /// Assert one source comes back byte for byte, or was rejected.
    ///
    /// Rejecting the input is an ordinary answer to fuzzer bytes, and the
    /// fuzz target returns on it too: there is nothing to compare until
    /// something was read.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    fn assert_roundtrip_fixed(source: &[u8]) {
        let mut shell = shell();
        let verdict = round_trips_byte_exactly(&mut shell, BStr::new(source));
        assert!(
            matches!(verdict, RoundTrip::Exact | RoundTrip::NotParsed),
            "{:?} did not come back byte for byte: {verdict:?}",
            BStr::new(source),
        );
    }

    /// Shapes that used to print as a fixed point and now have to print as
    /// themselves.
    ///
    /// A fixed point was the weaker question -- printing twice reaching the
    /// same text says nothing about whether that text was what was written,
    /// and `false ; x=hi` reached a fixed point as `false\nx=hi`, which is a
    /// different spelling of the same program and no longer allowed.
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn former_fixed_points_now_come_back() {
        let mut shell = shell();
        assert_prints_itself(
            &mut shell,
            &[
                b"v=abc\nif [[ $v == a* ]]; then printf '%s\\n' \"$v\"; fi\n".as_slice(),
                b"false ; x=hi\n",
                b"echo hi\n{ sleep 1 ; echo derp ; } &\necho bye\nwait",
                b"cat <<EOF\necho \\\\\\$var\nEOF\ncat <<'EOF'\necho \\\\\\$var\nEOF\n",
                b"\\'",
                b"${(M)foo}",
                b"ty} | {  t  \n3#\n}\n# ",
                b"a[${x:-]}]=y",
                b"\"${a+\\'}\"",
            ],
        );
    }

    /// A `]` inside `${...}` is the expansion's own byte, so the subscript
    /// around it stays open. Counting it as the closing bracket left the
    /// printer holding a word whose brackets it could not put back, and
    /// the unbalanced form is still a parse error rather than a tree.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn an_unclosed_subscript_is_refused() {
        let mut shell = shell();
        assert_eq!(
            round_trips_byte_exactly(&mut shell, BStr::new(b"a[[${ ]]}")),
            RoundTrip::NotParsed,
        );
    }

    /// Two further reductions of the artifact behind
    /// `crash_67407b0220a752d0f6932c9c9a3349b7e7ff9413`. Each outlived the
    /// fix the one before it forced, because each was the printer deciding
    /// which quotes an operand needed. Nothing decides that now: the run
    /// the word was read as is what goes back, and a quote that protects
    /// nothing is still a quote the source wrote.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn an_operand_quote_that_protects_nothing_survives() {
        assert_roundtrip_fixed(b"\"${a+\"\"${a#\x00${ ''$''}''a}}\"$(\"\")\"\"''");
        assert_roundtrip_fixed(b"\"${a+\"\"${a#\x00${ ''$''}'a'}}\"$(\"\")\"\"''");
    }

    /// The tree a printed program parses back to is the one it came from,
    /// down to every part the grammar carries. Positions are the exception,
    /// and deliberately so: printing relocates all of them.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_a_program_recovers_the_program() {
        let mut shell = shell();
        for source in [
            b"false ; x=hi".as_slice(),
            b"cat <<EOF\nbody\nEOF\n",
            b"a=(1 2 3)\necho \"${a[@]}\"",
            b"for ((i = 0; i < 2; i++)); do echo $i; done",
            b"case x in a) echo a;; *) echo b;; esac",
            b"echo one | grep -q one && echo yes || echo no",
            b"while false; do echo x; done",
            b"echo hi 2>&1 >/dev/null",
            b"{ echo one; echo two; } > /dev/null",
            b"echo $(( 1 + 2 ))",
        ] {
            let verdict = round_trips_byte_exactly(&mut shell, BStr::new(source));
            assert!(
                verdict == RoundTrip::Exact,
                "{:?} did not come back byte for byte: {verdict:?}",
                BStr::new(source),
            );
        }
    }

    /// Rejecting the input is an ordinary answer to arbitrary bytes: there is
    /// no program to print, so there is nothing the property can say.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_says_nothing_about_a_rejected_program() {
        let mut shell = shell();
        assert_eq!(
            round_trips_byte_exactly(&mut shell, BStr::new(b"if")),
            RoundTrip::NotParsed,
        );
    }

    /// A `"` inside a `${...}` operand toggles the quoting the word arrived
    /// in. Dropping the toggle used to reopen the parameter grammar to the
    /// `}` it was protecting, which printed a stable program that ran
    /// differently -- the corruption the fixed-point property could not see.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_keeps_a_braced_quoted_operand() {
        let mut shell = shell();
        let source = b"echo \"${a+\"a}b\"}\"";
        assert_eq!(
            round_trips_byte_exactly(&mut shell, BStr::new(source)),
            RoundTrip::Exact,
        );
    }

    /// The three ways to introduce a definition are three trees, and each
    /// goes back as the bytes it was read from -- including the layout,
    /// which no longer gets opened out. `declare -f` keeps Bash's frame
    /// around a body; this is the renderer under it.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_keeps_a_definition_style() {
        let mut shell = shell();
        assert_prints_itself(
            &mut shell,
            &[
                b"f() { echo one; }".as_slice(),
                b"function f { echo one; }",
                b"function f () { echo one; }",
            ],
        );
    }

    /// `'a'` and `"a"` protect the same byte, so which one was written is
    /// not recoverable from the run and the parser records it. A backslash
    /// inside the run is the same question one level down: it spells itself
    /// only when the part after it is protected too.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_keeps_a_run_in_its_own_quote() {
        let mut shell = shell();
        assert_prints_itself(
            &mut shell,
            &[
                b"printf \"%s\\n\" hi".as_slice(),
                b"printf '%s\\n' hi",
                b"echo \"a  b\"",
                b"echo 'a  b'",
                b"echo \"a\\\\b\"",
                b"echo $\"hello\"",
            ],
        );
    }

    /// A here-document delimiter is the body's, not the printer's: a body
    /// holding a line that spells some other delimiter can only be closed
    /// again with the word the source wrote. A body that ended at end of
    /// input carries the line the reader saw, which is what Bash feeds and
    /// what lets a printed document close at all.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_keeps_a_here_document_delimiter() {
        let mut shell = shell();
        assert_prints_itself(
            &mut shell,
            &[
                b"cat <<MOF\nhello\nEOF\nMOF\n".as_slice(),
                b"cat <<'Q'\nbody\nQ\n",
                b"<<a\nx",
            ],
        );
    }

    /// A `$` that starts nothing is an ordinary byte. Protecting it anyway
    /// spelled the same byte with a part the source never wrote, which is
    /// the whole shape of over-protection: `\$`, `\\`, `\'` all read back as
    /// one part more than they went in as.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_leaves_an_inert_dollar_alone() {
        let mut shell = shell();
        assert_prints_itself(
            &mut shell,
            &[
                b"echo $".as_slice(),
                b"echo a$",
                b"echo \"$\"",
                b"echo $ x",
                b"echo $x",
                b"echo \"$x\"",
                b"echo $((1))",
                b"echo $'a'",
                b"echo a!b",
                b"! true",
            ],
        );
    }

    /// The braces are not decoration: they decide what a redirection or a
    /// `&` after them attaches to, so a list that lost them is a different
    /// program. `{ a;}<f` used to print as `a < f`, which redirects the
    /// command rather than the group.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_keeps_a_brace_group() {
        let mut shell = shell();
        for source in [
            b"{ a;}<f".as_slice(),
            b"{ a; b; } && c",
            b"a && { b || c; }",
            b"{ a; } &",
            b"time { a; b; }",
            b"f() { a; }",
            b"f() ( a )",
        ] {
            let verdict = round_trips_byte_exactly(&mut shell, BStr::new(source));
            assert!(
                verdict == RoundTrip::Exact,
                "{:?} did not come back byte for byte: {verdict:?}",
                BStr::new(source),
            );
        }
    }

    /// A byte that is only special where it begins something keeps its own
    /// spelling everywhere else. `#` opens a comment only where a word
    /// begins, and a `$` against a backslash starts nothing at all.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn printing_leaves_a_positional_byte_alone() {
        let mut shell = shell();
        assert_prints_itself(
            &mut shell,
            &[
                b"echo a#b".as_slice(),
                b"echo \"#\"",
                b"a#",
                b"echo $\\a",
                b"${a }",
                b"${a b}",
                b"${(M)x}",
                b"${!#a}",
                b"${ \\a}",
                b"${x-\\a}",
                b"$'\"'",
                b"$'a\\tb'",
                b"\"${a%\\a}\"",
                b"\"${a \\}}\"",
                b"((a))",
                b"(( a ))",
                b"echo \"=\"",
                b"echo \"/\"",
                b"a\"=\"b",
                b"a[ ]=v",
                b"a[1 + 1]=v",
                b"a=([1 + 1]=v)",
                b"\"${ -}\"",
                b"\"${ =}\"",
                b"\"${a \\}}\"",
                b"\"${a-=}\"",
                b"\"${a-\\=}\"",
                b"''$\\\\",
                b"$[())]",
                b"${x-${(M)y}}",
                b"${#a }",
                b"${#a ''}",
                b"\"${a%'\"'}\"",
                b"${a#'#'}",
                b"${a#\"?\"}",
                b"\"${a+\"\"x}\"",
            ],
        );
    }

    /// A backslash with nothing after it is a line continuation joining to
    /// nothing. The word it belongs to is `a`, which is what Bash runs, and
    /// the byte is still one the source wrote -- so it goes back, and what
    /// comes back reads as the same word.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn a_continuation_that_joins_nothing_still_goes_back() {
        let mut shell = shell();
        assert_eq!(
            round_trips_byte_exactly(&mut shell, BStr::new(b"echo a\\")),
            RoundTrip::Exact,
        );
    }

    /// Two spellings the shell reads as one thing are one tree, and the
    /// tree no longer has to choose between them: each goes back as it was
    /// written. Bash discards a backslash inside `$(( ))` before
    /// evaluating, and `$[ ]` is its own older spelling of the same
    /// expansion -- which used to be rewritten as `$(( ))` and, when its
    /// parentheses did not balance, as `$((0))`.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn each_arithmetic_spelling_goes_back_as_written() {
        let mut shell = shell();
        assert_prints_itself(&mut shell, &[b"$((\\$))".as_slice(), b"$[1]", b"$[())]"]);
    }

    /// Every shape the round-trip fuzzer found before
    /// [`spec:nsh:req:idiom.printable-ast`] existed, paired with the artifact
    /// it was reduced from.
    ///
    /// A corpus, not a suite. One defect reached the artifact directory many
    /// times over -- 284 artifacts closed on four fixes -- so what each shape
    /// is *for* lives in the named cases above and these only keep it from
    /// coming back. When the printer prints what it parsed, this list is
    /// checked with [`printing_is_reversible`] instead, and the fixed point
    /// stops needing its own assertion because reversibility implies it.
    pub(crate) const ROUNDTRIP_CORPUS: &[(&str, &[u8])] = &[
        ("00128129edeba44b19fa00f4544cd649b211579e", b"\"${1='f}\""),
        ("010ba1fccf3af318153989abf14f81cc45ac74ae", b"<<t\n${a-'}"),
        (
            "0199407e1ac576e2808c329f58c291bd883e60cd",
            b"<<a\n${s:'}\"'}",
        ),
        (
            "022176d51773e6b18a95d34f5702af0dde5e8a87",
            b"a[['']${x%]''\"${}\"}",
        ),
        (
            "0263990bb28210be2880ff7ef3e9ceb01d2b69dc",
            b"\"${P?\"\"'R}\"",
        ),
        (
            "046fb0ed417218d50e99d8db0290afff26265eda",
            b"'H c 'f()(r);''",
        ),
        (
            "076ec3e5f633671517bd244c5549cc2ba59607df",
            b"\"${s/\"'\"\\'''^}\"",
        ),
        ("09f3466eed8d98ec92f0753ccf20c8e8f7dda499", b"$((\\ ))"),
        (
            "0d83b650cfc97d78a925e4bd63cb5da075b22a90",
            b"<<F\n${@:'\"\"'}",
        ),
        ("0df179ab1ed2f0ab21b47a2353d857fcfe23481e", b"n \x00$''"),
        (
            "0fb14d16963a8365b445bd90c5453b9e913c73ab",
            b"case H in h)'';esac\nfor x in;do\nx\ndone",
        ),
        (
            "1c531161da553603f14ee001a322379fda95c58f",
            b"{<<EOF\n${##'\"\"'}\nEOF\n}",
        ),
        ("230fec3d836c7cee43654d5fe016f04a6ea6d59b", b"<<F\n${y+:}"),
        ("268c41699358c22443c0083d32b2fe61962c4b14", b"<<F\n${s='}"),
        (
            "28f1e47dcfcfc7c8b0d05d6ced78f5ccf1e6d8ee",
            b"\"${v%\"${N-\"${?}\"}\"\\'\\'}\"''",
        ),
        (
            "30083e4af84bcbb074bbbe1d2f6cf833c2df36a0",
            b"<<3\n${e=${s}'}",
        ),
        ("35d94ec3a6bb06dbf6352eeefbfee480ed189b51", b"<<t\n${f-'}"),
        (
            "38773c9045ea867c61e5229399968b73bb45a07f",
            b"<<a\n${2:'${'}",
        ),
        (
            "389baef0afbc70433eca399bb7b0875c2117e476",
            b"''\"${y[]+'{}\"",
        ),
        ("3e76e9572e46fee5cb1ba6112d71d4077a3b3980", b"\"\"''$\\\\''"),
        (
            "407017a4faa592e5d62d38354861bfe057051f35",
            b"<<F\n${@/${}\"''''''${}'\"}",
        ),
        (
            "433ea746fac641968bf915cabcecef41b7a361e9",
            b"case H in h)esac\n''''||{ e;e;};''|(2);''",
        ),
        ("4591d46cb978be8bc4ee6bd3a21de0fc5e6055f8", b"\"${U-b'}\""),
        ("45fcf338a9484debb1d8df3e904b1a223af6725c", b"\"${o%\\'}\""),
        (
            "460552cdfeb04e5d6c95a8216894cad010186245",
            b"\"${v%\"'}\"}\"",
        ),
        (
            "49bbfbe474b9e1a0f785e2eb9f7e30c952ffc3b8",
            b"\"${P@\"${P}'\"``''}\"",
        ),
        (
            "4bbac9b1e278840acd24d4c7277c20563e4aff27",
            b"$[${x,'\"\"('}) ))",
        ),
        ("4d70555073f170833ead436f9c03e8a74782431d", b"<<T\n${f-=}"),
        ("4fbe3575b4e681210b9f7687b959f794100f96f3", b"<<t\n${a='}"),
        (
            "53c48d685998004ce4643483c41eee1cf5471300",
            b"\"${P#\"'\"\n}\"",
        ),
        (
            "54aea85035d68d3fa9796a9fbbadd9299a22ea92",
            b"<<F\n${x%''\\'\"\"}",
        ),
        ("59743efca9e3728cc761a78713f8a296bdfabddf", b">c for"),
        (
            "5c2ea037ba269c719717a7ea4eeb5f39573f619e",
            b"<<t\n${T:\"'\"}",
        ),
        (
            "5ea810f1cad92b5bbf26a449f8d15f604576c81f",
            b"$[(${\x92[]})))",
        ),
        (
            "60839836dd5e742c2870c555862cad78d1e57cb1",
            b"a[\"\"['']\x00]$''",
        ),
        ("60af6aa04508ca4d7ea455c2316b1923b6b6a186", b"<<F\n${f--}"),
        (
            "636c61c78d5426b2ef2692af23e4e6cdca8ade83",
            b"(\"$($\\S'')\")",
        ),
        (
            "6404a17682ebd58ea5b1e509c2b41a8a974750c9",
            b"<<\"\">$(e\nc)\n|",
        ),
        ("666b03c8e864fae4d58055fb70f1a846973c49a3", b"''$\\T"),
        (
            "66b2003168ed9fd6de867d6c021d269d020726ad",
            b"case $\\H in h)esac",
        ),
        (
            "67407b0220a752d0f6932c9c9a3349b7e7ff9413",
            b"\"${a[]+${a}${a}\"\"${a#${a-''}''}\\'a}\"$(\"\")\"\"''",
        ),
        (
            "6be107a5c41375bca592447f3d43985ed50921ba",
            b"a[\"\"\x00]$''\"\"\"\"''']'",
        ),
        (
            "6bff0427b3a3a4d70cae2ff85205f39b2123dbae",
            b"<<E\n${##'}\"'}",
        ),
        (
            "6ca5e2ea6e11a9c4ac418ac005c1e68a462e725c",
            b"${ \"\x00\"\"\"''${ \"\"$''\"\"\"\"}$(())\n(())}",
        ),
        ("6fa20513d77029235049674775d8501511b328e6", b"<<y\n${y[]+]}"),
        (
            "7003b418277c2a395370db0f32b54e737320e22b",
            b"\"${T:\"\"${r=\"\"\"\"${?}\"}\"}\"'}\"}\"''",
        ),
        ("715df6a374d44a4729a6f1a8e69684d14c12246e", b"$\\=\"\""),
        (
            "72938460d0e1e942742a8ca7195d0830a82db522",
            b"{ time {\n1\n2\n}\n}>''\nfor a do if 0;then ''\nfi done",
        ),
        (
            "763f85e723e8768c8d5bb3825dcfc9780d1d7060",
            b"\"${x#\"}'\"}\"",
        ),
        ("7efb6f73043735388aa5913e32666606c846f3fa", b"''$\\["),
        (
            "84fede883c05e23ca2e8be5eba194d2d72aa136b",
            b"$()\nif e;then\n''\nfi\n$(())''$\\\\''",
        ),
        (
            "86dee836d3eaeea7d35ae9d5d70f70277ec09e4f",
            b"while o;do\ncase t in c)\x00$''\ny\nesac\ndone",
        ),
        ("8798ff87cc4afe4416b8e7d6d86b1802a12e5221", b"\"${1[]+' }\""),
        ("88b6be99b2793a581259a5ab3412694c6a2c36b1", b"<<h\n${##\\)}"),
        ("9497445df693aa59f29db317cab86840e07679b1", b"(\"${v-'s}\")"),
        ("96ccc1e5e68439d55c970b6aac4815a9e5e508f7", b"<<3\n${d??}"),
        (
            "9c7fd8c7fc037d762417a10813a0c2738fb7e5c2",
            b"\"\"$({ o;}<<0)``",
        ),
        (
            "9dfa902037835ee32394ac2ea5ea5095c6b2e531",
            b"case $ in h)esac\n(e)<<}\n${x/\\z}",
        ),
        (
            "9eba2805b234852acc3700b237692048b05fb0e7",
            b"\"${v%\"}'\"}\"",
        ),
        (
            "9ed78a8911152ab494391a1b36e90f65ba9da563",
            b"<<c ''\n${r%\\'\\'}",
        ),
        ("9ee32c9051b541b0dfe80b27aac3e9646bdc9861", b"<<E\n${x/\\)}"),
        ("a00530386591ff77fdcc84f09828e0a91f5d7d84", b"<<F\n${##\\2}"),
        (
            "a2e3e629dab9cb0c0dd10fe0eab52d8edf413e1e",
            b"a[''${]\"\"]}\"\"\nfor e in \"\";do y\ndone|t",
        ),
        ("a578a48225fe9c8ec0e299b246cf7ceca53de0e1", b"\"${t-c'}\""),
        (
            "a76d5fd4a82b8ff7a359f4acd3ad63b99b21b5f9",
            b"a[[''\"\"\"${a%]]${}\"\"}\"",
        ),
        ("ab332be713f0b44f5f81f7d7e275bebb9e4b21fe", b"$[(]]''"),
        (
            "adab4b91a4f8b40f51065cf28074d7a6e21aa833",
            b"<<F\n${f:'}\"'''}",
        ),
        (
            "af6694ea2365823df05dc709867f7666c313884d",
            b"{(())}\ncase H in h)esac\n=()b\n{ T\nd;}||h",
        ),
        (
            "b02b4cad482ef3b9e29684c40ce48a1971bc6779",
            b"<<F\n${s/'\"\"'}",
        ),
        (
            "b2e8a3257357e988a6957dc9faa06b28fd824ec0",
            b"<<O\n${f%''${o%''}\"'\"}",
        ),
        (
            "b47b0fc6be6f92e958aeafb9cdbe2e8864aa81be",
            b"\"${v%\"\"${v}'}'\"'\"$}\"",
        ),
        (
            "b5d204562d6323dbecd3e09d2ffd8e53df9f87d6",
            b"<<E\n${f:'\"\"'}",
        ),
        (
            "bb3645b09a9c65cbab9bcc84bba25c312849c71c",
            b"{\tuntil \"\";do\tcase \"\" in\nesac\tdone\n}\n\"${3-'\n}\"''",
        ),
        ("c0e6052a29ee92df24df6dd9a5d5d2dc3382adb7", b"$((\\2))"),
        ("c10eb8b82f9fc4cf401e5cc60a3b681da2465498", b"<<E\n${v+-}"),
        (
            "c31a8730ed084c37e815621fcedb8dc588b3f9c9",
            b"<<C\n${o%'}\"'}",
        ),
        ("c3f689e80843ebbe90db3ad733b71e30cf23072f", b"<<t\n${!==}"),
        ("c53b853f37ab92de1865d9a6144fde5d935a1258", b"<<O\n${o%\\'}"),
        (
            "c64d2a2fe72278ee72b0d7221536466d3345aef2",
            b"\"\"\"${f%\\'}\"",
        ),
        ("c730ffc80de4af7751d196117c8d678501aa9d57", b"\"${a+l'}\""),
        ("c908740cd3dcd875dfc96d4424a3d378cf722499", b"\x00$''"),
        (
            "c90dabc0d00d235400edbd2be74bbe052b6d2985",
            b"('')\n$('')\"$((\\2))\"''\"\"''\"\"''",
        ),
        ("cb3746bdb195b60ae67fcfdca5ff71849484d5f5", b"\"${f-d'}\""),
        (
            "cb89ccb96b81ac450e3445c9337237e166ff6a80",
            b"<<\xff\n${x/\\)}",
        ),
        ("ce120df4ef8104717cd4cb943daa43136ac55eb0", b"<<}\n${x/\\)}"),
        ("d04d19f62a0b643eef5ebad2a0356bbebbe920b6", b"$((\\)))"),
        ("d0b6e4d27e1084101f8478f11c96d39d808f3edf", b"''$\\"),
        (
            "d3fc0cb9f343a681a1544b7bfda0e31a993643e3",
            b"\"${f%d\\'}\"\"\"",
        ),
        (
            "d4fb2d0731498cc3d91379c1ccff8e572ac19a45",
            b"\"$(''\na[\"\"\"\"\"$a\"\"\"\"\"\"\"\"\"'\x00'''\"\"]$''\"\"'')\"",
        ),
        ("d53412fd7202bacba87e817a513357c1f0b8eda5", b"$((\\;))"),
        (
            "d83f138c813b197cc657b7982d9f3bdfc119c2b2",
            b"\"${n:\"''\"\"\"}\"",
        ),
        (
            "d98551bf5359b68b77d39d3e66398e66fc385898",
            b"a[[${ \"\"]\"\"]\"\"}",
        ),
        (
            "dae8dab038f2f8b9610f0b66808c16b6fab1ed5e",
            b"''\n(w)\n\"\"''''$\\\\''\"\"\n(\"\"\"\")",
        ),
        (
            "df6157ff235d8722e2316f3a77194a9ba407f48b",
            b"e[${ [}]\n '']''",
        ),
        ("e08aac52b8286cb8fdb5867f72f508109b4cf50a", b"<<s\n${v/\\f}"),
        ("e2c01168d1f0feb808b5ec947a1272452bb561a1", b"$((\\u))"),
        (
            "eb40121e5f0d4f1aa90f5df3d91b45f431400b02",
            b"<<F\n${e-\"\\\"\\\"\"}",
        ),
        ("ee701d7526fac0600167a34431c4c410e1a42357", b"''$((\\0))"),
        ("f1b84b0bfc893f91fc8c2e876ac4ea09a5eebd5d", b"\"\"$\\S"),
        (
            "f76f6d7c93662759301e619bb6db3ae489f10366",
            b"$(\"\")$(())$((\\d))",
        ),
        (
            "f8782f7e65944b4a44f148392b211c46378cde83",
            b"'O'(){ f\n}>i<<\xca\n${3-\\c}",
        ),
    ];

    /// The mark check has to be able to fail, or it says nothing.
    ///
    /// This is the half a byte comparison cannot see. Concatenating a
    /// tree's bytes reads the outermost run only, so a child pointing at
    /// someone else's tokens still prints the right program -- and breaks
    /// `declare -f`, which renders a subtree. Three defects reached a
    /// shipped renderer that way while the log was provably complete, so
    /// the check is only worth having if a wrong mark trips it.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn a_run_from_the_wrong_place_is_caught() {
        let mut shell = shell();
        let parse = |shell: &mut Shell, source: &[u8]| {
            crate::resource::with_resources(shell, |shell, _resources| {
                crate::input::set_input_string(shell, BStr::new(source));
                match crate::parser::parse_command(shell, false) {
                    Ok(crate::parser::ParseResult::Tree(Some(node))) => node,
                    _ => panic!("{:?} did not parse", BStr::new(source)),
                }
            })
        };
        let sound = parse(&mut shell, b"a; b");
        assert!(crate::nodes::emit::misplaced_run(&sound).is_none());

        let elsewhere = parse(&mut shell, b"zzz").tokens().clone();
        let crate::nodes::Node::Sequence(mut list) = sound else {
            panic!("a sequence")
        };
        list.left = Box::new((*list.left).with_tokens(elsewhere));
        let damaged = crate::nodes::Node::Sequence(list);
        assert!(
            crate::nodes::emit::misplaced_run(&damaged).is_some(),
            "a child holding a run from another program went unnoticed",
        );
    }

    /// One construct of each shape, held to the bytes it went in as.
    ///
    /// One of every construct a shell has, shared by the properties that
    /// need breadth rather than depth.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    pub(crate) const CONSTRUCTS: &[&[u8]] = &[
        b"echo a b",
        b"a=1 b=2 cmd x",
        b"a && b || ! c",
        b"a | b | c",
        b"a; b& c",
        b"(a; b)",
        b"{ a; b; }",
        b"if a; then b; elif c; then d; else e; fi",
        b"while a; do b; done",
        b"until a; do b; done",
        b"for x in 1 2 3; do echo $x; done",
        b"select x in a b; do echo $x; done",
        b"case $x in a|b) c ;; (d) e ;& *) f ;; esac",
        b"f() { a; }",
        b"function g { a; }",
        b"function h () { a; }",
        b"time -p a | b",
        b"cat <f >g 2>&1 <&0 >>h",
        b"cat <<A <<'B'\n1\nA\n2\nB\n",
        b"cat <<<'here'",
        b"echo $x ${y} ${z:-d} ${#a} ${b#c} ${d/e/f}",
        b"echo $(true) `false` $((1 + 2)) $[3]",
        b"echo 'a' \"b\" \\c $'d' $\"e\"",
        b"[[ a == b* && c != d ]]",
        b"((x = 1 + 2))",
        b"for ((i = 0; i < 2; i++)); do echo $i; done",
        b"declare -A m=([k]=v [l]=w)",
        b"a[1]=x b+=y",
        b"echo <(true) >(false)",
        b"echo a  #  trailing",
        b"echo a \\\n  b",
    ];

    /// Breadth rather than depth: the corpus below is what the fuzzer
    /// found, and this is what a shell is made of, so a construct that
    /// stops coming back says which construct rather than which artifact.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn one_of_every_construct_comes_back() {
        let mut shell = shell();
        assert_prints_itself(&mut shell, CONSTRUCTS);
    }

    /// The two root causes the first byte-exact campaign found, reduced.
    ///
    /// `<<E\nEO` is the recorder, not the renderer: a here-document line
    /// that starts with the delimiter and then does not match it is
    /// pushed back to be read again, and a pushed string stacks inside
    /// the frame it was pushed on rather than becoming one of its own, so
    /// the reader recorded those bytes twice and the log said `EOO`. The
    /// token-stream property would have caught it and never saw the
    /// shape; comparing a printed program against its source did, because
    /// it is the same question asked of every input the fuzzer invents.
    ///
    /// `# x` with no newline after it is the other end: end of input is
    /// not a token, so nothing moves the reader past a trailing comment
    /// and those bytes belong to no unit at all.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    #[test]
    fn the_shapes_the_first_campaign_reduced_to() {
        let mut shell = shell();
        assert_prints_itself(
            &mut shell,
            &[
                b"<<E\nEO".as_slice(),
                b"cat <<E\nEO",
                b"<<E\nEOF\nE\n",
                b"# x",
                b"a\n# x",
                b"a  ",
            ],
        );
    }

    /// Every shape in the corpus comes back as the bytes it went in as.
    ///
    /// This used to ask for a fixed point, which is the weaker question:
    /// printing twice reaching the same text says nothing about whether
    /// that text is what was written, and every shape here is an artifact
    /// of the printer writing something else.
    ///
    /// Every shape at once rather than one test each: a failure should say
    /// which shapes came back, not which single shape came back first.
    // [spec:nsh:req:idiom.printable-ast+2/test]
    /// Two spellings of one program build one tree.
    ///
    /// [`spec:nsh:req:idiom.canonical-tree+1`] over one of every
    /// construct a shell has, with the second spelling *derived* rather
    /// than listed: the source is one, and rendering the parsed tree from
    /// its structure is the other. A hand-written list of pairs is a
    /// corpus, and two nodes running have now shown the corpus to be the
    /// limit rather than the property.
    ///
    /// The breadth is the same corpus the byte-exact property uses, and
    /// for the same reason: a construct that stops being canonical says
    /// which construct rather than which artifact.
    // [spec:nsh:req:idiom.canonical-tree+1/test]
    #[test]
    fn two_spellings_of_one_program_build_one_tree() {
        let mut shell = shell();
        for source in CONSTRUCTS {
            match builds_one_tree_per_program(&mut shell, BStr::new(source)) {
                Canonicity::OneTree | Canonicity::NotParsed | Canonicity::Aliased => {}
                Canonicity::TwoTrees { respelled } => panic!(
                    "{:?} and {:?} are one program and parsed to two trees",
                    BStr::new(source),
                    respelled
                ),
                Canonicity::Unreadable { respelled } => panic!(
                    "{:?} was respelled as {:?}, which does not parse",
                    BStr::new(source),
                    respelled
                ),
            }
        }
    }

    /// The fallback speller cannot yet spell every tree, and this counts it.
    ///
    /// A number rather than silence. Holding the fallback to arbitrary
    /// parsed trees goes well past what it was written for -- its
    /// contract is nodes the shell built, which are simple -- and over
    /// the corpus the byte-exact campaigns reduced to, it writes a
    /// different program for 52 of 101. Every one of them is the
    /// speller's, not the parser's: no input yet found makes the parser
    /// build two trees for one program.
    ///
    /// It was 53 until `$[…]` stopped counting parentheses. One artifact,
    /// `$[(${\x92[]})))`, parsed only because the `))` inside it closed
    /// the expansion; Bash ends `$[` at a bracket and refuses that
    /// program, so the parser now refuses it too and the speller is no
    /// longer asked to spell a tree that should not have been built.
    ///
    /// THE CLASSES, so a fix can be aimed rather than searched for. An
    /// operand inside `${...}` is spelled with quotes that a
    /// here-document body reads as bytes. A literal `$` before an inert
    /// run spells as `$'...'`, which reads back as one ANSI-C quote
    /// rather than two parts. A NUL byte is dropped. An empty arithmetic
    /// expansion spells its empty word as `''`. `$[...]` is respelled
    /// `$((...))`, and an array subscript can be truncated.
    ///
    /// THIS IS ALSO THE PROPERTY'S NON-VACUITY. The test above would pass
    /// just as happily against a comparison that returned `OneTree` for
    /// everything; these 52 are the evidence that it does not.
    // [spec:nsh:req:idiom.canonical-tree+1/test]
    #[test]
    fn the_fallback_speller_is_not_yet_total() {
        let mut shell = shell();
        let failures = ROUNDTRIP_CORPUS
            .iter()
            .filter(|(_, source)| {
                !matches!(
                    builds_one_tree_per_program(&mut shell, BStr::new(source)),
                    Canonicity::OneTree | Canonicity::NotParsed | Canonicity::Aliased
                )
            })
            .count();
        assert_eq!(
            failures,
            52,
            "the fallback speller writes a different program for {failures} of {} shapes, not 52 -- \
             if that is fewer, lower the number and say which class went; if more, something regressed",
            ROUNDTRIP_CORPUS.len()
        );
    }

    #[test]
    fn the_round_trip_corpus_comes_back_exactly() {
        let mut shell = shell();
        let mut changed = Vec::new();
        for (artifact, source) in ROUNDTRIP_CORPUS {
            match round_trips_byte_exactly(&mut shell, BStr::new(source)) {
                /* Rejecting the input is an ordinary answer, and so is an
                 * alias, which replaces text before the parser sees it. */
                RoundTrip::Exact | RoundTrip::NotParsed | RoundTrip::Aliased => {}
                _ => changed.push(*artifact),
            }
        }
        assert!(
            changed.is_empty(),
            "{} of {} corpus shapes did not come back: {changed:?}",
            changed.len(),
            ROUNDTRIP_CORPUS.len(),
        );
    }
    /// The oracle check has to reject the Bash that was actually used.
    ///
    /// The shape the gate node taught: an oracle needs a demonstration it
    /// can fail before its agreement means anything. Here the failure to
    /// demonstrate is the exact one this node exists for -- the ambient
    /// `/usr/bin/bash`, which was 5.2.37 where the pin says 5.3.15 -- plus
    /// a reference that is not there at all, which the targets used to
    /// treat as "nothing to compare" and pass.
    // [spec:nsh:req:compat.bash.reference-profile/test]
    // [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
    #[test]
    fn an_unpinned_bash_is_refused() {
        let pinned = crate::fuzzing::reference::pinned_version().expect("a recorded pin");
        assert!(
            pinned.starts_with("5.3."),
            "the calibration record pins {pinned:?}, which is not a 5.3 build",
        );

        let missing = std::path::Path::new("/nonexistent/nsh-fuzz-oracle/bash");
        assert!(
            crate::fuzzing::reference::verify(missing).is_err(),
            "a reference that cannot be run was accepted",
        );

        /* The ambient Bash is only a useful negative when it is not
         * itself the pinned build, which is true wherever these two
         * versions differ and is the situation this node was filed for. */
        let ambient = std::path::Path::new("/usr/bin/bash");
        if ambient.exists() {
            let reported = std::process::Command::new(ambient)
                .arg("--version")
                .output()
                .expect("the ambient bash runs");
            let first = String::from_utf8_lossy(&reported.stdout);
            let first = first.lines().next().unwrap_or_default().to_owned();
            if !first.contains(&pinned) {
                assert!(
                    crate::fuzzing::reference::verify(ambient).is_err(),
                    "the ambient {first:?} was accepted as the pinned {pinned:?}",
                );
            }
        }
    }
}
