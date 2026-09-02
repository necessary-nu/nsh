//! What an associative array's compound assignment refuses, and how an
//! assignment is spelled back, measured against the pinned Bash 5.3.
//!
//! Six divergences were found while fixing the blank-in-a-subscript data
//! loss and left for this file to close. Five of them are those two
//! questions:
//!
//! * a bare word in a `[key]=value` list is refused, and the refusal
//!   abandons the command list rather than being silently dropped;
//! * an empty key is refused where an empty *index* is zero;
//! * `+=` inside an associative compound assignment appends to the value
//!   the key held before the assignment began, not to the one the
//!   elements before it have built;
//! * a list that opens with a bare word is Bash 5.1's `( key value ... )`
//!   form, in which even `[a]=1` is a literal key;
//! * `${m[@]@A}` spells the whole `declare -p` line, as three fields;
//! * and `readonly -A` and `export -A` are accepted, where the letter
//!   says how a compound operand is read and nothing more.
//!
//! Nothing here is a recorded expectation. Every case runs in both
//! shells and the two answers are compared, so there is no literal to go
//! stale: if Bash changes its mind, this reports it rather than passing.
//!
//! Two things are deliberately not compared. Diagnostic wording is
//! registered as differing, so only stdout and the exit status are read
//! -- but *whether* a case reports is still measured, because a refusal
//! shows up in the status and in the commands that no longer run. And an
//! associative array's iteration order is its hash order, which the two
//! shells do not share, so no case prints more than one key at a time
//! except through `${#m[@]}` and a named subscript.

#[path = "../../nsh/tests/pinned_bash/mod.rs"]
mod pinned_bash;

use std::path::Path;

/// A bare word in a `[key]=value` list, which names no element.
///
/// The last two rows are the ones that were losing data: the shell kept
/// a partial array and answered 0, so a script that mistyped one element
/// carried on with the rest of the assignment missing.
const REFUSED_BARE_WORD: &[&str] = &[
    "declare -A m=([a]=1 foo)\ndeclare -p m\n",
    "declare -A m=([a b]=c d)\ndeclare -p m\n",
    "declare -A m=([a]=1 [b]=2 foo [c]=3)\necho \"${#m[@]} ${m[a]} ${m[b]} [${m[c]}]\"\n",
    /* The refusal is an assignment error: it abandons the list it was
     * written in and the next line runs. */
    "echo first; declare -A m=([a]=1 foo); echo second\n",
    "echo first\ndeclare -A m=([a]=1 foo)\necho second\n",
    "f() { declare -A m=([a]=1 foo); echo inner; }; f; echo after\n",
    "declare -A m=([a]=1 foo)\necho status=$?\n",
    /* An indexed array has no such refusal to make. */
    "declare -a a=([0]=1 foo)\ndeclare -p a\n",
    /* An operand the built-in had to read as text only reports. */
    "declare -A \"m=([k]=v x)\"\necho status=$?\ndeclare -p m\n",
];

/// The empty subscript, which is a key an associative array does not
/// have and an index that is zero.
const REFUSED_EMPTY_SUBSCRIPT: &[&str] = &[
    "declare -A m\nm[\"\"]=x\necho unreached\n",
    "declare -A m\nm[\"\"]=x\necho after=$? size=${#m[@]}\n",
    "declare -A m\ne=\nm[$e]=x\necho unreached\n",
    "declare -A m\nm[\"\"]+=x\necho unreached\n",
    "declare -A m=([\"\"]=x)\necho unreached\n",
    "declare -A m=([a]=1 [\"\"]=x [c]=3)\necho \"${#m[@]} ${m[a]}\"\n",
    "declare -A m\nm+=([\"\"]=x)\necho unreached\n",
    /* A statement's `a[""]` is the expression `""`, which is zero; a
     * compound *element* spelling the same thing is refused whatever
     * the array's kind. */
    "declare -a a\na[\"\"]=x\ndeclare -p a\n",
    "declare -a a=([\"\"]=x)\necho unreached\n",
    "declare -a a=([\"\"]=x [1]=y)\ndeclare -p a\n",
    "declare -a a=([]=x)\necho unreached\n",
    "e=\ndeclare -a a=([$e]=x)\necho unreached\n",
    "declare -a a=([0]=z [\"\"]=x)\ndeclare -p a\n",
    /* The key/value form reports an empty key and keeps reading. */
    "declare -A m=(\"\" v)\necho after=$?\ndeclare -p m\n",
    "declare -A m=(\"\" v k w)\necho \"$? ${#m[@]} ${m[k]}\"\n",
];

