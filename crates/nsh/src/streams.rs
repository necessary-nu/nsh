//! The shell's own three streams.
//!
//! Implements [dec:nsh:host-owns-streams]: the library reads and writes
//! streams it is given, not descriptors 0, 1 and 2.
//!
//! This is *not* about redirection. `>`, `<`, `2>&1` and `exec 3>&1` name
//! file descriptors because that is what the shell language means by
//! them, and they go on manipulating real descriptors exactly as dash
//! does. What this module owns is the narrower question of where the
//! shell's *own* three streams come from -- the ones it prompts on, reads
//! a script from, writes built-in output to and reports errors on.
//!
//! ## Two ways to be given streams
//!
//! An embedder has one of two problems, and they want opposite things.
//!
//! [`install`] is for a host that can lend the shell descriptors 0, 1 and
//! 2 for the duration. It saves whatever the host had there, `dup2`s the
//! supplied descriptors into place, and [`Borrowed::restore`] puts the
//! host's back. Everything downstream is then byte-identical to dash --
//! redirection, `exec`, and every child process inherit correctly with no
//! further help -- because within the shell's execution environment the
//! standard descriptors really are standard. This is the mode with full
//! fidelity, and it is what a frontend should use.
//!
//! [`set`] is for a host that cannot afford that. `dup2` on descriptor 1
//! is process-wide: a host embedding the shell on a worker thread, or
//! while its own code is writing to stdout, cannot have its descriptors
//! swapped out from under it. Such a host names three descriptors it
//! already owns and the shell writes there instead. The cost is exact:
//! the shell's own I/O follows, but the *language's* descriptor numbers
//! do not. `echo hi` reaches the supplied stream;
//! `echo hi >file` and any external command still mean the process's
//! descriptor 1, because that is what the number in the script denotes.
//!
//! Making those agree needs a per-instance descriptor table, which cannot
//! be built while the shell keeps its state in statics. It is recorded as
//! deferred on [dec:nsh:host-owns-streams] and lands with
//! [dec:nsh:no-ambient-state].
//!
//! The default is [`Streams::INHERIT`] -- 0, 1 and 2 -- under which every
//! path here is the identity and the shell is dash.

use core::fmt;

use libc::c_int;

/// The three descriptors the shell uses for its own I/O.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct Streams {
    /// Where the shell parses from.
    pub stdin: c_int,
    /// Where the shell and its built-ins write.
    pub stdout: c_int,
    /// Where the shell writes diagnostics, unbuffered.
    pub stderr: c_int,
}

impl Streams {
    /// Descriptors 0, 1 and 2: the shell takes over the host's streams,
    /// which is what a shell started as a process does.
    pub const INHERIT: Streams = Streams {
        stdin: 0,
        stdout: 1,
        stderr: 2,
    };

    /// [`Streams::INHERIT`] as a function, for a caller building a
    /// `Streams` in expression position beside the other two.
    pub fn inherit() -> Streams {
        Streams::INHERIT
    }

    /// Three descriptors the caller already has.
    ///
    /// The shell does not take ownership and does not close them: they
    /// outlive it, which is what lets a frontend lend the shell its own
    /// standard descriptors and take them back.
    ///
    /// These are the shell's own three writers and its own parse input.
    /// The sketch said they were "also the base of the shell's descriptor
    /// table", so that redirection, pipelines and forked external commands
    /// would resolve through them; that rests on the per-instance
    /// logical-to-real table, which `docs/api-design.md` §10 calls the
    /// largest bet in the document and which is not built. A forked
    /// command still inherits the process's descriptors. [`install`] is
    /// what moves those, and it is process-wide.
    pub fn from_fds(stdin: c_int, stdout: c_int, stderr: c_int) -> Streams {
        Streams {
            stdin,
            stdout,
            stderr,
        }
    }

