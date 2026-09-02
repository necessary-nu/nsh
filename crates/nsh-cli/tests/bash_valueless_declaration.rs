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

/// The names a `declare -p` listing carries, as the set a script's own
/// declarations add to it.
///
/// The two shells do not start with the same names -- this one has no
/// `FUNCNAME` or `BASH_SOURCE` and does have `PS1` -- so no row compares
/// a whole listing. Each takes the listing's name set before and after
/// its declarations and prints the difference, which cancels whatever
/// the shell brought with it and is still a comparison of sets rather
/// than of one line.
///
/// `grep -vxF` takes each line of `$b` as a fixed whole-line pattern, so
/// the subtraction needs no file and no process substitution: these
/// scripts run with the test runner's own working directory, which is
/// not a place to be writing.
fn listed_by_attribute() -> Vec<String> {
    const NAMES: &str =
        "names() { declare -p | sed -E 's/^declare -[a-zA-Z-]+ //; s/=.*//' | sort; }\n";
    const DELTA: &str = "printf '%s\\n' \"$(names)\" | grep -vxF \"$b\" | tr '\\n' ' '\necho\n";

    let mut cases: Vec<String> = [
        // Every valueless declaration, one at a time and then together.
        "declare -i zq\n",
        "readonly zq\n",
        "declare zq\n",
        "declare -a zq\n",
        "declare -A zq\n",
        "declare -x zq\n",
        "declare -l zq\n",
        "declare -t zq\n",
        "zt=1\ndeclare -n zq=zt\n",
        "declare -i zq1\nreadonly zq2\ndeclare zq3\ndeclare -a zq4\n\
         declare -A zq5\ndeclare -x zq6\n",
        // An assignment does not take a name back out of the listing.
        "declare -i zq\nzq=3\n",
        "declare -a zq\nzq[2]=v\n",
        // Nor does a second declaration of the same name.
        "declare -i zq\ndeclare -r zq\n",
    ]
    .iter()
    .map(|body| format!("{NAMES}b=$(names)\n{body}{DELTA}"))
    .collect();

    /* A name a function made local is in the listing while the body
     * runs and gone from it afterwards, so the set is taken from
     * inside. */
    cases.push(format!(
        "{NAMES}f(){{ local zqa; local zqb=1; declare zqc; \
         printf '%s\\n' \"$(names)\" | grep -vxF \"$1\" | tr '\\n' ' '; echo; }}\n\
         b=$(names)\nf \"$b\"\n"
    ));
    cases.push(format!(
        "{NAMES}f(){{ local zqa; }}\nb=$(names)\nf\n{DELTA}"
    ));
    cases
}

/// The listing asked one name at a time, and the attribute filters it
/// was split from `${!prefix@}` for.
const LISTED_BY_NAME: &[&str] = &[
    "declare -i zq\ndeclare -pi | grep -c ' zq$'\n",
    "readonly zq\ndeclare -pr | grep -c ' zq$'\n",
    "declare -a zq\ndeclare -pa | grep -c ' zq$'\n",
    "declare -A zq\ndeclare -pA | grep -c ' zq$'\n",
    "declare -x zq\ndeclare -px | grep -c ' zq$'\n",
    "zt=1\ndeclare -n zq=zt\ndeclare -pn | grep -c ' zq'\n",
    "declare -i zq\ndeclare -pr | grep -c ' zq$'\n",
    "declare -a zq\ndeclare -pA | grep -c ' zq$'\n",
    // The listing agrees with the same name asked for directly.
    "declare -i zq\ndeclare -p zq\ndeclare -p | grep ' zq$'\n",
    "readonly zq\ndeclare -p zq\ndeclare -p | grep ' zq$'\n",
    "declare zq\ndeclare -p zq\ndeclare -p | grep ' zq$'\n",
    "declare -a zq\ndeclare -p zq\ndeclare -p | grep ' zq$'\n",
    "f(){ local zq; declare -p zq; declare -p | grep ' zq$'; }\nf\n",
];