/// `+=` inside a compound assignment, which reads a different base in an
/// associative array from the one it reads in an indexed one.
const APPENDING_ELEMENT: &[&str] = &[
    "declare -A m=([k]+=1 [k]+=2)\ndeclare -p m\n",
    "declare -A m=([k]=z)\ndeclare -A m=([k]+=1)\ndeclare -p m\n",
    "declare -A m=([k]=z)\ndeclare -A m=([k]+=1 [k]+=2)\ndeclare -p m\n",
    "declare -A m=([k]=z)\nm=([k]+=1)\ndeclare -p m\n",
    "declare -A m=([k]=z)\ndeclare -A m=([k]=a [k]+=1)\ndeclare -p m\n",
    "declare -A m=([k]=z)\ndeclare -A m=([k]+=1 [k]=a)\ndeclare -p m\n",
    "declare -A m=([k]=z [j]=y)\ndeclare -A m=([j]+=Q)\ndeclare -p m\n",
    /* An appending assignment has no separate before: its elements build
     * on the array that is still there. */
    "declare -A m=([k]=z)\nm+=([k]+=1 [k]+=2)\ndeclare -p m\n",
    /* An unset name has nothing to append to, in either spelling. */
    "declare -A m=([k]=z)\nunset m\ndeclare -A m=([k]+=1)\ndeclare -p m\n",
    /* Two statements append the way they always did. */
    "declare -A m\nm[k]+=1\nm[k]+=2\ndeclare -p m\n",
    /* An indexed array reads the running value on both sides. */
    "declare -a a=([0]+=1 [0]+=2)\ndeclare -p a\n",
    "declare -a a=([0]=z)\ndeclare -a a=([0]+=1 [0]+=2)\ndeclare -p a\n",
    "declare -a a=([0]=z)\na+=([0]+=1 [0]+=2)\ndeclare -p a\n",
    /* And the operand read as text spells the same assignment. */
    "declare -A \"m=([k]+=1 [k]+=2)\"\ndeclare -p m\n",
];

/// Bash 5.1's `( key value key value )` form, chosen when the list opens
/// with a bare word.
const KEY_VALUE_PAIRS: &[&str] = &[
    "declare -A m=(foo)\ndeclare -p m\n",
    "declare -A m=(foo bar)\ndeclare -p m\n",
    "declare -A m=(a b c)\necho \"${#m[@]} ${m[a]} [${m[c]}]\"\n",
    "declare -A m=(a b c d)\necho \"${#m[@]} ${m[a]} ${m[c]}\"\n",
    "declare -A m=(k1 v1 k1 v2)\ndeclare -p m\n",
    /* The first element decides, so a written subscript after one is a
     * literal key. */
    "declare -A m=(foo [a]=1)\ndeclare -p m\n",
    "declare -A m=(foo bar [a]=1 x)\necho \"${#m[@]} ${m[foo]}\"\n",
    "declare -A m=(k1 v1 [k1]+=x)\necho \"${#m[@]} ${m[k1]}\"\n",
    "declare -A m=(x[a b]=1)\ndeclare -p m\n",
    /* Neither split nor globbed nor brace-expanded: one field per
     * element, expanded the way an assignment's right-hand side is. */
    "p='a b'\ndeclare -A m=($p)\ndeclare -p m\n",
    "p='a b c d'\ndeclare -A m=($p)\ndeclare -p m\n",
    "p='x y'\ndeclare -A m=($p $p)\ndeclare -p m\n",
    "declare -A m=(* v)\ndeclare -p m\n",
    "declare -A m=({a,b} v)\ndeclare -p m\n",
    "declare -A m=($(echo x) v)\ndeclare -p m\n",
    "HOME=/h\ndeclare -A m=(~ v)\ndeclare -p m\n",
    "k='p q'\ndeclare -A m=(\"$k\" v)\ndeclare -p m\n",
    "declare -A m=('a b' 'c d')\ndeclare -p m\n",
    "declare -A m=(\"a\nb\" v)\ndeclare -p m\n",
    /* Every spelling of the assignment reaches the form. */
    "declare -A m\nm=(a)\ndeclare -p m\n",
    "declare -A m=(k1 v1)\nm+=(k2 v2)\necho \"${#m[@]} ${m[k1]} ${m[k2]}\"\n",
    "declare -A m=([a]=1)\nm+=(k v)\necho \"${#m[@]} ${m[a]} ${m[k]}\"\n",
    "declare -A \"m=(a b)\"\ndeclare -p m\n",
    "c='m=(a b)'\ndeclare -A \"$c\"\ndeclare -p m\n",
    "f() { local -A m=(a b); declare -p m; }\nf\n",
    /* An indexed array never takes it. */
    "declare -a a=(a b c)\ndeclare -p a\n",
];

