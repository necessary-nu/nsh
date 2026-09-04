//! The attributes the shell's own published facts carry, measured
//! against the pinned Bash 5.3.
//!
//! `variables::special::publish` entered the identities, the counters
//! and the version array as ordinary values, so `UID`, `EUID` and `PPID`
//! were writable here and read-only there, `BASH_VERSINFO` was an
//! unprotected array, and `OPTIND`, `RANDOM`, `SRANDOM` and `BASHPID`
//! were plain scalars where the reference makes them integers.
//!
//! It is not a listing question. A read-only `UID` is what makes `UID=0`
//! fail, which a script checking `[ "$UID" = 0 ]` after something tried
//! to set it is relying on; an integer `OPTIND` is what makes
//! `OPTIND=abc` zero rather than `abc`, and `OPTIND+=1` arithmetic
//! rather than concatenation.
//!
//! THE NAMES CAME FROM A DIFF, not from a list. The node named six from
//! what a read-only listing shows; a `declare -p` diff of the two
//! shells' whole start-up sets found eight, because `BASHPID`,
//! `OPTIND`, `RANDOM` and `SRANDOM` carry `-i` and appear in no
//! read-only listing at all. [`published_names`] is that diff as a test:
//! the reference's whole published set, less the four names of
//! [`A_SURFACE_THIS_SHELL_HAS_NOT_GOT`], is this shell's. Nothing here
//! enumerates the shared names, so a name that appears on either side
//! without being decided is a failure rather than an omission.
//!
//! A VALUE IS NOT COMPARED HERE and a name is. Two shells started side
//! by side disagree about `PPID`, `BASHPID`, `RANDOM`, `SECONDS` and
//! `PWD` by construction, so the rows that walk the whole set cut the
//! value off and compare the letters and the name. Diagnostic wording is
//! a registered difference, so only stdout and the exit status are read.
//!
//! Nothing here is a recorded expectation: every case runs in both
//! shells and the two answers are compared.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A fact the reference makes read-only refuses a write, and the
/// refusal is what the attribute is for.
const A_READ_ONLY_FACT_REFUSES_A_WRITE: &[&str] = &[
    "UID=0\necho unreached\n",
    "EUID=0\necho unreached\n",
    "PPID=0\necho unreached\n",
    "BASH_VERSINFO[0]=9\necho unreached\n",
    "UID=0\necho status=$?\n",
    "unset UID\necho \"status=$? UID=${UID:+set}\"\n",
    "unset EUID\necho \"status=$? EUID=${EUID:+set}\"\n",
    "unset PPID\necho \"status=$? PPID=${PPID:+set}\"\n",
    "unset BASH_VERSINFO\necho \"status=$? n=${#BASH_VERSINFO[@]}\"\n",
    "readonly -p | grep -c ' UID='\n",
    "readonly -p | grep -c ' EUID='\n",
    "readonly -p | grep -c ' PPID='\n",
    "readonly -p | grep -c ' BASH_VERSINFO='\n",
    "readonly -p | grep -c ' BASHOPTS='\n",
    "readonly -p | grep -c ' SHELLOPTS='\n",
    /* The ones the reference leaves writable stay writable, and the
     * assignment is accepted even where the value is recomputed. */
    "BASHPID=5\necho status=$?\n",
    "SRANDOM=5\necho status=$?\n",
    "RANDOM=5\necho status=$?\n",
    "SECONDS=5\necho status=$?\n",
    "readonly -p | grep -c ' BASHPID='\n",
    "readonly -p | grep -c ' RANDOM='\n",
    "readonly -p | grep -c ' OPTIND='\n",
];

