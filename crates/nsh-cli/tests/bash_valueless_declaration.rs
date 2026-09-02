//! What a name that carries attributes and no value spells back,
//! measured against the pinned Bash 5.3.
//!
//! `declare -a z` was given its invisible-variable spelling by
//! `bash_declared_array.rs`, and every other valueless name was left
//! spelling nothing: `declare -i n`, `readonly x`, `declare -x e` and a
//! function's own `local q` all print a declaration in the reference and
//! printed an empty string here. Read through a subscript they were
//! worse than empty -- `${n[@]@A}` invented `declare -i n=''`, claiming
//! a value the name does not have.
//!
//! TWO THINGS THIS FILE PINS THAT WERE NOT THE DEFECT IT WAS WRITTEN
//! FOR.
//!
//! `local` is an attribute. Bash prefixes `${name@A}` with `declare `
//! exactly when the name carries one, and a name local to the body now
//! running carries `att_local` even when it carries nothing else: a
//! global `q=1` spells `q='1'` and a `local q=1` spells `declare q='1'`,
//! with no letters between. It is the *current* body that decides, so a
//! callee sees the caller's local as an ordinary name.
//!
//! There is one letter table, not two. This shell had a second one in
//! Bash's `attribute_string` order for `declare -p` and printed
//! `declare -lr` where the reference prints `declare -rl`.
//! [`THE_LETTER_ORDER`] walks every combination of the attributes that
//! can be held at once and asks all three printers, which is what caught
//! it.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//! Diagnostic wording is a registered difference, so only stdout and the
//! exit status are read.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// `${name@A}` on a name that holds no value.
///
/// The empty rows are half the question: a bare `declare y` at the
/// global level carries no attribute at all and prints nothing, which is
/// what makes this a test of the attributes rather than of the entry.
const SPELLED_BY_ATTRIBUTE: &[&str] = &[
    "declare -i n\necho \"[${n@A}]\"\n",
    "readonly x\necho \"[${x@A}]\"\n",
    "declare -x e\necho \"[${e@A}]\"\n",
    "declare -l lo\necho \"[${lo@A}]\"\n",
    "declare -u up\necho \"[${up@A}]\"\n",
    "declare -t tr\necho \"[${tr@A}]\"\n",
    "declare -ir ir\necho \"[${ir@A}]\"\n",
    /* Nothing to carry is nothing to print, and an undeclared name and
     * a positional have nothing to carry. */
    "declare y\necho \"[${y@A}]\"\n",
    "echo \"[${undeclared@A}]\"\n",
    "echo \"[${1@A}]\"\n",
    "set -- a\necho \"[${1@A}]\"\n",
    /* Read whole, the same name must not grow a value it does not
     * have. This printed `declare -i n=''` before. */
    "declare -i n\necho \"[${n[@]@A}]\"\n",
    "declare -i n\necho \"[${n[*]@A}]\"\n",
    "readonly x\necho \"[${x[@]@A}]\"\n",
    "declare y\necho \"[${y[@]@A}]\"\n",
    "declare -i n\necho \"[${n[3]@A}]\"\n",
    /* And the declaration it prints has to read back as itself. */
    "declare -i n\nt=\"${n@A}\"\nunset n\neval \"$t\"\ndeclare -p n\n",
    "readonly x\nt=\"${x@A}\"\ndeclare -p x\n",
    /* An assigned name keeps spelling its value, which is the contrast
     * the rows above are read against. */
    "declare -i n=3\necho \"[${n@A}]\"\n",
    "x=1\necho \"[${x@A}]\"\n",
    "x=\nprintf '[%s]\\n' \"${x@A}\"\n",
    "declare -a z=(1 2)\nprintf '[%s]' \"${z[@]@A}\"\necho\n",
];

/// `${name@a}`, which answers for the name and not for the value.
///
/// A name with nothing in it has no elements for the map to run over,
/// and the reference still answers once. The `=()` rows are the
/// boundary: an assigned empty array has an empty walk rather than no
/// walk, and answers nothing.
const THE_LETTERS_OF_A_VALUELESS_NAME: &[&str] = &[
    "declare -a z\necho \"[${z[@]@a}]\"\n",
    "declare -A m\necho \"[${m[@]@a}]\"\n",
    "declare -i n\necho \"[${n[@]@a}]\"\n",
    "readonly x\necho \"[${x[@]@a}]\"\n",
    "declare -x e\necho \"[${e[*]@a}]\"\n",
    "declare -a z\necho \"[${z@a}]\"\n",
    "declare -i n\necho \"[${n@a}]\"\n",
    "declare -i n\necho \"[${n[3]@a}]\"\n",
    "declare y\necho \"[${y[@]@a}]\"\n",
    "echo \"[${undeclared[@]@a}]\"\n",
    /* An empty array has elements to walk and no element in them. */
    "declare -a z=()\necho \"[${z[@]@a}]\"\n",
    "declare -A m=()\necho \"[${m[@]@a}]\"\n",
    /* And a name with elements answers once per element. */
    "declare -a z=(1 2)\nprintf '[%s]' \"${z[@]@a}\"\necho\n",
    "declare -ar z=(1 2)\nprintf '[%s]' \"${z[@]@a}\"\necho\n",
    "x=1\necho \"[${x@a}]\"\n",
];

