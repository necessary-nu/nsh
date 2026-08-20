//! Literal port of `src/output.c` / `src/output.h`.
//! Rules: `docs/spec/port/src/output.md`.
//!
//! The C introduced its own shell output routines because:
//!	When a builtin command is interrupted we have to discard
//!		any pending output.
//!	When a builtin command appears in back quotes, it can save
//!		the output in malloc-backed memory rather than fork and
//!		read the output through a pipe.
//!	Our output routines may be smaller than the stdio routines.
//!
//! ## Structural deviations in this module
//!
//! The C output cursor triplet (`buf`, `nextc`, `end`) becomes an owned
//! `Option<Vec<u8>>`. `None` preserves the pre-allocation state needed by
//! `outmem`'s exact-fill rule; `Vec::len` is the pending range and `bufsize`
//! remains the logical limit.
//!
//! The three destinations are fields of [`ShellIo`], the unit that moves
//! onto `Shell` when the remaining ambient state is threaded. `stdout()` and
//! its siblings are temporary raw-pointer accessors for the still-literal
//! callers; unlike the old `out1`/`out2` pointer statics they cannot carry a
//! second, independently mutable view of which destination is selected.

use std::io::{self, Write};

const OUTBUFSIZ: usize = 8192; /* BUFSIZ */

// [spec:dash:def:output.output]
pub struct Output {
    /// Optional pending-output storage. `None` preserves dash's lazy
    /// allocation state; `Some(_)` is the initialized buffer state.
    pub buf: Option<Vec<u8>>,
    pub bufsize: usize,
    destination: crate::fd::FdRef,
}

impl Output {
    fn new(destination: crate::fd::FdRef, bufsize: usize) -> Self {
        Self {
            buf: None,
            bufsize,
            destination,
        }
    }

    pub(crate) fn set_destination(&mut self, destination: crate::fd::FdRef) {
        self.destination = destination;
    }

    fn discard(&mut self) {
        if let Some(buf) = self.buf.as_mut() {
            buf.clear();
        }
    }
}

/// The shell's three output destinations.
///
/// This is one field in the designed `Shell`; keeping the aggregate intact
/// here makes moving it onto that instance a move rather than another output
/// rewrite.
// [spec:posix:req:xcu.stdout.terminal-background]
// [spec:posix:req:xcu.stdout.env-independence]
// [spec:posix:req:xcu.stdout.display-verb]
// [spec:posix:req:xcu.defaults.output-files-none]
pub struct ShellIo {
    stdout: Output,
    stderr: Output,
    previous_stderr: Output,
}

impl ShellIo {
    pub(crate) fn new(stdout: crate::fd::FdRef, stderr: crate::fd::FdRef) -> Self {
        Self {
            stdout: Output::new(stdout, OUTBUFSIZ),
            stderr: Output::new(stderr, 0),
            previous_stderr: Output::new(crate::fd::FdRef::default(), 0),
        }
    }

    pub(crate) fn stdout(&mut self) -> &mut Output {
        &mut self.stdout
    }

    pub(crate) fn stderr(&mut self) -> &mut Output {
        &mut self.stderr
    }

    pub(crate) fn previous_stderr(&mut self) -> &mut Output {
        &mut self.previous_stderr
    }

    // [spec:dash:def:output.flushall-fn]
    // [spec:dash:sem:output.flushall-fn]
    pub(crate) fn flushall(&mut self) -> io::Result<()> {
        self.stdout.flush()
    }

    /// The writer `dest` names.
    ///
    /// The borrow lasts exactly as long as one write, which is the whole
    /// point of [`Dest`].
    pub(crate) fn get(&mut self, dest: Dest) -> &mut Output {
        match dest {
            Dest::Stdout => &mut self.stdout,
            Dest::Stderr => &mut self.stderr,
            Dest::PreviousStderr => &mut self.previous_stderr,
        }
    }
}

