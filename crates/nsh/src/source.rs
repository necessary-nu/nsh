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
use nsh_platform::NativeStrExt as _;

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use crate::status::ExitStatus;

/// Where [`Shell::run`] reads commands.
///
/// Bytes or an owned descriptor-backed source, and nothing else. There is
/// deliberately no `impl Read` source: the input stack must distinguish the
/// shell's logical stdin from files it owns. That identity gates line editing
/// and the stdin tee, follows logical redirection, and determines which file
/// frame fork reset drops. A `dyn Read` can express none of those semantics.
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
        self.run_source(source)
    }

    fn run_source(&mut self, source: Source) -> Result<ExitStatus, Error> {
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
        match &source.0 {
            Kind::Bytes(b) => {
                crate::input::setinputstring(self, BStr::new(b));
            }
            Kind::File(p) => {
                /* Nothing has been pushed and no floor moved when this
                 * fails, so leaving by `?` needs no unwind. */
                let path = p.to_shell_bytes();
                crate::input::setinputfile(self, BStr::new(&path), 0)?;
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
        match outcome {
            Ok(Flow::Done(status)) => {
                self.status = status;
                Ok(ExitStatus::from_raw(status))
            }
            Ok(Flow::Exit { status }) => {
                /* Apply the status carried by `exit` before entering the
                 * EXIT action. `Flow::END` carries `None` because the
                 * command status already on the shell is authoritative.
                 *
                 * `exitshell` then runs the EXIT trap, gives job control
                 * back and flushes -- and returns the status rather than
                 * ending the process, which is the whole of
                 * [dec:nsh:host-owns-the-process] at this seam. */
                if let Some(status) = status {
                    self.status = status;
                }
                crate::init::exitreset(self);
                let exit_status = crate::trap::exitshell(self, status);
                self.exited = Some(exit_status);
                Ok(exit_status)
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
                crate::init::exitreset(self);
                Err(e)
            }
        }
    }

    /// Run `command` with `args` as its `$0` and positional parameters.
    ///
    /// This is `sh -c command name arg…`: `args[0]` is `$0`, and the rest
    /// are `$1`, `$2`, ….
    ///
    /// **Passing data as a positional parameter rather than interpolating
    /// it into `command` is the only way to keep it out of the parser**,
    /// so this is the injection-safe form, and that is why it sits on the
    /// surface beside [`Shell::run`] rather than being left to the caller
    /// to assemble. A file name with a quote in it, a value that begins
    /// with `-`, a `$(…)` an attacker chose: all of them are one argument
    /// here and none of them is syntax.
    ///
    /// The parameters persist afterwards, exactly as `sh -c` leaves them.
    pub fn run_command(&mut self, command: &BStr, args: &[&BStr]) -> Result<ExitStatus, Error> {
        if let Some(arg0) = args.first() {
            self.options.set_arg0(arg0);
        }
        let rest: Vec<&BStr> = args.iter().skip(1).copied().collect();
        crate::options::setparam(self, &rest);
        self.run(command)
    }

    /// Expand one word as the shell would in command position, and give
    /// back the fields it becomes.
    ///
    /// Tilde, parameter, command and arithmetic substitution, then field
    /// splitting on `$IFS`, then pathname expansion, then quote removal.
    /// One word is zero, one or many fields: `$x` with `x='a b'` is two,
    /// `$x` with `x=''` is none, `*.txt` is however many files match.
    ///
    /// **This executes.** Command substitution is part of word expansion,
    /// so `expand_word(b"$(rm -rf /)")` runs the command. There is no mode
    /// that expands without executing, because the shell language does not
    /// have one, and offering one that quietly skipped `$(…)` would be a
    /// different language wearing this one's syntax.
    pub fn expand_word(&mut self, word: &BStr) -> Result<Vec<BString>, Error> {
        self.expand(word, crate::expand::EXP_FULL | crate::expand::EXP_TILDE)
    }

    /// Expand one word as if it appeared inside double quotes: no field
    /// splitting, no pathname expansion, always exactly one result.
    ///
    /// The same caveat about command substitution applies. This is the
    /// flag a here-document is expanded under, which is what makes it the
    /// shell's own idea of "quoted" rather than a second one.
    pub fn expand_word_quoted(&mut self, word: &BStr) -> Result<BString, Error> {
        let mut fields = self.expand(word, crate::expand::EXP_QUOTED)?;
        /* `expandarg` without `EXP_FULL` pushes exactly one field, so
         * the empty case is unreachable rather than defaulted. */
        Ok(fields.pop().unwrap_or_default())
    }

    /// Tokenize `word` as one word and expand it under `flag`.
    ///
    /// The tokenizer is not skipped, and that is the point: `expandarg`
    /// takes an `NARG` node, whose text carries the `CTL*` markers that
    /// say where a `$` was quoted and where a `*` is a glob. Handing it
    /// raw bytes would expand a *different* word from the one the shell
    /// would have seen. So this is what the parser does for an argument —
    /// `readtoken` into `wordtext`, `makename` into a node — with the
    /// keyword and alias checks off, because a single word in isolation is
    /// neither.
    // [spec:nsh:req:idiom.lexer-tokens]
    fn expand(&mut self, word: &BStr, flag: core::ffi::c_int) -> Result<Vec<BString>, Error> {
        let mark = self.input.mark();
        let old_floor = self.input.floor();

        crate::input::setinputstring(self, word);
        self.input.set_floor(self.input.mark());
        let expanded = (|sh: &mut Shell| -> Result<Vec<BString>, Error> {
            let t = crate::parser::readtoken(sh, crate::parser::TokenContext::NONE)?;
            if t != crate::parser::TokenKind::Word {
                /* An empty word is the honest answer for empty input, and
                 * anything else here is syntax the caller wrote rather
                 * than a word — a `;` or a `|` cannot be expanded. */
                return Ok(Vec::new());
            }
            let n = crate::parser::makename(sh);
            let mut list = crate::expand::arglist::new();
            crate::expand::expandarg(sh, &n, Some(&mut list), flag)?;
            Ok(list
                .list
                .into_iter()
                .map(|s| {
                    /* The unsplit field is the expansion buffer whole,
                     * terminator and all, because its other readers -- a
                     * here-document, `expredir` -- go on to read it as a C
                     * string. A field cannot contain a NUL, so dropping
                     * one trailing byte is exact rather than a heuristic,
                     * and it is a no-op for the split fields, which
                     * `ifsbreakup` already cut short of it. */
                    let mut t = s.text;
                    if t.last() == Some(&0) {
                        t.pop();
                    }
                    t
                })
                .collect())
        })(self);

        crate::input::unwindfiles(self, mark);
        self.input.set_floor(old_floor);
        expanded
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

    #[test]
    fn exec_requires_host_authority() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder()
            .streams(crate::streams::Streams::capture().unwrap())
            .build()
            .unwrap();

        let executable = std::env::current_exe().unwrap().to_shell_bytes();
        let mut command = BString::from("exec ");
        command.extend_from_slice(&crate::mystring::single_quote(BStr::new(&executable)));
        command.extend_from_slice(b" replaced");
        let status = sh.run(BStr::new(command.as_slice())).unwrap();
        assert_eq!(status.code(), 126);
        assert!(sh.has_exited());
        let diagnostic = sh.take_captured_stderr().unwrap();
        let denied =
            std::io::Error::from_raw_os_error(nsh_platform::permission_denied_error_code());
        let mut suffix = nsh_platform::Locale::c()
            .unwrap()
            .error_message(&denied)
            .into_bytes();
        suffix.push(b'\n');
        assert!(diagnostic.as_slice().ends_with(&suffix));
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
        let (read, write) = nsh_platform::pipe().unwrap();
        nsh_platform::write_all(&write, b"exit 5\n").unwrap();
        drop(write);
        let mut sh = Shell::builder()
            .streams(
                crate::streams::Streams::from_fds(&read, std::io::stdout(), std::io::stderr())
                    .unwrap(),
            )
            .build()
            .unwrap();
        let st = sh.run(Source::stream()).unwrap();
        assert_eq!(st.code(), 5);
        drop(read);
    }

    /// The whole reason `run_command` is on the surface: a value with
    /// quotes, a `$` and a leading `-` in it goes in as data and comes out
    /// as data, because it never reaches the parser.
    #[test]
    fn a_positional_parameter_is_data_rather_than_syntax() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder()
            .streams(crate::streams::Streams::capture().unwrap())
            .build()
            .unwrap();
        let hostile = BStr::new(b"a file with 'quotes' and $HOME in it");
        sh.run_command(
            BStr::new(b"printf '%s' \"$1\""),
            &[BStr::new(b"myapp"), hostile],
        )
        .unwrap();
        let out = sh.take_captured_stdout().unwrap();
        assert_eq!(out, hostile);
    }

    /// Capture is a file, so a script that writes more than a pipe buffer
    /// cannot deadlock against a host that has not read yet -- which is
    /// the reason it is a file. 256 KiB is well past the 64 KiB pipe.
    #[test]
    fn a_capture_holds_more_than_a_pipe_would() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder()
            .streams(crate::streams::Streams::capture().unwrap())
            .build()
            .unwrap();
        sh.run(b"i=0; while [ $i -lt 4096 ]; do printf '%064d' $i; i=$((i+1)); done")
            .unwrap();
        let out = sh.take_captured_stdout().unwrap();
        assert_eq!(out.len(), 4096 * 64);
        /* And it is emptied, so "since the last call" is true. */
        assert!(sh.take_captured_stdout().unwrap().is_empty());
    }

    /// One word is zero, one or many fields, and splitting is on `$IFS`.
    #[test]
    fn an_unquoted_word_splits_and_a_quoted_one_does_not() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder().build().unwrap();
        sh.set_var(BStr::new(b"x"), BStr::new(b"a b c")).unwrap();
        assert_eq!(sh.expand_word(BStr::new(b"$x")).unwrap().len(), 3);
        assert_eq!(
            sh.expand_word_quoted(BStr::new(b"$x")).unwrap(),
            BStr::new(b"a b c")
        );
        sh.set_var(BStr::new(b"e"), BStr::new(b"")).unwrap();
        assert_eq!(sh.expand_word(BStr::new(b"$e")).unwrap().len(), 0);
    }

    /// The tokenizer is not skipped, which is what makes a quoted `$` and
    /// a defaulted parameter mean here what they mean in a script.
    #[test]
    fn expansion_sees_the_word_the_parser_would_have() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder().build().unwrap();
        assert_eq!(
            sh.expand_word_quoted(BStr::new(b"${NSH_UNSET_PROBE:-vi}"))
                .unwrap(),
            BStr::new(b"vi")
        );
        assert_eq!(
            sh.expand_word(BStr::new(b"\\$HOME")).unwrap(),
            vec![BString::from(&b"$HOME"[..])]
        );
    }

    /// The table, read and written from outside the language.
    #[test]
    fn a_variable_set_from_outside_is_the_one_a_script_reads() {
        let _g = crate::testutil::lock();
        let mut sh = Shell::builder().build().unwrap();
        sh.set_var(BStr::new(b"greeting"), BStr::new(b"hello"))
            .unwrap();
        assert_eq!(sh.var(BStr::new(b"greeting")), Some(BStr::new(b"hello")));
        sh.run(b"greeting=$greeting-again").unwrap();
        assert_eq!(
            sh.var(BStr::new(b"greeting")),
            Some(BStr::new(b"hello-again"))
        );
        assert!(sh.vars().iter().any(|(k, _)| k == "greeting"));
        assert!(sh.unset_var(BStr::new(b"greeting")).unwrap());
        assert_eq!(sh.var(BStr::new(b"greeting")), None);
        assert!(!sh.unset_var(BStr::new(b"greeting")).unwrap());
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
