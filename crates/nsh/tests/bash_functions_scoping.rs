//! Bash function forms, dynamic scoping, declaration attributes, and
//! name references, including what changes when the dialect does.

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

/// Both Bash spellings define an ordinary function, and the definition
/// survives being redefined while a call of it is still running.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn both_bash_function_forms_define_a_function() {
    expect(
        b"function bare { echo bare:$#:$1; }\n\
          function parens() { echo parens:$#:$*; }\n\
          bare one\n\
          parens one two\n\
          function replace { echo first; replace() { echo second; }; echo still-first; }\n\
          replace\n\
          replace\n",
        b"bare:1:one\nparens:2:one two\nfirst\nstill-first\nsecond\n",
    );
}

/// A call installs its own positional frame and puts the caller's back.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_call_owns_its_positional_frame() {
    expect(
        b"set -- outer1 outer2\n\
          function inner { echo in:$#:$1:$2; shift; echo shifted:$#:$1; }\n\
          inner a b c\n\
          echo out:$#:$1:$2\n",
        b"in:3:a:b\nshifted:2:b\nout:2:outer1:outer2\n",
    );
}

/// A local is visible to everything the function calls and is restored
/// when the frame that declared it returns.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn locals_nest_and_stay_dynamically_visible() {
    expect(
        b"inner() { echo inner=$v; v=changed; }\n\
          middle() { local v=middle; inner; echo middle=$v; }\n\
          outer() { local v=outer; middle; echo outer=$v; }\n\
          v=global\n\
          outer\n\
          echo global=$v\n",
        b"inner=middle\nmiddle=changed\nouter=outer\nglobal=global\n",
    );
}

/// Recursion gets one frame per call, and `return` ends only its own.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn recursion_and_return_unwind_one_frame() {
    expect(
        b"function down { local n=$1; if [ \"$n\" -gt 0 ]; then down $(( n - 1 )); fi; echo n=$n; }\n\
          down 3\n\
          function early { echo before; return 7; echo after; }\n\
          early\n\
          echo status=$?\n\
          function looping { while true; do return 4; done; echo unreachable; }\n\
          looping\n\
          echo loop=$?\n",
        b"n=0\nn=1\nn=2\nn=3\nbefore\nstatus=7\nloop=4\n",
    );
}

/// A declaration in a function body is local unless `-g` sends it out.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_declaration_is_local_inside_a_function() {
    expect(
        b"d=outer\n\
          function scoped { declare d=inner; echo in=$d; }\n\
          scoped\n\
          echo out=$d\n\
          function global { declare -g g=set; declare l=hidden; }\n\
          global\n\
          echo g=$g l=[$l]\n\
          function fresh { local d; echo fresh=[$d]; }\n\
          fresh\n\
          echo restored=$d\n",
        b"in=inner\nout=outer\ng=set l=[]\nfresh=[]\nrestored=outer\n",
    );
}

/// `-i`, `-u`, `-l`, `-x`, and `+x` change how a later value is stored.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn declaration_attributes_shape_later_assignments() {
    expect(
        b"declare -i n\n\
          n='2 + 3'\n\
          echo n=$n\n\
          declare -u up=abc\n\
          echo up=$up\n\
          declare -l low=ABC\n\
          low=DEF\n\
          echo low=$low\n\
          declare -x sent=here\n\
          declare -p sent\n\
          declare +x sent\n\
          declare -p sent\n\
          declare undecided\n\
          declare -p undecided\n",
        b"n=5\nup=ABC\nlow=def\ndeclare -x sent=\"here\"\n\
              declare -- sent=\"here\"\ndeclare -- undecided\n",
    );
}

/// A name reference redirects reads, writes, and `unset`, and follows a
/// chain of references to the variable at its end.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_name_reference_redirects_every_access() {
    expect(
        b"target=first\n\
          declare -n ref=target\n\
          echo read=$ref\n\
          ref=second\n\
          echo through=$target\n\
          declare -n chain=ref\n\
          echo chain=$chain\n\
          arr=(zero one two)\n\
          declare -n element='arr[1]'\n\
          echo element=$element\n\
          declare -n whole=arr\n\
          whole[2]=replaced\n\
          echo array=${arr[2]}\n\
          unset ref\n\
          echo unset=[$target]\n",
        b"read=first\nthrough=second\nchain=second\nelement=one\n\
              array=replaced\nunset=[]\n",
    );
}

/// A `local -n` reference resolves in the caller's dynamic scope, which
/// is what makes passing a local array by name work.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_local_reference_uses_the_callers_scope() {
    expect(
        b"function assign_to { local -n slot=$1; slot=$2; }\n\
          function holder { local held=before; assign_to held after; echo held=$held; }\n\
          holder\n\
          function read_from { local -n view=$1; local index=$2; echo element=${view[$index]}; }\n\
          shared=(ga bu zo meu)\n\
          read_from shared 2\n\
          echo outer=[$slot]\n",
        b"held=after\nelement=zo\nouter=[]\n",
    );
}

