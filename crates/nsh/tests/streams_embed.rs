//! Does the library actually run on streams it is given?
//!
//! The unit tests in `streams.rs` check `install` and `restore` in
//! isolation. These run the whole shell and read what came out, which is
//! the only thing that shows [dec:nsh:host-owns-streams] is a property of
//! the shell rather than of one module.
//!
//! Everything here forks. `main_fn` ends in `exitshell`, which `_exit`s --
//! that is dash's shape and undoing it is [dec:nsh:errors-are-values], not
//! this -- so a test that called it in process would take the test runner
//! with it.

use nsh::streams::{self, Streams};
use std::io::Read;
use std::os::unix::io::FromRawFd;

unsafe fn pipe() -> (i32, i32) {
    let mut fds = [0i32; 2];
    assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
    (fds[0], fds[1])
}

fn read_all(fd: i32) -> String {
    let mut s = String::new();
    let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
    f.read_to_string(&mut s).expect("read pipe");
    s
}

/// Run `script` in a forked child, with `prepare` deciding how the child
/// is given its streams. Returns the child's exit status.
fn run_shell(script: &str, prepare: impl FnOnce()) -> i32 {
    let argv: Vec<Vec<u8>> = vec![b"sh".to_vec(), b"-c".to_vec(), script.as_bytes().to_vec()];
    unsafe {
        let pid = libc::fork();
        assert!(pid >= 0, "fork failed");
        if pid == 0 {
            // The port raises `Longjmp` as ordinary control flow -- every
            // shell error and every `exit` is one -- and the default hook
            // prints a panic banner for each. The `dash` binary filters
            // them in `main`; do the same so the test's stderr is the
            // shell's, not the runtime's.
            let default_hook = std::panic::take_hook();
            std::panic::set_hook(Box::new(move |info| {
                if info.payload().downcast_ref::<nsh::error::Longjmp>().is_some() {
                    return;
                }
                default_hook(info);
            }));
            prepare();
            nsh::shellmain::main_fn(argv.len() as libc::c_int, argv, streams::streams());
        }
        let mut status = 0i32;
        assert_eq!(libc::waitpid(pid, &mut status, 0), pid);
        if libc::WIFEXITED(status) {
            libc::WEXITSTATUS(status)
        } else {
            128 + libc::WTERMSIG(status)
        }
    }
}

/// `set` mode: the shell's own writes -- built-ins, errors, prompts --
/// follow the stream it was handed, with no `dup2` anywhere.
#[test]
fn a_builtin_writes_to_the_stream_the_shell_was_given() {
    let (r, w) = unsafe { pipe() };
    let st = run_shell("echo from the library", || unsafe {
        streams::set(Streams {
            stdin: 0,
            stdout: w,
            stderr: 2,
        });
    });
    unsafe { libc::close(w) };
    assert_eq!(st, 0);
    assert_eq!(read_all(r), "from the library\n");
}

/// The documented limit of `set` mode, pinned as a test rather than left
/// as a claim in a comment: the *language's* descriptor numbers still mean
/// the process's descriptors, so an external command's output goes to
/// descriptor 1 and not to the stream the shell was handed.
///
/// Making these agree needs a per-instance descriptor table, which is
/// deferred to [dec:nsh:no-ambient-state].
#[test]
fn set_does_not_carry_to_an_external_command() {
    let (shell_r, shell_w) = unsafe { pipe() };
    let (fd1_r, fd1_w) = unsafe { pipe() };
    let st = run_shell("echo builtin; /bin/echo external", || unsafe {
        libc::dup2(fd1_w, 1);
        libc::close(fd1_w);
        streams::set(Streams {
            stdin: 0,
            stdout: shell_w,
            stderr: 2,
        });
    });
    unsafe {
        libc::close(shell_w);
        libc::close(fd1_w);
    }
    assert_eq!(st, 0);
    assert_eq!(read_all(shell_r), "builtin\n");
    assert_eq!(read_all(fd1_r), "external\n");
}

/// `install` mode is the one with full fidelity: descriptor 1 *is* the
/// stream inside the shell, so built-ins, external commands and
/// redirection all agree without the shell knowing anything about it.
#[test]
fn install_carries_to_builtins_redirection_and_external_commands() {
    let (r, w) = unsafe { pipe() };
    let script = "echo builtin; /bin/echo external; { echo redirected > /dev/stdout; }";
    let st = run_shell(script, || unsafe {
        // Deliberately not restored: this child is about to become a
        // shell that ends in `_exit`, so there is no "afterwards" in
        // which to hand the descriptors back.
        let _lent = streams::install(Streams {
            stdin: 0,
            stdout: w,
            stderr: 2,
        })
        .expect("install");
        core::mem::forget(_lent);
    });
    unsafe { libc::close(w) };
    assert_eq!(st, 0);
    assert_eq!(read_all(r), "builtin\nexternal\nredirected\n");
}

/// A shell reading its script from a stream it was given, rather than
/// from descriptor 0.
#[test]
fn the_shell_reads_a_script_from_the_stream_it_was_given() {
    let (script_r, script_w) = unsafe { pipe() };
    let (out_r, out_w) = unsafe { pipe() };
    unsafe {
        let script = b"echo one\necho two\n";
        assert_eq!(
            libc::write(script_w, script.as_ptr() as *const libc::c_void, script.len()),
            script.len() as isize
        );
        libc::close(script_w);
    }

    // `sh` with no operand reads commands from its standard input.
    let argv: Vec<Vec<u8>> = vec![b"sh".to_vec()];
    let st = unsafe {
        let pid = libc::fork();
        assert!(pid >= 0);
        if pid == 0 {
            let lent = streams::install(Streams {
                stdin: script_r,
                stdout: out_w,
                stderr: 2,
            })
            .expect("install");
            core::mem::forget(lent); // see the note in the test above
            nsh::shellmain::main_fn(argv.len() as libc::c_int, argv, Streams::INHERIT);
        }
        let mut status = 0i32;
        libc::waitpid(pid, &mut status, 0);
        libc::WEXITSTATUS(status)
    };
    unsafe {
        libc::close(out_w);
        libc::close(script_r);
    }
    assert_eq!(st, 0);
    assert_eq!(read_all(out_r), "one\ntwo\n");
}
