//! The deliberately narrow interface used by the separate cargo-fuzz
//! workspace.
//!
//! This module is public only with the `fuzzing` feature. It provides an
//! opaque parse-and-print operation so the fuzzer can exercise the printer's
//! semantic fixed-point without exposing the AST or parser as library API.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::nodes::Node;

/// Parse `source` without executing it and return its canonical rendering.
///
/// The input frame and all parser-local resources are restored before this
/// returns, including when parsing rejects the source. Calling this twice on
/// its own output is the parse-and-print fuzzing property's fixed-point.
pub fn canonical_source(shell: &mut Shell, source: &BStr) -> Result<BString, Error> {
    crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string(shell, source);
        let mut rendered = BString::new(Vec::new());
        loop {
            match crate::parser::parse_command(shell, false)? {
                crate::parser::ParseResult::Eof => break,
                crate::parser::ParseResult::Tree(Some(node)) => {
                    let command = crate::nodes::source::command(&shell.locale, &node);
                    if command.is_empty() {
                        continue;
                    }
                    rendered.extend_from_slice(&command);
                    if command.last() != Some(&b'\n') {
                        rendered.push(b'\n');
                    }
                }
                crate::parser::ParseResult::Tree(None) => {}
            }
        }
        Ok(rendered)
    })
}

/// What printing a parsed program and reading it back again produced.
///
/// A verdict rather than a pair of trees: the property is what the fuzz
/// workspace needs, and the syntax tree stays inside the crate that owns it.
// [spec:nsh:req:idiom.printable-ast]
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Reversibility {
    /// The source did not parse, so there was nothing to print. An ordinary
    /// answer to arbitrary bytes, and not a finding.
    NotParsed,
    /// The printed program did not parse.
    NotReparsed { printed: BString },
    /// The printed program parsed as a different program.
    Changed { printed: BString },
    /// The printed program parsed as the same program.
    Reversible { printed: BString },
}

/// Whether printing `source`'s program and parsing the result recovers it.
///
/// This is [`spec:nsh:req:idiom.printable-ast`] made checkable. Source
/// positions do not take part, because printing relocates every one of them
/// and a position is provenance rather than identity -- see
/// [`crate::nodes::SourceLine`].
// [spec:nsh:req:idiom.printable-ast]
pub fn printing_is_reversible(shell: &mut Shell, source: &BStr) -> Reversibility {
    let Ok(program) = parse_program(shell, source) else {
        return Reversibility::NotParsed;
    };
    let printed = print_program(shell, &program);
    match parse_program(shell, printed.as_ref()) {
        Err(_) => Reversibility::NotReparsed { printed },
        Ok(reparsed) if reparsed != program => Reversibility::Changed { printed },
        Ok(_) => Reversibility::Reversible { printed },
    }
}

/// Parse every top-level command in `source` without executing any of them.
///
/// The input frame and all parser-local resources are restored before this
/// returns, including when parsing rejects the source.
///
/// A top-level `;` produces a [`Node::Sequence`] while a newline produces two
/// commands, and the two spell the same program: the separator is layout, so
/// the sequence is flattened into the list it already is. Nothing below the
/// top level is touched, where a sequence is a child rather than punctuation.
fn parse_program(shell: &mut Shell, source: &BStr) -> Result<Vec<Node>, Error> {
    crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string(shell, source);
        let mut program = Vec::new();
        loop {
            match crate::parser::parse_command(shell, false)? {
                crate::parser::ParseResult::Eof => break,
                crate::parser::ParseResult::Tree(Some(node)) => {
                    push_command_sequence(&mut program, node);
                }
                crate::parser::ParseResult::Tree(None) => {}
            }
        }
        Ok(program)
    })
}

fn push_command_sequence(program: &mut Vec<Node>, node: Node) {
    match node {
        Node::Sequence(sequence) => {
            push_command_sequence(program, *sequence.left);
            push_command_sequence(program, *sequence.right);
        }
        node => program.push(node),
    }
}

