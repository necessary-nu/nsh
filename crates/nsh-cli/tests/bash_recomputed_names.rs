//! What a name whose value is recomputed on read spells back, measured
//! against the pinned Bash 5.3.
//!
//! These names hold nothing until something asks for them. The reference
//! prints `declare -i BASHPID` in a fresh listing and
//! `declare -i BASHPID="1868669"` once `$BASHPID` has been read; a
//! seeded value is not merely early, because `declare -p` then spells it
//! back as though the shell's pid were zero and `RANDOM` were always the
//! same number.
//!
//! ASKING BY NAME IS A READ AND LISTING IS NOT, which is the asymmetry
//! every row here turns on: `declare -p SECONDS` and
//! `declare -p | grep SECONDS` disagree about the same name in the same
//! shell. So does declaring it -- `declare SECONDS` is
//! `declare -i SECONDS="0"` there.
//!
//! `SECONDS` is the one name whose *letters* a read changes: it carries
//! none until it is read and `-i` afterwards. `BASHPID`, `RANDOM`,
//! `SRANDOM` and `OPTIND` carry `-i` from the start, and
//! `EPOCHSECONDS`, `EPOCHREALTIME` and `BASH_SUBSHELL` never carry it.
//!
//! NO ROW COMPARES ONE OF THESE VALUES DIRECTLY. A pid, a clock and a
//! generator answer differently in the two shells by construction, so
//! the rows ask about the *shape* -- a letter, whether a line carries a
//! value at all, whether that value is still the published seed.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A fresh listing, in which none of these names holds anything.
const A_FRESH_LISTING: &[&str] = &[
    "declare -p | grep -c '^declare -- SECONDS$'\n",
    "declare -p | grep -c '^declare -i BASHPID$'\n",
    "declare -p | grep -c '^declare -i RANDOM$'\n",
    "declare -p | grep -c '^declare -i SRANDOM$'\n",
    "declare -p | grep -c '^declare -- EPOCHSECONDS$'\n",
    "declare -p | grep -c '^declare -- EPOCHREALTIME$'\n",
    "declare -p | grep -c '^declare -- BASH_SUBSHELL$'\n",
    /* `OPTIND` is not one of these: it is a fact with a value, and it
     * carries one in the same listing. */
    "declare -p | grep -c '^declare -i OPTIND=\"1\"$'\n",
    /* Holding nothing is not being unset: a read finds a value, and
     * `${!prefix*}` names it. */
    "echo \"[${!BASHP*}]\"\n",
    "echo \"[${!SEC*}]\"\n",
    "test -v SECONDS && echo yes || echo no\n",
    "test -v BASHPID && echo yes || echo no\n",
    "echo \"[${EPOCHSECONDS+set}]\"\n",
];

/// A read, after which the listing carries what the read made.
const A_READ_FILLS_IT_IN: &[&str] = &[
    ": $SECONDS\ndeclare -p | grep -c '^declare -i SECONDS='\n",
    "echo \"[${SECONDS@a}]\"\n",
    "echo \"[${SECONDS@a}]\"\ndeclare -p | grep -c '^declare -i SECONDS='\n",
    ": $SECONDS\necho \"[${SECONDS@a}]\"\n",
    "echo \"[${SECONDS-unset}]\"\n",
    /* The letters of every other one are the same before and after. */
    "echo \"[${BASHPID@a}]\"\n",
    "echo \"[${RANDOM@a}]\"\n",
    "echo \"[${EPOCHSECONDS@a}]\"\n",
    "echo \"[${BASH_SUBSHELL@a}]\"\n",
    /* An assignment and an `unset` still reach them. */
    "SECONDS=100\necho \"$SECONDS\"\ndeclare -p SECONDS\n",
    "unset SECONDS\ndeclare -p SECONDS\necho st=$?\n",
    "unset RANDOM\nRANDOM=5\ndeclare -p RANDOM\n",
];

/// Asking by name, which is a read where the listing is not.
const ASKING_BY_NAME: &[&str] = &[
    "declare -p SECONDS\n",
    "declare -p BASH_SUBSHELL\n",
    "declare -p SECONDS\ndeclare -p SECONDS\n",
    /* None of these spells a published seed back, which is what makes
     * the line worth printing at all. */
    "declare -p BASHPID | grep -c '=\"0\"'\n",
    "declare -p RANDOM | grep -c '=\"0\"'\n",
    "declare -p SRANDOM | grep -c '=\"0\"'\n",
    "declare -p EPOCHSECONDS | grep -c '=\"0\"'\n",
    "declare -i BASHPID\ndeclare -p BASHPID | grep -c '=\"0\"'\n",
    /* Declaring one is a read of it too, whatever the letter. */
    "declare SECONDS\ndeclare -p SECONDS\n",
    "declare -i SECONDS\ndeclare -p SECONDS\n",
    "declare -x SECONDS\ndeclare -p SECONDS\n",
    "readonly SECONDS\ndeclare -p SECONDS\n",
    "export SECONDS\ndeclare -p SECONDS\n",
    "declare -i SECONDS\ndeclare -p | grep -c '^declare -i SECONDS='\n",
];

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

/// A recomputed name holds nothing until it is read, and is still not
/// unset.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_fresh_listing_carries_no_value() {
    agrees(A_FRESH_LISTING);
}

/// A read fills the name in, and gives `SECONDS` its letter.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_read_fills_the_name_in() {
    agrees(A_READ_FILLS_IT_IN);
}

/// Asking by name is a read, so the answer is the live value.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn asking_by_name_is_a_read() {
    agrees(ASKING_BY_NAME);
}
