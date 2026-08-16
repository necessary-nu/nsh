//! Where [`Shell::run`] reads commands, and what running them means.
//!
//! `docs/api-design.md` §4. The section's finding is that there was
//! nothing to invent: `sh -c` and the `eval` built-in are already the same
//! primitive, so
//!
//! > **`run` is `eval`, at the top level. Two `run` calls compose exactly
//! > as two `eval` commands do.**
//!
//! Everything the execution environment holds persists between calls --
//! variables, functions, aliases, options, traps, the working directory,
//! jobs, `$?`. What does not persist is the *parse*: `run(b"if true;
//! then")` is a syntax error for the same reason `eval 'if true; then'`
//! is. A `run` that could be continued by the next one would have to block
//! for more input, which is what [`Source::stream`] is for.

use std::path::{Path, PathBuf};

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use crate::status::ExitStatus;

/// Where [`Shell::run`] reads commands.
///
/// Bytes or a descriptor, and nothing else. There is deliberately no
/// `impl Read` source: the input stack is descriptor-based, and not
/// incidentally. `preadfd`'s `fd == 0` test is the question "is this parse
/// file the shell's standard input", and it gates both line editing and
/// the stdin tee; `forkreset` closes `parsefile->fd` across a fork;
/// `basepf.fd` is what the line editor is handed. A `dyn Read` can be
/// given to none of them.
///
/// A caller holding a reader writes it to a pipe or a file. That is the
/// honest cost, and it is a sentence of documentation instead of a second
/// input path with no oracle behind it.
pub struct Source(Kind);

enum Kind {
    /// The `-c` / `eval` shape.
    Bytes(BString),
    /// The `.` / `sh script` shape.
    File(PathBuf),
    /// The shell's own standard input: bare `sh`.
    Stream,
}

impl Source {
    /// Commands as text, which is the `-c` and `eval` shape.
    pub fn bytes(text: impl Into<BString>) -> Source {
        Source(Kind::Bytes(text.into()))
    }

    /// A file the shell opens and reads, which is the `.` and `sh script`
    /// shape.
    ///
    /// `$0` is *not* changed, so this is `.` without the `PATH` search
    /// rather than `sh script`. An embedder that wants the script's name
    /// in its diagnostics sets it on the builder.
    pub fn file(path: impl AsRef<Path>) -> Source {
        Source(Kind::File(path.as_ref().to_path_buf()))
    }

    /// The shell's own standard input, from the [`crate::streams::Streams`]
    /// it was built with.
    ///
    /// This is bare `sh`, and it is what an interactive frontend runs: it
    /// prompts, announces changed jobs, and tolerates `EOF` the way dash's
    /// `ignoreeof` does, because it *is* dash's top-level command loop.
    /// The other two shapes are not that loop and do not prompt.
    pub fn stream() -> Source {
        Source(Kind::Stream)
    }
}

impl From<&[u8]> for Source {
    fn from(text: &[u8]) -> Source {
        Source::bytes(BString::from(text))
    }
}

impl<const N: usize> From<&[u8; N]> for Source {
    fn from(text: &[u8; N]) -> Source {
        Source::bytes(BString::from(&text[..]))
    }
}

impl From<&BStr> for Source {
    fn from(text: &BStr) -> Source {
        Source::bytes(text.to_owned())
    }
}

impl From<BString> for Source {
    fn from(text: BString) -> Source {
        Source::bytes(text)
    }
}

impl From<Vec<u8>> for Source {
    fn from(text: Vec<u8>) -> Source {
        Source::bytes(BString::from(text))
    }
}

impl From<&str> for Source {
    fn from(text: &str) -> Source {
        Source::bytes(BString::from(text.as_bytes()))
    }
}

