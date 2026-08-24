//! The strongest oracle available: does this shell agree with GNU Bash?
//!
//! Rung 3 of `PLAN.md`'s ladder, and the largest hole in the shell's
//! testing. The 119 corpora under `tests/corpus/` are differential
//! against *dash* and cover POSIX mode only; everything the Bash dialect
//! adds -- arrays, `[[ ]]`, process substitution, `select`, `time`, the
//! sparse descriptor table -- has had no differential oracle at all.
//! `[dec:nsh:differential-is-the-oracle]` already says this is the
//! authority; this is the first thing that applies it to Bash mode.
//!
//! # Why a generator rather than bytes
//!
//! Random bytes reach the lexer well and the evaluator almost never:
//! nearly every mutation is a syntax error, and a syntax error tells you
//! nothing here because both shells reject it. So the fuzzer's bytes
//! drive a *generator* through `Arbitrary`, and what reaches the shells is
//! a syntactically valid script by construction. The mutator is then
//! exploring the space of programs rather than the space of typos.
//!
//! # Why the grammar is deliberately narrow
//!
//! Every construct here is *deterministic*. No `$$`, no `$RANDOM`, no
//! clock, no filesystem, no process ids, no `$PWD`, no external command
//! whose output could vary between two runs a millisecond apart. A
//! differential oracle is worthless if the two sides are allowed to
//! disagree for reasons that are not bugs, and the cheapest way to
//! guarantee that is to never generate the constructs that can.
//!
//! Only stdout and exit status are compared. Diagnostics are deliberately
//! *not*: the two shells word their errors differently on purpose, and
//! `[spec:nsh:req:compat.smoosh.error-contracts]` fixes this shell's
//! spelling, so comparing stderr would report a disagreement the project
//! has already decided.

#![no_main]

use libfuzzer_sys::arbitrary::{self, Arbitrary, Unstructured};
use libfuzzer_sys::fuzz_target;
use nsh::{Shell, Streams};
use std::io::Write as _;
use std::process::{Command, Stdio};

/// A word made only of bytes that mean the same thing to both shells.
#[derive(Arbitrary, Debug)]
enum Atom {
    Plain,
    Digits,
    Empty,
    Space,
    Star,
    Bracket,
    Question,
    Backslash,
    Tilde,
    Hash,
    Percent,
}

impl Atom {
    fn text(&self) -> &'static str {
        match self {
            Self::Plain => "abc",
            Self::Digits => "123",
            Self::Empty => "",
            Self::Space => "a b",
            Self::Star => "a*c",
            Self::Bracket => "[ab]",
            Self::Question => "a?c",
            Self::Backslash => "a\\\\b",
            Self::Tilde => "a~b",
            Self::Hash => "a#b",
            Self::Percent => "a%b",
        }
    }
}

/// One expansion of the variable `v`, which the preamble always sets.
#[derive(Arbitrary, Debug)]
enum Expansion {
    Plain,
    Quoted,
    Length,
    DefaultUnset(Atom),
    DefaultNull(Atom),
    Alternative(Atom),
    TrimShortestPrefix(Atom),
    TrimLongestPrefix(Atom),
    TrimShortestSuffix(Atom),
    TrimLongestSuffix(Atom),
    ReplaceFirst(Atom, Atom),
    ReplaceAll(Atom, Atom),
    UpperFirst,
    UpperAll,
    LowerAll,
    Quote,
    Substring(u8, u8),
    Positional,
    PositionalStar,
    PositionalQuoted,
    Count,
}