fn print_program(shell: &Shell, program: &[Node]) -> BString {
    let mut printed = BString::new(Vec::new());
    for node in program {
        let command = crate::nodes::source::command(&shell.locale, node);
        printed.extend_from_slice(&command);
        if command.last() != Some(&b'\n') {
            printed.push(b'\n');
        }
    }
    printed
}

#[cfg(test)]
mod tests {
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

    /// Assert each source prints as itself, one line per program.
    fn assert_prints_itself(shell: &mut Shell, sources: &[&[u8]]) {
        for source in sources {
            assert_eq!(
                printing_is_reversible(shell, BStr::new(source)),
                Reversibility::Reversible {
                    printed: BString::from([*source, b"\n"].concat()),
                },
                "{:?}",
                BStr::new(source),
            );
        }
    }

    fn assert_roundtrip_fixed(source: &[u8]) {
        let mut shell = shell();
        // Rejecting the input is an ordinary answer to fuzzer bytes, and the
        // fuzz target returns on it too: the property is about what the
        // printer emits, so there is nothing to check until it emits something.
        let Ok(once) = canonical_source(&mut shell, BStr::new(source)) else {
            return;
        };
        let twice = canonical_source(&mut shell, BStr::new(&once))
            .unwrap_or_else(|error| panic!("second canonicalization of {once:?}: {error:?}"));
        assert_eq!(once, twice);
    }