    /// Standard input from `/dev/null`, and output and error into buffers
    /// the caller reads back with
    /// [`crate::context::Shell::take_captured_stdout`].
    ///
    /// **An unlinked temporary file rather than a pipe, deliberately.** A
    /// pipe has a fixed kernel buffer, so a script that writes more than
    /// it before the host reads would block on the write while the host
    /// blocks on `run` -- a deadlock with no way out that does not amount
    /// to "read it on another thread", which is exactly the burden
    /// capturing is supposed to remove. A file has no such limit, and
    /// `memfd` keeps it off the filesystem and out of `$TMPDIR`.
    ///
    /// **What it does and does not hold, measured rather than assumed.**
    /// It holds everything the shell itself writes -- built-ins, `echo`,
    /// `printf`, diagnostics -- because those go through the shell's own
    /// writers, which are these descriptors. It does *not* hold what a
    /// forked external command writes, because that child inherits the
    /// process's descriptor 1 rather than the shell's, and the
    /// logical-to-real descriptor table that would change it is §10's
    /// largest bet and is not built. A script reaches such output the way
    /// a script always does -- `$(cmd)` reads a pipe the shell makes, and
    /// works here unchanged -- and a frontend that wants the real
    /// descriptors moved wants [`install`], which is process-wide and says
    /// so. `crates/nsh/examples/embed.rs` demonstrates both halves.
    ///
    /// The descriptors are the caller's to close. Nothing here closes
    /// them, for the same reason [`Streams::from_fds`] does not.
    pub fn capture() -> std::io::Result<Streams> {
        let stdin = unsafe {
            let fd = libc::open(c"/dev/null".as_ptr(), libc::O_RDONLY);
            if fd < 0 {
                return Err(std::io::Error::last_os_error());
            }
            fd
        };
        let out = memfd(c"nsh-stdout")?;
        let err = memfd(c"nsh-stderr")?;
        Ok(Streams {
            stdin,
            stdout: out,
            stderr: err,
        })
    }

    fn as_array(&self) -> [c_int; 3] {
        [self.stdin, self.stdout, self.stderr]
    }
}

/// An anonymous file that lives only as long as its descriptor.
fn memfd(name: &std::ffi::CStr) -> std::io::Result<c_int> {
    let fd = unsafe { libc::memfd_create(name.as_ptr(), 0) };
    if fd < 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(fd)
}

impl crate::context::Shell {
    /// Take everything written to the shell's stdout since the last call.
    ///
    /// Only meaningful under [`Streams::capture`]; on a descriptor that
    /// cannot seek -- a terminal, a pipe, the descriptors
    /// [`Streams::INHERIT`] names -- this is the `ESPIPE` that seeking one
    /// gives, which is the honest answer rather than an empty string.
    ///
    /// Owned bytes rather than a borrow, and that is
    /// [dec:nsh:public-surface]'s call rather than an oversight: a borrow
    /// would be tied to the `&mut self` that reads the file, so holding
    /// the output would lock the shell and `run`, look, `run` again --
    /// the reason to capture at all -- would not compile.
    /// `crates/nsh/examples/embed.rs` is where that was discovered.
    pub fn take_captured_stdout(&mut self) -> std::io::Result<bstr::BString> {
        /* The shell's stdout is buffered, so what is in the file is what
         * it has flushed. Everything written since the last flush has to
         * go in before the file is read, or a capture taken between two
         * `run`s truncates mid-line. stderr is unbuffered and needs no
         * equivalent. */
        self.io.flushall();
        let fd = self.streams.stdout;
        take_all(fd)
    }

    /// Take everything written to the shell's stderr since the last call.
    pub fn take_captured_stderr(&mut self) -> std::io::Result<bstr::BString> {
        let fd = self.streams.stderr;
        take_all(fd)
    }
}

/// Read a seekable descriptor from the beginning and empty it.
///
/// Truncating rather than remembering an offset is what makes "since the
/// last call" true after a `run` that the shell reset -- and it keeps the
/// file from growing without bound across a long-lived shell.
fn take_all(fd: c_int) -> std::io::Result<bstr::BString> {
    unsafe {
        if libc::lseek(fd, 0, libc::SEEK_SET) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut out: Vec<u8> = Vec::new();
        let mut buf = [0u8; 8192];
        loop {
            let n = libc::read(fd, buf.as_mut_ptr() as *mut libc::c_void, buf.len());
            if n < 0 {
                return Err(std::io::Error::last_os_error());
            }
            if n == 0 {
                break;
            }
            out.extend_from_slice(&buf[..n as usize]);
        }
        if libc::ftruncate(fd, 0) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        if libc::lseek(fd, 0, libc::SEEK_SET) < 0 {
            return Err(std::io::Error::last_os_error());
        }
        Ok(bstr::BString::from(out))
    }
}

impl Default for Streams {
    fn default() -> Self {
        Streams::INHERIT
    }
}

/// A descriptor operation failed while installing or restoring streams.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub struct StreamError {
    /// The descriptor being operated on.
    pub fd: c_int,
    /// `errno` as reported by the failing call.
    pub errno: c_int,
}

