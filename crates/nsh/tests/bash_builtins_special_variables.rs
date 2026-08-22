//! The Bash-only built-ins and the variables the shell maintains for a
//! script, checked through the public interface a shell embedder has.
//!
//! Each case that asserts a Bash behaviour also asserts what POSIX mode
//! does with the same script, because the two are one requirement: a
//! Bash-only name that leaked into the default dialect would be a
//! regression no survey in this repository would catch.

use bstr::BStr;
use nsh::{Shell, Streams};

/// Waiting for a child is a process-wide operation, so two shells in two
/// test threads can reap each other's children and read a status the
/// other one was waiting for. The cases here run one at a time.
static ONE_AT_A_TIME: std::sync::Mutex<()> = std::sync::Mutex::new(());

fn serialized() -> std::sync::MutexGuard<'static, ()> {
    ONE_AT_A_TIME
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

fn new_shell(bash: bool) -> Shell {
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
    drop(shell.take_captured_stderr());
    (status, stdout)
}

fn output(bash: bool, script: &[u8]) -> Vec<u8> {
    run(&mut new_shell(bash), script).1
}

// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn let_reports_the_last_expression_as_a_status() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"let x=1 y=x+2 z=y*3").0, 0);
    assert_eq!(run(&mut shell, b"echo $x $y $z").1, b"1 3 9\n");
    // `let` is false when the value is zero, which is the opposite sense
    // to an exit status and the reason this is not just `$(( ))`.
    assert_eq!(run(&mut shell, b"let q=0").0, 1);
    assert_eq!(run(&mut shell, b"let").0, 2);
    assert_eq!(run(&mut new_shell(false), b"let x=1").0, 127);
}

// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn type_reports_every_resolution_of_a_name() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"f() { :; }").0, 0);
    assert_eq!(run(&mut shell, b"type -t f").1, b"function\n");
    assert_eq!(run(&mut shell, b"type -t read").1, b"builtin\n");
    assert_eq!(run(&mut shell, b"type -t while").1, b"keyword\n");
    // A name with nothing behind it prints nothing and fails.
    assert_eq!(
        run(&mut shell, b"type -t no_such_command_at_all"),
        (1, vec![])
    );
    // `-P` searches the file system even for a name a built-in owns,
    // and reports failure when the search finds nothing.
    assert_eq!(run(&mut shell, b"type -P cd"), (1, vec![]));

    // POSIX's `type` has no options at all, so `-t` is a name to
    // describe rather than a request.
    assert_ne!(run(&mut new_shell(false), b"type -t read").0, 0);
}

// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn builtin_runs_the_name_a_function_took() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"echo() { printf 'shadowed\\n'; }").0, 0);
    assert_eq!(run(&mut shell, b"echo hello").1, b"shadowed\n");
    assert_eq!(run(&mut shell, b"builtin echo hello").1, b"hello\n");
    // `builtin` never leaves the registry, so an external name fails
    // rather than being searched for on the file system.
    assert_eq!(run(&mut shell, b"builtin cat").0, 127);
}

// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn enable_hides_a_builtin_from_command_lookup() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"type -t cd").1, b"builtin\n");
    assert_eq!(run(&mut shell, b"enable -n cd").0, 0);
    assert_eq!(run(&mut shell, b"type -t cd"), (1, vec![]));
    assert_eq!(run(&mut shell, b"enable cd").0, 0);
    assert_eq!(run(&mut shell, b"type -t cd").1, b"builtin\n");
    // One shell's `enable -n` is not another's.
    assert_eq!(run(&mut new_shell(true), b"type -t cd").1, b"builtin\n");
}

// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn read_fills_reply_an_array_and_a_count() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(
        run(
            &mut shell,
            b"printf 'a b\\n' | { read; echo \"[$REPLY]\"; }"
        )
        .1,
        b"[a b]\n"
    );
    assert_eq!(
        run(
            &mut shell,
            b"printf 'a b c\\n' | { read -a parts; echo ${#parts[@]} ${parts[2]}; }"
        )
        .1,
        b"3 c\n"
    );
    assert_eq!(
        run(
            &mut shell,
            b"printf 'abcxyz\\n' | { read -n 3; echo $REPLY; }"
        )
        .1,
        b"abc\n"
    );
    // `-N` counts characters and ignores the delimiter, so the record
    // is not split into fields at all.
    assert_eq!(
        run(
            &mut shell,
            b"printf 'a b c\\n' | { read -N 5 x y; echo \"[$x][$y]\"; }"
        )
        .1,
        b"[a b c][]\n"
    );
    assert_eq!(
        run(&mut shell, b"read -u 3 3<<'EOF'\nhi\nEOF\necho $REPLY").1,
        b"hi\n"
    );
}

// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn mapfile_stores_one_record_per_element() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(
        run(
            &mut shell,
            b"printf '1\\n2\\n3\\n' | { mapfile -t rows; echo ${#rows[@]} ${rows[1]}; }"
        )
        .1,
        b"3 2\n"
    );
    // The delimiter is kept unless `-t` asks for it to go, and
    // `readarray` is the same command under another name.
    assert_eq!(
        run(
            &mut shell,
            b"printf 'a:b:' | { readarray -d : rows; echo \"[${rows[0]}][${rows[1]}]\"; }"
        )
        .1,
        b"[a:][b:]\n"
    );
    assert_eq!(
        run(&mut new_shell(false), b"mapfile rows </dev/null").0,
        127
    );
}

// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn printf_can_assign_and_requote() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(
        run(&mut shell, b"printf -v dest '%s-%s' a b; echo $dest").1,
        b"a-b\n"
    );
    assert_eq!(
        run(&mut shell, b"printf '%q\\n' 'one two'").1,
        b"one\\ two\n"
    );
    assert_eq!(
        run(&mut shell, b"printf '%q\\n' \"$(printf 'a\\nb')\"").1,
        b"$'a\\nb'\n"
    );
    // POSIX `printf` has no options and no `%q`.
    assert_ne!(run(&mut new_shell(false), b"printf -v dest x").0, 0);
}

// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn the_directory_stack_tracks_the_working_directory() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"cd /").0, 0);
    assert_eq!(run(&mut shell, b"pushd /tmp").1, b"/tmp /\n");
    assert_eq!(run(&mut shell, b"dirs").1, b"/tmp /\n");
    // `cd` replaces the top entry, because the top entry is `$PWD`.
    assert_eq!(run(&mut shell, b"cd /; dirs").1, b"/ /\n");
    assert_eq!(run(&mut shell, b"popd").1, b"/\n");
    assert_eq!(run(&mut shell, b"popd").0, 1);
    assert_eq!(run(&mut new_shell(false), b"pushd /tmp").0, 127);
}

/// The facts a Bash script reads about the shell it is running in.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn published_facts_describe_this_shell() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    let version = run(&mut shell, b"echo $BASH_VERSION ${#BASH_VERSINFO[@]}").1;
    assert!(version.starts_with(b"5."), "{version:?}");
    assert!(version.ends_with(b" 6\n"), "{version:?}");
    assert_eq!(run(&mut shell, b"echo $BASH_SUBSHELL").1, b"0\n");
    assert_eq!(run(&mut shell, b"( echo $BASH_SUBSHELL )").1, b"1\n");
    assert_eq!(
        run(&mut shell, b"[ -n \"$OSTYPE$MACHTYPE$HOSTTYPE\" ]").0,
        0
    );
    assert_eq!(run(&mut shell, b"[ -n \"$UID$EUID\" ]").0, 0);
    // None of them exist in the default dialect.
    assert_eq!(
        output(false, b"echo [$BASH_VERSION][$OSTYPE][$UID]"),
        b"[][][]\n"
    );
}

/// `SECONDS` counts from where an assignment put it; `EPOCHSECONDS`
/// reports the wall clock; `RANDOM` is not replayable from a seed.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn clocks_and_the_generator_are_recomputed_on_read() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"SECONDS=100; echo $SECONDS").1, b"100\n");
    assert_eq!(
        run(&mut shell, b"[ \"$EPOCHSECONDS\" -gt 1600000000 ]").0,
        0
    );
    assert_eq!(
        run(
            &mut shell,
            b"case $EPOCHREALTIME in *.*) echo dotted;; esac"
        )
        .1,
        b"dotted\n"
    );

    let draws = run(
        &mut shell,
        b"RANDOM=42; a=$RANDOM; RANDOM=42; b=$RANDOM; c=$RANDOM; echo $((a!=b||b!=c))",
    )
    .1;
    assert_eq!(draws, b"1\n", "a seed must not replay a sequence");
}

