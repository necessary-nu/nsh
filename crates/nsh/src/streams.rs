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
    pub stdin: c_int,
    pub stdout: c_int,
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

    fn as_array(&self) -> [c_int; 3] {
        [self.stdin, self.stdout, self.stderr]
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

/// Which descriptors the shell's own I/O uses.
///
/// Read through [`streams`]; written only by [`set`] and [`install`].
static mut STREAMS: Streams = Streams::INHERIT;

/// The streams the shell is currently using.
///
/// # Safety
/// Reads a process-global. Sound as long as [`set`] and [`install`] are
/// not called concurrently with shell execution, which is the same
/// single-instance constraint the rest of the port is under until
/// [dec:nsh:no-ambient-state] lands.
#[inline]
pub unsafe fn streams() -> Streams {
    STREAMS
}

/// Point the shell's own I/O at `s` without disturbing the process's
/// standard descriptors.
///
/// See the module comment for what this does and does not carry: the
/// shell's own reads and writes follow, the language's descriptor numbers
/// do not.
///
/// # Safety
/// Must not be called while the shell is running.
pub unsafe fn set(s: Streams) {
    STREAMS = s;
    // `out1` and `out2` are the shell's own writes, so they follow the
    // streams rather than the numbers. Doing it here rather than in an
    // init fragment means there is no window in which the buffers point
    // somewhere the caller did not ask for.
    crate::output::output.fd = s.stdout;
    crate::output::errout.fd = s.stderr;
}

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
        set(s);
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
                errno: *libc::__errno_location(),
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
            let errno = *libc::__errno_location();
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
                errno: *libc::__errno_location(),
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

    // The shell's own I/O now goes through the standard descriptors,
    // because that is where its streams have been put.
    set(Streams::INHERIT);
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
            set(Streams::INHERIT);
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
            assert_eq!(streams(), Streams::INHERIT);
            b.restore();
            assert_eq!(streams(), Streams::INHERIT);
        }
    }

    /// `set` is the mode for a host that cannot have descriptor 1 swapped
    /// out from under it: the shell's own writes move, the process's
    /// standard descriptors do not.
    #[test]
    fn set_moves_the_shells_own_io_without_touching_descriptor_one() {
        let _g = lock();
        unsafe {
            let (r, w) = pipe();
            set(Streams {
                stdin: 0,
                stdout: w,
                stderr: 2,
            });
            assert_eq!(streams().stdout, w);
            assert!(libc::fcntl(1, libc::F_GETFD) >= 0);
            set(Streams::INHERIT);
            libc::close(r);
            libc::close(w);
        }
    }

    /// `out1` and `out2` are the shell's own writers, so they have to
    /// follow the streams. If they did not, `echo` would keep writing to
    /// descriptor 1 while everything else moved.
    #[test]
    fn the_shells_writers_follow_the_streams() {
        let _g = lock();
        unsafe {
            set(Streams {
                stdin: 7,
                stdout: 8,
                stderr: 9,
            });
            let (out, err) = (crate::output::output.fd, crate::output::errout.fd);
            assert_eq!((out, err), (8, 9));
            set(Streams::INHERIT);
            let (out, err) = (crate::output::output.fd, crate::output::errout.fd);
            assert_eq!((out, err), (1, 2));
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
            if *libc::__errno_location() != libc::EBADF {
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
            if streams() != Streams::INHERIT {
                libc::_exit(4);
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
