//! Does the library actually run on streams it is given?
//!
//! The unit tests in `streams.rs` check `install` and `restore` in
//! isolation. These run the whole shell and read what came out, which is
//! the only thing that shows [dec:nsh:host-owns-streams] is a property of
//! the shell rather than of one module.
//!
//! Everything here forks. `main_fn` runs a whole shell to completion --
//! including its EXIT trap and its job-control teardown -- so a test that
//! called it in process would leave the runner holding a shell that had
//! finished exiting. It returns a status rather than `_exit`ing since
//! [dec:nsh:host-owns-the-process], so each child ends itself.

use nsh::streams::{self, Streams};

fn read_all(fd: i32) -> String {
    let bytes = nsh_platform::read_to_end(fd).expect("read pipe");
    nsh_platform::close_fd(fd).expect("close pipe reader");
    String::from_utf8(bytes).expect("pipe output is UTF-8")
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
/// prompts -- go to the stream it was *built* with, with no `dup2`
/// anywhere. This was `set` mode.
#[test]
fn a_builtin_writes_to_the_stream_the_shell_was_given() {
    let (r, w) = nsh_platform::pipe().expect("create pipe");
    let st = run_shell("echo from the library", || Streams {
        stdin: 0,
        stdout: w,
        stderr: 2,
    });
    nsh_platform::close_fd(w).expect("close pipe writer");
    assert_eq!(st, 0);
    assert_eq!(read_all(r), "from the library\n");
}

/// The documented limit of constructor mode, pinned as a test rather than
/// left as a claim in a comment: the *language's* descriptor numbers still mean
/// the process's descriptors, so an external command's output goes to
/// descriptor 1 and not to the stream the shell was handed.
///
/// Making these agree needs a per-instance descriptor table, which is
/// deferred to [dec:nsh:no-ambient-state].
#[test]
fn set_does_not_carry_to_an_external_command() {
    let (shell_r, shell_w) = nsh_platform::pipe().expect("create shell pipe");
    let (fd1_r, fd1_w) = nsh_platform::pipe().expect("create fd 1 pipe");
    let st = run_shell("echo builtin; /bin/echo external", || {
        nsh_platform::duplicate_to(fd1_w, 1).expect("install fd 1");
        nsh_platform::close_fd(fd1_w).expect("close duplicated source");
        Streams {
            stdin: 0,
            stdout: shell_w,
            stderr: 2,
        }
    });
    nsh_platform::close_fd(shell_w).expect("close shell pipe writer");
    nsh_platform::close_fd(fd1_w).expect("close fd 1 pipe writer");
    assert_eq!(st, 0);
    assert_eq!(read_all(shell_r), "builtin\n");
    assert_eq!(read_all(fd1_r), "external\n");
}

/// `install` mode is the one with full fidelity: descriptor 1 *is* the
/// stream inside the shell, so built-ins, external commands and
/// redirection all agree without the shell knowing anything about it.
#[test]
fn install_carries_to_builtins_redirection_and_external_commands() {
    let (r, w) = nsh_platform::pipe().expect("create pipe");
    let script = "echo builtin; /bin/echo external; { echo redirected > /dev/stdout; }";
    let st = run_shell(script, || {
        // Deliberately not restored: this child is about to become a
        // shell that ends in `_exit`, so there is no "afterwards" in
        // which to hand the descriptors back.
        let lent = streams::install(Streams {
            stdin: 0,
            stdout: w,
            stderr: 2,
        })
        .expect("install");
        core::mem::forget(lent);
        // `install` put them on 0, 1 and 2, so the shell is built there.
        Streams::INHERIT
    });
    nsh_platform::close_fd(w).expect("close pipe writer");
    assert_eq!(st, 0);
    assert_eq!(read_all(r), "builtin\nexternal\nredirected\n");
}

/// A shell reading its script from a stream it was given, rather than
/// from descriptor 0.
#[test]
fn the_shell_reads_a_script_from_the_stream_it_was_given() {
    let (script_r, script_w) = nsh_platform::pipe().expect("create script pipe");
    let (out_r, out_w) = nsh_platform::pipe().expect("create output pipe");
    nsh_platform::write_all(script_w, b"echo one\necho two\n").expect("write script");
    nsh_platform::close_fd(script_w).expect("close script pipe writer");

    // `sh` with no operand reads commands from its standard input.
    let argv: Vec<Vec<u8>> = vec![b"sh".to_vec()];
    let st = nsh_platform::run_in_child(move || {
            let lent = streams::install(Streams {
                stdin: script_r,
                stdout: out_w,
                stderr: 2,
            })
            .expect("install");
            core::mem::forget(lent); // see the note in the test above
            /* `main_fn` returns now — [dec:nsh:host-owns-the-process] made
               ending the process the caller's act — so this fork's child
               has to end itself. Returning would carry it back into the
               test harness after the fork. */
            let status = nsh::shellmain::main_fn(argv, Streams::INHERIT);
            nsh_platform::exit_immediately(status.code().into());
        })
        .expect("run shell child");
    nsh_platform::close_fd(out_w).expect("close output pipe writer");
    nsh_platform::close_fd(script_r).expect("close script pipe reader");
    assert_eq!(st, 0);
    assert_eq!(read_all(out_r), "one\ntwo\n");
}