// [spec:nsh:req:idiom.no-ignored-results]
impl crate::context::Shell {
    fn command_output_error(&mut self, _error: io::Error) -> crate::error::Error {
        let line = self.eval.errlinno;
        self.diagnostics().command_warnx(b"I/O error");
        crate::error::Error::reported(line, crate::status::ExitStatus::ERROR)
    }

    /// Write command output and turn an immediate stream failure into the
    /// shell's ordinary, already-reported command error.
    pub(crate) fn write_output(
        &mut self,
        dest: Dest,
        bytes: &[u8],
    ) -> Result<(), crate::error::Error> {
        let result = self.io.get(dest).write_all(bytes);
        result.map_err(|error| self.command_output_error(error))
    }

    /// Format command output without hiding the writer's failure behind
    /// `fmt::Error`.
    pub(crate) fn write_output_fmt(
        &mut self,
        dest: Dest,
        arguments: std::fmt::Arguments<'_>,
    ) -> Result<(), crate::error::Error> {
        let result = self.io.get(dest).write_fmt(arguments);
        result.map_err(|error| self.command_output_error(error))
    }

    /// Flush buffered command output through the same explicit error path.
    pub(crate) fn flush_output(&mut self) -> Result<(), crate::error::Error> {
        let result = self.io.flushall();
        result.map_err(|error| self.command_output_error(error))
    }
}

/// Which of the shell's three writers a caller means.
///
/// This is the alternative to passing `*mut Output` around, and it exists
/// for a soundness reason rather than a stylistic one.
///
/// Six functions used to take their writer as a raw pointer — `jobs`'
/// `showjob`, `showjobs`, `showpipe` and `outcmd`, `eval`'s `eprintlist`
/// and `type`'s `describe_command` — and **nine call sites held that
/// pointer live across a call taking `&mut Shell`**. While it came from
/// `addr_of_mut!` on a static that was sound: the static outlives
/// everything, and no reborrow of the shell can invalidate a pointer that
/// never came from one. Taken from `&mut sh.io` instead — which is what
/// `docs/api-design.md` §5 makes `io` — every later reborrow of the shell
/// puts the pointer's provenance in question under Stacked and Tree
/// Borrows. It would still compile, and it might be undefined behaviour.
///
/// Naming the *destination* rather than pointing at it removes the
/// pointer, the aliasing and the provenance question together: the callee
/// resolves the `Dest` against the shell's [`ShellIo`] at each write, so
/// the borrow never spans a call. This is what has to be true before `io`
/// can become a field.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum Dest {
    /// The shell's buffered standard output.
    Stdout,
    /// The shell's unbuffered standard error.
    Stderr,
    /// The standard error saved across a redirection, which is where
    /// `set -x` tracing goes.
    PreviousStderr,
}

/* `static mut SHELL_IO` was here, with `stdout()`, `stderr()`,
 * `previous_stderr()`, `set_stream_fds()` and the transitional `io()`
 * that reached it. All six are gone: the aggregate is `Shell::io`, and
 * every writer reaches it through the receiver it already had.
 *
 * That is the `io` half of `move-state`'s blocked group. It could not
 * move while `set_stream_fds` was reached from `streams::set`, which has
 * no shell to be given; it moves now because the constructor takes the
 * streams instead. That is escape (2) of the two `move-state` recorded,
 * and the node log predicted it would be the shape. `docs/api-design.md`
 * 5's `io: ShellIo` row.
 */

/* ------------------------------------------------------------------ */
/* src/output.c                                                        */
/* ------------------------------------------------------------------ */

// The C's `outmem`, `outstr`, and `outcslow` compatibility entry points have
// no wrappers here. Callers compose the standard writer contract directly.
// [spec:dash:def:output.outmem-fn]
// [spec:dash:sem:output.outmem-fn]
// [spec:dash:def:output.outstr-fn]
// [spec:dash:sem:output.outstr-fn]
// [spec:dash:def:output.outcslow-fn]
// [spec:dash:sem:output.outcslow-fn]

struct OutputFormatter<'a> {
    output: &'a mut Output,
    error: Option<io::Error>,
}

