//! `<(list)` and `>(list)`: the name they substitute, how long it lives,
//! what the child they fork is and is not, and their absence from the
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
    // A refused substitution is a value here, not a panic: half of what
    // this file checks is which words the shell declines to substitute.
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

fn names_a_descriptor(word: &str) -> bool {
    let Some(number) = word
        .strip_prefix("/dev/fd/")
        .or_else(|| word.strip_prefix("/proc/self/fd/"))
    else {
        return false;
    };
    !number.is_empty() && number.bytes().all(|byte| byte.is_ascii_digit())
}

fn scratch_path(tag: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("nsh-psub-{tag}-{}", std::process::id()))
}

/// Wait for a substitution's child to finish its side of the pipe.
///
/// The shell deliberately does not wait for one, so the test does. A bound
/// rather than a spin: a substitution that never delivers is a failure, not
/// a hang.
fn contents_within(path: &std::path::Path, deadline: std::time::Duration) -> String {
    let start = std::time::Instant::now();
    loop {
        if let Ok(text) = std::fs::read_to_string(path)
            && !text.is_empty()
        {
            return text;
        }
        if start.elapsed() >= deadline {
            return String::new();
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
}

// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn input_substitution_names_a_readable_pipe() {
    assert_eq!(
        output(true, b"read line < <(echo hi); echo \"$line\""),
        "hi\n"
    );
    // The name is a word like any other: it concatenates, and one command
    // may carry several.
    let word = output(true, b"echo <(true)");
    assert!(names_a_descriptor(word.trim_end()), "{word:?}");
    let joined = output(true, b"echo pre<(true)post");
    let middle = joined
        .trim_end()
        .strip_prefix("pre")
        .and_then(|rest| rest.strip_suffix("post"))
        .unwrap_or_default();
    assert!(names_a_descriptor(middle), "{joined:?}");
    let two = output(true, b"echo <(true) <(true)");
    let words: Vec<&str> = two.split_whitespace().collect();
    assert_eq!(words.len(), 2, "{two:?}");
    assert!(words.iter().all(|word| names_a_descriptor(word)), "{two:?}");
    assert_ne!(words[0], words[1], "{two:?}");
}

// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn output_substitution_delivers_the_words() {
    let path = scratch_path("out");
    let _ = std::fs::remove_file(&path);
    let script = format!(
        "{{ echo payload; }} > >(read line; echo \"$line\" > {})",
        path.display()
    );

    let (status, stdout) = run(&mut shell(true), script.as_bytes());

    assert_eq!((status, stdout.as_slice()), (0, b"".as_slice()));
    assert_eq!(
        contents_within(&path, std::time::Duration::from_secs(10)),
        "payload\n"
    );
    let _ = std::fs::remove_file(&path);
}

/// The name lives exactly as long as the command that produced it.
///
/// Bash keeps every name until the outermost command finishes, so a loop
/// there opens one pipe per iteration and the numbers climb. Numbers that
/// stay in the same small range after a hundred and twenty-eight of them is
/// the observable form of the narrower rule. The range rather than one
/// exact number: these tests share a process, so a sibling test's open
/// files move the floor.
// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn a_name_dies_with_its_command() {
    const ROUNDS: usize = 128;
    let names = output(
        true,
        b"i=0; while [ $i -lt 128 ]; do echo <(true); i=$((i+1)); done",
    );
    let numbers: Vec<u32> = names
        .split_whitespace()
        .filter(|name| names_a_descriptor(name))
        .filter_map(|name| name.rsplit('/').next()?.parse().ok())
        .collect();

    assert_eq!(numbers.len(), ROUNDS, "{names:?}");
    let highest = numbers.iter().copied().max().expect("a name per round");
    assert!(highest < 64, "descriptor numbers climbed to {highest}");
}

/// The `for` list is one command, so its name outlives the loop body that
/// reads it — the case the per-command rule must not cut too short.
// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn a_list_name_outlives_the_loop_body() {
    assert_eq!(
        output(
            true,
            b"for w in <(echo body); do read line < $w; echo $line; done"
        ),
        "body\n"
    );
}

// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn substitutions_nest() {
    assert_eq!(
        output(
            true,
            b"read line < <(read inner < <(echo deep); echo \"[$inner]\"); echo \"$line\""
        ),
        "[deep]\n"
    );
}

/// Bash performs no substitution inside double quotes, and neither does
/// the default mode anywhere.
// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn quoting_and_the_default_mode_withhold_it() {
    assert_eq!(output(true, b"echo \"<(echo hi)\""), "<(echo hi)\n");
    assert_eq!(output(true, b"echo '<(echo hi)'"), "<(echo hi)\n");
    assert_eq!(output(true, b"echo \\<\\(echo hi\\)"), "<(echo hi)\n");

    let mut posix = shell(false);
    let (status, stdout) = run(&mut posix, b"read line < <(echo hi); echo \"$line\"");
    assert_ne!(status, 0);
    assert!(!String::from_utf8_lossy(&stdout).contains("/fd/"));
}

/// The child is nobody's job: it is not waited for, not reported, and not
/// what `$!` names.
// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn the_child_is_not_a_job() {
    assert_eq!(
        output(true, b"read line < <(echo hi); jobs; echo \"[${!-unset}]\""),
        "[unset]\n"
    );
    // A background job still sets `$!`, and a substitution beside it does
    // not take the name away. Nothing here waits: these tests share one
    // process with the rest of the suite, so a `wait` would be a wait on
    // whichever shell happened to reap the child first.
    let mixed = output(true, b"true & read line < <(echo hi); echo \"${!:+set}\"");
    assert_eq!(mixed, "set\n");
}

/// A substitution's own child disowns its parent's other names.
///
/// The reader behind `>(list)` sees end of file when the last write end
/// closes. The shell holds one; a sibling substitution forked after it was
/// opened inherits another, and would hold it for as long as that sibling
/// runs. The child clears the stack it inherited, so the reader is not
/// made to wait for a process that has nothing to do with it.
// [spec:nsh:req:compat.bash.safe-core/test]
// [spec:nsh:req:compat.bash.process-substitution/test]
#[test]
fn a_sibling_does_not_deny_end_of_file() {
    let scratch = scratch_path("sibling-eof");
    drop(std::fs::remove_file(&scratch));
    let script = format!(
        "true >(cat >/dev/null; echo released > {}) <(sleep 5)",
        scratch.display()
    );

    let (status, _) = run(&mut shell(true), script.as_bytes());
    assert_eq!(status, 0);
    // Well under the sibling's lifetime: waiting for it would be the
    // failure this is looking for.
    let released = contents_within(&scratch, std::time::Duration::from_millis(1500));
    drop(std::fs::remove_file(&scratch));

    assert_eq!(
        released, "released\n",
        "the sibling held the write end open"
    );
}
