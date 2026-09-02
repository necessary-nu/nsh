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

/// Every name both shells publish, through `declare -p`, with the value
/// cut off.
///
/// This is the diff the node asked to start from, kept as a check: the
/// letters and the name have to agree for all of them, and the two
/// shells' start-up sets are otherwise not comparable at all -- the
/// reference has `FUNCNAME`, `BASH_SOURCE`, `OLDPWD` and fifteen others
/// this shell does not publish, and this one has `PS1` and `PS2` that a
/// non-interactive reference does not.
///
/// `SECONDS` is the one shared name left out, and it is out because it
/// is measured rather than because it was missed: in the reference it
/// carries no letter until something *reads* it, and `-i` afterwards --
/// a fresh `declare -p` lists `declare -- SECONDS` and
/// `: $SECONDS; declare -p` lists `declare -i SECONDS`. Reproducing
/// that needs the read to set the attribute and needs `declare -p NAME`
/// to be a read, which it is not here;
/// `mark-seconds-when-it-is-read` holds both halves.
fn every_shared_name() -> Vec<String> {
    const SHARED: &[&str] = &[
        "BASH",
        "BASHOPTS",
        "BASHPID",
        "BASH_SUBSHELL",
        "BASH_VERSINFO",
        "BASH_VERSION",
        "DIRSTACK",
        "EPOCHREALTIME",
        "EPOCHSECONDS",
        "EUID",
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
/// said)`.
fn both(script: &str) -> ((Vec<u8>, i32), (Vec<u8>, i32)) {
    let nsh = Path::new(env!("CARGO_BIN_EXE_nsh"));
    let bash = pinned_bash::path();
    (
        pinned_bash::answer(nsh, &["-o", "bash"], script),
        pinned_bash::answer(&bash, &[], script),
    )
}

/// Every script in `cases` produces the reference's bytes and status.
fn agrees(cases: &[&str]) {
    for script in cases {
        let (ours, theirs) = both(script);
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
    agrees(A_READ_ONLY_FACT_REFUSES_A_WRITE);
}

/// A fact the reference publishes as an integer behaves as one.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn an_integer_fact_is_arithmetic() {
    agrees(AN_INTEGER_FACT_IS_ARITHMETIC);
}

/// Every name both shells publish carries the reference's letters.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_shared_names_carry_the_same_letters() {
    let cases = every_shared_name();
    let borrowed: Vec<&str> = cases.iter().map(String::as_str).collect();
    agrees(&borrowed);
}