/// A fact the reference makes an integer reshapes what is stored in it.
const AN_INTEGER_FACT_IS_ARITHMETIC: &[&str] = &[
    "OPTIND=abc\necho \"status=$? OPTIND=$OPTIND\"\n",
    "OPTIND=3\nOPTIND+=1\necho \"OPTIND=$OPTIND\"\n",
    "OPTIND=2+3\necho \"OPTIND=$OPTIND\"\n",
    "RANDOM=abc\necho status=$?\n",
    "SRANDOM=abc\necho status=$?\n",
    "echo \"${UID@a}\"\n",
    "echo \"${EUID@a}\"\n",
    "echo \"${PPID@a}\"\n",
    "echo \"${OPTIND@a}\"\n",
    "echo \"${RANDOM@a}\"\n",
    "echo \"${SRANDOM@a}\"\n",
    "echo \"${BASHPID@a}\"\n",
    "echo \"${BASH_VERSINFO@a}\"\n",
    "echo \"${BASHOPTS@a}\" \"${SHELLOPTS@a}\"\n",
    "echo \"${EPOCHSECONDS@a}\" \"${BASH_SUBSHELL@a}\" \"${EPOCHREALTIME@a}\"\n",
    "echo \"${LINENO@a}\" \"${PWD@a}\" \"${IFS@a}\"\n",
    /* `getopts` still walks its operands with the integer `OPTIND`. */
    "f(){ while getopts \"ab\" o; do echo \"opt=$o\"; done; echo \"ind=$OPTIND\"; }\nf -a -b\n",
    "f(){ while getopts \"a:\" o; do echo \"opt=$o arg=$OPTARG\"; done; echo \"ind=$OPTIND\"; }\nf -a v\n",
    /* And `$(( ))` reads the identities as the numbers they are. */
    "echo $(( UID - UID ))\n",
    "echo $(( EUID - EUID ))\n",
];

/// The two names that describe the session rather than the shell.
///
/// Measured on the pinned 5.3.15. With nothing supplied the reference
/// answers `declare -- TERM="dumb"` and `declare -- SHELL="/bin/bash"`,
/// neither exported; with either inherited it keeps the value and the
/// `-x` the import gave it, and an inherited empty string counts as an
/// answer rather than as nothing.
///
/// `SHELL` IS THE PASSWORD ENTRY'S LOGIN SHELL, which is why the rows can
/// compare the value and not just the name: both shells read the same
/// entry, so both say `/bin/bash` on a host whose `getent passwd` does.
/// It is not `$0` and not the binary's path -- a copy of the pinned Bash
/// run from another directory still answers `/bin/bash`, and so does the
/// same binary run through `exec -a totallyother`.
const THE_SESSION_HAS_TWO_NAMES: &[&str] = &[
    "declare -p TERM SHELL\n",
    "echo \"[$TERM][$SHELL]\"\n",
    "echo \"[${TERM+s}][${SHELL+s}]\"\n",
    "declare -p | grep -cE ' (TERM|SHELL)='\n",
    "[ -x \"$SHELL\" ] && echo runnable || echo not-runnable\n",
    "case $TERM in '') echo empty;; *) echo \"named=$TERM\";; esac\n",
    /* Whether either name reaches a child is the export mark, and the
     * mark is the import's rather than the default's. */
    "env | grep -cE '^(TERM|SHELL)='\n",
    "export TERM\nenv | grep -c '^TERM='\n",
];

