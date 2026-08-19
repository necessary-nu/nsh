//! Does the library actually run on streams it is given?
//!
//! The unit tests in `streams.rs` check ownership in isolation. These run
//! the whole shell and read what came out, which is the only thing that
//! shows [dec:nsh:host-owns-streams] is a property of the shell rather than
//! of one module.
//!
//! Everything here forks. `main_fn` runs a whole shell to completion --
//! including its EXIT trap and its job-control teardown -- so a test that
//! called it in process would leave the runner holding a shell that had
//! finished exiting. It returns a status rather than `_exit`ing since
//! [dec:nsh:host-owns-the-process], so each child ends itself.

use nsh::streams::Streams;

fn read_all(fd: &nsh_platform::Descriptor) -> String {
    let bytes = nsh_platform::read_to_end(fd).expect("read pipe");
    String::from_utf8(bytes).expect("pipe output is UTF-8")
}

fn shell_quote(text: &str) -> String {
    format!("'{}'", text.replace('\'', "'\\''"))
}

fn this_test_command(test: &str) -> String {
    let executable = std::env::current_exe().expect("test executable path");
    format!(
        "{} {} --exact --list",
        shell_quote(&executable.to_string_lossy()),
        shell_quote(test)
    )
}

/// Run `script` in a forked child, with `prepare` deciding how the child
/// is given its streams: it runs inside the child and *returns* the
/// `Streams` the shell is built on. Returns the child's exit status.
///
/// `prepare` returning the value rather than stashing it in a global is
/// the whole of what changed when `io` and `streams` moved onto the
/// instance. Two of the cases below were passing a `Streams` through
/// `streams::set` to reach `main_fn`'s parameter, which has taken one
/// since [dec:nsh:host-owns-streams] landed.
fn run_shell(script: &str, prepare: impl FnOnce() -> Streams) -> i32 {
    let argv: Vec<Vec<u8>> = vec![b"sh".to_vec(), b"-c".to_vec(), script.as_bytes().to_vec()];
    nsh_platform::run_in_child(move || {
        // The port raises `Longjmp` as ordinary control flow -- every
        // shell error and every `exit` is one -- and the default hook
        // prints a panic banner for each. The `dash` binary filters
        // them in `main`; do the same so the test's stderr is the
        // shell's, not the runtime's.
        let streams = prepare();
        /* `main_fn` returns now — [dec:nsh:host-owns-the-process] made
        ending the process the caller's act — so this fork's child
        has to end itself. Returning would carry it back into the
        test harness after the fork. */
        let status = nsh::shellmain::main_fn(argv, streams);
        nsh_platform::exit_immediately(status.code().into());
    })
    .expect("run shell child")
}

/// Constructor mode: the shell's own writes -- built-ins, errors,
/// prompts -- go to the stream it was *built* with, without changing the
/// process descriptor table. This was `set` mode.
#[test]
fn a_builtin_writes_to_the_stream_the_shell_was_given() {
    let (r, w) = nsh_platform::pipe().expect("create pipe");
    let st = run_shell("echo from the library", move || {
        Streams::from_fds(std::io::stdin(), &w, std::io::stderr()).expect("duplicate streams")
    });
    assert_eq!(st, 0);
    assert_eq!(read_all(&r), "from the library\n");
}

/// External commands materialize the shell's logical descriptor table in the
/// child. The host's descriptor 1 is deliberately a different pipe so this
/// also proves that materialization never leaks into the parent process.
#[test]
fn supplied_stdout_reaches_external_commands() {
    let (shell_r, shell_w) = nsh_platform::pipe().expect("create shell pipe");
    let (fd1_r, fd1_w) = nsh_platform::pipe().expect("create fd 1 pipe");
    let external = this_test_command("supplied_stdout_reaches_external_commands");
    let script = format!("echo builtin; {external}");
    let st = run_shell(&script, move || {
        nsh_platform::ProcessFdChanges::new([(
            1,
            Some(nsh_platform::duplicate_cloexec(&fd1_w, 10).unwrap()),
        )])
        .unwrap()
        .apply()
        .expect("install fd 1");
        Streams::from_fds(std::io::stdin(), &shell_w, std::io::stderr()).expect("duplicate streams")
    });
    assert_eq!(st, 0);
    let output = read_all(&shell_r);
    assert!(output.starts_with("builtin\n"), "{output:?}");
    assert!(
        output.contains("supplied_stdout_reaches_external_commands: test"),
        "{output:?}"
    );
    assert_eq!(read_all(&fd1_r), "");
}

