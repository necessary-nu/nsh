//! Entry point. The shell itself is the `nsh` library; this frontend owns
//! the host-process setup and returns the library's status to the OS.
//!
//! Three of the things this file does exist for one reason: Rust's
//! runtime does work between `_start` and this `main` that C's does not,
//! and a shell is close enough to the operating system that all of it
//! shows. What `std::rt::init` does, and what dash does instead:
//!
//!   * sets SIGPIPE to SIG_IGN — dash never touches SIGPIPE
//!   * opens /dev/null over any closed fd 0/1/2 — dash leaves them closed
//!   * catches SIGSEGV/SIGBUS on an alternate stack, to report a stack
//!     overflow — dash has no handler and dies on the signal
//!
//! Each is inherited or observable, so `nsh-platform` undoes each before
//! the shell starts. They were
//! found by diffing `/proc/self/status` and `/proc/self/fd` between the
//! two shells, which is worth repeating after any toolchain bump: this
//! list is a property of the Rust runtime, not of the port.

#![deny(unsafe_code)]

fn bash_invocation(raw_arg0: &[u8]) -> bool {
    let basename = raw_arg0
        .rsplit(|byte| *byte == b'/')
        .next()
        .unwrap_or_default();
    matches!(basename, b"bash" | b"-bash")
}

// [spec:nsh:req:compat.bash.selection]
/// Apply frontend-only dialect inference before the shell parses its options.
fn select_invocation_mode(argv: &mut Vec<Vec<u8>>) {
    if argv
        .first()
        .is_some_and(|raw_arg0| bash_invocation(raw_arg0))
    {
        argv.splice(1..1, [b"-o".to_vec(), b"bash".to_vec()]);
    }
}

use nsh_platform::NativeStrExt as _;

fn main() {
    nsh_platform::restore_shell_process_runtime_state();

    // A panic hook sat here, filtering out the `error::Longjmp` payload
    // the port used to implement C's `longjmp`: those unwinds were
    // ordinary control flow -- every shell error, interrupt, `exit` and
    // `set -e` went through one -- and the default hook printed a
    // "thread 'main' panicked" banner each time one was *raised*.
    //
    // `errors-are-values` deleted the mechanism, so there is no payload
    // to filter and every panic that reaches the hook is a genuine bug.
    // The default hook is the right one for that.

    // C's `main(int argc, char **argv)` receives raw NUL-terminated byte
    // strings. An argument need not be valid UTF-8, and dash passes such
    // bytes through untouched — `dash -c $'x=\xff; echo $x'` prints the
    // byte. `std::env::args()` unwraps a UTF-8 conversion and panics on
    // any non-UTF-8 argument, so the port died with status 101 where the
    // C ran normally.
    //
    // Keep the operating-system representation intact until this explicit
    // handoff to the byte-oriented shell language engine.
    let mut argv = nsh_platform::process_arguments()
        .iter()
        .map(|argument| argument.to_shell_bytes())
        .collect();
    select_invocation_mode(&mut argv);
    // The frontend is the thing entitled to the process's standard
    // descriptors, so it hands them to the shell explicitly rather than
    // letting the shell assume them. See [dec:nsh:host-owns-streams].
    // The library returns its status rather than ending the process:
    // [dec:nsh:host-owns-the-process] makes that the frontend's act, and
    // this is the frontend. `exitshell` has already flushed and torn down
    // job control, so there is nothing left to do but leave.
    let status = nsh::shellmain::main_fn(argv, nsh::streams::Streams::INHERIT);
    std::process::exit(status.code().into());
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:compat.bash.selection/test]
    #[test]
    fn only_exact_bash_basenames_infer_mode() {
        for inferred in [
            b"bash".as_slice(),
            b"-bash",
            b"/bin/bash",
            b"relative/-bash",
        ] {
            assert!(bash_invocation(inferred), "{inferred:?}");
        }
        for ordinary in [b"nsh".as_slice(), b"mybash", b"bash/", b""] {
            assert!(!bash_invocation(ordinary), "{ordinary:?}");
        }
    }

    #[test]
    fn inference_precedes_explicit_options() {
        let mut argv = vec![
            b"/bin/bash".to_vec(),
            b"+o".to_vec(),
            b"bash".to_vec(),
            b"-c".to_vec(),
            b":".to_vec(),
        ];

        select_invocation_mode(&mut argv);

        assert_eq!(
            argv,
            [
                b"/bin/bash".as_slice(),
                b"-o",
                b"bash",
                b"+o",
                b"bash",
                b"-c",
                b":"
            ]
        );
    }
}