impl Expansion {
    fn text(&self) -> String {
        match self {
            Self::Plain => "$v".into(),
            Self::Quoted => "\"$v\"".into(),
            Self::Length => "${#v}".into(),
            Self::DefaultUnset(a) => format!("${{u-{}}}", a.text()),
            Self::DefaultNull(a) => format!("${{v:-{}}}", a.text()),
            Self::Alternative(a) => format!("${{v:+{}}}", a.text()),
            Self::TrimShortestPrefix(a) => format!("${{v#{}}}", a.text()),
            Self::TrimLongestPrefix(a) => format!("${{v##{}}}", a.text()),
            Self::TrimShortestSuffix(a) => format!("${{v%{}}}", a.text()),
            Self::TrimLongestSuffix(a) => format!("${{v%%{}}}", a.text()),
            Self::ReplaceFirst(a, b) => format!("${{v/{}/{}}}", a.text(), b.text()),
            Self::ReplaceAll(a, b) => format!("${{v//{}/{}}}", a.text(), b.text()),
            Self::UpperFirst => "${v^}".into(),
            Self::UpperAll => "${v^^}".into(),
            Self::LowerAll => "${v,,}".into(),
            Self::Quote => "${v@Q}".into(),
            Self::Substring(o, l) => format!("${{v:{}:{}}}", o % 8, l % 8),
            Self::Positional => "$@".into(),
            Self::PositionalStar => "$*".into(),
            Self::PositionalQuoted => "\"$@\"".into(),
            Self::Count => "$#".into(),
        }
    }
}

/// A statement. Every arm terminates and none of them touch the world.
#[derive(Arbitrary, Debug)]
enum Stmt {
    Echo(Expansion),
    PrintfRow(Expansion),
    Assign(Atom),
    SetPositional(Atom, Atom),
    SetIfs(Atom),
    Arithmetic(u8, u8, ArithOp),
    IfTest(Expansion, Box<Stmt>),
    ForWords(Box<Stmt>),
    CaseMatch(Atom, Box<Stmt>),
    Subshell(Box<Stmt>),
    Sequence(Box<Stmt>, Box<Stmt>),
    AndOr(Atom),
    Negate(Box<Stmt>),
    DoubleBracket(Expansion, Atom),
    ArrayRound(Atom, Atom),
    Status,
}

#[derive(Arbitrary, Debug)]
enum ArithOp {
    Add,
    Sub,
    Mul,
    Div,
    Mod,
    Shl,
    Shr,
    Less,
    Equal,
}

impl ArithOp {
    fn text(&self) -> &'static str {
        match self {
            Self::Add => "+",
            Self::Sub => "-",
            Self::Mul => "*",
            Self::Div => "/",
            Self::Mod => "%",
            Self::Shl => "<<",
            Self::Shr => ">>",
            Self::Less => "<",
            Self::Equal => "==",
        }
    }
}

impl Stmt {
    fn render(&self, out: &mut String, depth: u32) {
        /* The generator bounds its own nesting: the parser refuses past
         * `MAX_COMMAND_DEPTH` and a refusal on one side only is not a
         * disagreement worth reporting from this target. */
        if depth > 6 {
            out.push_str("echo deep\n");
            return;
        }
        match self {
            Self::Echo(e) => out.push_str(&format!("echo {}\n", e.text())),
            Self::PrintfRow(e) => out.push_str(&format!("printf '[%s]\\n' {}\n", e.text())),
            Self::Assign(a) => out.push_str(&format!("v='{}'\n", a.text())),
            Self::SetPositional(a, b) => {
                out.push_str(&format!("set -- '{}' '{}'\n", a.text(), b.text()));
            }
            Self::SetIfs(a) => out.push_str(&format!("IFS='{}'\n", a.text())),
            Self::Arithmetic(l, r, op) => {
                /* A zero divisor is a diagnostic, not a value, and the two
                 * shells word it differently; force it non-zero. */
                let right = match op {
                    ArithOp::Div | ArithOp::Mod => (r % 9) + 1,
                    _ => r % 9,
                };
                out.push_str(&format!("echo $(( {} {} {} ))\n", l % 9, op.text(), right));
            }
            Self::IfTest(e, body) => {
                out.push_str(&format!("if [ -n \"{}\" ]; then\n", e.text()));
                body.render(out, depth + 1);
                out.push_str("fi\n");
            }
            Self::ForWords(body) => {
                out.push_str("for w in one two; do\n");
                body.render(out, depth + 1);
                out.push_str("done\n");
            }
            Self::CaseMatch(a, body) => {
                out.push_str(&format!("case \"$v\" in {})\n", a.text()));
                body.render(out, depth + 1);
                out.push_str(";; *) echo nomatch ;; esac\n");
            }
            Self::Subshell(body) => {
                out.push_str("(\n");
                body.render(out, depth + 1);
                out.push_str(")\n");
            }
            Self::Sequence(a, b) => {
                a.render(out, depth + 1);
                b.render(out, depth + 1);
            }
            Self::AndOr(a) => {
                out.push_str(&format!(
                    "[ -n '{}' ] && echo yes || echo no\n",
                    a.text(),
                ));
            }
            Self::Negate(body) => {
                out.push_str("! ");
                body.render(out, depth + 1);
            }
            Self::DoubleBracket(e, a) => {
                out.push_str(&format!(
                    "if [[ {} == {} ]]; then echo yes; else echo no; fi\n",
                    e.text(),
                    a.text(),
                ));
            }
            Self::ArrayRound(a, b) => {
                out.push_str(&format!(
                    "arr=('{}' '{}')\nprintf '<%s>' \"${{arr[@]}}\"; echo\necho ${{#arr[@]}}\n",
                    a.text(),
                    b.text(),
                ));
            }
            Self::Status => out.push_str("echo $?\n"),
        }
    }
}

