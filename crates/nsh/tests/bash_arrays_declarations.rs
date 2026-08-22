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