/// The listings that must *not* grow the same names.
///
/// `${!prefix@}` answers with the names that hold a value, `set` and a
/// bare `declare` list `name=value` pairs, and none of the three carries
/// an invisible name in Bash. `export -p` and `readonly -p` do carry one
/// when it holds their attribute, which is the other half of the same
/// rule.
const THE_OTHER_LISTINGS_ARE_UNMOVED: &[&str] = &[
    "declare -i zq1\nreadonly zq2\ndeclare zq3\ndeclare -a zq4\necho \"[${!zq@}]\"\n",
    "declare -i zq1\ndeclare -a zq4=()\necho \"[${!zq@}]\"\n",
    "declare -i zq1\nreadonly zq2\ndeclare zq3\nset | grep -c '^zq'\n",
    "declare -i zq1\nreadonly zq2\ndeclare zq3\ndeclare | grep -c '^zq'\n",
    "declare -i zq1\ndeclare -x zq5\nexport -p | grep -c ' zq'\n",
    "declare -i zq1\nreadonly zq2\nreadonly -p | grep -c ' zq'\n",
    "declare -x zq5\nexport -p | grep ' zq5'\n",
    "readonly zq2\nreadonly -p | grep ' zq2'\n",
];

/// The names this shell reserves for itself, which are not declarations.
///
/// `MAIL`, `MAILPATH`, `HISTSIZE`, `TERM` and the five locale names get
/// an entry at start-up so that a later assignment has a callback to
/// run. Bash has no variable there, and a listing that grew every entry
/// would have reported this shell's bookkeeping as the script's
/// declarations -- which is what the first attempt at this did, adding
/// seven names Bash does not have.
const A_RESERVED_SLOT_IS_NOT_A_DECLARATION: &[&str] = &[
    "declare -p MAIL\necho status=$?\n",
    "declare -p MAILPATH\necho status=$?\n",
    "declare -p HISTSIZE\necho status=$?\n",
    "declare -p LC_COLLATE\necho status=$?\n",
    "declare -p LC_NUMERIC\necho status=$?\n",
    "declare -p | grep -cE ' (MAIL|MAILPATH|HISTSIZE|LC_COLLATE|LC_NUMERIC)$'\n",
    // An assignment gives it a declaration, and unsetting takes it back.
    "MAIL=/x\ndeclare -p MAIL\ndeclare -p | grep -c ' MAIL='\n",
    "MAIL=/x\nunset MAIL\ndeclare -p MAIL\necho status=$?\n",
    "MAIL=/x\nunset MAIL\ndeclare -p | grep -c ' MAIL'\n",
    "HISTSIZE=9\ndeclare -p HISTSIZE\n",
    // Declaring one is a declaration like any other, wherever the
    // declaration leaves a mark on the entry.
    "declare -i MAILPATH\ndeclare -p MAILPATH\ndeclare -p | grep -c ' MAILPATH$'\n",
    "readonly MAIL\ndeclare -p MAIL\ndeclare -p | grep -c ' MAIL$'\n",
    "declare -x HISTSIZE\ndeclare -p HISTSIZE\ndeclare -p | grep -c ' HISTSIZE$'\n",
    "declare -a LC_CTYPE\ndeclare -p LC_CTYPE\ndeclare -p | grep -c ' LC_CTYPE$'\n",
    "declare -l MAIL\ndeclare -p MAIL\n",
    "declare -t MAILPATH\ndeclare -p MAILPATH\n",
    /* A bare `declare MAIL` is the one that cannot be told from the
     * slot it found, because it changes nothing about the entry. It is
     * `list-a-declaration-of-a-reserved-name`, and it is deliberately
     * not a row here: the table is a check, and a check that is red for
     * a defect somebody else is going to fix stops being one. */
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

/// `declare -p` lists a name that carries attributes and no value, as
/// the reference lists it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn the_listing_carries_a_name_with_no_value() {
    let cases = listed_by_attribute();
    let borrowed: Vec<&str> = cases.iter().map(String::as_str).collect();
    agrees(&borrowed);
    agrees(LISTED_BY_NAME);
}

/// The listings that answer a different question are unmoved.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_other_listings_do_not_grow_it() {
    agrees(THE_OTHER_LISTINGS_ARE_UNMOVED);
}

/// A name this shell reserved for a callback is not a declaration, in
/// the listing or asked for by name.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn a_reserved_slot_is_not_a_declaration() {
    agrees(A_RESERVED_SLOT_IS_NOT_A_DECLARATION);
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
