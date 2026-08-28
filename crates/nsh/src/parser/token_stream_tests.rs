//! The concatenation property: a parse's tokens are the bytes it read.

use bstr::{BStr, BString};

use super::tokens::SourceTokenKind;
use crate::context::Shell;

fn shell(bash: bool) -> Shell {
    Shell::builder()
        .streams(crate::Streams::capture().expect("captured streams"))
        .option(BStr::new(b"bash"), bash)
        .build()
        .expect("shell")
}

/// The bytes one parse of `source` recorded, and the bytes it read.
///
/// The second is measured independently of the token log, from the input
/// frame's own cursor: `position` is how far the reader advanced and
/// `unread_count` is how much of that it gave back. A parse that stops at
/// a syntax error stops there too, so the property stays exact rather than
/// collapsing into "the tokens are some prefix of the file".
// [spec:nsh:def:idiom.token-stream/test]
fn consumed(shell: &mut Shell, source: &BStr) -> (BString, usize) {
    let mut read = BString::new(Vec::new());
    let position = crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string(shell, source);
        loop {
            let outcome = crate::parser::parse_command(shell, false);
            for token in shell.input.tokens.tokens() {
                read.extend_from_slice(token.text());
            }
            let done = !matches!(outcome, Ok(crate::parser::ParseResult::Tree(_)));
            if done {
                let frame = crate::input::current_input_frame(&mut shell.input);
                assert!(frame.overlays.is_empty(), "parse ended inside an alias");
                return frame.position - frame.unread_count;
            }
        }
    });
    (read, position)
}

/// Assert that the tokens are exactly the bytes the reader handed over.
///
/// A survey script is thousands of bytes long, so the report is the first
/// byte the two disagree on and the text around it rather than both
/// strings.
// [spec:nsh:def:idiom.token-stream/test]
fn assert_accounts_for_every_byte(shell: &mut Shell, label: &str, source: &[u8]) {
    let (read, position) = consumed(shell, BStr::new(source));
    let expected = &source[..position.min(source.len())];
    if read == expected {
        return;
    }
    let offset = read
        .iter()
        .zip(expected)
        .position(|(token, byte)| token != byte)
        .unwrap_or(read.len().min(expected.len()));
    let window = offset.saturating_sub(20)..(offset + 20);
    panic!(
        "{label}: the parser read {position} bytes; the tokens differ from byte {offset}\n\
         read   {:?}\n\
         source {:?}",
        BStr::new(&read[window.start..window.end.min(read.len())]),
        BStr::new(&expected[window.start..window.end.min(expected.len())]),
    );
}

/// The bytes nothing kept before: the ones between the words.
// [spec:nsh:def:idiom.token-stream/test]
#[test]
fn trivia_is_retained_as_tokens() {
    let mut shell = shell(true);
    let source = BStr::new(b"a  b\t# c\nd \\\ne\n");
    let (read, position) = consumed(&mut shell, source);

    assert_eq!(position, source.len());
    assert_eq!(read, BString::from(source));

    crate::input::set_input_string(&mut shell, source);
    crate::parser::parse_command(&mut shell, false).expect("parse");
    let kinds: Vec<SourceTokenKind> = shell
        .input
        .tokens
        .tokens()
        .iter()
        .map(super::tokens::SourceToken::kind)
        .collect();

    assert!(kinds.contains(&SourceTokenKind::Blank), "{kinds:?}");
    assert!(kinds.contains(&SourceTokenKind::Comment), "{kinds:?}");
    assert!(kinds.contains(&SourceTokenKind::Newline), "{kinds:?}");
}

/// Every construct that reads bytes somewhere other than a plain word.
// [spec:nsh:def:idiom.token-stream/test]
#[test]
fn every_construct_accounts_for_its_bytes() {
    let mut shell = shell(true);
    for source in SHAPES {
        assert_accounts_for_every_byte(&mut shell, "shape", source);
    }
}

