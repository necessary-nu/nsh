//! What an `X(` that never closes means, put to both shells.
//!
//! An extended-glob group makes blanks, operators and newlines the
//! pattern's own bytes, so a group left open does not end the word at the
//! end of the line -- it goes on reading, and the commands after it stop
//! being commands. Bash opens the group in its parser and refuses the
//! script; the only way to tell the two readings apart is to put a command
//! after the group and ask whether it ran.
//!
//! `set -f` is in every script because five of these words are patterns
//! when they do close, and a pattern's answer is whatever the directory
//! the test happens to run in holds. The parse is what is being compared,
//! and turning pathname expansion off is what leaves only the parse.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// Every operator Bash gives a group, because a fix that reached only
/// `@(` would pass a test written against `@(` alone.
const OPERATORS: &[&str] = &["@", "?", "*", "+", "!"];

/// Script bodies with `X` standing for the operator. The `AFTER` line is
/// the measurement: a shell that refuses the script never prints it, and
/// a shell that reads the rest of the input into the word prints it
/// inside the `<>` rather than on its own.
const SHAPES: &[(&str, &str)] = &[
    (
        "open at end of input, with a command after it",
        "shopt -s extglob\nprintf \"<%s>\" X(a\necho AFTER\n",
    ),
    (
        "open at end of input, with nothing after it",
        "shopt -s extglob\nprintf \"<%s>\" X(a\n",
    ),
    (
        "open across a newline and closed on the next line",
        "shopt -s extglob\nprintf \"<%s>\" X(a\nb)\necho AFTER\n",
    ),
    (
        "closed on the line it opened on",
        "shopt -s extglob\nprintf \"<%s>\" X(a|b)\necho AFTER\n",
    ),
    (
        "a blank inside a closed group is the group's own byte",
        "shopt -s extglob\nprintf \"<%s>\" X(a b)\necho AFTER\n",
    ),
    (
        "an inner group left open inside a closed outer one",
        "shopt -s extglob\nprintf \"<%s>\" X(aX(b)\necho AFTER\n",
    ),
    (
        "an open group reaching past a pipeline operator",
        "shopt -s extglob\nprintf \"<%s>\" X(a | cat\necho AFTER\n",
    ),
    (
        "extglob off, where the parenthesis is a syntax error either way",
        "printf \"<%s>\" X(a\necho AFTER\n",
    ),
    (
        "quoting takes the operator away from the parser",
        "shopt -s extglob\nprintf \"<%s>\" \"X(a\"\necho AFTER\n",
    ),
    (
        "a case pattern whose group never closes",
        "shopt -s extglob\ncase ax in X(a echo NO;; esac\necho AFTER\n",
    ),
    (
        "a case pattern whose group closes",
        "shopt -s extglob\ncase ax in X(a)x) echo MATCH;; esac\necho AFTER\n",
    ),
];

/// Shapes that say nothing about which operator opened the group, kept
/// out of the table above so they are not run five times each.
///
/// A here-document body is read in a quoted syntax and the group reader
/// admits only `SyntaxContext::Base`, so a `@(` there must stay ordinary
/// text; the same holds inside quotes. The subscript row is the one place
/// two unterminated constructs are open at once, and Bash names the
/// bracket rather than the parenthesis, so the order of the two refusals
/// is observable.
const FIXED: &[(&str, &str)] = &[
    (
        "a here-document body is data, not a pattern",
        "set -f\nshopt -s extglob\ncat <<EOF\n@(a\nEOF\necho AFTER\n",
    ),
    (
        "a quoted here-document body is data too",
        "set -f\nshopt -s extglob\ncat <<\"EOF\"\n@(a\nEOF\necho AFTER\n",
    ),
    (
        "an alias body that leaves a group open",
        "set -f\nshopt -s extglob\nshopt -s expand_aliases\n\
         alias f='printf \"<%s>\" @(a'\nf\necho AFTER\n",
    ),
    (
        "a conditional gives groups without extglob, and refuses an open one",
        "set -f\n[[ ax == @(a ]]\necho AFTER\n",
    ),
    (
        "a conditional whose group closes",
        "set -f\n[[ ax == @(a)x ]] && echo MATCH\necho AFTER\n",
    ),
    (
        "a subscript and a group both left open",
        "set -f\nshopt -s extglob\na[@(b=1\necho AFTER\n",
    ),
];

/// Each case carries its name because the assertion prints it: a table
/// of 61 scripts that failed on row 34 would say nothing about which
/// shape moved.
fn scripts() -> Vec<(String, String)> {
    let mut cases: Vec<(String, String)> = FIXED
        .iter()
        .map(|(name, script)| ((*name).to_owned(), (*script).to_owned()))
        .collect();
    for operator in OPERATORS {
        for (name, shape) in SHAPES {
            cases.push((
                format!("{name}, with `{operator}(`"),
                format!("set -f\n{}", shape.replace('X', operator)),
            ));
        }
    }
    cases
}

// [spec:nsh:req:compat.bash.expansion-globbing/test]
// [spec:posix:sem:shell.tokenization-and-parsing/test]
#[test]
fn an_unclosed_extended_glob_answers_what_bash_answers() {
    /* Standard error is dropped by `answer`, which is what makes this
     * comparable: both shells refuse the same scripts and spell the
     * refusal differently. */
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    for (name, script) in scripts() {
        let ours = pinned_bash::answer(nsh, &["-o", "bash"], &script);
        let theirs = pinned_bash::answer(&bash, &[], &script);
        assert_eq!(
            (String::from_utf8_lossy(&ours.0).into_owned(), ours.1),
            (String::from_utf8_lossy(&theirs.0).into_owned(), theirs.1),
            "{name}\n{script}"
        );
    }
}

/// The half that needs no reference: whatever a group left open means, it
/// must not mean that the command after it becomes pattern text. A shell
/// that prints `AFTER` inside the `<>` has read a command as a pattern,
/// and that is wrong against any oracle, so it is asserted here rather
/// than compared.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn an_open_group_never_swallows_the_next_command() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for operator in OPERATORS {
        let script =
            format!("set -f\nshopt -s extglob\nprintf \"<%s>\" {operator}(a\necho AFTER\n");
        let (out, status) = pinned_bash::answer(nsh, &["-o", "bash"], &script);
        assert_eq!(out, b"", "`{operator}(a` produced a word");
        assert_ne!(status, 0, "`{operator}(a` was accepted");
    }
}

/// The POSIX dialect has no extended globs to leave open, so the Bash
/// dialect's refusal must not reach it. `@(` is a syntax error there for
/// the older reason -- an unquoted `(` where a word is expected -- and
/// stays one whether or not anything closes it.
// [spec:posix:sem:shell.tokenization-and-parsing/test]
#[test]
fn the_posix_dialect_still_refuses_every_parenthesis() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for operator in OPERATORS {
        for tail in ["", "b)\n"] {
            let script = format!("printf \"<%s>\" {operator}(a\n{tail}echo AFTER\n");
            let (out, status) = pinned_bash::answer(nsh, &[], &script);
            assert_eq!(out, b"", "`{operator}(a` produced a word in POSIX mode");
            assert_eq!(status, 2, "`{operator}(a` was accepted in POSIX mode");
        }
    }
}