/// `${PIPESTATUS[@]}` reports every member of the pipeline that just
/// ran, and a command with no `|` is a pipeline of one.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn pipeline_status_reports_every_member() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"false; echo ${PIPESTATUS[@]}").1, b"1\n");
    assert_eq!(
        run(&mut shell, b"true | false | true; echo ${PIPESTATUS[@]}").1,
        b"0 1 0\n"
    );
    // POSIX mode has no array syntax to read it with, and no entry to
    // read: `$PIPESTATUS` is an ordinary unset name there.
    assert_eq!(output(false, b"false; echo [$PIPESTATUS]"), b"[]\n");
}

/// `${name@a}` reads the declaration rather than the value.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn attribute_transform_reads_the_declaration() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"a=(one two); echo ${a@a}").1, b"a\n");
    assert_eq!(run(&mut shell, b"declare -r a; echo ${a@a}").1, b"ar\n");
    assert_eq!(
        run(&mut shell, b"declare -Ax m=([k]=v); echo ${m@a}").1,
        b"Ax\n"
    );
    // A name with no declaration has no letters, and is not an error.
    assert_eq!(
        run(&mut shell, b"echo [${nothing@a}]"),
        (0, b"[]\n".to_vec())
    );
}

/// `SHELLOPTS` and `BASHOPTS` answer for the option tables, so an
/// assignment to either cannot make them answer for the script.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn option_listings_follow_the_option_tables() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(
        run(
            &mut shell,
            b"case $SHELLOPTS in *xtrace*) echo on;; *) echo off;; esac"
        )
        .1,
        b"off\n"
    );
    assert_eq!(
        run(
            &mut shell,
            b"set -o noglob; case $SHELLOPTS in *noglob*) echo on;; *) echo off;; esac"
        )
        .1,
        b"on\n"
    );
    assert_eq!(run(&mut shell, b"SHELLOPTS=nonsense; echo $?").1, b"0\n");
    assert_eq!(
        run(
            &mut shell,
            b"case $SHELLOPTS in nonsense) echo taken;; *) echo ignored;; esac"
        )
        .1,
        b"ignored\n"
    );
    assert_eq!(
        run(
            &mut shell,
            b"shopt -s progcomp; case $BASHOPTS in *progcomp*) echo on;; esac"
        )
        .1,
        b"on\n"
    );
    assert_eq!(output(false, b"echo [$SHELLOPTS][$BASHOPTS]"), b"[][]\n");
}

/// `test -v` and `test -R` are Bash's, and admitting them in POSIX mode
/// would change what `test -v` already means there.
// [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
#[test]
fn variable_predicates_are_dialect_gated() {
    let _guard = serialized();
    let mut shell = new_shell(true);
    assert_eq!(run(&mut shell, b"x=1; test -v x; echo $?").1, b"0\n");
    assert_eq!(run(&mut shell, b"test -v nothing; echo $?").1, b"1\n");
    assert_eq!(
        run(&mut shell, b"a=(p q); [[ -v a[1] ]]; echo $?").1,
        b"0\n"
    );
    assert_eq!(run(&mut shell, b"[[ -v a[9] ]]; echo $?").1, b"1\n");
    assert_eq!(
        run(&mut shell, b"declare -n r=x; [[ -R r ]]; echo $?").1,
        b"0\n"
    );

    // POSIX's one-argument `test` asks whether the string `-v` is
    // non-empty, which it is.
    assert_eq!(output(false, b"test -v; echo $?"), b"0\n");
}

/// Bash's `echo` prints its argument; dash's decodes escapes in it.
// [spec:nsh:req:compat.bash.builtins-special-variables/test]
#[test]
fn echo_decodes_escapes_only_when_asked() {
    let _guard = serialized();
    assert_eq!(output(true, br"echo 'a\tb'"), b"a\\tb\n");
    assert_eq!(output(true, br"echo -e 'a\tb'"), b"a\tb\n");
    assert_eq!(output(true, br"echo -n -e 'a\tb'"), b"a\tb");
    assert_eq!(output(false, br"echo 'a\tb'"), b"a\tb\n");
}
