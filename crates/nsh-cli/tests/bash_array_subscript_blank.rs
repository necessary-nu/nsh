//! A blank inside `[...]`, written and printed back, measured against the
//! pinned Bash 5.3.
//!
//! `m[x y]=z` names the key `x y`, and the blank in it is data. Inside a
//! compound assignment's parentheses the same subscript was being read by
//! the ordinary word rules, so `declare -A m=([x y]=z)` arrived as the two
//! words `[x` and `y]=z` -- neither of which carries a subscript, and both
//! of which an associative array then dropped. The array came out empty
//! and nothing was reported.
//!
//! The printing side asks the same question in reverse. `declare -p`
//! exists to be read back by the shell that printed it, so a key holding a
//! blank has to come back quoted; printing `[x y]=` bare produced a line
//! whose own shell could not read it.
//!
//! The two rules the rows below pin:
//!
//! * A `[` opens a blank-spanning subscript only as an element's first
//!   byte. `a=(x[1 2]=v)` still splits at the blank, and so does a word
//!   outside a compound assignment altogether.
//! * A key is quoted exactly when leaving it bare would not read back as
//!   itself, which is Bash's `sh_contains_shell_metas` plus the
//!   unprintable characters that force `$'...'`. `[a]` and `[a=b]` stay
//!   bare; `[a b]` and `[a$b]` take double quotes.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::io::Write as _;
use std::path::Path;
use std::process::{Command, Stdio};