impl Shell {
    /// Run commands, and give back the status of the last one.
    ///
    /// This is `eval` at the top level — see the module documentation for
    /// what that buys and what it costs.
    ///
    /// # The input stack
    ///
    /// `run` records the current parse file, pushes the source above it,
    /// and moves the *unwind floor* to the pushed frame. On the way out —
    /// normally, on `Err`, or on `exit` — it unwinds back to the mark and
    /// puts the floor back, so the stack depth after a `run` is the depth
    /// before it. That is a `debug_assert` below rather than a promise in
    /// prose.
    ///
    /// **The floor move is the one place this diverges from dash**, and it
    /// is a correctness property rather than an optimisation. dash's `-c`
    /// uses `setinputstring`, which does not move `toppf`, so `reset`'s
    /// `popallfiles` unwinds past a `-c` string all the way to the shell's
    /// standard input. For a library that would let an inner error
    /// terminate the embedder's `run` by unwinding into a descriptor the
    /// embedder owns — which may be the host's terminal. The path is
    /// reachable only for an interactive shell (`sh -ic`), which is why
    /// the differential corpus cannot see it.
    ///
    /// # `exec` and the embedder's process image
    ///
    /// `run` passes no `EV_EXIT`, and that is not an omission.
    /// `evalcommand`'s `EV_EXIT` fast path `execve`s the last command of a
    /// script **in place**, which is why `dash -c 'ls'` replaces its own
    /// image with no `exec` written anywhere. A `run` built naively on the
    /// `-c` path would do that to the embedder on `sh.run(b"ls")`.
    /// `[dec:nsh:host-owns-the-process]` makes this non-negotiable; the
    /// optimisation stays available to `nsh-cli`, which does pass it.
    ///
    /// It has a second effect worth knowing: `evalsubshell`'s no-fork arm
    /// is `EV_EXIT`-only too, so from `run` the shell never runs
    /// `forkreset` in its own process either. `exec cmd` written out in
    /// full is still asked of the [`crate::host::Host`], which answers
    /// [`crate::host::NoHost`]'s refusal by default.
    ///
    /// # Calling it twice, and calling it from a callback
    ///
    /// Two calls compose. A call from inside a host callback does not
    /// exist: `run` takes `&mut self`, every callback is invoked while
    /// that borrow is held, and no [`crate::host::Host`] method takes a
    /// `Shell` — so the re-entrant case is a compile error rather than a
    /// documented hazard.
    ///
    /// # Errors
    ///
    /// The diagnostic has already been written to the shell's stderr, in
    /// dash's bytes and in dash's order; the [`Error`] is the same
    /// diagnostic as a value, for a caller that wants to branch on it. The
    /// shell is still usable afterwards — that is what makes two `run`s
    /// compose across a failure, and it is why the error path resets the
    /// evaluator exactly as dash's top-level handler does.
    pub fn run(&mut self, source: impl Into<Source>) -> Result<ExitStatus, Error> {
        let source = source.into();
        unsafe { self.run_source(source) }
    }

    unsafe fn run_source(&mut self, source: Source) -> Result<ExitStatus, Error> {
        let mark = self.input.mark();
        /* Read before the push, because for a file the push is what moves
         * it: `setinputfd` sets `toppf` itself when the file was opened
         * without `INPUT_PUSH_FILE`, and `setinputstring` never does. */
        let old_floor = self.input.floor();

        /* The text has to outlive the parse: `setinputstring` keeps the
         * caller's pointer rather than copying, which is the same reason
         * `evalstring` owns its `sstrdup` across the `popfile`. Holding it
         * here says so on the unwind path too, where the C's `stunalloc`
         * never ran. */
        let mut text: Vec<u8> = Vec::new();

        match &source.0 {
            Kind::Bytes(b) => {
                text.reserve(b.len() + 1);
                text.extend_from_slice(b.as_ref());
                text.push(0);
                crate::input::setinputstring(self, text.as_mut_ptr() as *mut libc::c_char);
            }
            Kind::File(p) => {
                let mut name: Vec<u8> =
                    std::os::unix::ffi::OsStrExt::as_bytes(p.as_os_str()).to_vec();
                name.push(0);
                /* Nothing has been pushed and no floor moved when this
                 * fails, so leaving by `?` needs no unwind. */
                crate::input::setinputfile(self, name.as_ptr() as *const libc::c_char, 0)?;
            }
            Kind::Stream => {
                /* Nothing to push: the shell's standard input is frame
                 * zero and is already current, so the unwind below is a
                 * no-op. That is the honest description of a `run` that
                 * reads the stream the shell was built with. */
            }
        }
        self.input.set_floor(self.input.mark());

        let outcome = match &source.0 {
            /* dash's own mapping, kept: `-c` is the parse-execute loop,
             * `sh script` is `cmdloop(0)`, and bare `sh` is `cmdloop(1)`.
             * The difference is not cosmetic — `cmdloop` prompts, reports
             * changed jobs, and counts consecutive `EOF`s for
             * `ignoreeof`. */
            Kind::Bytes(_) => crate::eval::parse_execute(self, 0),
            Kind::File(_) => crate::shellmain::cmdloop(self, 0),
            Kind::Stream => crate::shellmain::cmdloop(self, 1),
        };

        crate::input::unwindfiles(self, mark);
        self.input.set_floor(old_floor);
        debug_assert_eq!(
            self.input.mark(),
            mark,
            "run left the input stack at a different depth than it found it"
        );
        drop(text);

        match outcome {
            Ok(Flow::Done(status)) => {
                self.status = status;
                Ok(ExitStatus::from_raw(status))
            }
            Ok(Flow::Exit { by_exitcmd }) => {
                /* The two calls, in dash's order, and the order is the
                 * whole of how `exit 3` keeps its 3. `exitcmd` leaves the
                 * number in `savestatus` and says so with `by_exitcmd`;
                 * `exitreset` is what moves it into `$?`; and `exitshell`
                 * reads `$?` on its first line to hand it to the EXIT
                 * trap. Calling `exitshell` alone would run the trap with
                 * the status of whatever ran before the `exit`.
                 *
                 * `exitshell` then runs the EXIT trap, gives job control
                 * back and flushes -- and returns the status rather than
                 * ending the process, which is the whole of
                 * [dec:nsh:host-owns-the-process] at this seam. */
                crate::init::exitreset(self, by_exitcmd);
                let status = crate::trap::exitshell(self);
                self.exited = Some(status);
                Ok(status)
            }
            Err(e) => {
                /* What dash's top-level handler does with an exception,
                 * minus the parts that only make sense for a process:
                 * `$?` takes the status the raise carried, and the
                 * evaluator's skip state, loop nesting and `PS4` guard are
                 * reset so the next `run` starts clean. Without this a
                 * `break` that escaped its loop would still be pending on
                 * the next call. */
                self.status = e.status();
                crate::init::exitreset(self, false);
                Err(e)
            }
        }
    }

