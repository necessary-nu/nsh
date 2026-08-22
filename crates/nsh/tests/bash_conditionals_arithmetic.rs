//! `[[ ]]`, `=~`, `(( ))` and `for (( ))` in Bash mode, and their absence
//! from the default one.

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

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn conditional_matches_patterns_not_fields() {
    let mut bash = shell(true);
    assert_eq!(run(&mut bash, b"[[ foo.py == *.py ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ foo.p == *.py ]]").0, 1);
    // A quoted right-hand side is text, and an unquoted expansion is not.
    assert_eq!(run(&mut bash, b"[[ foo.py == '*.py' ]]").0, 1);
    assert_eq!(run(&mut bash, b"p='*.py'; [[ foo.py == $p ]]").0, 0);
    assert_eq!(run(&mut bash, b"p='*.py'; [[ foo.py == \"$p\" ]]").0, 1);
    // No field splitting and no pathname expansion.
    assert_eq!(run(&mut bash, b"v='one two'; [[ 'one two' == $v ]]").0, 0);
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn conditional_truth_and_precedence() {
    let mut bash = shell(true);
    assert_eq!(run(&mut bash, b"[[ a ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ '' ]]").0, 1);
    assert_eq!(run(&mut bash, b"[[ ! '' ]]").0, 0);
    // `&&` binds tighter than `||`, unlike a command list.
    assert_eq!(run(&mut bash, b"[[ t || '' && '' ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ ( t || '' ) && '' ]]").0, 1);
    // A newline continues the expression.
    assert_eq!(run(&mut bash, b"[[ a == a\n&& b == b\n]]").0, 0);
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn conditional_compares_arithmetic_operands() {
    let mut bash = shell(true);
    assert_eq!(run(&mut bash, b"[[ 1+2 -eq 3 ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ 017 -eq 15 ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ 0x0f -eq 15 ]]").0, 0);
    // Unset names are zero on both sides, so both spellings are equal.
    assert_eq!(run(&mut bash, b"[[ a -eq b ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ '' -eq 0 ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ b > a ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ b < a ]]").0, 1);
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn regex_operator_reports_its_groups() {
    let mut bash = shell(true);
    let (status, stdout) = run(
        &mut bash,
        b"[[ foo123 =~ ([a-z]+)([0-9]+) ]]; echo $?-${BASH_REMATCH[0]}-${BASH_REMATCH[1]}-${BASH_REMATCH[2]}",
    );
    assert_eq!((status, stdout), (0, b"0-foo123-foo-123\n".to_vec()));
    // Quoting a metacharacter makes it text, in the operand and in a group.
    assert_eq!(run(&mut bash, b"[[ 'a b' =~ ^(a\\ b)$ ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ 'a b' =~ '^(a b)$' ]]").0, 1);
    assert_eq!(run(&mut bash, b"[[ bar =~ foo|bar ]]").0, 0);
    assert_eq!(run(&mut bash, b"[[ '|' =~ '|' ]]").0, 0);
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn a_bad_regex_answers_with_status_two() {
    let mut bash = shell(true);
    let (status, stdout) = run(&mut bash, b"[[ foo =~ * ]]; echo done=$?");
    assert_eq!((status, stdout), (0, b"done=2\n".to_vec()));
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn arithmetic_command_status_follows_its_value() {
    let mut bash = shell(true);
    assert_eq!(run(&mut bash, b"(( 1 ))").0, 0);
    assert_eq!(run(&mut bash, b"(( 0 ))").0, 1);
    assert_eq!(run(&mut bash, b"(( -1 ))").0, 0);
    assert_eq!(run(&mut bash, b"(( ))").0, 1);
    let (status, stdout) = run(&mut bash, b"(( x = 1 )); (( y = x + 2 )); echo $x $y");
    assert_eq!((status, stdout), (0, b"1 3\n".to_vec()));
    let (_, stdout) = run(&mut bash, b"a=(4 5 6); (( s = a[0] + a[2] )); echo $s");
    assert_eq!(stdout, b"10\n".to_vec());
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn bash_arithmetic_operators_and_side_effects() {
    let mut bash = shell(true);
    let (_, stdout) = run(&mut bash, b"i=5; echo $((i++)) $i $((++i)) $i");
    assert_eq!(stdout, b"5 6 7 7\n".to_vec());
    let (_, stdout) = run(&mut bash, b"echo $((2 ** 10)) $((-2 ** 2)) $((1, 2, 3))");
    assert_eq!(stdout, b"1024 -4 3\n".to_vec());
    let (_, stdout) = run(&mut bash, b"echo $((64#a)) $((16#ff))");
    assert_eq!(stdout, b"10 255\n".to_vec());
    // A name's value is itself an expression.
    let (_, stdout) = run(&mut bash, b"x=1+2; echo $((x)) $((x * 2))");
    assert_eq!(stdout, b"3 6\n".to_vec());
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn arithmetic_for_loop_runs_and_breaks() {
    let mut bash = shell(true);
    let (status, stdout) = run(
        &mut bash,
        b"for ((a = 1; a <= 5; a++)); do\n\
          if (( a == 2 )); then continue; fi\n\
          if (( a == 4 )); then break; fi\n\
          echo $a\n\
          done",
    );
    assert_eq!((status, stdout), (0, b"1\n3\n".to_vec()));
    // An omitted condition is true, and a brace group is a body.
    let (_, stdout) = run(
        &mut bash,
        b"i=0; for (( ; ; )) { i=$((i + 1)); if (( i == 3 )); then break; fi }; echo $i",
    );
    assert_eq!(stdout, b"3\n".to_vec());
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn the_default_dialect_keeps_posix_test_behavior() {
    let mut posix = shell(false);
    // `[[` is an ordinary word, so it is looked up as a command.
    assert_eq!(run(&mut posix, b"[[ a == a ]]").0, 127);
    // `test` still compares strings rather than patterns.
    assert_eq!(run(&mut posix, b"test foo.py = '*.py'").0, 1);
    assert_eq!(run(&mut posix, b"[ foo.py = foo.py ]").0, 0);
    assert_eq!(run(&mut posix, b"[ 017 -eq 17 ]").0, 0);
    let (_, stdout) = run(&mut posix, b"echo $((1 + 2))");
    assert_eq!(stdout, b"3\n".to_vec());
}

// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn posix_arithmetic_rejects_the_bash_operators() {
    for script in [
        b"i=1; echo $((i++))".as_slice(),
        b"echo $((2 ** 3))",
        b"echo $((1, 2))",
        b"echo $((64#a))",
        b"a=b; echo $((a))",
    ] {
        let mut posix = shell(false);
        let outcome = posix
            .run(script)
            .map_or(2, |status| i32::from(status.code()));
        assert_ne!(outcome, 0, "{:?} must fail", BStr::new(script));
    }
}
