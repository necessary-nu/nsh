//! Brace expansion, Bash parameter operators, extended globs, `globstar`
//! and the glob-related shell options — and their absence from the
//! default mode.

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
    // A refused expansion is a value here, not a panic: half of what
    // this file checks is which words the shell declines to expand.
    let status = shell
        .run(script)
        .unwrap_or_else(|error| error.status())
        .code()
        .into();
    let stdout = shell
        .take_captured_stdout()
        .expect("capture stdout")
        .to_vec();
    shell.take_captured_stderr().expect("capture stderr");
    (status, stdout)
}

fn output(bash: bool, script: &[u8]) -> String {
    let mut shell = shell(bash);
    String::from_utf8_lossy(&run(&mut shell, script).1).into_owned()
}

// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn braces_multiply_alternatives_and_ranges() {
    assert_eq!(output(true, b"echo {a,b}"), "a b\n");
    assert_eq!(output(true, b"echo pre{a,b}post"), "preapost prebpost\n");
    assert_eq!(output(true, b"echo a{b,c}d{e,f}"), "abde abdf acde acdf\n");
    assert_eq!(output(true, b"echo {1..5}"), "1 2 3 4 5\n");
    assert_eq!(output(true, b"echo {5..1..2}"), "5 3 1\n");
    assert_eq!(output(true, b"echo {01..3}"), "01 02 03\n");
    assert_eq!(output(true, b"echo {a..e..2}"), "a c e\n");
    // A sequence with one term is still an expansion, so the braces go.
    assert_eq!(output(true, b"echo {1..1}-"), "1-\n");
}

/// The forms that look like brace expansions but are not, and the
/// quoting that takes the meaning away.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn braces_without_an_alternative_stay_literal() {
    assert_eq!(output(true, b"echo {a}"), "{a}\n");
    assert_eq!(output(true, b"echo {}"), "{}\n");
    assert_eq!(output(true, b"echo {a,b"), "{a,b\n");
    assert_eq!(output(true, b"echo '{a,b}'"), "{a,b}\n");
    assert_eq!(output(true, b"echo \"{a,b}\""), "{a,b}\n");
    assert_eq!(output(true, b"echo {a\\,b}"), "{a,b}\n");
    // The scan resumes after a brace that turned out to be ordinary.
    assert_eq!(output(true, b"echo a{b}c{d,e}"), "a{b}cd a{b}ce\n");
    assert_eq!(output(true, b"echo x{{a,b}}"), "x{a} x{b}\n");
    // An expansion is one unit, so its own braces are never the word's.
    assert_eq!(output(true, b"v=1; echo ${v}"), "1\n");
}

/// Braces multiply, so a short word can ask for an astronomical field
/// count. The count is charged before anything is built.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn brace_cardinality_is_bounded() {
    let mut bash = shell(true);
    let mut script = b"echo ".to_vec();
    for _ in 0..40 {
        script.extend_from_slice(b"{a,b}");
    }
    let (status, stdout) = run(&mut bash, &script);
    assert_ne!(status, 0, "an unbounded product must be refused");
    assert!(stdout.is_empty(), "no field list is built before the check");
    // The shell keeps running afterwards.
    assert_eq!(run(&mut bash, b"echo {a,b}"), (0, b"a b\n".to_vec()));
}

// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn parameter_operators_slice_substitute_and_recase() {
    assert_eq!(output(true, b"x=abcdefg; echo ${x:1:3}"), "bcd\n");
    assert_eq!(output(true, b"x=abcdefg; echo ${x: -4:3}"), "def\n");
    // A negative length is an end position, not a count.
    assert_eq!(output(true, b"x=abcdefg; echo ${x:3:-1}"), "def\n");
    assert_eq!(output(true, b"x=abcdefg; echo ${x:100:3}-"), "-\n");
    assert_eq!(
        output(true, b"i=1; x=abcdefg; echo ${x: i+4-2 : i+2}"),
        "def\n"
    );

    assert_eq!(output(true, b"s=xx_xx_xx; echo ${s/xx?/yy_}"), "yy_xx_xx\n");
    assert_eq!(
        output(true, b"s=xx_xx_xx; echo ${s//xx?/yy_}"),
        "yy_yy_xx\n"
    );
    assert_eq!(
        output(true, b"s=xx_xx_xx; echo ${s/#?xx/_yy}"),
        "xx_xx_xx\n"
    );
    assert_eq!(
        output(true, b"s=xx_xx_xx; echo ${s/%?xx/_yy}"),
        "xx_xx_yy\n"
    );
    // The replacement is longest-match, and an empty pattern changes
    // nothing.
    assert_eq!(
        output(true, b"s='begin <html></html> end'; echo ${s/<*>/[]}"),
        "begin [] end\n"
    );
    assert_eq!(output(true, b"v=abcde; echo -${v/}-"), "-abcde-\n");
    // The first byte of the pattern is literal, so `${x///}` replaces a
    // slash rather than naming an empty pattern.
    assert_eq!(output(true, b"y=/_/; echo ${y////c} ${y///}"), "c_c _\n");

    assert_eq!(
        output(true, b"x='ABC def'; echo ${x,} / ${x,,}"),
        "aBC def / abc def\n"
    );
    assert_eq!(
        output(true, b"x='abc DEF'; echo ${x^} / ${x^^}"),
        "Abc DEF / ABC DEF\n"
    );
    assert_eq!(output(true, b"x='AAA ABC'; echo ${x,,A}"), "aaa aBC\n");
    assert_eq!(
        output(true, b"x='abc DEF'; echo ${x@u} / ${x@U} / ${x@L}"),
        "Abc DEF / ABC DEF / abc def\n"
    );
}

// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn indirection_reads_the_named_variable() {
    assert_eq!(output(true, b"a=b; b=c; echo ${!a} ${a}"), "c b\n");
    assert_eq!(output(true, b"ref=x; echo ${!ref-default}"), "default\n");
    assert_eq!(output(true, b"ref=x; x=foo; echo ${!ref-default}"), "foo\n");
    assert_eq!(
        output(true, b"set -- one two; r='@'; echo ${!r}"),
        "one two\n"
    );
    assert_eq!(output(true, b"a=(p q); r='a[1]'; echo ${!r}"), "q\n");
    // `${!name[@]}` names the subscripts, and `${!prefix@}` the
    // variables, rather than reading either.
    assert_eq!(output(true, b"a=(p q r); echo ${!a[@]}"), "0 1 2\n");
    assert_eq!(output(true, b"ZOO=1 ZIP=2 z=3; echo ${!Z@}"), "ZIP ZOO\n");
    assert_eq!(output(true, b"ZOO=1 ZIP=2; echo \"${!Z*}\""), "ZIP ZOO\n");
}

// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn extended_globs_need_their_option_outside_a_conditional() {
    // `[[ ]]` has extended globs whether or not the option is on.
    assert_eq!(
        output(true, b"[[ --help == --@(help|verbose) ]] && echo yes"),
        "yes\n"
    );
    assert_eq!(output(true, b"[[ -- == --?(help) ]] && echo yes"), "yes\n");
    assert_eq!(output(true, b"[[ hh == +(h) ]] && echo yes"), "yes\n");
    assert_eq!(output(true, b"[[ '' == *(h) ]] && echo yes"), "yes\n");
    assert_eq!(output(true, b"[[ x == !(h) ]] && echo yes"), "yes\n");
    assert_eq!(output(true, b"[[ h == !(h) ]] || echo no"), "no\n");
    // Anchored at both ends, and nesting works.
    assert_eq!(output(true, b"[[ foo_ == @(foo) ]] || echo no"), "no\n");
    assert_eq!(
        output(
            true,
            b"[[ --verbose=2 == --@(help|verbose=@(1|2)) ]] && echo yes"
        ),
        "yes\n"
    );
    // `case` waits for `shopt -s extglob`.
    assert_eq!(
        output(
            true,
            b"shopt -s extglob\ncase abc in @(abc|x)) echo yes;; esac"
        ),
        "yes\n"
    );
    assert_eq!(
        output(true, b"shopt -s extglob\nx=abc; echo ${x/@(a|b)/Z}"),
        "Zbc\n"
    );
}