impl std::fmt::Write for OutputFormatter<'_> {
    fn write_str(&mut self, text: &str) -> std::fmt::Result {
        match self.output.write_all(text.as_bytes()) {
            Ok(()) => Ok(()),
            Err(error) => {
                self.error = Some(error);
                Err(std::fmt::Error)
            }
        }
    }
}

// [spec:nsh:req:idiom.output-results]
impl Write for Output {
    /// Write according to [`Write::write`]'s consumption contract.
    ///
    /// Unlike the legacy `outmem` path, an error flushing bytes from a
    /// previous call must stop this call before any of `bytes` is consumed.
    /// A direct descriptor write is deliberately a single successful syscall
    /// (apart from retrying `EINTR`) so a short write can be reported to the
    /// caller as a partial count.
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        if bytes.is_empty() {
            return Ok(0);
        }

        let mut nleft = self
            .buf
            .as_ref()
            .map_or(0, |buf| self.bufsize.saturating_sub(buf.len()));
        if nleft >= bytes.len() {
            self.buf.as_mut().unwrap().extend_from_slice(bytes);
            return Ok(bytes.len());
        }

        if self.bufsize != 0 {
            if self.buf.is_none() {
                self.buf = Some(Vec::with_capacity(self.bufsize));
            } else {
                self.flush()?;
            }
        }

        nleft = self
            .buf
            .as_ref()
            .map_or(0, |buf| self.bufsize.saturating_sub(buf.len()));
        if nleft > bytes.len() {
            self.buf.as_mut().unwrap().extend_from_slice(bytes);
            Ok(bytes.len())
        } else {
            write_fd_once(bytes, &self.destination)
        }
    }

    /// dash's `flushout`: push the pending range at the descriptor.
    ///
    /// The C had a free function over a `struct output *`; the port's is the
    /// trait method, because a writer that cannot be flushed through
    /// `Write::flush` is not a writer.
    // [spec:dash:def:output.flushout-fn]
    // [spec:dash:sem:output.flushout-fn]
    fn flush(&mut self) -> io::Result<()> {
        let len = self.buf.as_ref().map_or(0, Vec::len);
        if len == 0 {
            return Ok(());
        }

        let mut pending = self.buf.take().unwrap();
        /* Reset the pending range before writing. A failed or interrupted
         * write must not leave bytes queued for a second attempt. The
         * allocation stays live, so the raw range remains readable. */
        let result = self.destination.write_all(&pending[..len]);
        pending.clear();
        self.buf = Some(pending);
        result
    }

    fn write_fmt(&mut self, arguments: std::fmt::Arguments<'_>) -> io::Result<()> {
        let mut formatter = OutputFormatter {
            output: self,
            error: None,
        };

        match std::fmt::write(&mut formatter, arguments) {
            Ok(()) => Ok(()),
            Err(_) => match formatter.error {
                Some(error) => Err(error),
                None => panic!(
                    "a formatting trait implementation returned an error when the underlying \
                     stream did not"
                ),
            },
        }
    }
}

/*
 * Version of write which resumes after a signal is caught.
 */

fn write_fd_once(bytes: &[u8], fd: &crate::fd::FdRef) -> io::Result<usize> {
    let amount = bytes.len().min(isize::MAX as usize);
    fd.write_once(&bytes[..amount])
}

// The C's `xwrite` is the platform layer's fallible `write_all` operation.
// [spec:dash:def:output.xwrite-fn]
// [spec:dash:sem:output.xwrite-fn]

// The reference's unused C-stdio routines have no Rust counterparts.
// [spec:dash:def:output.initstreams-fn]
// [spec:dash:sem:output.initstreams-fn]
// [spec:dash:def:output.openmemout-fn]
// [spec:dash:sem:output.openmemout-fn]
// [spec:dash:def:output.closememout-fn]
// [spec:dash:sem:output.closememout-fn]

/* ------------------------------------------------------------------ */
/* src/output.h                                                        */
/* ------------------------------------------------------------------ */