/// A pipeline changes the logical endpoints inherited by both children, not
/// the host's process-wide standard descriptors.
#[test]
fn supplied_stdout_reaches_pipeline_output() {
    let (shell_r, shell_w) = nsh_platform::pipe().expect("create shell pipe");
    let (fd1_r, fd1_w) = nsh_platform::pipe().expect("create fd 1 pipe");
    let external = this_test_command("supplied_stdout_reaches_pipeline_output");
    let script = format!("{external} | {{ IFS= read -r line; printf 'PIPE:%s' \"$line\"; }}");
    let st = run_shell(&script, move || {
        nsh_platform::ProcessFdChanges::new([(
            1,
            Some(nsh_platform::duplicate_cloexec(&fd1_w, 10).unwrap()),
        )])
        .unwrap()
        .apply()
        .expect("install fd 1");
        Streams::from_fds(std::io::stdin(), &shell_w, std::io::stderr()).expect("duplicate streams")
    });
    assert_eq!(st, 0);
    assert_eq!(
        read_all(&shell_r),
        "PIPE:supplied_stdout_reaches_pipeline_output: test"
    );
    assert_eq!(read_all(&fd1_r), "");
}

/// A command-scoped redirection replaces and restores the shell's logical
/// stdout. The redirected bytes go to the file, then `cat` inherits the
/// restored supplied stream.
#[test]
fn supplied_stdout_survives_redirection() {
    let (file, path) =
        nsh_platform::create_temporary_file("nsh-stream-target").expect("create target");
    drop(file);
    let path_text = path.to_string_lossy();
    let script = format!(
        "echo redirected > '{path_text}'; echo restored; \
         IFS= read -r line < '{path_text}'; printf '%s\\n' \"$line\""
    );

    let (shell_r, shell_w) = nsh_platform::pipe().expect("create shell pipe");
    let (fd1_r, fd1_w) = nsh_platform::pipe().expect("create fd 1 pipe");
    let st = run_shell(&script, move || {
        nsh_platform::ProcessFdChanges::new([(
            1,
            Some(nsh_platform::duplicate_cloexec(&fd1_w, 10).unwrap()),
        )])
        .unwrap()
        .apply()
        .expect("install fd 1");
        Streams::from_fds(std::io::stdin(), &shell_w, std::io::stderr()).expect("duplicate streams")
    });
    let _ = nsh_platform::remove_file(&path);

    assert_eq!(st, 0);
    assert_eq!(read_all(&shell_r), "restored\nredirected\n");
    assert_eq!(read_all(&fd1_r), "");
}

/// A shell reading its script from a stream it was given, rather than
/// from descriptor 0.
#[test]
fn the_shell_reads_a_script_from_the_stream_it_was_given() {
    let (script_r, script_w) = nsh_platform::pipe().expect("create script pipe");
    let (out_r, out_w) = nsh_platform::pipe().expect("create output pipe");
    nsh_platform::write_all(&script_w, b"echo one\necho two\n").expect("write script");
    drop(script_w);

    // `sh` with no operand reads commands from its standard input.
    let argv: Vec<Vec<u8>> = vec![b"sh".to_vec()];
    let st = nsh_platform::run_in_child(move || {
        let supplied =
            Streams::from_fds(&script_r, &out_w, std::io::stderr()).expect("duplicate streams");
        /* `main_fn` returns now — [dec:nsh:host-owns-the-process] made
        ending the process the caller's act — so this fork's child
        has to end itself. Returning would carry it back into the
        test harness after the fork. */
        let status = nsh::shellmain::main_fn(argv, supplied);
        nsh_platform::exit_immediately(status.code().into());
    })
    .expect("run shell child");
    assert_eq!(st, 0);
    assert_eq!(read_all(&out_r), "one\ntwo\n");
}