#[derive(Arbitrary, Debug)]
struct Script(Vec<Stmt>);

impl Script {
    fn render(&self) -> String {
        let mut out = String::from("v=abc\nunset u\nset -- x y\n");
        for statement in self.0.iter().take(12) {
            statement.render(&mut out, 0);
        }
        out
    }
}

/// What this shell does with the script, in process.
/// `refused` is an `Err` from `run` -- the shell rejecting the script
/// rather than answering with a status. Kept separate on purpose: an
/// early version folded it into status 1 and then reported the *target's*
/// own substitution as a disagreement with Bash.
struct Outcome {
    stdout: Vec<u8>,
    status: i32,
    refused: bool,
}

fn under_nsh(script: &str) -> Option<Outcome> {
    let streams = Streams::capture().ok()?;
    let mut shell = Shell::builder()
        .streams(streams)
        .option(bstr::BStr::new(b"bash"), true)
        .build()
        .ok()?;
    let outcome = shell.run(script.as_bytes());
    let refused = outcome.is_err();
    let status = outcome.map_or(0, |s| s.code().into());
    let stdout = shell.take_captured_stdout().ok()?.to_vec();
    drop(shell.take_captured_stderr());
    Some(Outcome {
        stdout,
        status,
        refused,
    })
}

/// What GNU Bash does with it.
fn under_bash(script: &str) -> Option<(Vec<u8>, i32)> {
    let mut child = Command::new("bash")
        .arg("-s")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;
    child.stdin.take()?.write_all(script.as_bytes()).ok()?;
    let out = child.wait_with_output().ok()?;
    Some((out.stdout, out.status.code().unwrap_or(-1)))
}

fuzz_target!(|data: &[u8]| {
    let mut unstructured = Unstructured::new(data);
    let Ok(generated) = Script::arbitrary(&mut unstructured) else {
        return;
    };
    if generated.0.is_empty() {
        return;
    }
    let script = generated.render();

    let (Some(ours), Some((theirs, their_status))) = (under_nsh(&script), under_bash(&script))
    else {
        return;
    };

    /* Both shells rejecting the script is agreement, not a finding: the
     * generator can still emit something neither accepts, and the two
     * word their refusals differently by design. A refusal on *one* side
     * is a finding, and the interesting direction is kept below. */
    if ours.refused && their_status != 0 {
        return;
    }
    assert!(
        !ours.refused,
        "nsh refused a script bash ran\n--- script ---\n{script}\
         --- bash (status {their_status}) ---\n{}",
        bstr::BStr::new(&theirs),
    );

    assert!(
        ours.stdout == theirs && ours.status == their_status,
        "disagreed with bash\n--- script ---\n{script}\
         --- nsh    (status {}) ---\n{}\n\
         --- bash   (status {their_status}) ---\n{}",
        ours.status,
        bstr::BStr::new(&ours.stdout),
        bstr::BStr::new(&theirs),
    );
});