/// A negated group whose continuation is revisited at the same offsets
/// must not read another context's answer out of the match memo.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn negated_groups_do_not_share_a_memo_answer() {
    // The same continuation `x` is asked about from inside and outside a
    // negated group, at overlapping subject offsets.
    assert_eq!(output(true, b"[[ ax == !(a)x ]] || echo no"), "no\n");
    assert_eq!(output(true, b"[[ bx == !(a)x ]] && echo yes"), "yes\n");
    assert_eq!(output(true, b"[[ aax == !(a)x ]] && echo yes"), "yes\n");
    assert_eq!(output(true, b"[[ aa == @(a)@(a) ]] && echo yes"), "yes\n");
    assert_eq!(output(true, b"[[ aa == !(a)!(a) ]] && echo yes"), "yes\n");
}

/// Every form this node adds is inert while Bash mode is off: the words
/// survive unexpanded, and the shell reports no error for them.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn the_default_mode_keeps_every_new_form_literal() {
    let matrix: &[(&[u8], &str)] = &[
        (b"echo {a,b}", "{a,b}\n"),
        (b"echo {1..3}", "{1..3}\n"),
        (b"echo a{b,c}d", "a{b,c}d\n"),
        (b"x=abcdefg; echo ${x:1:3}", ""),
        (b"x=abc; echo ${x/a/z}", ""),
        (b"x=abc; echo ${x//a/z}", ""),
        (b"x=abc; echo ${x^^}", ""),
        (b"x=abc; echo ${x,,}", ""),
        (b"x=abc; echo ${x@Q}", ""),
        (b"a=b; b=c; echo ${!a}", ""),
        (b"echo $[1+2]", "$[1+2]\n"),
        (b"echo $\"hi\"", "$hi\n"),
    ];
    for (script, expected) in matrix {
        let mut posix = shell(false);
        let (status, stdout) = run(&mut posix, script);
        let stdout = String::from_utf8_lossy(&stdout);
        assert_eq!(
            stdout,
            *expected,
            "default mode changed for {}",
            BStr::new(script)
        );
        if expected.is_empty() {
            assert_ne!(
                status,
                0,
                "{} should be a bad substitution",
                BStr::new(script)
            );
        } else {
            assert_eq!(status, 0, "{} should succeed", BStr::new(script));
        }
    }
}

/// The glob options and `GLOBIGNORE` exist only in Bash mode, and the
/// `shopt` namespace refuses to name them otherwise.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn glob_options_are_absent_from_the_default_mode() {
    let mut posix = shell(false);
    // `shopt` itself is a Bash-mode builtin, so the names cannot even be
    // reached; the option list is unchanged by their absence.
    assert_ne!(run(&mut posix, b"shopt -s extglob").0, 0);
    assert_ne!(run(&mut posix, b"shopt -s nullglob").0, 0);
    // A `**` component is two stars, and a leading dot still hides a name.
    assert_eq!(
        run(&mut posix, b"GLOBIGNORE='*'; echo /dev/nul*").1,
        b"/dev/null\n".to_vec()
    );

    let mut bash = shell(true);
    assert_eq!(run(&mut bash, b"shopt -s extglob").0, 0);
    assert_eq!(run(&mut bash, b"shopt -q extglob").0, 0);
    assert_eq!(run(&mut bash, b"shopt -u extglob; shopt -q extglob").0, 1);
    for name in [
        b"dotglob".as_slice(),
        b"failglob",
        b"globstar",
        b"nocaseglob",
        b"nocasematch",
        b"nullglob",
    ] {
        let mut script = b"shopt -s ".to_vec();
        script.extend_from_slice(name);
        assert_eq!(
            run(&mut bash, &script).0,
            0,
            "{} should be a known shopt name",
            BStr::new(name)
        );
    }
}
