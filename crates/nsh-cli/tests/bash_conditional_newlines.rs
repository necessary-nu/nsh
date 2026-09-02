//! Where a newline may fall inside `[[ ]]`, and which line the construct
//! records, both measured against the pinned Bash 5.3.
//!
//! Bash's conditional parser skips newlines wherever it is about to read
//! an operand or a connective -- after `[[`, after `&&` and `||`, after
//! `(` and after `!` -- and nowhere else. So `[[ 1 = 1 &&` may end a line
//! and `[[ 1 =` may not, and a bare word term must be followed, on the
//! line it was written on, by whatever ends it: `[[ $x ]]` parses and
//! `[[ $x` with the `]]` on the next line does not. This shell had it the
//! other way round in every one of those positions, skipping newlines
//! only on the closing side: four shapes Bash runs were refused here and
//! one Bash refuses was accepted.
//!
//! The line a conditional records is the same question asked again.
//! `[[ ]]` records neither the line it opens on nor the line it closes
//! on. Bash stamps the line its parser has reached into the *top* node of
//! the expression as that node is built, so a test holds its last
//! operand's line, a group holds its `)`, an `&&` holds the line of
//! whatever follows its right operand, and a `!` builds no node of its
//! own and so holds its operand's. `[[ 1 = 2 <newline> ]]` is therefore
//! line 1 and `[[ 1 = 1 && 1 = 2 <newline> ]]` is line 2. None of that
//! could be pinned while the shapes that demonstrate it were unparseable,
//! which is why the two tables are one file.
//!
//! Both tables are differential. Every case is put to both shells and
//! their answers compared, so nothing here is a recorded expectation that
//! can outlive the reference that produced it.

/// Shared with `nsh`'s own differential tests rather than copied: one
/// answer to "which Bash", and one to how a script is put to a shell.
#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// Scripts whose parse is the measurement: each either runs and prints or
/// is refused, and the two shells have to do the same one.
const PARSED: &[&str] = &[
    /* The four positions where Bash skips a newline and this shell did
     * not, plus the repetitions that prove it is a skip and not one
     * tolerated newline. */
    "[[\n1 = 1 ]] && echo yes\n",
    "[[\n\n\n1 = 1 ]] && echo yes\n",
    "[[ # a comment where the operand will be\n1 = 1 ]] && echo yes\n",
    "[[ 1 = 1 &&\n2 = 2 ]] && echo yes\n",
    "[[ 1 = 2 ||\n2 = 2 ]] && echo yes\n",
    "[[ (\n1 = 1 ) ]] && echo yes\n",
    "[[ (\n(\n1 = 1 ) ) ]] && echo yes\n",
    "[[ !\n1 = 2 ]] && echo yes\n",
    "[[ !\n!\n1 = 1 ]] && echo yes\n",
    "[[ 1 = 1 &&\n( 2 = 2 ) ]] && echo yes\n",
    "[[ !\n-n \"\" ]] && echo yes\n",
    "[[\nabc =~ ^a ]] && echo yes\n",
    /* The mirror positions, which already parsed here and have to go on
     * parsing: it is the position that decides and not the newline. */
    "[[ 1 = 1\n]] && echo yes\n",
    "[[ 1 = 1\n&& 2 = 2 ]] && echo yes\n",
    "[[ ( 1 = 1\n) ]] && echo yes\n",
    /* And the positions Bash refuses a newline in. The first three are
     * an operand Bash is already committed to reading; the last three are
     * the bare-word term, whose node Bash builds only once it has read
     * what follows, and which it therefore will not let a newline end. */
    "[[ 1 =\n1 ]] && echo yes\n",
    "[[ -n\nx ]] && echo yes\n",
    "[[ abc =~\n^a ]] && echo yes\n",
    "[[ x\n]] && echo yes\n",
    "[[ x\n&& y ]] && echo yes\n",
    "[[ ( x\n) ]] && echo yes\n",
    /* A conditional with nothing in it is refused while parsing rather
     * than answered with a status, newlines or no newlines. */
    "[[ ]] && echo yes\n",
    "[[\n]] && echo yes\n",
    /* The newlines around a conditional belong to the grammar rather than
     * to the conditional, and a here-document body still arrives after
     * the line its operator was written on. */
    "if [[\n1 = 1 ]]; then echo yes; fi\n",
    "[[\n1 = 1 ]] && cat <<EOF\nbody\nEOF\n",
];

