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

use core::ptr::addr_of_mut;
use std::io::{self, Write};

use libc::{c_char, c_int, c_void, size_t};

use crate::error::{INTOFF, INTON};
use crate::shell::likely;

const OUTBUFSIZ: size_t = 8192; /* BUFSIZ */
pub const MEM_OUT: c_int = -3; /* output to dynamically allocated memory */

pub const OUTPUT_ERR: c_int = 0o1; /* error occurred on output */

// [spec:dash:def:output.output]
pub struct Output {
    /// Optional pending-output storage. `None` preserves dash's lazy
    /// allocation state; `Some(_)` is the initialized buffer state.
    pub buf: Option<Vec<u8>>,
    pub bufsize: size_t,
    pub fd: c_int,
    pub flags: c_int,
}

impl Output {
    const fn new(fd: c_int, bufsize: size_t) -> Self {
        Self {
            buf: None,
            bufsize,
            fd,
            flags: 0,
        }
    }

    fn remember_error<T>(&mut self, result: io::Result<T>) -> io::Result<T> {
        if result.is_err() {
            self.flags |= OUTPUT_ERR;
        }
        result
    }

    fn write_bytes(&mut self, bytes: &[u8]) -> io::Result<()> {
        if bytes.is_empty() {
            return Ok(());
        }

        let mut nleft = self
            .buf
            .as_ref()
            .map_or(0, |buf| self.bufsize.saturating_sub(buf.len()));
        if likely(nleft >= bytes.len()) {
            self.buf.as_mut().unwrap().extend_from_slice(bytes);
            return Ok(());
        }

        let mut first_error = None;
        if self.bufsize == 0 {
            /* unbuffered — fall through to the direct write */
        } else if self.buf.is_none() {
            /* The inactive MEM_OUT growth branches from the C do not apply
             * to any shipped configuration. */
            unsafe { INTOFF() };
            self.buf = Some(Vec::with_capacity(self.bufsize));
            unsafe { INTON() };
        } else if let Err(error) = self.flush() {
            first_error = Some(error);
        }

        nleft = self
            .buf
            .as_ref()
            .map_or(0, |buf| self.bufsize.saturating_sub(buf.len()));
        /* This second comparison is deliberately strict. The first write
         * into a lazily allocated buffer bypasses it when it would exactly
         * fill the buffer; an already allocated buffer uses the `>=` fast
         * path above. */
        if nleft > bytes.len() {
            self.buf.as_mut().unwrap().extend_from_slice(bytes);
        } else if let Err(error) = self.remember_error(write_fd(bytes, self.fd)) {
            if first_error.is_none() {
                first_error = Some(error);
            }
        }

        match first_error {
            Some(error) => Err(error),
            None => Ok(()),
        }
    }

    fn discard(&mut self) {
        if let Some(buf) = self.buf.as_mut() {
            buf.clear();
        }
        self.flags = 0;
    }
}

/// The shell's three output destinations.
///
/// This is one field in the designed `Shell`; keeping the aggregate intact
/// here makes moving it onto that instance a move rather than another output
/// rewrite.
pub struct ShellIo {
    stdout: Output,
    stderr: Output,
    previous_stderr: Output,
}