// [spec:dash:def:output.freestdout-fn]
// [spec:dash:sem:output.freestdout-fn]
#[inline]
pub fn freestdout(io: &mut ShellIo) {
    io.stdout().discard();
}

// The C's `outc` macro is `Write::write_all` over a one-byte slice.
// [spec:dash:def:output.outc-fn]
// [spec:dash:sem:output.outc-fn]

/* The five `out1*`/`out2*` macros in output.h — `out1c`, `out2c`,
 * `out1mem`, `out1str`, `out2str` — were each `outc`/`outmem`/`outstr`
 * against one of the two pointer statics. With the statics gone their
 * callers name the destination and write to it, so the macros have no
 * remaining meaning and are not ported. */

// ---------------------------------------------------------------------
// Unit tests for this module's functions.
//
// Everything here writes to a real file descriptor, because that is what
// distinguishes this module's behaviour: whether a byte is sitting in the
// buffer or has reached the fd is the whole question for outmem, flushout
// and outc. Each test owns a pipe and a `struct output` pointing at it.
// ---------------------------------------------------------------------
#[cfg(test)]
mod tests {
    use super::*;

    fn destination(fd: Option<crate::fd::SharedFd>) -> crate::fd::FdRef {
        let destination = crate::fd::FdRef::default();
        destination.replace(fd);
        destination
    }

    /// A `struct output` writing into a pipe, with a buffer of `bufsize`.
    struct Sink {
        out: Box<Output>,
        r: Option<nsh_platform::Descriptor>,
        w: Option<crate::fd::SharedFd>,
    }

    impl Sink {
        fn new(bufsize: usize, allocated: bool) -> Sink {
            let (read, write) = nsh_platform::pipe().expect("create output test pipe");
            let write = crate::fd::SharedFd::from_owned(write).unwrap();
            let out = Box::new(Output {
                buf: allocated.then(|| Vec::with_capacity(bufsize)),
                bufsize: bufsize as usize,
                destination: destination(Some(write.clone())),
            });
            Sink {
                out,
                r: Some(read),
                w: Some(write),
            }
        }
        fn read(&self) -> &nsh_platform::Descriptor {
            self.r.as_ref().expect("reader is still owned")
        }
        fn write(&self) -> &crate::fd::SharedFd {
            self.w.as_ref().expect("writer is still owned")
        }
        fn close_output(&mut self) {
            self.out.set_destination(crate::fd::FdRef::default());
        }
        fn restore_output(&mut self) {
            let write = self.write().clone();
            self.out.set_destination(destination(Some(write)));
        }
        /// Bytes that have actually reached the pipe.
        fn drained(&mut self) -> Vec<u8> {
            // Close the writer so the read sees EOF rather than blocking.
            self.close_output();
            drop(self.w.take());
            nsh_platform::read_to_end(self.read()).expect("drain output test pipe")
        }
        /// Bytes still sitting in the buffer, unflushed.
        fn buffered(&self) -> usize {
            self.out.buf.as_ref().map_or(0, Vec::len)
        }
    }