    /// The status of the last command the shell ran, which is `$?`.
    pub fn status(&self) -> ExitStatus {
        ExitStatus::from_raw(self.status)
    }

    /// Has the shell run `exit`?
    ///
    /// A shell that has exited ran its `EXIT` trap and gave up job control
    /// on the way out, which is what dash does before `_exit`. It is still
    /// a live value and [`Shell::run`] will still run commands on it —
    /// there is no process to have ended — but an embedder driving it in a
    /// loop wants to stop here, and a script's `exit 3` is how it says so.
    pub fn has_exited(&self) -> bool {
        self.exited.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The property §4.1 is named for: everything the execution
    /// environment holds survives from one call to the next, and only the
    /// parse does not.
    #[test]
    fn two_runs_compose_like_two_lines_of_one_script() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder().build().unwrap();
        sh.run(b"count=7").unwrap();
        let st = sh.run(b"exit $count").unwrap();
        assert_eq!(st.code(), 7);
    }

    /// A `run` is a complete parse unit, exactly as `eval` is.
    #[test]
    fn an_incomplete_command_is_a_syntax_error_rather_than_a_continuation() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder().build().unwrap();
        assert!(sh.run(b"if true; then").is_err());
    }

    /// The stack depth after a `run` is the depth before it, on the error
    /// path as much as the normal one -- and a shell that failed is still
    /// a shell.
    #[test]
    fn a_failed_run_leaves_the_shell_usable() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder().build().unwrap();
        assert!(sh.run(b"if true; then").is_err());
        let st = sh.run(b"true").unwrap();
        assert_eq!(st.code(), 0);
    }

    /// `exit` inside a `run` is a status and a flag, not a dead process.
    #[test]
    fn exit_reports_itself_rather_than_ending_the_host() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder().build().unwrap();
        assert!(!sh.has_exited());
        let st = sh.run(b"exit 3").unwrap();
        assert_eq!(st.code(), 3);
        assert!(sh.has_exited());
    }

    /// `Source::file` opens and reads it, and the stack it pushed comes
    /// back down -- which for a file is the arm where `setinputfd` moved
    /// the floor rather than `run` doing it.
    #[test]
    fn a_file_source_is_read_and_leaves_the_stack_where_it_found_it() {
        let _g = crate::testutil::lock();
        let path = std::env::temp_dir().join(format!("nsh-source-{}", std::process::id()));
        std::fs::write(&path, b"answer=41\nanswer=$((answer + 1))\n").unwrap();
        let mut sh = Shell::builder().build().unwrap();
        sh.run(Source::file(&path)).unwrap();
        sh.run(b"exit $answer").unwrap();
        assert_eq!(sh.status().code(), 42);
        std::fs::remove_file(&path).unwrap();
    }

    /// `Source::stream` reads the descriptor the shell was built with,
    /// which is the arm with nothing to push.
    #[test]
    fn a_stream_source_reads_the_shells_own_standard_input() {
        let _g = crate::testutil::lock();
        let mut fds: [libc::c_int; 2] = [0; 2];
        unsafe {
            assert_eq!(libc::pipe(fds.as_mut_ptr()), 0);
            let script = b"exit 5\n";
            assert_eq!(
                libc::write(fds[1], script.as_ptr() as *const libc::c_void, script.len()),
                script.len() as isize
            );
            libc::close(fds[1]);
        }
        let mut sh = Shell::builder()
            .streams(crate::streams::Streams {
                stdin: fds[0],
                stdout: 1,
                stderr: 2,
            })
            .build()
            .unwrap();
        let st = sh.run(Source::stream()).unwrap();
        assert_eq!(st.code(), 5);
        unsafe { libc::close(fds[0]) };
    }

    /// The `EXIT` trap runs, which is the reason `run` calls `exitshell`
    /// at all rather than just recording the status.
    #[test]
    fn the_exit_trap_runs_when_a_script_exits() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder().build().unwrap();
        sh.run(b"trap 'x=ran' EXIT").unwrap();
        sh.run(b"exit 0").unwrap();
        sh.run(b"exit $([ \"$x\" = ran ] && echo 0 || echo 9)")
            .unwrap();
        assert_eq!(sh.status().code(), 0);
    }
}