/// The eight names that are state the shell already kept.
///
/// Measured on the pinned 5.3.15, fed on standard input. Seven are in a
/// start-up listing and one is not: `BASH_EXECUTION_STRING` exists only
/// under `-c`, which is why the rows that ask about it run the shell
/// again rather than asking this one.
///
/// Six of the seven are *invisible* -- `declare -x OLDPWD`,
/// `declare -i HISTCMD`, `declare -- BASH_COMMAND`, `BASH_ARGV0` and
/// `BASH_MONOSECONDS` with no value, `OPTERR` alone carrying `"1"` --
/// and each answers a named lookup from whatever holds the state. That
/// is the whole point of the set: publishing any of them as an empty
/// name would make the listing agree while answering a script wrongly.
///
/// Three values are asked about rather than read, because neither shell's
/// answer is a constant. `BASH_ARGV0` and `_` at rest are `$0`, which is
/// a different string in the two shells, so the rows compare them *to*
/// `$0`.
/// `BASH_MONOSECONDS` is a clock, so the rows ask what a clock has to be
/// -- digits, and never smaller on a later read -- rather than what it
/// says. Its origin is not the reference's and `docs/divergences.md`
/// records that.
const STATE_THE_SHELL_ALREADY_KEEPS: &[&str] = &[
    "declare -p | grep -E ' (OLDPWD|OPTERR|HISTCMD|BASH_COMMAND|BASH_ARGV0|BASH_MONOSECONDS)' | sed -E 's/=.*//' | sort\n",
    "for n in OLDPWD OPTERR HISTCMD BASH_COMMAND BASH_ARGV0 BASH_MONOSECONDS; do declare -p $n | sed -E 's/=.*//'; done\n",
    "echo \"[${OLDPWD+s}][${OPTERR+s}][${HISTCMD+s}][${BASH_COMMAND+s}][${BASH_ARGV0+s}]\"\n",
    "for n in OLDPWD OPTERR HISTCMD BASH_COMMAND BASH_ARGV0 _; do test -v $n; echo \"$n=$?\"; done\n",
    "echo \"[$OPTERR][$HISTCMD][${OLDPWD-unset}]\"\n",
    "echo $(( HISTCMD + 0 )) $(( OPTERR + 1 ))\n",
    "[ \"$BASH_ARGV0\" = \"$0\" ] && echo argv0-is-zero || echo differs\n",
    "[ \"$_\" = \"$0\" ] && echo underscore-starts-at-zero || echo differs\n",
    "case $BASH_MONOSECONDS in ''|*[!0-9]*) echo not-a-number;; *) echo digits;; esac\n",
    "a=$BASH_MONOSECONDS\nb=$BASH_MONOSECONDS\n[ \"$b\" -ge \"$a\" ] && echo monotonic || echo went-back\n",
    /* `$_` is the last word of the command before, and the command word
     * itself when nothing follows it. A command that is only an
     * assignment leaves it empty rather than leaving the last one. */
    "echo hi\necho \"1=[$_]\"\ntrue a b c\necho \"2=[$_]\"\nx=5\necho \"3=[$_]\"\n:\necho \"4=[$_]\"\n",
    "f(){ echo inner arg; echo \"in=[$_]\"; }\nf\necho \"out=[$_]\"\n",
    "echo\necho \"bare=[$_]\"\n",
    /* `$BASH_COMMAND` is the command running, and a trap action's own
     * commands do not move it. */
    "trap 'echo cmd=[$BASH_COMMAND]' DEBUG\necho one\n:\nx=1\n",
    "trap 'echo cmd=[$BASH_COMMAND]' DEBUG\necho   one    two\necho \"a  b\"\necho \";\"\n",
    "echo \"[$BASH_COMMAND]\"\nf(){ echo \"[$BASH_COMMAND]\"; }\nf q\n",
    /* `OLDPWD` is where `cd -` goes back to, and it is empty until a
     * `cd` has moved the shell. */
    "cd /usr\ncd /tmp\necho \"[$OLDPWD]\"\ndeclare -p OLDPWD\n",
    "cd /usr\ncd -\npwd\n",
    /* `OPTERR=0` silences `getopts` and changes nothing else. The
     * wording of the diagnostic is a registered difference, so the rows
     * count lines rather than read them. */
    "f(){ while getopts 'a' o; do :; done; echo \"o=[$o]\"; }\nOPTERR=0\nf -z 2>&1 | wc -l\nOPTERR=1\nf -z 2>&1 | wc -l\n",
    "f(){ while getopts 'a:' o; do :; done; }\nOPTERR=0\nf -a 2>&1 | wc -l\n",
    /* `BASH_ARGV0` is `$0`, and assigning it re-points `$0`. */
    "BASH_ARGV0=zed\necho \"0=[$0] argv0=[$BASH_ARGV0]\"\ndeclare -p BASH_ARGV0\n",
    /* A shell that was given no `-c` string has no name for one. */
    "declare -p BASH_EXECUTION_STRING\necho \"status=$?\"\n",
    "echo \"[${BASH_EXECUTION_STRING-unset}]\"\n",
];

/// The same names asked of a shell started with `-c`, where the
/// reference has one more of them.
const A_COMMAND_STRING_IS_A_NAME: &[&str] = &[
    "declare -p BASH_EXECUTION_STRING",
    "echo \"[$BASH_EXECUTION_STRING]\"",
    "echo \"[${BASH_EXECUTION_STRING+set}]\"; declare -p BASH_EXECUTION_STRING | sed -E 's/=.*//'",
];

