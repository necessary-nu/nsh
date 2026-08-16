//! Entry point. The real work is in `shellmain::main_fn`, which is the
//! literal port of `main()` in `src/main.c`.
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
//! Each is inherited or observable, so each is undone below. They were
//! found by diffing `/proc/self/status` and `/proc/self/fd` between the
//! two shells, which is worth repeating after any toolchain bump: this
//! list is a property of the Rust runtime, not of the port.

use core::sync::atomic::{AtomicUsize, Ordering};

/// The SIGPIPE disposition this process was started with, captured before
/// Rust's runtime can overwrite it.
///
/// dash does not manage SIGPIPE at all: it never appears in `setsignal`,
/// so a dash and everything it forks present whatever the process that
/// exec'd them presented. Rust's runtime does not leave that alone --
/// `std::rt::init` sets SIGPIPE to SIG_IGN before `main` is entered, and
/// the disposition is inherited across fork *and* exec, so the setting
/// reaches every external command the shell ever runs.
///
/// What that costs is not subtle. A pipeline whose reader exits early is
/// supposed to kill its writer:
///
///     i=0; while [ $i -lt 100000 ]; do echo lots; i=$((i+1)); done | head -2
///
/// dash prints nothing -- `echo` takes SIGPIPE and dies. The port printed
/// `echo: I/O error` about 99,930 times, because with SIGPIPE ignored the
/// write returns EPIPE, the built-in reports it, and the loop runs to
/// completion. Every `| head`, `| grep -q` and `yes |` in a script is the
/// same shape.
///
/// Restoring it needs the *inherited* value, not SIG_DFL: a shell started
/// from a daemon that ignores SIGPIPE has to keep ignoring it, which is
/// what dash does and what dash's own `S_HARD_IGN` handling assumes. By
/// the time `main` runs, Rust has already overwritten it, so this is read
/// from an `.init_array` constructor -- those run inside
/// `__libc_start_main`, before it calls `main`, and therefore before
/// Rust's runtime init, which `main` is what invokes.
static INHERITED_SIGPIPE: AtomicUsize = AtomicUsize::new(usize::MAX);

/// Bit i is set if fd i was already closed when the process started.
///
/// Rust's `sanitize_standard_fds` opens `/dev/null` over any of fd 0, 1
/// and 2 that is not open, so a script run as `sh -c … 0<&-` saw
/// `/dev/null` on stdin where dash sees EBADF. That is a real difference
/// to a shell: `read` returns end-of-file instead of failing, and
/// `exec 3<&0` succeeds instead of erroring.
static CLOSED_STD_FDS: AtomicUsize = AtomicUsize::new(0);

#[used]
#[unsafe(link_section = ".init_array")]
static CAPTURE_PRE_RUNTIME_STATE: extern "C" fn() = capture_pre_runtime_state;

extern "C" fn capture_pre_runtime_state() {
    unsafe {
        let mut old: libc::sigaction = core::mem::zeroed();
        if libc::sigaction(libc::SIGPIPE, core::ptr::null(), &mut old) == 0 {
            INHERITED_SIGPIPE.store(old.sa_sigaction, Ordering::Relaxed);
        }
        let mut closed = 0usize;
        for fd in 0..3 {
            if libc::fcntl(fd, libc::F_GETFD) == -1 {
                closed |= 1 << fd;
            }
        }
        CLOSED_STD_FDS.store(closed, Ordering::Relaxed);
    }
}

fn main() {
    // Undo Rust's SIGPIPE = SIG_IGN. See INHERITED_SIGPIPE above. Only a
    // plain SIG_DFL/SIG_IGN is restored: if the parent had installed a
    // real handler the function pointer is meaningless in this image, and
    // Rust has already replaced it in any case.
    unsafe {
        let inherited = INHERITED_SIGPIPE.load(Ordering::Relaxed);
        if inherited == libc::SIG_DFL || inherited == libc::SIG_IGN {
            let mut act: libc::sigaction = core::mem::zeroed();
            act.sa_sigaction = inherited;
            act.sa_flags = 0;
            libc::sigemptyset(&mut act.sa_mask);
            libc::sigaction(libc::SIGPIPE, &act, core::ptr::null_mut());
        }

        // Undo Rust's stack-overflow reporting. dash installs no handler
        // for SIGSEGV or SIGBUS, so it dies on the signal and dumps core;
        // the port printed "thread 'main' has overflowed its stack" and
        // exited normally instead. `f() { f; }; f` is the whole test. The
        // difference is visible in the shell's own SigCgt as well as in
        // how it dies, and tests/README.md tells the reader that segfaults
        // under the fuzz corpora are expected output -- which is only true
        // if the port still segfaults.
        libc::signal(libc::SIGSEGV, libc::SIG_DFL);
        libc::signal(libc::SIGBUS, libc::SIG_DFL);

        // Undo Rust's `sanitize_standard_fds`. See CLOSED_STD_FDS above.
        let closed = CLOSED_STD_FDS.load(Ordering::Relaxed);
        for fd in 0..3 {
            if closed & (1 << fd) != 0 {
                libc::close(fd as libc::c_int);
            }
        }
    }

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
    let status = nsh::shellmain::main_fn(
        argv.len() as libc::c_int,
        argv,
        nsh::streams::Streams::INHERIT,
    );
    std::process::exit(status.code().into());
}