/// One script and what the pinned Bash prints for it on standard output.
///
/// Stdout only: the diagnostics this shell writes are registered as
/// differing in wording, and every row here is about what was stored or
/// how it was spelled back.
///
/// An associative array's iteration order is its hash order, which the two
/// shells do not share, so no row prints more than one key at a time.
const CASES: &[(&str, &str)] = &[
    /* The compound assignment's subscript, which is what was lost. */
    (
        "declare -A m=([x y]=z)\ndeclare -p m\n",
        "declare -A m=([\"x y\"]=\"z\" )\n",
    ),
    (
        "declare -A m=([a b]=1 [c]=2)\necho \"${#m[@]} ${m[a b]} ${m[c]}\"\n",
        "2 1 2\n",
    ),
    (
        "declare -A m=([a b]=1)\nfor k in \"${!m[@]}\"; do echo \"<$k>\"; done\n",
        "<a b>\n",
    ),
    (
        "declare -A m=([a b c]=1)\ndeclare -p m\n",
        "declare -A m=([\"a b c\"]=\"1\" )\n",
    ),
    /* The subscript is data all the way to its matching bracket: shell
     * operators and a comment character are its own bytes, and a nested
     * pair does not close it. */
    (
        "declare -A m=([a;b]=1)\ndeclare -p m\n",
        "declare -A m=([\"a;b\"]=\"1\" )\n",
    ),
    (
        "declare -A m=([a)b]=1)\ndeclare -p m\n",
        "declare -A m=([\"a)b\"]=\"1\" )\n",
    ),
    (
        "declare -A m=([a #b]=1)\ndeclare -p m\n",
        "declare -A m=([\"a #b\"]=\"1\" )\n",
    ),
    (
        "declare -A m=([a[b c]d]=1)\ndeclare -p m\n",
        "declare -A m=([\"a[b c]d\"]=\"1\" )\n",
    ),
    /* An indexed array's subscript is an arithmetic expression, where the
     * blanks were always the expression's. */
    (
        "declare -a a=([2 ]=x)\ndeclare -p a\n",
        "declare -a a=([2]=\"x\")\n",
    ),
    (
        "a=([1 + 1]=x)\ndeclare -p a\n",
        "declare -a a=([2]=\"x\")\n",
    ),
    (
        "a=(one [1 +1]=x)\ndeclare -p a\n",
        "declare -a a=([0]=\"one\" [2]=\"x\")\n",
    ),
    /* Every spelling of the assignment reaches it. */
    (
        "declare -A m\nm+=([a b]=1)\ndeclare -p m\n",
        "declare -A m=([\"a b\"]=\"1\" )\n",
    ),
    (
        "f() { local -A m=([a b]=1); declare -p m; }\nf\n",
        "declare -A m=([\"a b\"]=\"1\" )\n",
    ),
    (
        "declare -A m=([\"a b\"]=z)\ndeclare -p m\n",
        "declare -A m=([\"a b\"]=\"z\" )\n",
    ),
    /* Only an element's first byte opens one. A bracket after other bytes,
     * and a bracket in a word that is not an element at all, both leave
     * the blank a field separator. */
    (
        "a=(x[1 2]=v)\ndeclare -p a\n",
        "declare -a a=([0]=\"x[1\" [1]=\"2]=v\")\n",
    ),
    (
        "a=(*[a b]*)\ndeclare -p a\n",
        "declare -a a=([0]=\"*[a\" [1]=\"b]*\")\n",
    ),
    /* The bracket closes and the ordinary rules resume, so the blank after
     * the value still ends the element. */
    (
        "a=([0]=c d)\ndeclare -p a\n",
        "declare -a a=([0]=\"c\" [1]=\"d\")\n",
    ),
    /* The printed key: quoted exactly when bare would not read back. */
    (
        "declare -A m\nm[x y]=z\ndeclare -p m\n",
        "declare -A m=([\"x y\"]=\"z\" )\n",
    ),
    (
        "declare -A m\nm[a]=1\ndeclare -p m\n",
        "declare -A m=([a]=\"1\" )\n",
    ),
    (
        "declare -A m\nm['a=b']=1\ndeclare -p m\n",
        "declare -A m=([a=b]=\"1\" )\n",
    ),
    (
        "declare -A m\nm['a$b']=1\ndeclare -p m\n",
        "declare -A m=([\"a\\$b\"]=\"1\" )\n",
    ),
    (
        "declare -A m\nm['#']=1\ndeclare -p m\n",
        "declare -A m=([\"#\"]=\"1\" )\n",
    ),
    /* `#` and `~` are syntax only where they stand, so neither forces
     * quotes in the middle of a key. */
    (
        "declare -A m\nm['a#b']=1\ndeclare -p m\n",
        "declare -A m=([a#b]=\"1\" )\n",
    ),
    (
        "declare -A m\nm['a~b']=1\ndeclare -p m\n",
        "declare -A m=([a~b]=\"1\" )\n",
    ),
    (
        "declare -A m\nm['a=~b']=1\ndeclare -p m\n",
        "declare -A m=([\"a=~b\"]=\"1\" )\n",
    ),
    /* An unprintable character takes `$'...'`, as the value beside it
     * already did. */
    (
        "declare -A m\nm[$'a\\nb']=1\ndeclare -p m\n",
        "declare -A m=([$'a\\nb']=\"1\" )\n",
    ),
    (
        "declare -A m\nm[$'a\\tb']=1\ndeclare -p m\n",
        "declare -A m=([$'a\\tb']=\"1\" )\n",
    ),
    /* Both halves at once: the line `declare -p` prints is a line the
     * shell that printed it reads back. */
    (
        "declare -A m\nm['x y']=z\nt=$(declare -p m)\nunset m\neval \"$t\"\n\
         echo \"${#m[@]} [${m[x y]}]\"\n",
        "1 [z]\n",
    ),
];

/// Feed one case to a shell on standard input and return its stdout.
fn output(shell: &Path, dialect: &[&str], script: &str) -> String {
    let mut child = Command::new(shell)
        .args(dialect)
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .env("LC_ALL", "C")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .unwrap_or_else(|error| panic!("start {}: {error}", shell.display()));
    child
        .stdin
        .take()
        .expect("the child's standard input")
        .write_all(script.as_bytes())
        .expect("write the script");
    let output = child.wait_with_output().expect("wait for the shell");
    String::from_utf8(output.stdout).expect("these scripts print ASCII")
}

// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_subscript_carries_its_blanks_both_ways() {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for (script, reference) in CASES {
        assert_eq!(
            output(nsh, &["-o", "bash"], script),
            *reference,
            "for\n{script}"
        );
    }
}

/// The table is the reference's answer, not this repository's opinion.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_recorded_output_is_the_references_own() {
    let bash = pinned_bash::path();
    for (script, reference) in CASES {
        assert_eq!(
            output(&bash, &[], script),
            *reference,
            "the reference disagrees with the recorded output for\n{script}"
        );
    }
}