/// The five names that describe the call in progress, at rest and in one.
///
/// Measured on the pinned 5.3.15, fed on standard input. At rest the
/// reference has all five and this shell had none of them: `FUNCNAME`
/// *declared* with no value, `BASH_SOURCE`, `BASH_LINENO` and
/// `BASH_ARGV` assigned empty, and `BASH_ARGC` answering `([0]="0")` to
/// a named lookup while a whole listing still shows `()`.
///
/// That asymmetry is not a rendering difference. `BASH_ARGC` and
/// `BASH_ARGV` are *pushed* by the first read taken with nothing on the
/// call stack, from the shell's own positional parameters, and then
/// stand: `set -- x y z` after a read leaves them spelling what the
/// shell started with, and a read taken inside a function pushes
/// nothing, which is why both answer `()` there.
///
/// `BASH_SOURCE`'s value is the one thing not compared directly. Its
/// entry for a function defined on standard input is `$0`, which is a
/// different string in the two shells by construction, so the rows ask
/// whether it equals `$0` rather than what it is.
const THE_CALL_IN_PROGRESS_IS_FIVE_NAMES: &[&str] = &[
    "for n in FUNCNAME BASH_SOURCE BASH_LINENO BASH_ARGC BASH_ARGV; do declare -p $n; done\n",
    "declare -p | grep -E '^declare -a (FUNCNAME|BASH_SOURCE|BASH_LINENO|BASH_ARGC|BASH_ARGV)'\n",
    "echo \"[${FUNCNAME+s}][${BASH_SOURCE+s}][${BASH_LINENO+s}][${BASH_ARGC+s}][${BASH_ARGV+s}]\"\n",
    "for n in FUNCNAME BASH_SOURCE BASH_LINENO BASH_ARGC BASH_ARGV; do test -v $n; echo \"$n=$?\"; done\n",
    "echo \"${#FUNCNAME[@]}/${#BASH_SOURCE[@]}/${#BASH_LINENO[@]}/${#BASH_ARGC[@]}/${#BASH_ARGV[@]}\"\n",
    /* An empty array has no element zero either way, so `set -u`
     * diagnoses every one of the four the shell has not filled and is
     * silent about the `[@]` form of all five. */
    "set -u\necho \"[${FUNCNAME[@]}][${BASH_SOURCE[@]}][${BASH_LINENO[@]}][${BASH_ARGV[@]}]\"\necho after\n",
    "set -u\necho \"[$BASH_ARGC]\"\necho after\n",
    "set -u\n( echo \"[$FUNCNAME]\" )\necho \"status=$?\"\n",
    "set -u\n( echo \"[$BASH_SOURCE]\" )\necho \"status=$?\"\n",
    /* The push happens once, from wherever the parameters stood when it
     * happened, and nothing afterwards moves it. */
    "set -- x y z\necho \"[${BASH_ARGC[@]}][${BASH_ARGV[@]}]\"\n",
    "echo \"[$BASH_ARGC]\"\nset -- x y z\necho \"[${BASH_ARGC[@]}][${BASH_ARGV[@]}]\"\n",
    "declare -p | grep -c 'BASH_ARGC=()'\necho \"[$BASH_ARGC]\"\ndeclare -p | grep 'BASH_ARGC'\n",
    "f(){ echo \"in=[$BASH_ARGC][${BASH_ARGV[@]}]\"; }\nf q\necho \"out=[$BASH_ARGC]\"\n",
    /* A call in progress, and the same five names once it is over. */
    "f(){ declare -p FUNCNAME BASH_LINENO BASH_ARGC BASH_ARGV; }\nf a b\n",
    "f(){ echo \"${#BASH_SOURCE[@]}/${#FUNCNAME[@]}/${#BASH_LINENO[@]}\"; }\nf\n",
    "f(){ [ \"${BASH_SOURCE[0]}\" = \"$0\" ] && echo same || echo differ; }\nf\n",
    "f(){ :; }\nf\nfor n in FUNCNAME BASH_SOURCE BASH_LINENO; do declare -p $n; done\n",
    "g(){ declare -p FUNCNAME BASH_LINENO; }\nf(){ g; }\nf\n",
    /* A dot script reaches `BASH_SOURCE` and leaves `FUNCNAME` declared.
     * The file goes under `mktemp -d` because the suite runs sandboxed
     * with only `target/` writable, and a redirection the shell cannot
     * perform makes a row agree for the wrong reason. The source name is
     * compared without its directory for the same reason `$0` is: the
     * two shells are handed different temporary directories. */
    "d=$(mktemp -d)\nprintf 'declare -p FUNCNAME BASH_LINENO\\n' > \"$d/lib.sh\"\n. \"$d/lib.sh\"\nrm -rf \"$d\"\n",
    "d=$(mktemp -d)\nprintf 'echo \"[${BASH_SOURCE[0]##*/}][${#BASH_SOURCE[@]}]\"\\n' > \"$d/lib.sh\"\n. \"$d/lib.sh\"\nrm -rf \"$d\"\n",
];