/// `local`, which is an attribute of the body now running.
const LOCAL_IS_AN_ATTRIBUTE: &[&str] = &[
    "f() { local q; echo \"[${q@A}]\"; }\nf\n",
    "f() { local q=v; echo \"[${q@A}]\"; }\nf\n",
    "f() { local -i q=1; echo \"[${q@A}]\"; }\nf\n",
    "f() { declare q=v; echo \"[${q@A}]\"; }\nf\n",
    "f() { local q; echo \"[${q@a}]\"; }\nf\n",
    /* A global with the same shape carries no attribute and is bare. */
    "q=v\necho \"[${q@A}]\"\n",
    "f() { declare -g q=v; echo \"[${q@A}]\"; }\nf\n",
    /* The declaring body is the only one that sees it. */
    "g() { echo \"[${q@A}]\"; }\nf() { local q=1; g; }\nf\n",
    "g() { local r=2; echo \"[${q@A}][${r@A}]\"; }\nf() { local q=1; g; }\nf\n",
    "f() { local q=1; }\nf\necho \"[${q@A}]\"\n",
    /* A local that shadows a global is the local while the body runs
     * and the global afterwards. */
    "q=g\nf() { local q=1; echo \"[${q@A}]\"; }\nf\necho \"[${q@A}]\"\n",
];

/// A reference that points at nothing, which has nothing to spell.
///
/// Both transforms read through a `declare -n`, so both answer for the
/// name it holds; a reference holding no name at all answers for
/// neither itself nor anything else. `declare -p` still prints it, which
/// is why this is a table about the transforms.
const A_REFERENCE_TO_NOTHING: &[&str] = &[
    "declare -n r\necho \"[${r@A}][${r@a}]\"\n",
    "declare -rn r\necho \"[${r@A}][${r@a}]\"\n",
    "declare -n r=nowhere\necho \"[${r@A}][${r@a}]\"\n",
    "declare -n r\ndeclare -p r\n",
    /* A reference that does point somewhere answers for the target,
     * whether or not the target holds anything. */
    "declare -i t\ndeclare -n r=t\necho \"[${r@A}][${r@a}]\"\n",
    "declare -a t\ndeclare -n r=t\necho \"[${r@A}][${r@a}]\"\n",
    "t=1\ndeclare -n r=t\necho \"[${r@A}][${r@a}]\"\n",
    "readonly t\ndeclare -n r=t\necho \"[${r@A}][${r@a}]\"\n",
];

/// Every combination of attributes a name can carry at once, through
/// the three printers that spell its letters.
///
/// `-l` and `-u` cannot both be held, `-n` needs a target and takes no
/// array kind, and `-a` and `-A` exclude each other; what is left is
/// this. The letters must be the same three ways round, because in the
/// reference they come out of one function.
fn the_letter_order() -> Vec<String> {
    const LETTERS: &[u8] = b"ilrtux";
    let mut cases = Vec::new();
    for kind in ["", "a", "A"] {
        for bits in 0..(1u32 << LETTERS.len()) {
            let held: Vec<u8> = LETTERS
                .iter()
                .enumerate()
                .filter(|(at, _)| bits & (1 << at) != 0)
                .map(|(_, letter)| *letter)
                .collect();
            if held.contains(&b'l') && held.contains(&b'u') {
                continue;
            }
            let flags = format!("{kind}{}", String::from_utf8(held).expect("ascii letters"));
            if flags.is_empty() {
                continue;
            }
            /* The value is a numeral, and the associative one names its
             * key: what the integer attribute does to a compound
             * assignment's elements is its own question, and this table
             * is only asking about the letters. */
            let value = match kind {
                "a" => "v=(1)",
                "A" => "v=([k]=1)",
                _ => "v=1",
            };
            cases.push(format!(
                "declare -{flags} {value}\ndeclare -p v\necho \"[${{v@a}}]\"\necho \"[${{v@A}}]\"\n"
            ));
        }
    }
    cases
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

/// A name carrying attributes and no value spells the declaration the
/// reference spells, and no value beside it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.value-model/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn an_attribute_is_a_declaration_to_spell() {
    agrees(SPELLED_BY_ATTRIBUTE);
}

/// `${name@a}` answers the reference's letters for a name that holds no
/// value, and maps over the elements of one that does.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_letters_answer_without_a_value() {
    agrees(THE_LETTERS_OF_A_VALUELESS_NAME);
}

/// A name local to the running body carries the `declare` the reference
/// gives it, and a global of the same shape does not.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_local_name_is_spelled_as_one() {
    agrees(LOCAL_IS_AN_ATTRIBUTE);
}

/// A reference holding no name spells nothing, as the reference spells
/// nothing for it.
// [spec:nsh:req:compat.bash.functions-scoping/test]
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_reference_to_nothing_spells_nothing() {
    agrees(A_REFERENCE_TO_NOTHING);
}

/// `declare -p`, `${name@a}` and `${name@A}` order their letters as the
/// reference orders them, for every combination a name can hold.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_letters_come_out_in_one_order() {
    let cases = the_letter_order();
    let borrowed: Vec<&str> = cases.iter().map(String::as_str).collect();
    agrees(&borrowed);
}