impl ShellIo {
    pub(crate) const fn new(stdout_fd: c_int, stderr_fd: c_int) -> Self {
        Self {
            stdout: Output::new(stdout_fd, OUTBUFSIZ),
            stderr: Output::new(stderr_fd, 0),
            previous_stderr: Output::new(0, 0),
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
}

// The surrounding shell still has ambient state. `move-state` moves this
// already-owned aggregate onto the concrete Shell instance after
// `thread-context` makes that instance reachable at every call site.
static mut SHELL_IO: ShellIo = ShellIo::new(1, 2);

/// The shell's buffered standard-output writer.
#[inline]
pub unsafe fn stdout() -> *mut Output {
    addr_of_mut!(SHELL_IO.stdout)
}

/// The shell's unbuffered standard-error writer.
#[inline]
pub unsafe fn stderr() -> *mut Output {
    addr_of_mut!(SHELL_IO.stderr)
}

/// The saved standard-error writer used by `set -x` across redirection.
#[inline]
pub unsafe fn previous_stderr() -> *mut Output {
    addr_of_mut!(SHELL_IO.previous_stderr)
}

/// Point the shell-owned writers at the descriptors supplied by its host.
#[inline]
pub unsafe fn set_stream_fds(stdout_fd: c_int, stderr_fd: c_int) {
    (*stdout()).fd = stdout_fd;
    (*stderr()).fd = stderr_fd;
}

/*
 * #ifdef notyet
 * struct output memout = { .fd = MEM_OUT, ... };
 * #endif
 */

/* ------------------------------------------------------------------ */
/* src/output.c                                                        */
/* ------------------------------------------------------------------ */

// [spec:dash:def:output.outmem-fn]
// [spec:dash:sem:output.outmem-fn]
pub unsafe fn outmem(p: *const c_char, len: size_t, dest: *mut Output) {
    if len == 0 {
        return;
    }
    let bytes = core::slice::from_raw_parts(p as *const u8, len);
    let _ = (*dest).write_bytes(bytes);
}

// [spec:dash:def:output.outstr-fn]
// [spec:dash:sem:output.outstr-fn]
pub unsafe fn outstr(p: *const c_char, file: *mut Output) {
    let len: size_t;

    len = libc::strlen(p);
    outmem(p, len, file);
}

// [spec:dash:def:output.outcslow-fn]
// [spec:dash:sem:output.outcslow-fn]
pub unsafe fn outcslow(c: c_int, dest: *mut Output) {
    let buf: c_char = c as c_char;
    outmem(&buf as *const c_char, 1, dest);
}

// [spec:dash:def:output.flushall-fn]
// [spec:dash:sem:output.flushall-fn]
pub unsafe fn flushall() {
    let _ = (*stdout()).flush();
    /*
     * #ifdef FLUSHERR
     *	flushout(&errout);
     * #endif
     * — FLUSHERR is not defined in the shipped build.
     */
}

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

struct FormattingInterruptGuard;

impl FormattingInterruptGuard {
    fn enter() -> Self {
        unsafe { INTOFF() };
        Self
    }
}

impl Drop for FormattingInterruptGuard {
    fn drop(&mut self) {
        unsafe { INTON() };
    }
}

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
        if likely(nleft >= bytes.len()) {
            self.buf.as_mut().unwrap().extend_from_slice(bytes);
            return Ok(bytes.len());
        }

        if self.bufsize != 0 {
            if self.buf.is_none() {
                unsafe { INTOFF() };
                self.buf = Some(Vec::with_capacity(self.bufsize));
                unsafe { INTON() };
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
            let result = write_fd_once(bytes, self.fd);
            self.remember_error(result)
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
        if len == 0 || self.fd < 0 {
            return Ok(());
        }

        let buf = self.buf.as_mut().unwrap();
        let bytes = buf.as_ptr();
        /* Reset the pending range before writing. A failed or interrupted
         * write must not leave bytes queued for a second attempt. The
         * allocation stays live, so the raw range remains readable. */
        buf.clear();
        let result = unsafe { write_fd(core::slice::from_raw_parts(bytes, len), self.fd) };
        self.remember_error(result)
    }

    fn write_fmt(&mut self, arguments: std::fmt::Arguments<'_>) -> io::Result<()> {
        let _interrupts = FormattingInterruptGuard::enter();
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

fn write_fd_once(bytes: &[u8], fd: c_int) -> io::Result<usize> {
    let amount = bytes.len().min(crate::system::SSIZE_MAX as usize);
    loop {
        let written = unsafe { libc::write(fd, bytes.as_ptr() as *const c_void, amount) };
        if written < 0 {
            let error = io::Error::last_os_error();
            if error.raw_os_error() == Some(libc::EINTR) {
                continue;
            }
            return Err(error);
        }
        return Ok(written as usize);
    }
}

fn write_fd(mut bytes: &[u8], fd: c_int) -> io::Result<()> {
    while !bytes.is_empty() {
        let written = write_fd_once(bytes, fd)?;
        bytes = &bytes[written..];
    }
    Ok(())
}

// [spec:dash:def:output.xwrite-fn]
// [spec:dash:sem:output.xwrite-fn]
pub unsafe fn xwrite(fd: c_int, p: *const c_void, n: size_t) -> c_int {
    if n == 0 {
        return 0;
    }
    let bytes = core::slice::from_raw_parts(p as *const u8, n);
    if write_fd(bytes, fd).is_ok() { 0 } else { -1 }
}

/*
 * The three routines below sit inside `#ifdef notyet` *and*
 * `#ifdef USE_GLIBC_STDIO`, neither of which is defined in any shipped
 * configuration: `struct output` has no `stream` member and there is no
 * `memout`.  Their annotations therefore ride on equally inactive
 * bodies, with the C retained as a comment.
 */

// [spec:dash:def:output.initstreams-fn]
// [spec:dash:sem:output.initstreams-fn]
pub unsafe fn initstreams() {
    /* output.stream = stdout; */
    /* errout.stream = stderr; */
}

// [spec:dash:def:output.openmemout-fn]
// [spec:dash:sem:output.openmemout-fn]
pub unsafe fn openmemout() {
    /* INTOFF; */
    /* memout.stream = open_memstream(&memout.buf, &memout.bufsize); */
    /* INTON; */
}

// [spec:dash:def:output.closememout-fn]
// [spec:dash:sem:output.closememout-fn]
pub unsafe fn __closememout() -> c_int {
    /* int error; */
    /* error = fclose(memout.stream); */
    /* memout.stream = NULL; */
    /* return error; */
    0
}

/* ------------------------------------------------------------------ */
/* src/output.h                                                        */
/* ------------------------------------------------------------------ */

// [spec:dash:def:output.freestdout-fn]
// [spec:dash:sem:output.freestdout-fn]
#[inline]
pub unsafe fn freestdout() {
    (*stdout()).discard();
}

// [spec:dash:def:output.outc-fn]
// [spec:dash:sem:output.outc-fn]
#[inline]
pub unsafe fn outc(ch: c_int, file: *mut Output) {
    if let Some(buf) = (*file).buf.as_mut() {
        if buf.len() < (*file).bufsize {
            buf.push(ch as u8);
            return;
        }
    }
    outcslow(ch, file);
}

/* The five `out1*`/`out2*` macros in output.h — `out1c`, `out2c`,
 * `out1mem`, `out1str`, `out2str` — were each `outc`/`outmem`/`outstr`
 * against one of the two pointer statics. With the statics gone their
 * callers name the destination and write to it, so the macros have no
 * remaining meaning and are not ported. */

/* `#define outerr(f) (f)->flags` */
#[inline(always)]
pub unsafe fn outerr(f: *mut Output) -> c_int {
    (*f).flags
}

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
    use crate::testutil::CStr0;

    /// A `struct output` writing into a pipe, with a buffer of `bufsize`.
    struct Sink {
        out: Box<Output>,
        r: c_int,
        w: c_int,
    }

    impl Sink {
        fn new(bufsize: usize, allocated: bool) -> Sink {
            let mut fds = [0 as c_int; 2];
            assert_eq!(unsafe { libc::pipe(fds.as_mut_ptr()) }, 0);
            let out = Box::new(Output {
                buf: allocated.then(|| Vec::with_capacity(bufsize)),
                bufsize: bufsize as size_t,
                fd: fds[1],
                flags: 0,
            });
            Sink {
                out,
                r: fds[0],
                w: fds[1],
            }
        }
        fn p(&mut self) -> *mut Output {
            &mut *self.out as *mut Output
        }
        /// Bytes that have actually reached the pipe.
        fn drained(&mut self) -> Vec<u8> {
            unsafe {
                // Close the writer so the read sees EOF rather than blocking.
                libc::close(self.w);
                self.w = -1;
                let mut got = Vec::new();
                let mut tmp = [0u8; 4096];
                loop {
                    let n = libc::read(self.r, tmp.as_mut_ptr() as *mut c_void, tmp.len());
                    if n <= 0 {
                        break;
                    }
                    got.extend_from_slice(&tmp[..n as usize]);
                }
                got
            }
        }
        /// Bytes still sitting in the buffer, unflushed.
        fn buffered(&self) -> usize {
            self.out.buf.as_ref().map_or(0, Vec::len)
        }
    }

    impl Drop for Sink {
        fn drop(&mut self) {
            unsafe {
                if self.w >= 0 {
                    libc::close(self.w);
                }
                libc::close(self.r);
            }
        }
    }

    // [spec:dash:sem:output.outmem-fn/test]
    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn outmem_buffers_until_the_buffer_cannot_hold_the_write() {
        unsafe {
            let mut s = Sink::new(16, true);
            let p = s.p();
            outmem(CStr0::new("abc").p(), 3, p);
            // Fits: buffered, nothing on the fd yet.
            assert_eq!(s.buffered(), 3);
            outmem(CStr0::new("defghij").p(), 7, p);
            assert_eq!(s.buffered(), 10);
            let _ = (*p).flush();
            assert_eq!(s.buffered(), 0);
            assert_eq!(s.drained(), b"abcdefghij");
        }
        unsafe {
            // A write larger than the buffer cannot be buffered, so the
            // buffer is flushed and the payload goes straight out.
            let mut s = Sink::new(4, true);
            let p = s.p();
            outmem(CStr0::new("ab").p(), 2, p);
            outmem(CStr0::new("0123456789").p(), 10, p);
            let _ = (*p).flush();
            assert_eq!(s.drained(), b"ab0123456789");
        }
        unsafe {
            /* On the first write the C allocates before its strict `>`
             * test, so an exact fill bypasses the newly allocated buffer. */
            let mut lazy = Sink::new(4, false);
            let p = lazy.p();
            outmem(CStr0::new("abcd").p(), 4, p);
            assert_eq!(lazy.buffered(), 0);
            assert_eq!(lazy.drained(), b"abcd");

            /* Once allocated, the initial `>=` test buffers an exact fill. */
            let mut allocated = Sink::new(4, true);
            let p = allocated.p();
            outmem(CStr0::new("abcd").p(), 4, p);
            assert_eq!(allocated.buffered(), 4);
            let _ = (*p).flush();
            assert_eq!(allocated.drained(), b"abcd");
        }
        unsafe {
            /* A failed flush discards the pending range before writing but
             * retains its allocation for the next builtin. */
            let mut failed = Sink::new(16, true);
            let p = failed.p();
            let base = (*p).buf.as_ref().unwrap().as_ptr();
            let capacity = (*p).buf.as_ref().unwrap().capacity();
            outmem(CStr0::new("discarded").p(), 9, p);
            (*p).fd = 9999;
            let _ = (*p).flush();
            assert_ne!((*p).flags & OUTPUT_ERR, 0);
            assert_eq!(failed.buffered(), 0);
            assert_eq!((*p).buf.as_ref().unwrap().as_ptr(), base);
            assert_eq!((*p).buf.as_ref().unwrap().capacity(), capacity);

            (*p).fd = failed.w;
            (*p).flags = 0;
            outmem(CStr0::new("kept").p(), 4, p);
            let _ = (*p).flush();
            assert_eq!((*p).flags & OUTPUT_ERR, 0);
            assert_eq!(failed.drained(), b"kept");
        }
        unsafe {
            /* outmem is length-based, so an embedded NUL is data rather
             * than a terminator and arbitrary shell bytes survive. */
            let bytes = [b'a', 0, b'b', 0xff];
            let mut binary = Sink::new(8, true);
            let p = binary.p();
            outmem(bytes.as_ptr() as *const c_char, bytes.len(), p);
            assert_eq!(binary.buffered(), bytes.len());
            let _ = (*p).flush();
            assert_eq!(binary.drained(), bytes);
        }
        unsafe {
            /* Exercise every byte through the owned buffer, including the
             * signed-c_char boundary that text-only storage would corrupt. */
            let bytes: Vec<u8> = (0..=u8::MAX).collect();
            let mut binary = Sink::new(300, true);
            let p = binary.p();
            let base = (*p).buf.as_ref().unwrap().as_ptr();
            outmem(bytes.as_ptr() as *const c_char, bytes.len(), p);
            assert_eq!(binary.buffered(), bytes.len());
            let _ = (*p).flush();
            assert_eq!((*p).buf.as_ref().unwrap().as_ptr(), base);
            assert_eq!(binary.drained(), bytes);
        }
    }

    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn flushout_is_a_noop_when_empty_or_closed() {
        unsafe {
            let mut s = Sink::new(16, true);
            let p = s.p();
            // Nothing buffered: no write, no error flag.
            let _ = (*p).flush();
            assert_eq!((*p).flags & OUTPUT_ERR, 0);
            // A negative fd is the "discard" case and must not be written
            // to; the buffer is left alone.
            outmem(CStr0::new("xy").p(), 2, p);
            (*p).fd = -1;
            let _ = (*p).flush();
            assert_eq!(s.buffered(), 2);
            (*p).fd = s.w;
            let _ = (*p).flush();
            assert_eq!(s.drained(), b"xy");
        }
    }

    // [spec:dash:sem:output.flushout-fn/test]
    #[test]
    fn flushout_records_an_error_on_a_bad_descriptor() {
        unsafe {
            let mut s = Sink::new(16, true);
            let p = s.p();
            outmem(CStr0::new("z").p(), 1, p);
            (*p).fd = 9999; // never opened
            let _ = (*p).flush();
            assert_ne!((*p).flags & OUTPUT_ERR, 0);
            assert_ne!(outerr(p), 0);
            (*p).fd = -1; // keep Drop quiet
        }
    }

    // [spec:dash:sem:output.outstr-fn/test]
    // [spec:dash:sem:output.outcslow-fn/test]
    // [spec:dash:sem:output.outc-fn/test]
    #[test]
    fn outstr_outcslow_and_outc_append_bytes() {
        unsafe {
            let mut s = Sink::new(64, true);
            let p = s.p();
            outstr(CStr0::new("hi").p(), p);
            outcslow('!' as c_int, p);
            outc(' ' as c_int, p);
            outc('o' as c_int, p);
            // outstr stops at the NUL; outcslow and outc each add one byte.
            assert_eq!(s.buffered(), 5);
            let _ = (*p).flush();
            assert_eq!(s.drained(), b"hi! o");
        }
    }

    #[test]
    fn write_reports_the_current_operation() {
        let mut s = Sink::new(0, false);
        s.out.flags = OUTPUT_ERR;
        s.out.fd = 9999;
        assert!(s.out.write_all(b"bad").is_err());

        s.out.fd = s.w;
        assert!(s.out.write_all(b"good").is_ok());
        assert_ne!(s.out.flags & OUTPUT_ERR, 0);
        assert_eq!(s.drained(), b"good");
    }

    #[test]
    fn write_fmt_restores_interrupt_state() {
        let _g = crate::testutil::lock();
        let suppressint_before = unsafe { crate::error::suppressint };

        let mut buffered = Sink::new(64, true);
        write!(&mut *buffered.out, "{}={}", "x", 42).unwrap();
        assert_eq!(buffered.out.buf.as_deref(), Some(&b"x=42"[..]));
        assert_eq!(unsafe { crate::error::suppressint }, suppressint_before);
        buffered.out.flush().unwrap();
        assert_eq!(buffered.drained(), b"x=42");

        let mut failed = Sink::new(0, false);
        failed.out.fd = 9999;
        assert!(write!(&mut *failed.out, "{}", "bad").is_err());
        assert_eq!(unsafe { crate::error::suppressint }, suppressint_before);
    }

    #[test]
    fn write_consumes_nothing_when_flush_fails() {
        let mut s = Sink::new(4, true);
        s.out.buf.as_mut().unwrap().extend_from_slice(b"abc");
        s.out.fd = 9999;

        assert!(s.out.write(b"xy").is_err());
        assert_eq!(s.out.buf.as_deref(), Some(&b""[..]));
        assert_ne!(s.out.flags & OUTPUT_ERR, 0);

        s.out.fd = s.w;
        assert_eq!(s.out.write(b"xy").unwrap(), 2);
        s.out.flush().unwrap();
        assert_eq!(s.drained(), b"xy");
    }

    #[test]
    fn outmem_continues_past_a_flush_error() {
        unsafe {
            let mut s = Sink::new(4, true);
            let p = s.p();
            outmem(CStr0::new("abc").p(), 3, p);
            (*p).fd = 9999;

            outmem(CStr0::new("xy").p(), 2, p);
            assert_eq!((*p).buf.as_deref(), Some(&b"xy"[..]));
            assert_ne!((*p).flags & OUTPUT_ERR, 0);

            (*p).fd = s.w;
            let _ = (*p).flush();
            assert_eq!(s.drained(), b"xy");
        }
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn write_reports_a_kernel_short_write() {
        let mut s = Sink::new(0, false);
        unsafe {
            let flags = libc::fcntl(s.w, libc::F_GETFL);
            assert!(flags >= 0);
            assert_eq!(libc::fcntl(s.w, libc::F_SETFL, flags | libc::O_NONBLOCK), 0);

            let pipe_buf = libc::fpathconf(s.w, libc::_PC_PIPE_BUF);
            assert!(pipe_buf > 0);
            let pipe_buf = pipe_buf as usize;
            let fill = vec![b'q'; pipe_buf];
            let mut filled = 0usize;
            loop {
                let written = libc::write(s.w, fill.as_ptr() as *const c_void, fill.len());
                if written > 0 {
                    filled += written as usize;
                    continue;
                }
                assert_eq!(written, -1);
                let error = io::Error::last_os_error();
                let raw_error = error.raw_os_error();
                assert!(raw_error == Some(libc::EAGAIN) || raw_error == Some(libc::EWOULDBLOCK));
                break;
            }

            let mut drained = vec![0u8; pipe_buf];
            let mut offset = 0usize;
            while offset < drained.len() {
                let amount = libc::read(
                    s.r,
                    drained[offset..].as_mut_ptr() as *mut c_void,
                    drained.len() - offset,
                );
                assert!(amount > 0);
                offset += amount as usize;
            }

            let payload = vec![b'x'; pipe_buf * 2];
            let written = s.out.write(&payload).unwrap();
            assert!(written > 0);
            assert!(written < payload.len());
            assert_eq!(s.out.flags & OUTPUT_ERR, 0);

            let got = s.drained();
            assert_eq!(got.len(), filled - pipe_buf + written);
            assert_eq!(&got[got.len() - written..], &payload[..written]);
        }
    }

    #[test]
    fn shell_io_instances_own_independent_writers() {
        let mut first = ShellIo::new(10, 11);
        let mut second = ShellIo::new(20, 21);

        first.stdout().flags = OUTPUT_ERR;
        first.stderr().fd = 12;
        first.previous_stderr().fd = 13;

        assert_eq!(first.stdout().fd, 10);
        assert_eq!(first.stderr().fd, 12);
        assert_eq!(first.previous_stderr().fd, 13);
        assert_eq!(second.stdout().fd, 20);
        assert_eq!(second.stderr().fd, 21);
        assert_eq!(second.previous_stderr().fd, 0);
        assert_eq!(second.stdout().flags, 0);
    }

    // [spec:dash:sem:output.xwrite-fn/test]
    #[test]
    fn xwrite_writes_everything_or_reports_failure() {
        unsafe {
            let mut s = Sink::new(1, true);
            let payload = vec![b'q'; 200_000];
            // Larger than a pipe buffer, so this only succeeds if xwrite
            // loops over partial writes -- which is its whole purpose.
            let w = s.w;
            let reader = std::thread::spawn({
                let r = s.r;
                move || {
                    let mut got = 0usize;
                    let mut tmp = [0u8; 8192];
                    loop {
                        let n = libc::read(r, tmp.as_mut_ptr() as *mut c_void, tmp.len());
                        if n <= 0 {
                            break;
                        }
                        got += n as usize;
                    }
                    got
                }
            });
            assert_eq!(
                xwrite(w, payload.as_ptr() as *const c_void, payload.len()),
                0
            );
            libc::close(w);
            s.w = -1;
            assert_eq!(reader.join().unwrap(), 200_000);
            // A closed descriptor fails rather than looping.
            assert_eq!(xwrite(9999, payload.as_ptr() as *const c_void, 1), -1);
        }
    }

    // [spec:dash:sem:output.freestdout-fn/test]
    #[test]
    fn freestdout_resets_the_buffer_and_error_flag() {
        let _g = crate::testutil::lock();
        unsafe {
            let out = stdout();
            let saved_buf = (*out).buf.take();
            let saved = ((*out).bufsize, (*out).flags, (*out).fd);
            (*out).buf = Some(Vec::with_capacity(16));
            (*out).buf.as_mut().unwrap().extend_from_slice(b"abcde");
            (*out).bufsize = 16;
            (*out).flags = OUTPUT_ERR;

            freestdout();

            let buffered = (*out).buf.as_ref().unwrap().len();
            let flags = (*out).flags;
            assert_eq!(buffered, 0);
            assert_eq!(flags, 0);
            (*out).buf = saved_buf;
            ((*out).bufsize, (*out).flags, (*out).fd) = saved;
        }
    }

    // [spec:dash:sem:output.flushall-fn/test]
    #[test]
    fn flushall_drains_the_stdout_writer() {
        let _g = crate::testutil::lock();
        unsafe {
            let out = stdout();
            let saved_buf = (*out).buf.take();
            let saved = ((*out).bufsize, (*out).fd, (*out).flags);
            let stdout_before = stdout();
            let mut s = Sink::new(64, true);
            // Point the global stdout stream at the pipe.
            (*out).buf = None;
            (*out).bufsize = s.out.bufsize;
            (*out).fd = s.w;
            (*out).flags = 0;

            (*out).write_all(b"n=3").unwrap();
            // Buffered, not yet written.
            assert_eq!((*out).buf.as_ref().unwrap().len(), 3);
            flushall();
            assert_eq!((*out).buf.as_ref().unwrap().len(), 0);
            let stdout_after = stdout();
            assert_eq!(stdout_after, stdout_before);
            assert_eq!(stdout_after, out);

            (*out).buf = saved_buf;
            ((*out).bufsize, (*out).fd, (*out).flags) = saved;
            assert_eq!(s.drained(), b"n=3");
        }
    }

    // The three routines below sit inside `#ifdef notyet` and
    // `#ifdef USE_GLIBC_STDIO`, neither defined in any shipped
    // configuration: `struct output` has no `stream` member and there is
    // no `memout`. Their bodies are empty in both languages, so what can
    // be asserted is that they are callable and inert -- which is the
    // contract, and is what would break if someone gave one a body.
    //
    // [spec:dash:sem:output.initstreams-fn/test]
    // [spec:dash:sem:output.openmemout-fn/test]
    // [spec:dash:sem:output.closememout-fn/test]
    #[test]
    fn inactive_glibc_stdio_hooks_are_inert() {
        let _g = crate::testutil::lock();
        unsafe {
            let out = stdout();
            let err = stderr();
            let before = (
                (*out).buf.as_ref().map(|buf| (buf.len(), buf.capacity())),
                (*out).fd,
                (*err).fd,
            );
            initstreams();
            openmemout();
            assert_eq!(__closememout(), 0);
            assert_eq!(
                (
                    (*out).buf.as_ref().map(|buf| (buf.len(), buf.capacity())),
                    (*out).fd,
                    (*err).fd,
                ),
                before,
            );
        }
    }
}