/// A prompt is a name for a shell somebody is watching.
///
/// Measured on the pinned 5.3.15: fed on standard input the reference
/// answers `declare: PS1: not found` for both names, `${PS1+set}` is
/// empty, and `PS4="+ "` is the only `PS` name in its listing. An
/// *interactive* reference has both -- `PS1="\s-\v\$ "` and
/// `PS2="> "` under `--norc` -- so the condition is the invocation and
/// not the dialect, and no row here can ask about it: a script on a pipe
/// is never interactive on either side.
///
/// The rows with `PS1` in the environment are the ones that make the
/// point. The reference does not merely decline to *default* the name,
/// it takes an inherited one away and does not pass it on, so
/// `${PS1-unset}` reads `unset` and `env` carries no `PS1` into a child.
const A_PROMPT_IS_ONLY_FOR_A_WATCHED_SHELL: &[&str] = &[
    "declare -p PS1\necho status=$?\n",
    "declare -p PS2\necho status=$?\n",
    "echo \"[${PS1-unset}][${PS2-unset}]\"\n",
    "echo \"[${PS1+set}][${PS2+set}][${PS4+set}]\"\n",
    "declare -p | grep -c ' PS1='\n",
    "declare -p | grep -c ' PS2='\n",
    "declare -p | grep -c ' PS4='\n",
    "set | grep -c '^PS1='\n",
    "env | grep -c '^PS1='\n",
    "echo \"${#PS1}/${#PS2}\"\n",
    /* A script that wants the name has it: withholding is not a
     * refusal, and the assignment lands as an ordinary variable. */
    "PS1=mine\ndeclare -p PS1\n",
    "PS2=mine\ndeclare -p PS2\n",
    "unset PS1\necho status=$?\n",
    "PS1=mine\nunset PS1\necho \"status=$? [${PS1-unset}]\"\n",
];

/// The names of a surface this shell has not got, which the reference
/// publishes and this one does not.
///
/// `[spec:nsh:req:compat.bash.names.only-what-the-reference-has]` allows
/// such a name two endings -- absent and recorded as a sanctioned
/// divergence, or genuinely wired to the facility -- and forbids a value
/// that describes nothing. All four take the first;
/// `docs/divergences.md` carries the argument for each and
/// `bash.divergences.publish-names.table-views` holds the wiring for the
/// two that could still have it.
///
/// This list is the *whole* of the difference between the two published
/// sets, which is what makes [`the_published_set_is_the_references_less_four`]
/// a claim rather than an enumeration: a name that appears on either side
/// without being decided fails that test.
const A_SURFACE_THIS_SHELL_HAS_NOT_GOT: &[&str] = &[
    "BASH_ALIASES",
    "BASH_CMDS",
    "BASH_LOADABLES_PATH",
    "COMP_WORDBREAKS",
];

/// `SECONDS` is asked about nowhere here, and it is out because it is
/// measured rather than because it was missed: in the reference it
/// carries no letter until something *reads* it, and `-i` afterwards --
/// a fresh `declare -p` lists `declare -- SECONDS` and
/// `: $SECONDS; declare -p` lists `declare -i SECONDS`. A row asking for
/// its letters would read it and so could not see the difference at all;
/// `mark-seconds-when-it-is-read` holds that half.
const MEASURED_BY_READING_IT: &str = "SECONDS";

/// The names in one shell's start-up `declare -p`, in the order it wrote
/// them.
///
/// Only the name is taken. Two shells started side by side disagree about
/// `PPID`, `BASHPID`, `RANDOM`, `SECONDS` and `PWD` by construction, so a
/// whole-set comparison that kept the values could only ever fail.
fn published_names(listing: &[u8]) -> Vec<String> {
    String::from_utf8_lossy(listing)
        .lines()
        .filter_map(|line| line.strip_prefix("declare -"))
        .filter_map(|rest| rest.split_once(' '))
        .map(|(_letters, named)| {
            named
                .split(['=', '[', ' '])
                .next()
                .unwrap_or_default()
                .to_owned()
        })
        .filter(|name| {
            !name.is_empty()
                && name
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'_')
        })
        .collect()
}

/// Both shells on one script, as `(what nsh said, what the pinned Bash
/// said)`, with `environment` inherited by each.
fn both(environment: &[(&str, &str)], script: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer_with_env(nsh, &["-o", "bash"], environment, script),
        pinned_bash::answer_with_env(&bash, &[], environment, script),
    )
}

/// Every script in `cases` produces the reference's bytes and status,
/// started with `environment` and nothing else.
fn agrees(environment: &[(&str, &str)], cases: &[&str]) {
    for script in cases {
        let (ours, theirs) = both(environment, script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "output differed for\n{script}"
        );
        assert_eq!(ours.1, theirs.1, "status differed for\n{script}");
    }
}