impl fmt::Display for StreamError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // `strerror` is not thread-safe, but this type has to be printable
        // without one, so render the number and let the caller do better.
        write!(f, "fd {}: errno {}", self.fd, self.errno)
    }
}

impl std::error::Error for StreamError {}

/* `static mut STREAMS`, `streams()` and `set()` were here.
 *
 * They are gone, and what replaced them is a *parameter*: the shell is
 * constructed with the streams it is to use (`Shell::new(streams)`), so
 * `Streams` is a field of the instance and `ShellIo`'s descriptors are a
 * field initialiser beside it. `move-state` recorded this as escape (2)
 * and could not take it, because `set` had two callers with no shell to
 * be given — the five integration cases under `crates/nsh/tests/`. It
 * turned out they never needed one: `main_fn` has taken a `Streams`
 * argument since [dec:nsh:host-owns-streams] landed, and three of those
 * cases were passing a value *through the global* to reach a parameter
 * that was already there.
 *
 * What that costs is stated rather than hidden: `set` moved a *live*
 * shell's writers and the constructor cannot. Nothing in the crate did
 * that — `main_fn` called `set` before `Shell::new` — and the two unit
 * tests that asserted it now assert the constructor's contract instead.
 * An embedder that wants to redirect a running shell redirects it in the
 * language, which is what the language is for. */

/// The host's descriptors 0, 1 and 2, saved so [`restore`] can put them
/// back.
///
/// A slot is -1 when the descriptor was closed to begin with, which is a
/// state a shell has to preserve: `sh -c … 0<&-` must see EBADF on a read
/// rather than end-of-file, and undoing Rust's `sanitize_standard_fds` in
/// the frontend exists for the same reason.
#[derive(Debug)]
#[must_use = "the host's descriptors stay swapped out until this is restored"]
pub struct Borrowed {
    saved: [c_int; 3],
    installed: bool,
}

/// Lend the shell descriptors 0, 1 and 2, backed by `s`.
///
/// On success the shell's execution environment has `s` on the standard
/// descriptors and the host's originals are held in the returned
/// [`Borrowed`]. `s` itself is left open and still belongs to the caller.
///
/// Installing [`Streams::INHERIT`] is a no-op, so a frontend pays nothing.
///
/// # Safety
/// Manipulates the process's standard descriptors, so nothing else in the
/// process may be using them for the lifetime of the returned [`Borrowed`].
pub unsafe fn install(s: Streams) -> Result<Borrowed, StreamError> {
    if s == Streams::INHERIT {
        return Ok(Borrowed {
            saved: [-1; 3],
            installed: false,
        });
    }

    // Copy the supplied descriptors out of the 0..3 range *before* moving
    // any of them into it. Without this an aliasing set -- stdout on 0,
    // say -- would have its source overwritten by an earlier `dup2` and
    // the shell would end up with two copies of one stream.
    let mut staged = [-1i32; 3];
    let mut saved = [-1i32; 3];
    let cleanup = |fds: &[c_int]| {
        for &fd in fds {
            if fd >= 0 {
                libc::close(fd);
            }
        }
    };

    for (i, &fd) in s.as_array().iter().enumerate() {
        let hi = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10);
        if hi < 0 {
            let e = StreamError {
                fd,
                errno: crate::system::errno(),
            };
            cleanup(&staged);
            return Err(e);
        }
        staged[i] = hi;
    }

    // Save the host's originals. A closed descriptor is not an error --
    // it is a state to reproduce on restore -- so only a failure that is
    // not EBADF aborts.
    for fd in 0..3i32 {
        let hi = libc::fcntl(fd, libc::F_DUPFD_CLOEXEC, 10);
        if hi < 0 {
            let errno = crate::system::errno();
            if errno != libc::EBADF {
                let e = StreamError { fd, errno };
                cleanup(&staged);
                cleanup(&saved);
                return Err(e);
            }
        }
        saved[fd as usize] = hi;
    }

    for (i, &hi) in staged.iter().enumerate() {
        if libc::dup2(hi, i as c_int) < 0 {
            let e = StreamError {
                fd: i as c_int,
                errno: crate::system::errno(),
            };
            // Undo the moves already made, then hand the host back what
            // it had. Leaving it half-swapped would be worse than failing.
            let partial = Borrowed {
                saved,
                installed: true,
            };
            partial.restore();
            cleanup(&staged);
            return Err(e);
        }
    }
    cleanup(&staged);

    // The shell's own I/O needs no adjustment: `s` is on the standard
    // descriptors now, so a shell built with `Streams::INHERIT` -- which
    // is what a caller of `install` passes to `Shell::new` -- writes
    // exactly there. This function no longer touches shell state at all,
    // which is what lets it stay a process-level helper with no receiver.
    Ok(Borrowed {
        saved,
        installed: true,
    })
}

