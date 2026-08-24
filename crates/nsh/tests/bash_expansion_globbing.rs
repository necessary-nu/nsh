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

/// A here-string feeds one expanded word, plus a newline, to the
/// descriptor. The operator exists only in Bash mode.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn here_strings_feed_one_expanded_word() {
    let mut bash = shell(true);
    assert_eq!(run(&mut bash, b"cat <<< hello").1, b"hello\n".to_vec());
    assert_eq!(run(&mut bash, b"x='a b'; cat <<< $x").1, b"a b\n".to_vec());
    assert_eq!(
        run(&mut bash, b"cat <<< \"$(echo sub)\"").1,
        b"sub\n".to_vec()
    );
    // No splitting and no pathname expansion: one field, quotes removed.
    assert_eq!(run(&mut bash, b"cat <<< '*'").1, b"*\n".to_vec());
    assert_eq!(
        run(&mut bash, b"read a b <<< 'x y'; echo \"$a-$b\"").1,
        b"x-y\n".to_vec()
    );
    // The third `<` is only an operator in Bash mode.
    let mut posix = shell(false);
    assert_ne!(run(&mut posix, b"cat <<< hello").0, 0);
}

/// `$(<file)` reads the file into the word. Nothing runs, and nothing
/// re-reads the bytes as shell syntax.
// [dec:nsh:safety-trumps-compatibility]
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn dollar_paren_reads_a_file() {
    let mut bash = shell(true);
    assert_eq!(
        run(&mut bash, b"echo \"[$(</dev/null)]\"").1,
        b"[]\n".to_vec()
    );
    // A file that cannot be opened yields nothing rather than failing
    // the shell.
    assert_eq!(
        run(&mut bash, b"echo \"[$(<no/such/file)]\"").1,
        b"[]\n".to_vec()
    );
    // The shorthand is Bash's; elsewhere it runs a command with no words.
    let mut posix = shell(false);
    assert_eq!(
        run(&mut posix, b"echo \"[$(</dev/null)]\"").1,
        b"[]\n".to_vec()
    );
}

/// Bash expands a redirection operand as an ordinary word and refuses
/// the redirect unless exactly one field survives.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn redirection_operands_must_name_one_file() {
    let mut bash = shell(true);
    // Splitting, brace expansion and an expansion that yields nothing
    // all leave a count Bash will not accept.
    assert_eq!(run(&mut bash, b"f='a b'; echo hi > $f").0, 1);
    assert_eq!(run(&mut bash, b"echo hi > a-{one,two}").0, 1);
    assert_eq!(run(&mut bash, b"echo hi > $unset_name").0, 1);
    // An empty operand is one field, so it reaches the open and fails
    // there -- with Bash's status rather than dash's.
    assert_eq!(run(&mut bash, b"echo hi > ''").0, 1);
    assert_eq!(run(&mut bash, b"echo hi > /no/such/directory/f").0, 1);
    // The POSIX dialect neither splits the word nor takes status 1.
    let mut posix = shell(false);
    assert_eq!(run(&mut posix, b"echo hi > /no/such/directory/f").0, 2);
}

/// Indirect expansion resolves a name and then keeps going: a subscript,
/// a suffix operator and an `@`-transform all apply to the resolved
/// name, and a resolution that is not a name is refused.
// [dec:nsh:safety-trumps-compatibility]
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn indirection_reaches_past_the_name() {
    let mut bash = shell(true);
    assert_eq!(
        run(&mut bash, b"x=xx; xx=aaabcc; xd=x; echo ${!xd}").1,
        b"xx\n".to_vec()
    );
    // A subscript binds to the reference, not to the resolved name.
    assert_eq!(
        run(&mut bash, b"x=xx; a=(asdf x); echo ${!a[1]}").1,
        b"xx\n".to_vec()
    );
    // Suffix operators and transforms apply after the name resolves.
    assert_eq!(
        run(&mut bash, b"x=xx; xx=aaabcc; echo ${!x:2}").1,
        b"abcc\n".to_vec()
    );
    assert_eq!(
        run(&mut bash, b"x=xx; xx=aaabcc; echo ${!x#*a}").1,
        b"aabcc\n".to_vec()
    );
    assert_eq!(
        run(&mut bash, b"y=yy; yy=; echo ${!y:-foo}").1,
        b"foo\n".to_vec()
    );
    // `${!a[@]}` still lists subscripts, but an operator makes the same
    // spelling an indirection through `${a[@]}`.
    assert_eq!(
        run(&mut bash, b"v=val; declare -A r=([k]=v); echo ${!r[@]}").1,
        b"k\n".to_vec()
    );
    assert_eq!(
        run(&mut bash, b"v=val; declare -A r=([k]=v); echo ${!r[@]:1}").1,
        b"al\n".to_vec()
    );
    // A resolution that is not a name is refused with status 1.
    assert_eq!(run(&mut bash, b"a='bad var name'; echo ${!a}").0, 1);
    assert_eq!(run(&mut bash, b"b='/'; echo ${!b}").0, 1);
    assert_eq!(run(&mut bash, b"echo ${!never_set}").0, 1);
    // A name that holds an array has no scalar to read, and that is not
    // an error: it resolves to nothing.
    assert_eq!(
        run(&mut bash, b"declare -A m=([k]=v); echo \"[${!m}]\"").1,
        b"[]\n".to_vec()
    );
    // After `!`, a `#` can only be the whole target.
    assert_ne!(run(&mut bash, b"echo ${!#x}").0, 0);
}