    // [spec:dash:sem:output.outmem-fn/test]
    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn writes_buffer_until_full() {
        let mut s = Sink::new(16, true);
        s.out.write_all(b"abc").unwrap();
        assert_eq!(s.buffered(), 3);
        s.out.write_all(b"defghij").unwrap();
        assert_eq!(s.buffered(), 10);
        s.out.flush().unwrap();
        assert_eq!(s.buffered(), 0);
        assert_eq!(s.drained(), b"abcdefghij");

        let mut s = Sink::new(4, true);
        s.out.write_all(b"ab").unwrap();
        s.out.write_all(b"0123456789").unwrap();
        s.out.flush().unwrap();
        assert_eq!(s.drained(), b"ab0123456789");

        let mut lazy = Sink::new(4, false);
        lazy.out.write_all(b"abcd").unwrap();
        assert_eq!(lazy.buffered(), 0);
        assert_eq!(lazy.drained(), b"abcd");

        let mut allocated = Sink::new(4, true);
        allocated.out.write_all(b"abcd").unwrap();
        assert_eq!(allocated.buffered(), 4);
        allocated.out.flush().unwrap();
        assert_eq!(allocated.drained(), b"abcd");

        let mut failed = Sink::new(16, true);
        let base = failed.out.buf.as_ref().unwrap().as_ptr();
        let capacity = failed.out.buf.as_ref().unwrap().capacity();
        failed.out.write_all(b"discarded").unwrap();
        failed.close_output();
        assert!(failed.out.flush().is_err());
        assert_eq!(failed.buffered(), 0);
        assert_eq!(failed.out.buf.as_ref().unwrap().as_ptr(), base);
        assert_eq!(failed.out.buf.as_ref().unwrap().capacity(), capacity);

        failed.restore_output();
        failed.out.write_all(b"kept").unwrap();
        failed.out.flush().unwrap();
        assert_eq!(failed.drained(), b"kept");

        let bytes = [b'a', 0, b'b', 0xff];
        let mut binary = Sink::new(8, true);
        binary.out.write_all(&bytes).unwrap();
        assert_eq!(binary.buffered(), bytes.len());
        binary.out.flush().unwrap();
        assert_eq!(binary.drained(), bytes);

        let bytes: Vec<u8> = (0..=u8::MAX).collect();
        let mut binary = Sink::new(300, true);
        let base = binary.out.buf.as_ref().unwrap().as_ptr();
        binary.out.write_all(&bytes).unwrap();
        assert_eq!(binary.buffered(), bytes.len());
        binary.out.flush().unwrap();
        assert_eq!(binary.out.buf.as_ref().unwrap().as_ptr(), base);
        assert_eq!(binary.drained(), bytes);
    }

    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn flushout_reports_a_closed_logical_descriptor() {
        let mut s = Sink::new(16, true);
        s.out.flush().unwrap();
        s.out.write_all(b"xy").unwrap();
        s.close_output();
        assert!(s.out.flush().is_err());
        assert_eq!(s.buffered(), 0);
    }

    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn flush_returns_bad_descriptor() {
        let mut s = Sink::new(16, true);
        s.out.write_all(b"z").unwrap();
        s.close_output();
        assert!(s.out.flush().is_err());
    }

    #[test]
    fn write_all_appends_bytes() {
        let mut s = Sink::new(64, true);
        s.out.write_all(b"hi! o").unwrap();
        assert_eq!(s.buffered(), 5);
        s.out.flush().unwrap();
        assert_eq!(s.drained(), b"hi! o");
    }

    #[test]
    fn write_reports_the_current_operation() {
        let mut s = Sink::new(0, false);
        s.close_output();
        assert!(s.out.write_all(b"bad").is_err());

        s.restore_output();
        assert!(s.out.write_all(b"good").is_ok());
        assert_eq!(s.drained(), b"good");
    }

    #[test]
    fn write_fmt_buffers_and_reports_errors() {
        let mut buffered = Sink::new(64, true);
        write!(&mut *buffered.out, "{}={}", "x", 42).unwrap();
        assert_eq!(buffered.out.buf.as_deref(), Some(&b"x=42"[..]));
        buffered.out.flush().unwrap();
        assert_eq!(buffered.drained(), b"x=42");

        let mut failed = Sink::new(0, false);
        failed.close_output();
        assert!(write!(&mut *failed.out, "{}", "bad").is_err());
    }

    #[test]
    fn write_consumes_nothing_when_flush_fails() {
        let mut s = Sink::new(4, true);
        s.out.buf.as_mut().unwrap().extend_from_slice(b"abc");
        s.close_output();

        assert!(s.out.write(b"xy").is_err());
        assert_eq!(s.out.buf.as_deref(), Some(&b""[..]));

        s.restore_output();
        assert_eq!(s.out.write(b"xy").unwrap(), 2);
        s.out.flush().unwrap();
        assert_eq!(s.drained(), b"xy");
    }