/// `${name@A}`, the assignment that would put a name back.
///
/// A name declared without ever being assigned is left out: Bash keeps
/// such a name invisible and prints `declare -a z` where this shell
/// prints `declare -a z=()`, which is a `declare -p` difference of its
/// own and is filed as `spell-an-unassigned-array-back`.
const SPELLED_BACK: &[&str] = &[
    "declare -A m\nm[k]=v\nprintf '[%s]' \"${m[@]@A}\"\necho\n",
    "declare -A m\nm['a b']=1\necho \"${m[@]@A}\"\n",
    "declare -a a=(x 'y z')\nprintf '[%s]' \"${a[@]@A}\"\necho\n",
    "declare -a a=(x y)\nprintf '[%s]' \"${a[*]@A}\"\necho\n",
    "declare -a e=()\nprintf '[%s]' \"${e[@]@A}\"\necho\n",
    "declare -A m\nm[k]=v\nprintf '[%s]' ${m[@]@A}\necho\n",
    "declare -A m\nm[k]=v\nIFS=\nprintf '[%s]' \"${m[@]@A}\"\necho\n",
    "declare -rA m=([k]=v)\nprintf '[%s]' \"${m[@]@A}\"\necho\n",
    "f() { local -A m=([k]=v); echo \"${m[@]@A}\"; }\nf\n",
    /* The bare name reads the scalar, and carries the flags with it. */
    "x=hello\necho \"${x@A}\"\n",
    "x=\nprintf '[%s]' \"${x@A}\"\necho\n",
    "declare -i n=5\necho \"${n@A}\"\n",
    "declare -rx q=1\necho \"${q@A}\"\n",
    "declare -x s=v\necho \"${s@A}\"\n",
    "declare -l L=AB\necho \"${L@A}\"\n",
    "declare -a a=(x 'y z')\nprintf '[%s]' \"${a@A}\"\necho\n",
    "declare -a a=(x)\na[5]=y\nprintf '[%s]' \"${a@A}\"\necho\n",
    "declare -A m\nm[k]=v\necho \"${m[k]@A}\"\n",
    "x=hi\nprintf '[%s]' \"${x[@]@A}\"\necho\n",
    /* A reference spells the name it points at. */
    "declare -n r=t\nt=5\nprintf '[%s]' \"${r@A}\"\necho\n",
    /* A parameter with no declaration has none to print. */
    "printf '[%s]' \"${nope[@]@A}\"\necho\n",
    "printf '[%s]' \"${nope@A}\"\necho\n",
    "set -- p\nprintf '[%s]' \"${1@A}\"\necho\n",
    "set -- p\nprintf '[%s]' \"${#@A}\"\necho\n",
    /* The positionals spell `set --`, and no positionals spell nothing. */
    "set -- p q\nprintf '[%s]' \"${@@A}\"\necho\n",
    "set -- p\nprintf '[%s]' \"${*@A}\"\necho\n",
    "set --\nprintf '[%s]' \"${@@A}\"\necho\n",
    /* What it spells reads back, which is the whole point of it. */
    "declare -A m\nm['x y']=z\nt=\"${m[@]@A}\"\nunset m\neval \"$t\"\necho \"${#m[@]} [${m[x y]}]\"\n",
    "declare -a a=(1 2 3)\nt=\"${a[@]@A}\"\nunset a\neval \"$t\"\ndeclare -p a\n",
];

/// `readonly -a`, `readonly -A` and their `export` spellings.
const DECLARING_BUILTINS: &[&str] = &[
    "readonly -a a=(1 2)\ndeclare -p a\n",
    "readonly -A m=([a b]=1)\ndeclare -p m\n",
    "readonly -A m=(k v)\ndeclare -p m\n",
    "export -a a=(1 2)\ndeclare -p a\necho status=$?\n",
    "export -A m=([a]=1)\ndeclare -p m\necho status=$?\n",
    /* The letter says how a compound operand is read and nothing else,
     * so a name with no value beside it does not become an array. */
    "readonly -A m\ndeclare -p m\necho status=$?\n",
    "export -A m\ndeclare -p m\n",
    "a=(1)\nreadonly -A a\necho status=$?\ndeclare -p a\n",
    /* The value still lands, and the name is still read-only after. */
    "readonly -A m=([k]=v)\nm[j]=w\necho unreached\n",
    "readonly -A m=([k]=v)\nm[j]=w\ndeclare -p m\n",
    /* The letters `readonly` does not take stay refused. */
    "readonly -i n=5\ndeclare -p n\n",
    "readonly x=1\necho status=$?\ndeclare -p x\n",
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

/// A bare word in a `[key]=value` list is refused as the reference
/// refuses it.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
// [spec:nsh:req:oracle.cannot-measure-is-a-failure/test]
#[test]
fn a_bare_word_needs_a_subscript() {
    agrees(REFUSED_BARE_WORD);
}

/// An empty subscript is refused where it names a key and is zero where
/// it is an index, as it is in the reference.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.error-boundary/test]
#[test]
fn an_empty_subscript_is_refused_as_a_key() {
    agrees(REFUSED_EMPTY_SUBSCRIPT);
}

/// `+=` inside a compound assignment appends to what the reference
/// appends to.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_appending_element_reads_the_references_base() {
    agrees(APPENDING_ELEMENT);
}

/// A list opening with a bare word is read as key/value pairs, as it is
/// in the reference.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_leading_bare_word_makes_the_list_pairs() {
    agrees(KEY_VALUE_PAIRS);
}

/// `${name@A}` spells what the reference spells, in the fields the
/// reference spells it in.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn the_transform_spells_the_references_assignment() {
    agrees(SPELLED_BACK);
}

/// `readonly` and `export` take the array letters the reference takes
/// and use them for what the reference uses them for.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_declaring_builtins_take_the_array_letters() {
    agrees(DECLARING_BUILTINS);
}