/// `GLOBIGNORE` is colon-separated, but a colon inside a bracket
/// expression belongs to the pattern.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn globignore_keeps_bracket_expressions_whole() {
    let mut bash = shell(true);
    // Split on the inner colons, `[[:alpha:]]` would become three
    // patterns, none of which hides the name; hiding every match leaves
    // the pattern literal.
    assert_eq!(
        run(&mut bash, b"GLOBIGNORE='/dev/[[:alpha:]]*'; echo /dev/nul*").1,
        b"/dev/nul*\n".to_vec()
    );
    assert_eq!(
        run(&mut bash, b"GLOBIGNORE='/dev/[[:digit:]]*'; echo /dev/nul*").1,
        b"/dev/null\n".to_vec()
    );
    // The plain separator still separates.
    assert_eq!(
        run(&mut bash, b"GLOBIGNORE='/dev/zz:/dev/null'; echo /dev/nul*").1,
        b"/dev/nul*\n".to_vec()
    );
    // A bracket that never closes is ordinary text, so its colon splits.
    assert_eq!(
        run(&mut bash, b"GLOBIGNORE='/dev/[[:alpha:'; echo /dev/nul*").1,
        b"/dev/null\n".to_vec()
    );
}

/// A suffix operator or an `@`-transform reaches each element and the
/// join happens afterwards.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn star_operators_map_before_they_join() {
    let mut bash = shell(true);
    assert_eq!(
        run(&mut bash, b"a=('-x-' 'y-y' '-z-'); echo \"${a[*]#-}\"").1,
        b"x- y-y z-\n".to_vec()
    );
    assert_eq!(
        run(&mut bash, b"a=(a b); echo \"${a[*]@Q}\"").1,
        b"'a' 'b'\n".to_vec()
    );
    // `@Q` on a name that holds no readable element answers with nothing
    // rather than with an empty quotation.
    assert_eq!(
        run(&mut bash, b"declare -A A=([k]=v); echo \"[${A@Q}]\"").1,
        b"[]\n".to_vec()
    );
    assert_eq!(
        run(&mut bash, b"e=''; echo \"[${e@Q}]\"").1,
        b"['']\n".to_vec()
    );
    // `${!prefix@}` lists a name declared as an empty array.
    assert_eq!(
        run(&mut bash, b"hello1=1 hello2=2; hello=(); echo ${!hello@}").1,
        b"hello hello1 hello2\n".to_vec()
    );
}

/// `${x@Q}` and `printf %q` claim to produce text the shell reads back as
/// the original value. That claim is checkable against the shell itself,
/// and it is a security property rather than a cosmetic one: `@Q` is what
/// a script reaches for when it has to put untrusted data back into shell
/// syntax, so a value that escapes its quoting there is command
/// injection -- the data-to-syntax path
/// [dec:nsh:safety-trumps-compatibility] names.
///
/// `$'\E'` used to break it. The shell *emitted* `\E` from `@Q` -- Bash's
/// second spelling of `\e` -- and its own `$'...'` reader did not accept
/// it, so the shell could not read its own quoted output. Found by the
/// `quoting` fuzz target and minimised to one byte, `0x1b`.
// [spec:nsh:req:compat.bash.expansion-globbing/test]
#[test]
fn quoting_round_trips_for_any_byte() {
    let mut shell = shell(true);
    let hostile: Vec<Vec<u8>> = vec![
        b"\x1b".to_vec(),
        b"a\x1bb".to_vec(),
        b"a b".to_vec(),
        b"a'b".to_vec(),
        b"a\"b".to_vec(),
        b"a$(id)b".to_vec(),
        b"a`id`b".to_vec(),
        b"a\\b".to_vec(),
        b"a\tb".to_vec(),
        b"a\nb".to_vec(),
        b"a\rb".to_vec(),
        b"*".to_vec(),
        b"$x".to_vec(),
        b"!".to_vec(),
        b"".to_vec(),
        b"a;id;b".to_vec(),
        b"\x01\x02\x7f".to_vec(),
        b"\xff\xfe".to_vec(),
        b"-n".to_vec(),
        b"~root".to_vec(),
        (1u8..=255).collect(),
    ];

    for value in hostile {
        let (status, _) = run(
            &mut shell,
            format!(
                "X={}\n\
                 eval \"y=${{X@Q}}\"\n\
                 [ \"$y\" = \"$X\" ] || exit 9\n\
                 printf -v q '%q' \"$X\"\n\
                 eval \"z=$q\"\n\
                 [ \"$z\" = \"$X\" ] || exit 8\n",
                quote_for_test(&value),
            )
            .as_bytes(),
        );
        assert_eq!(
            status,
            0,
            "did not round-trip: {:?}",
            BStr::new(value.as_slice()),
        );
    }
}

/// Single-quote a byte string for embedding in a test script, the one way
/// that needs nothing from the shell under test.
fn quote_for_test(value: &[u8]) -> String {
    let mut out = String::from("$'");
    for byte in value {
        out.push_str(&format!("\\x{byte:02x}"));
    }
    out.push('\'');
    out
}
