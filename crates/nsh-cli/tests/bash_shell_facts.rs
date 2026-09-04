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
//! read-only listing at all. [`every_shared_name`] is that diff as a
//! test.
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

/// Every name both shells publish, through `declare -p`, with the value
/// cut off.
///
/// This is the diff the node asked to start from, kept as a check: the
/// letters and the name have to agree for all of them, and the two
/// shells' start-up sets are still not comparable whole -- the reference
/// has `OLDPWD`, `SHELL`, `TERM` and ten others this shell does not
/// publish, thirteen of the nineteen it started with. `PS1` and `PS2` are not shared either, and the reference is
/// the one without them:
/// [`A_PROMPT_IS_ONLY_FOR_A_WATCHED_SHELL`] is where those two are asked.
///
/// `SECONDS` is the one shared name left out, and it is out because it
/// is measured rather than because it was missed: in the reference it
/// carries no letter until something *reads* it, and `-i` afterwards --
/// a fresh `declare -p` lists `declare -- SECONDS` and
/// `: $SECONDS; declare -p` lists `declare -i SECONDS`. A row here would
/// read it and so could not see the difference at all;
/// `mark-seconds-when-it-is-read` holds that half.
fn every_shared_name() -> Vec<String> {
    const SHARED: &[&str] = &[
        "BASH",
        "BASHOPTS",
        "BASHPID",
        "BASH_ARGC",
        "BASH_ARGV",
        "BASH_LINENO",
        "BASH_SOURCE",
        "BASH_SUBSHELL",
        "BASH_VERSINFO",
        "BASH_VERSION",
        "DIRSTACK",
        "EPOCHREALTIME",
        "EPOCHSECONDS",
        "EUID",
        "FUNCNAME",
        "GROUPS",
        "HOSTNAME",
        "HOSTTYPE",
        "IFS",
        "LC_ALL",
        "LINENO",
        "MACHTYPE",
        "OPTIND",
        "OSTYPE",
        "PATH",
        "PPID",
        "PS4",
        "PWD",
        "RANDOM",
        "SHELLOPTS",
        "SHLVL",
        "SRANDOM",
        "UID",
    ];
    SHARED
        .iter()
        .map(|name| format!("declare -p {name} | sed -E 's/=.*//'\n"))
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

/// Every name both shells publish carries the reference's letters.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_shared_names_carry_the_same_letters() {
    let cases = every_shared_name();
    let borrowed: Vec<&str> = cases.iter().map(String::as_str).collect();
    agrees(&[], &borrowed);
}