    #[test]
    fn write_all_returns_flush_error() {
        let mut s = Sink::new(4, true);
        s.out.write_all(b"abc").unwrap();
        s.close_output();

        assert!(s.out.write_all(b"xy").is_err());
        assert_eq!(s.out.buf.as_deref(), Some(&b""[..]));

        s.restore_output();
        s.out.write_all(b"xy").unwrap();
        s.out.flush().unwrap();
        assert_eq!(s.drained(), b"xy");
    }

    #[test]
    fn write_reports_a_kernel_short_write() {
        if !nsh_platform::reports_pipe_short_writes() {
            return;
        }
        let mut s = Sink::new(0, false);
        nsh_platform::set_nonblocking(s.write(), true).unwrap();

        let pipe_buf = nsh_platform::PIPE_BUFFER;
        let fill = vec![b'q'; pipe_buf];
        let mut filled = 0usize;
        loop {
            match nsh_platform::write_once(s.write(), &fill) {
                Ok(written) => filled += written,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => break,
                Err(error) => panic!("unexpected pipe fill error: {error}"),
            }
        }

        nsh_platform::read_exact(s.read(), pipe_buf).unwrap();

        let payload = vec![b'x'; pipe_buf * 2];
        let written = s.out.write(&payload).unwrap();
        assert!(written > 0);
        assert!(written < payload.len());

        let got = s.drained();
        assert_eq!(got.len(), filled - pipe_buf + written);
        assert_eq!(&got[got.len() - written..], &payload[..written]);
    }

    // [spec:nsh:req:idiom.filesystem-account-bytes/test]
    #[test]
    fn shell_io_instances_own_independent_writers() {
        let open = || {
            let fd = nsh_platform::anonymous_file("shell-io-test").unwrap();
            destination(Some(crate::fd::SharedFd::from_owned(fd).unwrap()))
        };
        let mut first = ShellIo::new(open(), open());
        let mut second = ShellIo::new(open(), open());

        first.stdout().buf = Some(b"first".to_vec());
        first.stderr().set_destination(crate::fd::FdRef::default());
        first.previous_stderr().set_destination(destination(Some(
            crate::fd::SharedFd::from_owned(
                nsh_platform::anonymous_file("previous-stderr").unwrap(),
            )
            .unwrap(),
        )));

        assert!(first.stdout().destination.is_open());
        assert!(!first.stderr().destination.is_open());
        assert!(first.previous_stderr().destination.is_open());
        assert!(second.stdout().destination.is_open());
        assert!(second.stderr().destination.is_open());
        assert!(!second.previous_stderr().destination.is_open());
        assert!(second.stdout().buf.is_none());
    }

    // [spec:dash:sem:output.freestdout-fn/test]
    #[test]
    fn freestdout_resets_the_buffer() {
        let mut io = ShellIo::new(crate::fd::FdRef::default(), crate::fd::FdRef::default());
        io.stdout().buf = Some(Vec::with_capacity(16));
        io.stdout()
            .buf
            .as_mut()
            .unwrap()
            .extend_from_slice(b"abcde");
        io.stdout().bufsize = 16;
        freestdout(&mut io);

        assert_eq!(io.stdout().buf.as_ref().unwrap().len(), 0);
    }

    // [spec:dash:sem:output.flushall-fn/test]
    #[test]
    fn flushall_drains_the_stdout_writer() {
        let mut s = Sink::new(64, true);
        let mut io = ShellIo::new(s.out.destination.clone(), crate::fd::FdRef::default());
        io.stdout().bufsize = s.out.bufsize;

        io.stdout().write_all(b"n=3").unwrap();
        assert_eq!(io.stdout().buf.as_ref().unwrap().len(), 3);
        io.flushall().unwrap();
        assert_eq!(io.stdout().buf.as_ref().unwrap().len(), 0);
        drop(io);
        assert_eq!(s.drained(), b"n=3");
    }
}