/// A reference over something that is not a name is not followed, and a
/// circular chain reads as unset rather than looping.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn an_unusable_reference_is_not_followed() {
    expect(
        b"plain=1\n\
          declare -n plain\n\
          echo plain=$plain\n\
          declare -n one=two\n\
          declare -n two=one\n\
          echo cycle=[$one]\n\
          declare -n empty\n\
          echo empty=[$empty]\n\
          value=filled\n\
          empty=value\n\
          echo filled=$empty\n",
        b"plain=1\ncycle=[]\nempty=[]\nfilled=filled\n",
    );
}

/// The Bash spellings and the Bash `local` flags exist only in Bash
/// mode; the POSIX dialect keeps `local`'s flag-free reading.
// [spec:nsh:req:compat.bash.functions-scoping/test]
// [spec:nsh:req:compat.bash.default-isolation/test]
#[test]
fn the_dialect_selects_the_declaration_language() {
    let mut posix = shell(false);
    // `function` is an ordinary word in the POSIX grammar, so the body
    // that follows it never parses as a definition.
    assert!(posix.run(b"function only_bash { echo no; }").is_err());
    posix.take_captured_stdout().expect("capture stdout");
    posix.take_captured_stderr().expect("capture stderr");
    assert_eq!(run(&mut posix, b"declare -i n").0, 127);
    assert_eq!(
        run(&mut posix, b"posix_form() { echo posix; }\nposix_form").1,
        b"posix\n"
    );

    let mut bash = shell(true);
    assert_eq!(
        run(&mut bash, b"function only_bash { echo yes; }\nonly_bash").1,
        b"yes\n"
    );
    assert_eq!(run(&mut bash, b"declare -i n\nn=4/2\necho $n").1, b"2\n");

    // Turning the dialect off mid-session takes the built-in away
    // without disturbing what it already declared.
    assert_eq!(run(&mut bash, b"set +o bash").0, 0);
    assert_eq!(run(&mut bash, b"echo $n").1, b"2\n");
    assert_eq!(run(&mut bash, b"declare -i m").0, 127);
}

/// `declare -f` prints a body in Bash's canonical layout, and `type`
/// prints the same text under its own sentence.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_definition_prints_in_bash_layout() {
    expect(
        b"f () { echo; }\ndeclare -f f\n",
        b"f () \n{ \n    echo\n}\n",
    );
    expect(
        b"f () { echo; }\ntype -a f\n",
        b"f is a function\nf () \n{ \n    echo\n}\n",
    );
    expect(
        b"outer() { echo a; if x; then echo b; fi; }\ndeclare -f outer\n",
        b"outer () \n{ \n    echo a;\n    if x; then\n        echo b;\n    fi\n}\n",
    );
}

/// The printed text is source: re-reading it defines the same function,
/// and printing that one again gives the same bytes.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_printed_definition_reads_back_as_itself() {
    let script: &[u8] = b"f() { echo 'a  b' \"$1\" un*q; case $x in p|q) echo m;; esac; }\n\
                          code=$(declare -f f)\n\
                          unset -f f\n\
                          eval \"$code\"\n\
                          f one\n\
                          test \"$code\" = \"$(declare -f f)\" && echo stable\n";
    expect(script, b"a  b one un*q\nstable\n");
}

/// A body that is not a brace group is still printed as one, which is
/// what Bash does and what makes every definition re-readable.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn a_subshell_body_prints_inside_braces() {
    expect(
        b"f() ( echo sub )\ndeclare -f f\n",
        b"f () \n{ \n    ( echo sub )\n}\n",
    );
}

/// `-F` answers with names, and adds the definition's line and file once
/// `extdebug` asks for them. Neither form declares anything: `declare -f
/// x` must not bring an `x` into being.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn the_name_listing_reports_a_definitions_origin() {
    expect(
        b"b() { :; }\na() { :; }\ndeclare -F\n",
        b"declare -f a\ndeclare -f b\n",
    );
    expect(b"ble/foo() { :; }\ndeclare -F ble/foo\n", b"ble/foo\n");
    expect(
        b"shopt -s extdebug\nf() { :; }\ndeclare -F f\n",
        b"f 2 main\n",
    );

    let mut shell = shell(true);
    assert_eq!(run(&mut shell, b"declare -f nosuch").0, 1);
    assert_eq!(run(&mut shell, b"declare -F nosuch").0, 1);
    assert_eq!(run(&mut shell, b"declare -p nosuch 2>/dev/null").0, 1);
}

/// Bash lets one `unset` reach either table, so a name with no variable
/// behind it unsets the function.
// [spec:nsh:req:compat.bash.functions-scoping/test]
#[test]
fn unset_reaches_the_function_table() {
    expect(
        b"foo() { echo bar; }\nfoo\nunset foo\ndeclare -F\necho gone\n",
        b"bar\ngone\n",
    );
    // A variable of the same name is what `unset` takes first.
    expect(
        b"foo=v\nfoo() { echo fn; }\nunset foo\necho \"[$foo]\"\nfoo\n",
        b"[]\nfn\n",
    );
}
