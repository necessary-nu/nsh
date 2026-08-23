//! Array and compound assignments written as operands of a declaration
//! built-in, where the attribute has to exist before the value lands.

use bstr::BStr;
use nsh::{Shell, Streams};

fn shell(bash: bool) -> Shell {
    Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"bash"), bash)
        .build()
        .expect("build shell")
}

fn run(shell: &mut Shell, script: &[u8]) -> (i32, Vec<u8>) {
    let status = shell.run(script).expect("run script").code().into();
    let stdout = shell
        .take_captured_stdout()
        .expect("capture stdout")
        .to_vec();
    shell.take_captured_stderr().expect("capture stderr");
    (status, stdout)
}

/// Run one Bash-mode script and hold it to exactly what Bash prints.
fn expect(script: &[u8], stdout: &[u8]) {
    let mut shell = shell(true);
    let (status, printed) = run(&mut shell, script);
    assert_eq!(status, 0, "script failed: {}", BStr::new(script));
    assert_eq!(BStr::new(&printed), BStr::new(stdout));
}

/// A compound value written after a declaration built-in is an operand
/// of that built-in, not a prefix assignment, and it used to reach the
/// expander as a non-word node and abort the script.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_declaration_operand_carries_a_compound_value() {
    expect(
        b"declare -a a=(x y z)\n\
          echo $?:${a[@]}:${#a[@]}\n\
          declare -A m=([k]=v [j]=w)\n\
          echo $?:${m[k]}:${m[j]}\n\
          declare plain=(1 2)\n\
          echo $?:${plain[@]}\n\
          declare one[0]=v one[2]=w\n\
          echo $?:${one[0]}:${one[2]}:${#one[@]}\n\
          typeset -a t=(p q)\n\
          echo $?:${t[@]}\n",
        b"0:x y z:3\n0:v:w\n0:1 2\n0:v:w:2\n0:p q\n",
    );
}

/// The kind attribute has to be applied first: an associative subscript
/// is a literal key, and the same text would be arithmetic on the
/// indexed array a missing `-A` would have produced.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn the_kind_attribute_lands_before_the_value() {
    expect(
        b"declare -A m=([2+2]=key [x]=named)\n\
          echo ${m[2+2]}:${m[x]}\n\
          declare -a i=([2+2]=index)\n\
          echo ${i[4]}:${#i[@]}\n\
          declare d=(one two)\n\
          declare -p d\n",
        b"key:named\nindex:1\ndeclare -a d=([0]=\"one\" [1]=\"two\")\n",
    );
}

/// `local -a` declares the array in the running function's frame, so the
/// caller's value comes back when the call returns.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn local_keeps_a_compound_value_in_the_frame() {
    expect(
        b"v=(outer)\n\
          f() { local -a v=(1 2); echo in:${v[@]}; }\n\
          g() { local -A m=([k]=inner); echo in:${m[k]}; }\n\
          f\n\
          echo out:${v[@]}\n\
          g\n\
          echo out:${m[k]}-\n",
        b"in:1 2\nout:outer\nin:inner\nout:-\n",
    );
}

/// A declaration that turns its own name read-only still stores the
/// value it was written with; the attribute applies to later writes.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_read_only_declaration_stores_its_own_value() {
    expect(
        b"declare -r q=(7 8)\n\
          echo $?:${q[@]}\n\
          readonly r=(1 2)\n\
          echo $?:${r[@]}\n\
          export e=(5 6)\n\
          echo $?:${e[@]}\n",
        b"0:7 8\n0:1 2\n0:5 6\n",
    );
}

/// Nothing about this reaches the POSIX dialect: `(` after an
/// assignment is a syntax error there, so the parser never builds the
/// node the declaration path holds back.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn posix_mode_still_rejects_a_compound_operand() {
    let mut shell = shell(false);
    let outcome = shell.run(b"declare -a a=(x y)\n");
    let status: i32 = match outcome {
        Ok(flow) => flow.code().into(),
        Err(_) => 2,
    };
    shell.take_captured_stdout().expect("capture stdout");
    shell.take_captured_stderr().expect("capture stderr");
    assert_ne!(status, 0, "POSIX mode accepted a compound operand");
}

/// An assignment word's subscript is read to its matching bracket, so
/// the blanks and operators inside it are the expression's own bytes.
/// Only a position where an assignment may begin reads it that way: the
/// same text as an argument is three ordinary words.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_assignment_word_takes_a_bracketed_subscript() {
    expect(
        b"a[1 * 1]=x\n\
          a[ 1 + 2 ]=z\n\
          echo $?:${a[1]}:${a[3]}\n\
          i=(0 1 2)\n\
          b[i[0]]=p b[ i[1]+i[2] ]=q\n\
          echo ${b[0]}:${b[3]}\n\
          echo c[1 + 2]=v\n\
          c[3 + 4]=w echo done\n\
          echo ${c[7]-unset}\n",
        b"0:x:z\np:q\nc[1 + 2]=v\ndone\nunset\n",
    );
}

