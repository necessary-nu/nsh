use bstr::BStr;
use nsh::{Shell, Streams};

fn shell(bash: bool) -> Shell {
    Shell::builder()
        .streams(Streams::capture().expect("create capture streams"))
        .option(BStr::new(b"bash"), bash)
        .build()
        .expect("build shell")
}

fn run(shell: &mut Shell, script: &[u8]) -> (i32, Vec<u8>, Vec<u8>) {
    let status = shell.run(script).expect("run script").code().into();
    let stdout = shell
        .take_captured_stdout()
        .expect("capture stdout")
        .to_vec();
    let stderr = shell
        .take_captured_stderr()
        .expect("capture stderr")
        .to_vec();
    (status, stdout, stderr)
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
#[test]
fn shopt_mode_isolation() {
    let mut bash = shell(true);
    let mut other_bash = shell(true);
    let mut posix = shell(false);

    let (status, stdout, _) = run(&mut bash, b"type shopt");
    assert_eq!(status, 0);
    assert!(
        stdout
            .windows(b"shell builtin".len())
            .any(|part| part == b"shell builtin")
    );

    let (status, _, _) = run(&mut posix, b"shopt");
    assert_eq!(status, 127);

    assert_eq!(run(&mut bash, b"shopt -s expand_aliases").0, 0);
    assert_eq!(run(&mut other_bash, b"shopt -q expand_aliases").0, 1);
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
#[test]
fn dialect_change_invalidates_cache() {
    let mut shell = shell(true);
    assert_eq!(run(&mut shell, b"type shopt").0, 0);
    assert_eq!(run(&mut shell, b"set +o bash").0, 0);
    assert_ne!(run(&mut shell, b"type shopt").0, 0);
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
#[test]
fn restorable_option_state() {
    let mut shell = shell(true);

    let (status, set_state, _) = run(&mut shell, b"set +o");
    assert_eq!(status, 0);
    /* In Bash mode the dialect switch is reported as `posix`, off --
     * `[spec:nsh:req:compat.bash.posix-option]`. What matters is that the
     * saved state names the switch at all, so re-executing it restores the
     * dialect, which the round trip below checks. */
    assert!(
        set_state
            .split(|byte| *byte == b'\n')
            .any(|line| line == b"set +o posix")
    );
    assert_eq!(run(&mut shell, &set_state).0, 0);
    assert_eq!(run(&mut shell, b"set +o").1, set_state);

    let (status, human_state, _) = run(&mut shell, b"set -o");
    assert_eq!(status, 0);
    assert!(
        human_state
            .split(|byte| *byte == b'\n')
            .any(|line| line.starts_with(b"posix") && line.ends_with(b"off"))
    );

    let (status, stdout, _) = run(&mut shell, b"shopt -o -p bash");
    assert_eq!((status, stdout), (0, b"set -o bash\n".to_vec()));

    let (status, off, _) = run(&mut shell, b"shopt -p expand_aliases");
    assert_eq!(status, 1);
    assert_eq!(off, b"shopt -u expand_aliases\n");
    assert_eq!(run(&mut shell, b"shopt -s expand_aliases").0, 0);
    let (status, on, _) = run(&mut shell, b"shopt -p expand_aliases");
    assert_eq!(status, 0);
    assert_eq!(on, b"shopt -s expand_aliases\n");
    assert_eq!(run(&mut shell, &off).0, 0);
    assert_eq!(run(&mut shell, b"shopt -q expand_aliases").0, 1);

    let (status, _, _) = run(&mut shell, b"shopt -s expand_aliases not_an_option");
    assert_eq!(status, 1);
    assert_eq!(run(&mut shell, b"shopt -q expand_aliases").0, 0);
}

// [spec:nsh:req:compat.bash.options-builtins-dispatch/test]
#[test]
fn expand_aliases_mode_defaults() {
    let mut bash = shell(true);
    assert_eq!(run(&mut bash, b"alias answer='printf bash-alias'").0, 0);
    assert_eq!(run(&mut bash, b"answer").0, 127);
    assert_eq!(run(&mut bash, b"shopt -s expand_aliases").0, 0);
    assert_eq!(run(&mut bash, b"answer").1, b"bash-alias");

    let mut posix = shell(false);
    assert_eq!(run(&mut posix, b"alias answer='printf posix-alias'").0, 0);
    assert_eq!(run(&mut posix, b"answer").1, b"posix-alias");
}