/// A write to a fact the reference protects is refused here too.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_read_only_fact_refuses_a_write() {
    agrees(&[], A_READ_ONLY_FACT_REFUSES_A_WRITE);
}

/// A fact the reference publishes as an integer behaves as one.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn an_integer_fact_is_arithmetic() {
    agrees(&[], AN_INTEGER_FACT_IS_ARITHMETIC);
}

/// Both shells running one `-c` string, which is the one invocation
/// shape `both` cannot put a script to: `answer` writes to standard
/// input, and a `-c` shell never reads it.
fn both_as_command(script: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer(nsh, &["-o", "bash", "-c", script], ""),
        pinned_bash::answer(&bash, &["-c", script], ""),
    )
}

/// `TERM` and `SHELL` carry the reference's defaults, and an inherited
/// value of either is left exactly as it arrived.
// [spec:nsh:req:compat.bash.names.environment-facts/test]
#[test]
fn the_session_has_two_names() {
    agrees(&[], THE_SESSION_HAS_TWO_NAMES);
    agrees(
        &[("TERM", "xterm-256color"), ("SHELL", "/bin/zsh")],
        THE_SESSION_HAS_TWO_NAMES,
    );
    agrees(&[("TERM", ""), ("SHELL", "")], THE_SESSION_HAS_TWO_NAMES);
}

/// Every name that is state this shell already kept answers from it.
// [spec:nsh:req:compat.bash.names.ordinary-state/test]
#[test]
fn state_the_shell_already_keeps() {
    agrees(&[], STATE_THE_SHELL_ALREADY_KEEPS);
    for script in A_COMMAND_STRING_IS_A_NAME {
        let (ours, theirs) = both_as_command(script);
        assert_eq!(
            String::from_utf8_lossy(&ours.0),
            String::from_utf8_lossy(&theirs.0),
            "output differed for -c {script}"
        );
        assert_eq!(ours.1, theirs.1, "status differed for -c {script}");
    }
}

/// All five call-stack names exist wherever the reference has them.
// [spec:nsh:req:compat.bash.names.call-stack/test]
#[test]
fn the_call_in_progress_is_five_names() {
    agrees(&[], THE_CALL_IN_PROGRESS_IS_FIVE_NAMES);
}

/// Neither prompt is on the table of a shell nobody is watching.
// [spec:nsh:req:compat.bash.names.only-what-the-reference-has/test]
#[test]
fn a_prompt_is_only_for_a_watched_shell() {
    agrees(&[], A_PROMPT_IS_ONLY_FOR_A_WATCHED_SHELL);
    agrees(
        &[("PS1", "inherited$ "), ("PS2", "inherited> ")],
        A_PROMPT_IS_ONLY_FOR_A_WATCHED_SHELL,
    );
}

/// The reference's published set, less the four names of a surface this
/// shell has not got, is this shell's published set.
///
/// An enumeration of shared names can only say "these agree". This says
/// "and nothing else is there", which is the half that catches a name
/// published by accident, and it is the whole reason the exclusion list
/// has to stay exactly four long.
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn the_published_set_is_the_references_less_four() {
    let (ours, theirs) = both(&[], "declare -p\n");
    let mine = published_names(&ours.0);
    let expected: Vec<String> = published_names(&theirs.0)
        .into_iter()
        .filter(|name| !A_SURFACE_THIS_SHELL_HAS_NOT_GOT.contains(&name.as_str()))
        .collect();

    /* A listing neither shell could produce would make every assertion
     * below vacuously true, and the failure would read as a pass. */
    assert!(
        expected.len() > 40,
        "the reference published {} names, which is not a start-up listing",
        expected.len()
    );
    assert_eq!(mine, expected);
}

/// Every name both shells publish carries the reference's letters.
///
/// The set is taken from the run rather than written down, so a name that
/// appears later is asked about without anyone remembering to add it.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_shared_names_carry_the_same_letters() {
    let (_, theirs) = both(&[], "declare -p\n");
    let cases: Vec<String> = published_names(&theirs.0)
        .into_iter()
        .filter(|name| {
            !A_SURFACE_THIS_SHELL_HAS_NOT_GOT.contains(&name.as_str())
                && name != MEASURED_BY_READING_IT
        })
        .map(|name| format!("declare -p {name} | sed -E 's/=.*//'\n"))
        .collect();
    assert!(
        cases.len() > 40,
        "only {} shared names to ask about",
        cases.len()
    );
    let borrowed: Vec<&str> = cases.iter().map(String::as_str).collect();
    agrees(&[], &borrowed);
}