impl Borrowed {
    /// Put the host's descriptors 0, 1 and 2 back.
    ///
    /// A descriptor that was closed when [`install`] ran is closed again
    /// rather than left holding the shell's stream.
    pub fn restore(self) {
        if !self.installed {
            return;
        }
        unsafe {
            for fd in 0..3i32 {
                let old = self.saved[fd as usize];
                if old >= 0 {
                    libc::dup2(old, fd);
                    libc::close(old);
                } else {
                    libc::close(fd);
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::testutil::{forked, lock};
    use std::io::Read;
    use std::os::unix::io::FromRawFd;

    unsafe fn pipe() -> (c_int, c_int) {
        let mut fds = [0i32; 2];
        assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
        (fds[0], fds[1])
    }

    unsafe fn wr(fd: c_int, b: &[u8]) -> bool {
        libc::write(fd, b.as_ptr() as *const libc::c_void, b.len()) == b.len() as isize
    }

    fn read_exactly(fd: c_int, n: usize) -> Vec<u8> {
        let mut buf = vec![0u8; n];
        let mut f = unsafe { std::fs::File::from_raw_fd(fd) };
        f.read_exact(&mut buf).expect("read from pipe");
        buf
    }

    // ---- in-process: these touch only this module's own state ----------

    #[test]
    fn inherit_is_a_no_op() {
        let _g = lock();
        unsafe {
            let b = install(Streams::INHERIT).expect("inherit installs");
            b.restore();
            /* Nothing to read back: `install` no longer writes shell
             * state, so what it did is visible on the descriptors and
             * the forked cases below are what check them. */
            assert!(libc::fcntl(1, libc::F_GETFD) >= 0);
        }
    }

    /// The shell reads and writes the streams it was *built* with, and it
    /// does so without touching the process's standard descriptors. This
    /// is what `set` used to assert; the mode is now the constructor's
    /// argument rather than a global, so the assertion moved with it.
    #[test]
    fn a_shell_is_built_on_the_streams_it_is_given() {
        let _g = lock();
        unsafe {
            let (r, w) = pipe();
            let sh = crate::context::Shell::new(Streams {
                stdin: 0,
                stdout: w,
                stderr: 2,
            });
            assert_eq!(sh.streams.stdout, w);
            /* Descriptor 1 is untouched: this mode exists for a host that
             * cannot have it swapped out from under it. */
            assert!(libc::fcntl(1, libc::F_GETFD) >= 0);
            libc::close(r);
            libc::close(w);
        }
    }

    /// The shell's own writers are on the descriptors it was given. If
    /// they were not, `echo` would keep writing to descriptor 1 while
    /// everything else moved.
    ///
    /// This is the other half of what `set` asserted, and the half that
    /// made escape (2) look unavailable: `set` moved a *live* shell's
    /// writers. Nothing in the crate did that -- `main_fn` called `set`
    /// before `Shell::new` -- so the property that was actually load
    /// bearing is this one.
    #[test]
    fn the_shells_writers_are_the_streams_it_was_built_with() {
        let _g = lock();
        unsafe {
            let mut sh = crate::context::Shell::new(Streams {
                stdin: 7,
                stdout: 8,
                stderr: 9,
            });
            assert_eq!((sh.io.stdout().fd, sh.io.stderr().fd), (8, 9));

            let mut inherited = crate::context::Shell::new(Streams::INHERIT);
            assert_eq!(
                (inherited.io.stdout().fd, inherited.io.stderr().fd),
                (1, 2)
            );
        }
    }

    #[test]
    fn a_stream_error_prints_its_descriptor() {
        let e = StreamError {
            fd: 7,
            errno: libc::EBADF,
        };
        assert_eq!(e.to_string(), format!("fd 7: errno {}", libc::EBADF));
    }

    // ---- forked: these move the process's standard descriptors ---------
    //
    // `dup2` on descriptor 1 is process-wide, and cargo's test harness is
    // itself writing to descriptor 1 from other threads. Run them in a
    // child, where the swap is the child's business alone; the parent
    // reads the pipes afterwards. An earlier draft of these ran in
    // process under `lock()` and captured the harness's own "ok" progress
    // output into the pipe under test.

    #[test]
    fn install_redirects_stdout_and_restore_gives_it_back() {
        let (r, w) = unsafe { pipe() };
        let (host_r, host_w) = unsafe { pipe() };
        const LENT: &[u8] = b"through the lent stream\n";
        const BACK: &[u8] = b"host again\n";

        let st = forked(|| unsafe {
            // What the host has on descriptor 1 before lending it out.
            libc::dup2(host_w, 1);
            libc::close(host_w);

            let b = match install(Streams {
                stdin: 0,
                stdout: w,
                stderr: 2,
            }) {
                Ok(b) => b,
                Err(_) => libc::_exit(2),
            };
            if !wr(1, LENT) {
                libc::_exit(3);
            }
            b.restore();
            // Descriptor 1 is the host's again, so this must land in
            // host_r and not in the stream the shell was lent.
            if !wr(1, BACK) {
                libc::_exit(4);
            }
            libc::_exit(0);
        });

        assert_eq!(st, 0, "child failed at step {}", st);
        assert_eq!(read_exactly(r, LENT.len()), LENT);
        assert_eq!(read_exactly(host_r, BACK.len()), BACK);
        unsafe {
            libc::close(w);
            libc::close(host_w);
        }
    }

    /// The staging step in `install` exists for this case: a caller whose
    /// stdout is descriptor 0. Moving it straight into place would clobber
    /// the source of a stream not yet copied.
    #[test]
    fn aliasing_supplied_descriptors_survive_installation() {
        let (r, w) = unsafe { pipe() };
        const MSG: &[u8] = b"both\n";

        let st = forked(|| unsafe {
            // Put the write end on descriptor 0 -- also the first
            // descriptor `install` writes.
            libc::dup2(w, 0);
            libc::close(w);

            let b = match install(Streams {
                stdin: 0,
                stdout: 0,
                stderr: 2,
            }) {
                Ok(b) => b,
                Err(_) => libc::_exit(2),
            };
            if !wr(1, MSG) {
                libc::_exit(3);
            }
            b.restore();
            libc::_exit(0);
        });

        assert_eq!(st, 0, "child failed at step {}", st);
        assert_eq!(read_exactly(r, MSG.len()), MSG);
        unsafe { libc::close(w) };
    }

    /// A descriptor that was closed before `install` must be closed again
    /// after `restore`, not left holding the shell's stream. `sh -c … 0<&-`
    /// depends on the difference: EBADF, not end-of-file.
    #[test]
    fn a_closed_descriptor_is_restored_closed() {
        let (r, w) = unsafe { pipe() };

        let st = forked(|| unsafe {
            libc::close(0);
            let b = match install(Streams {
                stdin: r,
                stdout: 1,
                stderr: 2,
            }) {
                Ok(b) => b,
                Err(_) => libc::_exit(2),
            };
            if libc::fcntl(0, libc::F_GETFD) < 0 {
                libc::_exit(3);
            }
            b.restore();
            if libc::fcntl(0, libc::F_GETFD) != -1 {
                libc::_exit(4);
            }
            if crate::system::errno() != libc::EBADF {
                libc::_exit(5);
            }
            libc::_exit(0);
        });

        assert_eq!(st, 0, "child failed at step {}", st);
        unsafe {
            libc::close(r);
            libc::close(w);
        }
    }

    #[test]
    fn installing_a_bad_descriptor_reports_it_and_changes_nothing() {
        let st = forked(|| unsafe {
            // Nothing may allocate a descriptor between the close and the
            // install: the kernel hands out the lowest free number, so a
            // single intervening `dup` makes `bad` valid again.
            let bad = libc::dup(1);
            libc::close(bad);

            let err = match install(Streams {
                stdin: 0,
                stdout: bad,
                stderr: 2,
            }) {
                Ok(_) => libc::_exit(2),
                Err(e) => e,
            };
            if err.fd != bad || err.errno != libc::EBADF {
                libc::_exit(3);
            }
            // A failed install must leave the host's descriptors alone.
            if libc::fcntl(1, libc::F_GETFD) < 0 {
                libc::_exit(5);
            }
            libc::_exit(0);
        });

        assert_eq!(st, 0, "child failed at step {}", st);
    }
}