/// The corpus the round-trip fuzzer reduced, read as bytes rather than as
/// programs: whatever else these shapes do, the reader must account for
/// every byte of them.
// [spec:nsh:def:idiom.token-stream/test]
#[cfg(feature = "fuzzing")]
#[test]
fn the_roundtrip_corpus_accounts_for_its_bytes() {
    let mut shell = shell(true);
    for (artifact, source) in crate::fuzzing::tests::ROUNDTRIP_CORPUS {
        assert_accounts_for_every_byte(&mut shell, artifact, source);
    }
}

/// The pinned third-party survey scripts, which are real shell rather than
/// reduced fuzzer output and are the only inputs here at the scale a
/// script is actually written at.
// [spec:nsh:def:idiom.token-stream/test]
#[test]
fn the_survey_scripts_account_for_their_bytes() {
    let mut posix = shell(false);
    let mut bash = shell(true);
    let mut files = 0_usize;
    for path in survey_scripts() {
        let source = std::fs::read(&path).expect("survey script");
        let label = path.display().to_string();
        files += 1;
        assert_accounts_for_every_byte(&mut posix, &label, &source);
        assert_accounts_for_every_byte(&mut bash, &label, &source);
    }
    assert!(
        files > 100,
        "expected the survey corpus, found {files} files"
    );
}

/// The `.sh` files under `tests/surveys`, sorted so a failure is stable.
// [spec:nsh:def:idiom.token-stream/test]
fn survey_scripts() -> Vec<std::path::PathBuf> {
    let root = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../tests/surveys")
        .canonicalize()
        .expect("survey root");
    let mut found = Vec::new();
    let mut pending = vec![root];
    while let Some(directory) = pending.pop() {
        for entry in std::fs::read_dir(directory).expect("survey directory") {
            let path = entry.expect("survey entry").path();
            if path.is_dir() {
                pending.push(path);
            } else if path.extension().is_some_and(|suffix| suffix == "sh") {
                found.push(path);
            }
        }
    }
    found.sort();
    found
}

/// One shape per construct that reads bytes outside an ordinary word, so a
/// failure names the construct rather than a fuzzer artifact.
const SHAPES: &[&[u8]] = &[
    b"echo hi\n",
    b"  echo   hi  \n",
    b"# comment only\n",
    b"echo hi # trailing\n",
    b"echo \\\n  hi\n",
    b"ec\\\nho hi\n",
    b"echo 'a b' \"c d\" $'e\\tf'\n",
    b"echo \"${a:-b}\" ${b#c} ${d//e/f}\n",
    b"echo $(date) `date` $((1 + 2)) $[3]\n",
    b"cat <<EOF\nbody $x\nEOF\n",
    b"cat <<'EOF'\nliteral $x\nEOF\n",
    b"cat <<-EOF\n\tstripped\n\tEOF\n",
    b"cat <<A <<B\none\nA\ntwo\nB\n",
    b"cat <<EOF\n$(echo nested)\nEOF\n",
    b"cat <<<here\n",
    b"if true; then echo a; elif false; then echo b; else echo c; fi\n",
    b"for i in a b c; do echo $i; done\n",
    b"for i in; do echo $i; done\n",
    b"while read -r line; do echo $line; done < f\n",
    b"until false; do break; done\n",
    b"case $x in a|b) echo one;; c) echo two;& *) echo three;; esac\n",
    b"f() { echo body; }\n",
    b"function g { echo body; }\n",
    b"a=1 b=2 command arg >out 2>&1 <in >>append\n",
    b"exec 3<&0 4>&1 5<>rw\n",
    b"true && false || ! true\n",
    b"a | b | c &\n",
    b"{ echo a; } ; ( echo b )\n",
    b"[[ $v == a* && $v =~ ^a.*z$ ]]\n",
    b"((x = 1 + 2))\n",
    b"select item in a b; do echo $item; done\n",
    b"time echo timed\n",
    b"declare -a arr=(1 2 3)\n",
    b"arr[0]=x\n",
    b"echo ${arr[@]:1:2}\n",
    b"coproc echo hi\n",
    b"diff <(echo a) >(echo b)\n",
    b"echo a\r\n",
    b"echo \xc3\xa9 caf\xc3\xa9\n",
    b"echo a;;\n",
    b"echo \"unterminated\n",
    b"echo ${\n",
    b"\n\n\n",
    b"",
    b"echo a\n# tail comment with no newline",
];