/// Three lines read before each case's own first line, so that a reported
/// line names a line of the body plus three. The third is a command of
/// its own, so a line left over from an earlier command is told apart
/// from every line the body occupies.
const PRELUDE: &str = "set -o errtrace\ntrap 'echo line=$LINENO' ERR\n:\n";

/// Conditionals that are false, so the `ERR` action reports the line the
/// construct recorded. The `ERR` action is the channel because a
/// conditional's own status is checked, which is what reads that line
/// back out without the body of anything else having overwritten it.
const RECORDED: &[&str] = &[
    /* A test holds the line its last operand is on, which a line
     * continuation moves and a newline before the `]]` does not. */
    "[[ 1 = 2 ]]\n",
    "[[ 1 = 2\n]]\n",
    "[[ 1 \\\n= 2 ]]\n",
    "[[ 1 = 2 \\\n]]\n",
    "[[\n1 = 2 ]]\n",
    "[[\n\n\n1 = 2 ]]\n",
    "[[ -n ''\n]]\n",
    "[[\n-n '' ]]\n",
    /* A bare word is the one term whose node is built after the token
     * that ends it has been read, so a continuation before the `]]`
     * moves it where the same continuation after a test does not. */
    "[[ '' ]]\n",
    "[[\n'' ]]\n",
    "[[ '' \\\n]]\n",
    /* A group holds its `)`. */
    "[[ (\n1 = 2 ) ]]\n",
    "[[ ( 1 = 2\n) ]]\n",
    "[[ (\n1 = 2\n) ]]\n",
    "[[ ( 1 = 2 )\n]]\n",
    "[[ ( 1 = 2 ) \\\n]]\n",
    "[[ ( ( 1 = 2\n) ) ]]\n",
    /* An `&&` or `||` holds the line of whatever follows its right
     * operand, which is the one answer that is neither an operand's line
     * nor the construct's opening line. */
    "[[ 1 = 1 &&\n1 = 2 ]]\n",
    "[[ 1 = 1 &&\n1 = 2\n]]\n",
    "[[ 1 = 1 &&\n\n1 = 2 ]]\n",
    "[[ 1 = 1 && 1 = 2\n]]\n",
    "[[ 1 = 1 && 1 = 2 \\\n]]\n",
    "[[ 1 = 1 &&\n1 = 1 &&\n1 = 2 ]]\n",
    "[[ 1 = 2 ||\n1 = 2 ]]\n",
    "[[ ( 1 = 1 )\n&& 1 = 2 ]]\n",
    /* A `!` builds no node, so it reports whatever it negates. */
    "[[ !\n1 = 1 ]]\n",
    "[[ ! 1 = 1\n]]\n",
];

/// Put every script to both shells and require the same answer of each.
///
/// Standard output and exit status together, because a refused parse is
/// an answer here as much as a printed line is, and the two are told
/// apart by the status alone.
fn both_shells_agree(scripts: &[&str], prelude: &str) {
    let bash = pinned_bash::path();
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    for script in scripts {
        let source = format!("{prelude}{script}");
        let (reference, reference_status) = pinned_bash::answer(&bash, &[], &source);
        let (answer, status) = pinned_bash::answer(nsh, &["-o", "bash"], &source);
        assert_eq!(
            (String::from_utf8_lossy(&answer).into_owned(), status),
            (
                String::from_utf8_lossy(&reference).into_owned(),
                reference_status
            ),
            "the two shells disagree about\n{source}"
        );
    }
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn a_conditional_accepts_the_newlines_bash_accepts() {
    both_shells_agree(PARSED, "");
}

// [spec:nsh:req:compat.bash.traps-introspection/test]
#[test]
fn a_conditional_records_the_line_bash_records() {
    both_shells_agree(RECORDED, PRELUDE);
}