/// A subscripted element's value is an assignment operand: no brace
/// expansion, no field splitting, and tilde expansion after every
/// colon. An element without a subscript is an ordinary word.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_element_value_expands_as_an_assignment() {
    expect(
        b"HOME=/home/user\n\
          v='1 2 3'\n\
          a=([0]=-{p,q}- [1]=$v [2]=~ [3]=~:~)\n\
          echo \"${a[0]},${a[1]},${a[2]},${a[3]}\"\n\
          echo ${#a[@]}\n\
          b=($v)\n\
          echo ${#b[@]}\n",
        b"-{p,q}-,1 2 3,/home/user,/home/user:/home/user\n4\n3\n",
    );
}

/// Each element's subscript is evaluated against the array the elements
/// before it have already written, and the values are all expanded
/// first, so the right-hand sides read the array as it was.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_compound_subscript_sees_the_earlier_elements() {
    expect(
        b"a=([0]=1+2+3 [a[0]]=10 [a[6]]=hello)\n\
          echo ${!a[@]}\n\
          b=(old1 old2 old3)\n\
          b=(\"${b[2]}\" \"${b[0]}\" \"${b[1]}\")\n\
          echo ${b[@]}\n",
        b"0 6 10\nold3 old1 old2\n",
    );
}

/// An array does not become the other kind: the elements would mean
/// something else, so the declaration fails and the value stays.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_array_refuses_the_other_kind() {
    expect(
        b"declare -a a=(1 2)\n\
          declare -A a\n\
          echo $?:${a[@]}\n\
          declare -A m=([k]=v)\n\
          declare -a m\n\
          echo $?:${m[k]}\n\
          s=text\n\
          declare -a s\n\
          echo $?:${s[0]}\n",
        b"1:1 2\n1:v\n0:text\n",
    );
}

/// An operator reaching the elements through `[*]` is applied to each
/// of them and joined afterwards, not applied to the joined text.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_star_subscript_applies_the_operator_per_element() {
    expect(
        b"a=('-x-' 'y-y' '-z-')\n\
          echo \"${a[*]#-}\"\n\
          b=(p q)\n\
          echo \"${b[*]@Q}\"\n\
          set -- p q\n\
          echo \"${*@Q}\"\n",
        b"x- y-y z-\n'p' 'q'\n'p' 'q'\n",
    );
}

/// A slice of an array counts subscripts rather than surviving
/// elements, so the holes an `unset` left still take up room, and an
/// offset that reaches back past the start selects nothing.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_sparse_array_is_sliced_by_subscript() {
    expect(
        b"a=(v0 v1 v2 v3 v4 v5)\n\
          unset -v 'a[2]' 'a[3]'\n\
          echo [${a[@]:2}]\n\
          echo [${a[@]:0:3}]\n\
          echo [${a[@]: -1}]\n\
          echo [${a[@]: -7}]\n\
          echo [${a[@]:6}]\n",
        b"[v4 v5]\n[v0 v1 v4]\n[v5]\n[]\n[]\n",
    );
}

/// An array has no environment spelling: an exported name that holds
/// one contributes nothing, and a command prefix passes a compound
/// value's text rather than an element.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_array_has_no_environment_spelling() {
    expect(
        b"export E\n\
          E=(one two)\n\
          env | grep '^E=' || echo none\n\
          B=(b b) env | grep '^B='\n\
          export a[7]=8\n\
          echo $?:${a[7]-unset}\n",
        b"none\nB=(b b)\n1:unset\n",
    );
}

/// A name declared as an array holds nothing until an element exists,
/// which `-v` and an unsubscripted read both answer for.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_empty_array_reads_as_unset() {
    expect(
        b"typeset -a a\n\
          test -v a; echo $?\n\
          echo ${a-unset},${a:-empty}\n\
          a[0]=1\n\
          test -v a; echo $?\n\
          echo ${a-unset}\n\
          b=('' '')\n\
          echo ${b-unset}-\n",
        b"1\nunset,empty\n0\n1\n-\n",
    );
}

/// A declaration operand that arrived as one word still spells an
/// assignment, and a compound value in it is read as one where the name
/// is an array.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn a_one_word_operand_spells_an_assignment() {
    expect(
        b"code='x=(1 2 3)'\n\
          typeset -a \"$code\"\n\
          echo $?:${x[@]}\n\
          declare +a 'y=(2 3)'\n\
          echo ${y}\n\
          declare -a 'z=4'\n\
          echo ${z[0]}:${#z[@]}\n\
          cmd=typeset\n\
          w='a b'\n\
          $cmd v=$w\n\
          echo [$v]\n",
        b"0:1 2 3\n(2 3)\n4:1\n[a]\n",
    );
}

/// One element takes a string, so a list assigned to one is refused and
/// the array is left as it was.
// [spec:nsh:req:compat.bash.arrays-declarations/test]
#[test]
fn an_element_refuses_a_list() {
    expect(
        b"a=(1 '2 3')\n\
          a[0]+=(4 5)\n\
          echo $?:${#a[@]}\n\
          typeset -n ref='a[0]'\n\
          ref[0]=foo\n\
          echo $?:${a[0]}\n",
        b"1:2\n1:1\n",
    );
}
