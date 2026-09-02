//! A subscript whose bytes spell an assignment, read back, measured
//! against the pinned Bash 5.3.
//!
//! `m['a=1']=x` stored the key and `${m['a=1']}` answered nothing for it.
//! The parser had delimited the name correctly -- it counts brackets and
//! honours quoting, as Bash's `skipsubscript` does -- and the expansion
//! then cut the name it was handed at the first `=`, which is dash's rule
//! for a `${name=word}` that arrives in one buffer and is not this
//! shell's, the operand being a part of its own. So every key holding an
//! `=` was unreachable through a subscript: `m['a=1']`, `m['=']`,
//! `m['[a]=1']` alike. Bash 5.1's key/value form makes such keys easy to
//! produce, which is where this was met.
//!
//! Nothing here is a recorded expectation. Every case runs in both shells
//! and the two answers are compared, so there is no literal to go stale.
//! Only stdout and the exit status are read: this shell's diagnostic
//! wording is a registered difference, and an associative array's
//! iteration order is its hash order, which the two shells do not share.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A key holding an `=`, written and read back through every spelling of
/// the subscript.
const KEY_SPELLING_AN_ASSIGNMENT: &[&str] = &[
    "declare -A m\nm['a=1']=x\necho \"[${m['a=1']}]\"\n",
    "declare -A m\nm['=']=x\necho \"[${m['=']}]\"\n",
    "declare -A m\nm['[a]=1']=x\necho \"[${m['[a]=1']}]\"\n",
    "declare -A m\nm['[a]+=1']=x\necho \"[${m['[a]+=1']}]\"\n",
    "declare -A m\nm['x[a]=1']=v\necho \"[${m['x[a]=1']}]\"\n",
    /* Quoted three ways and unquoted, which all name one key. */
    "declare -A m\nm['a=1']=x\necho \"[${m[a=1]}]\"\n",
    "declare -A m\nm['a=1']=x\necho \"[${m[\"a=1\"]}]\"\n",
    "declare -A m\nk='a=1'\nm[$k]=x\necho \"[${m['a=1']}]\" \"[${m[$k]}]\"\n",
    /* The key still misses when it is a different key. */
    "declare -A m\nm['nope=1']=x\necho \"[${m['other=2']}]\"\n",
    /* And it is one key, not a prefix of one. */
    "declare -A m\nm['a=1']=x\nfor k in \"${!m[@]}\"; do echo \"[$k]\"; done\n",
    "declare -A m\nm['a=1']=x\ndeclare -p m\n",
];

/// The same key under every operator that reads a parameter, because the
/// name was cut before the operator was consulted.
const EVERY_OPERATOR_READS_IT: &[&str] = &[
    "declare -A m\nm['a=1']=x\necho \"[${m['a=1']:-D}]\" \"[${m['a=1']-D}]\" \"[${m['a=1']+S}]\"\n",
    "declare -A m\nm['a=1']=x\necho \"[${#m['a=1']}]\"\n",
    "declare -A m\nm['a=1']=x\necho \"[${m['a=1']#x}]\" \"[${m['a=1']%x}]\"\n",
    "declare -A m\nm['a=1']=x\necho \"[${m['a=1']@Q}]\"\n",
    "declare -A m\nm['a=1']=xyz\necho \"[${m['a=1']/x/z}]\"\n",
    "declare -A m\nm['a=1']=xyz\necho \"[${m['a=1']:1:1}]\"\n",
    "declare -A m\nm['a=1']=x\necho \"[${m['a=1']?nope}]\"\necho rc=$?\n",
    "declare -A m\nm['a=1']=x\nt=${m['a=1']}\necho \"[$t]\"\n",
    "declare -A m\nm['a=1']=x\nm['a=1']+=y\necho \"[${m['a=1']}]\"\n",
    "declare -A m\nm['a=1']=x\nunset m['a=1']\necho \"${#m[@]}\"\n",
];

/// An indexed array's subscript is an expression, and an expression may
/// contain an assignment of its own.
const AN_INDEX_MAY_ASSIGN: &[&str] = &[
    "declare -a a=(0 1 2)\necho \"[${a[b=1]}]\" \"[$b]\"\n",
    "declare -a a=(0 1 2)\necho \"[${a[1+1]}]\"\n",
    "declare -A m=([k]=v)\necho \"[${m[k]}]\"\n",
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

/// A key whose bytes spell an assignment reads back as the reference
/// reads it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_key_holding_an_equals_reads_back() {
    agrees(KEY_SPELLING_AN_ASSIGNMENT);
}

/// Every parameter operator finds the same element the reference finds.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn the_operator_does_not_shorten_the_name() {
    agrees(EVERY_OPERATOR_READS_IT);
}

/// An index that assigns evaluates as the reference evaluates it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_index_expression_may_assign() {
    agrees(AN_INDEX_MAY_ASSIGN);
}