    #[test]
    fn canonical_source_is_a_fixed_point() {
        let mut shell = shell();
        let once = canonical_source(
            &mut shell,
            BStr::new(b"v=abc\nif [[ $v == a* ]]; then printf '%s\\n' \"$v\"; fi\n"),
        )
        .expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, twice);
    }

    #[test]
    fn canonical_source_keeps_top_level_sequence_fixed() {
        let mut shell = shell();
        let once = canonical_source(&mut shell, BStr::new(b"false ; x=hi\n"))
            .expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, BString::from(b"false\nx=hi\n".as_slice()));
        assert_eq!(once, twice);
    }

    #[test]
    fn canonical_source_keeps_backgrounded_sequence_fixed() {
        let mut shell = shell();
        let once = canonical_source(
            &mut shell,
            BStr::new(b"echo hi\n{ sleep 1 ; echo derp ; } &\necho bye\nwait"),
        )
        .expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(
            once,
            BString::from(
                b"echo hi\n{ \n    sleep 1;\n    echo derp\n} &\necho bye\nwait\n".as_slice()
            )
        );
        assert_eq!(once, twice);
    }

    #[test]
    fn canonical_source_flushes_final_here_document() {
        let mut shell = shell();
        let source = BStr::new(
            br#"cat <<EOF
echo \\\$var
EOF
cat <<'EOF'
echo \\\$var
EOF
"#,
        );
        let once = canonical_source(&mut shell, source).expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, source);
        assert_eq!(once, twice);
    }

    #[test]
    fn canonical_source_keeps_an_escaped_single_quote_fixed() {
        let mut shell = shell();
        let once = canonical_source(&mut shell, BStr::new(b"\\'")).expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, BString::from(b"\\'\n".as_slice()));
        assert_eq!(once, twice);
    }

    /// An invalid expansion fails on bytes the source wrote, and the tree
    /// still holds them. Printing `${}` in their place spelled a different
    /// failure and threw away whatever the braces were around.
    // [spec:nsh:req:idiom.printable-ast/test]
    #[test]
    fn canonical_source_keeps_an_invalid_parameter_fixed() {
        let source = b"${(M)foo}";
        let mut shell = shell();
        let once = canonical_source(&mut shell, BStr::new(source)).expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, BString::from(b"${(M)foo}\n".as_slice()));
        assert_eq!(once, twice);
    }

    #[test]
    fn canonical_source_keeps_a_brace_group_pipeline_fixed() {
        let source = b"ty} | {  t  \n3#\n}\n# ";
        let mut shell = shell();
        let once = canonical_source(&mut shell, BStr::new(source)).expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, twice);
    }

    /// A `]` inside `${...}` is the expansion's own byte, so the subscript
    /// around it stays open. Counting it as the closing bracket left the
    /// printer holding a word whose brackets it could not put back.
    #[test]
    fn canonical_source_keeps_a_subscript_open_across_parameters() {
        let source = b"a[${x:-]}]=y";
        let mut shell = shell();
        let once = canonical_source(&mut shell, BStr::new(source)).expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, BString::from(b"a[${x:-]}]=y\n".as_slice()));
        assert_eq!(once, twice);
        assert!(canonical_source(&mut shell, BStr::new(b"a[[${ ]]}")).is_err());
    }

    /// Inside `"` an apostrophe is an ordinary byte, so the backslash before
    /// one protects nothing and the two spell themselves. Writing a second
    /// backslash for the first grew the word by two bytes a round and
    /// expanded to one more backslash each time.
    // [spec:nsh:req:idiom.printable-ast/test]
    #[test]
    fn canonical_source_keeps_an_escaped_apostrophe_fixed() {
        let source = b"\"${a+\\'}\"";
        let mut shell = shell();
        let once = canonical_source(&mut shell, BStr::new(source)).expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, BString::from(b"\"${a+\\'}\"\n".as_slice()));
        assert_eq!(once, twice);
    }

    /// Two further reductions of the artifact behind
    /// `crash_67407b0220a752d0f6932c9c9a3349b7e7ff9413`, each of which
    /// outlived the fix the one before it forced: inside a `"` this printer
    /// opened, an operand quote protects nothing but a `}`, and writing one
    /// back anyway hands the next parse an operand it reads differently.
    #[test]
    fn canonical_source_drops_a_redundant_operand_quote() {
        assert_roundtrip_fixed(b"\"${a+\"\"${a#\x00${ ''$''}''a}}\"$(\"\")\"\"''");
        assert_roundtrip_fixed(b"\"${a+\"\"${a#\x00${ ''$''}'a'}}\"$(\"\")\"\"''");
    }

    /// The tree a printed program parses back to is the one it came from,
    /// down to every part the grammar carries. Positions are the exception,
    /// and deliberately so: printing relocates all of them.
    // [spec:nsh:req:idiom.printable-ast/test]
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
            let verdict = printing_is_reversible(&mut shell, BStr::new(source));
            assert!(
                matches!(verdict, Reversibility::Reversible { .. }),
                "{:?} printed to something else: {verdict:?}",
                BStr::new(source),
            );
        }
    }

    /// Rejecting the input is an ordinary answer to arbitrary bytes: there is
    /// no program to print, so there is nothing the property can say.
    // [spec:nsh:req:idiom.printable-ast/test]
    #[test]
    fn printing_says_nothing_about_a_rejected_program() {
        let mut shell = shell();
        assert_eq!(
            printing_is_reversible(&mut shell, BStr::new(b"if")),
            Reversibility::NotParsed,
        );
    }

    /// A `"` inside a `${...}` operand toggles the quoting the word arrived
    /// in. Dropping the toggle used to reopen the parameter grammar to the
    /// `}` it was protecting, which printed a stable program that ran
    /// differently -- the corruption the fixed-point property could not see.
    // [spec:nsh:req:idiom.printable-ast/test]
    #[test]
    fn printing_keeps_a_braced_quoted_operand() {
        let mut shell = shell();
        let source = b"echo \"${a+\"a}b\"}\"";
        assert_eq!(
            printing_is_reversible(&mut shell, BStr::new(source)),
            Reversibility::Reversible {
                printed: BString::from(b"echo \"${a+\"a}b\"}\"\n".as_slice()),
            },
        );
    }

    /// The three ways to introduce a definition are three trees, so a
    /// renderer that picks one hands the next parse a definition it was not
    /// given. `declare -f` still normalises, the way Bash's does.
    // [spec:nsh:req:idiom.printable-ast/test]
    #[test]
    fn printing_keeps_a_definition_style() {
        let mut shell = shell();
        for (source, printed) in [
            (
                b"f() { echo one; }".as_slice(),
                b"f () \n{ \n    echo one\n}\n".as_slice(),
            ),
            (
                b"function f { echo one; }",
                b"function f \n{ \n    echo one\n}\n",
            ),
            (
                b"function f () { echo one; }",
                b"function f () \n{ \n    echo one\n}\n",
            ),
        ] {
            assert_eq!(
                printing_is_reversible(&mut shell, BStr::new(source)),
                Reversibility::Reversible {
                    printed: BString::from(printed)
                },
                "{:?}",
                BStr::new(source),
            );
        }
    }

    /// `'a'` and `"a"` protect the same byte, so which one was written is
    /// not recoverable from the run and the parser records it. A backslash
    /// inside the run is the same question one level down: it spells itself
    /// only when the part after it is protected too.
    // [spec:nsh:req:idiom.printable-ast/test]
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
    // [spec:nsh:req:idiom.printable-ast/test]
    #[test]
    fn printing_keeps_a_here_document_delimiter() {
        let mut shell = shell();
        for (source, printed) in [
            (
                b"cat <<MOF\nhello\nEOF\nMOF\n".as_slice(),
                b"cat <<MOF\nhello\nEOF\nMOF\n".as_slice(),
            ),
            (b"cat <<'Q'\nbody\nQ\n", b"cat <<'Q'\nbody\nQ\n"),
            (b"<<a\nx", b" <<a\nx\na\n"),
        ] {
            assert_eq!(
                printing_is_reversible(&mut shell, BStr::new(source)),
                Reversibility::Reversible {
                    printed: BString::from(printed)
                },
                "{:?}",
                BStr::new(source),
            );
        }
    }

    /// A `$` that starts nothing is an ordinary byte. Protecting it anyway
    /// spelled the same byte with a part the source never wrote, which is
    /// the whole shape of over-protection: `\$`, `\\`, `\'` all read back as
    /// one part more than they went in as.
    // [spec:nsh:req:idiom.printable-ast/test]
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
    // [spec:nsh:req:idiom.printable-ast/test]
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
            let verdict = printing_is_reversible(&mut shell, BStr::new(source));
            assert!(
                matches!(verdict, Reversibility::Reversible { .. }),
                "{:?} printed to something else: {verdict:?}",
                BStr::new(source),
            );
        }
    }

    /// A byte that is only special where it begins something keeps its own
    /// spelling everywhere else. `#` opens a comment only where a word
    /// begins, and a `$` against a backslash starts nothing at all.
    // [spec:nsh:req:idiom.printable-ast/test]
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
            ],
        );
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
    const ROUNDTRIP_CORPUS: &[(&str, &[u8])] = &[
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

    /// Printing the corpus twice reaches the same text every time.
    ///
    /// Every shape at once rather than one test each: a failure should say
    /// which shapes came back, not which single shape came back first.
    // [spec:nsh:req:idiom.printable-ast/test]
    #[test]
    fn the_round_trip_corpus_prints_fixed_points() {
        let mut shell = shell();
        let mut unstable = Vec::new();
        for (artifact, source) in ROUNDTRIP_CORPUS {
            // Rejecting the input is an ordinary answer, and the fuzz target
            // returns on it too: there is nothing to check until the printer
            // emits something.
            let Ok(once) = canonical_source(&mut shell, BStr::new(source)) else {
                continue;
            };
            match canonical_source(&mut shell, BStr::new(&once)) {
                Ok(twice) if twice == once => {}
                _ => unstable.push(*artifact),
            }
        }
        assert!(
            unstable.is_empty(),
            "{} of {} corpus shapes no longer print a fixed point: {unstable:?}",
            unstable.len(),
            ROUNDTRIP_CORPUS.len(),
        );
    }
}
