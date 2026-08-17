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
    // These stay `Vec<u8>` rather than `String`: a `String` holding
    // non-UTF-8 bytes violates its own invariant, and building one with
    // `from_utf8_unchecked` would be undefined behaviour even though the
    // only thing done with it here is `as_bytes`.
    let argv: Vec<Vec<u8>> = std::env::args_os()
        .map(std::os::unix::ffi::OsStringExt::into_vec)
        .collect();
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
