# The public API

Status: design. It settles `[dec:nsh:public-surface]`'s deferred question
and `docs/idiomatization.md` §7.4, §7.5 and §7.6, and it proposes edits to
two decisions. Nothing here is implemented.

The artefact this document produces is `crates/nsh/src/api.rs`: the same
surface as compiling Rust with `todo!()` bodies and
`#![deny(missing_docs)]` on. Read it alongside. It exists because three of
the decisions below — whether a built-in can re-enter evaluation, whether
`Host` can take a `&mut Shell`, whether a captured stream can be borrowed —
are answered by the borrow checker and not by prose, and two of them came
out the other way from the sketch in `[dec:nsh:public-surface]`.

`crates/nsh/examples/embed.rs` is the usage, and it was written first. It
has already rejected one signature from the recorded sketch; §8 says which.

Everything below is measured on `3cfff64` in the `wt/public-api-design`
worktree.

---

## 1. The example, which is the specification

```rust
let mut sh = Shell::builder()
    .arg0(BStr::new(b"myapp"))
    .inherit_env()
    .streams(Streams::capture()?)
    .build()?;

sh.set_var(BStr::new(b"PATH"), BStr::new(b"/usr/bin:/bin"))?;
let status: ExitStatus = sh.run(b"for f in *.txt; do wc -l \"$f\"; done")?;
let out: BString       = sh.take_captured_stdout()?;

sh.run(b"count=$(ls | wc -l)")?;     // two runs compose like two lines
sh.run(b"echo \"$count files\"")?;   // of one script

let fields: Vec<BString> = sh.expand_word(BStr::new(b"~/src/*.rs"))?;
sh.run_command(BStr::new(b"printf '%s\\n' \"$1\""),
               &[BStr::new(b"myapp"), untrusted])?;   // no quoting, ever
```

Four properties, and the last one is why the type is a type:

* **No second process.** Parsing and execution happen in the caller's
  process. External commands still fork; the shell does not.
* **No quoting round-trip.** Bytes in, bytes out, no `String` anywhere
  (`[dec:nsh:bytes-not-text]`). `run_command`'s positional parameters are
  the injection-safe path in, and they are the reason it is on the surface
  next to `run` rather than folded into it.
* **Errors arrive as values** — and the diagnostic still lands on stderr
  where dash put it. §3.
* **Two shells share nothing**, except the C library's own process
  globals. §6 names them rather than pretending.

---

## 2. The type surface

Ten types, and the methods on them. The full signatures are in
`crates/nsh/src/api.rs`; this table is the shape and the reason.

| Item | Why it is on the surface |
|---|---|
| `Shell` | The instance. §5 is what it owns. |
| `Builder` | argv, environment, streams, host, options, cwd, diagnostic hook. |
| `Shell::run(impl Into<Source>) -> Result<ExitStatus, Error>` | Run a script. §4 is what two calls mean. |
| `Shell::run_command(&BStr, &[&BStr])` | The `-c` shape. `args[0]` is `$0`. |
| `Shell::expand_word(&BStr) -> Result<Vec<BString>, Error>` | Word expansion without a command. Fields, because one word is zero, one or many. |
| `Shell::expand_word_quoted(&BStr) -> Result<BString, Error>` | The same as if double-quoted: one field, no splitting, no globbing. |
| `Shell::var / set_var / unset_var / vars` | The variable table, as bytes. |
| `Shell::status() -> ExitStatus`, `has_exited() -> bool` | `$?`, and whether the script ended by `exit` or `set -e`. |
| `Shell::take_captured_stdout / take_captured_stderr` | Only under `Streams::capture`. |
| `Streams` | `inherit()`, `from_fds(..)`, `capture()`. §7. |
| `Source` | `bytes(..)`, `file(..)`, `stream()`. Nothing else — §4.4. |
| `Host` (trait) | What a library may not do on its own authority. §5.4. |
| `Disposition`, `SignalSink` | `Host`'s vocabulary. |
| `Error` | §3. |
| `ExitStatus`, `Signal` | `u8` and a numeric newtype. |

**The honest item count is fifty, not twenty.** Measured on the sketch:

    $ grep -cE '^\s*pub (fn|const|struct|enum|trait|type|mod)' crates/nsh/src/api.rs
    50

Seven structs, two enums, one trait, one associated const and thirty-nine
functions; sixty-three if the ten `Error` variants and three `Disposition`
variants are counted, which `missing_docs` does. `[dec:nsh:public-surface]`
says "roughly twenty documented items" and then lists ten table *entries*
whose methods expand to two and a half times that. Against roughly a
thousand today it is still two orders of magnitude — which is the property
the decision is about — but the number is wrong and should be corrected
rather than met by under-counting. §9 proposes the edit.

### 2.1 Three items are not on the list, deliberately

**`Streams::install` is gone.** It existed because `main_fn` needed the
process's descriptors on 0, 1 and 2 to get full fidelity. The per-instance
descriptor table now gives `from_fds` that fidelity without touching the
host's process descriptors, so the two modes collapsed into one. §7 records
the implementation and the two gaps it deliberately leaves.

**No `Shell::args()` reader, no `export_var`.** No concrete embedder wants
either. `expand_word(b"$1")` reads a positional parameter; `Builder::env`
sets the exported environment. If an embedder appears that needs to export
a variable after construction, `export_var(&BStr, &BStr)` is the addition
and it is additive.

**No `impl Read` source.** §4.4.

---

## 3. The error taxonomy

### 3.1 Three things behind four integers

`error.rs:80-83` has `EXINT`, `EXERROR`, `EXEND` and `EXEXIT`. Only one of
them is an error.

* **Diagnostics** — "not found", "Bad substitution", a syntax error. These
  become `Error`.
* **Control flow** — `exit`, `return`, `break`, `continue`, the `set -e`
  abort, `EXEND`, `EXEXIT`. These go in the `Ok` position, carried by a
  crate-private `Flow`.
* **Interrupt** — `EXINT`. `Error::Interrupted(Signal)`, kept distinct
  because a host has to tell "your script failed" from "the user pressed
  ^C".

### 3.2 The diagnostic is written *and* returned

`tests/harness/dscase.sh:64-71` runs every case with `2>&1` and compares
the merged stream, so the *interleaving* of diagnostics with command output
is under test in all 61,498 cases. Deferring the write to the frontend
emits every diagnostic at the end of the run.

The mechanism is one funnel, `Shell::report`:

```rust
pub(crate) fn report(&mut self, e: Error) -> Error;   // writes, then returns
```

and every raise site becomes `return Err(self.report(e))`. `report` renders
through `Error::message()`, so the bytes on the stream and the bytes in the
value cannot drift.

Two details of dash's write that `report` has to reproduce and that are
easy to lose:

* **stderr is unbuffered.** `errout` has `bufsize: 0`
  (`output.rs:62-69`), so `outmem` skips both buffered paths and lands on
  a raw `write(2)` at `output.rs:363`. A diagnostic is three unbuffered
  writes — prefix, body, newline — and needs no flush.
* **stdout is flushed *after* the diagnostic.** `exverror` writes the
  message to fd 2 and only then calls `flushall()` (`error.rs:324-326`),
  which flushes stdout alone (`output.rs:388-395`; the `flushout(errout)`
  is behind `FLUSHERR`, which this build does not define). So a built-in
  that wrote to the 8 KiB stdout buffer and *then* failed produces its
  diagnostic before its own output in the merged stream. That ordering is
  observable and pinned by the corpus.

The prefix — `sh: 1: cd: ` — is built at `error.rs:267-295` from `arg0`,
`errlinno` and `commandname`. Those are shell state, not error state, so
`Error::message()` returns the text *without* it and `report` adds it. This
is a deviation from `docs/idiomatization.md` §1.3's
`diagnostic() -> impl Display`, and it is also why the return type is
`BString` and not `impl Display`: a "not found" message contains a
filename, and a filename is not text.

### 3.3 Most diagnostics never reach a `Result`

```
$ dash -c 'nosuchcmd; nosuchcmd2; echo done'
dash: 1: nosuchcmd: not found
dash: 1: nosuchcmd2: not found
done
status=0
```

`find_command` prints and does not raise (`exec.rs:616-621`,
`exec.rs:626`); `evalcommand` takes status 127 and carries on
(`eval.rs:1063-1065`). The same name inside a forked child *does* abort,
because `shellexec` raises `EXEND` there (`exec.rs:155`).

So the contract is:

> `run` returns `Err(e)` when a diagnostic **aborted the run** — dash's
> `EXERROR` reaching the top level. A diagnostic dash reports and carries
> on past does not appear in the return value at all.

The mechanism that decides which is which already exists and is exact:
`eval.rs:1069-1073` re-raises a built-in's exception unless it was
`EXERROR` from a non-special built-in, which is POSIX's "an error in a
special built-in exits a non-interactive shell".

**The sink is a hook, not a store — settled at `c9c2434`.** `move-state`
transferred `arg0`, `errlinno` and `commandname` to `public-api` as a
choice: thread the diagnostic spine, or give the diagnostic its own sink
here so the spine never needs a receiver. It recommended the second to
avoid the first. **They were never alternatives.** §3.2 fixes `report` as
a `&mut self` method and requires the write to happen at the raise point,
so a sink that assembled the prefix elsewhere would have to defer the
write to wherever it is polled — the one thing §3.2 forbids. And the
write needs `&mut` on the stderr `Output`, so even a sink owning the
prefix would not free `report` from a receiver. What the sink replaces is
the *statics*: `errlinno` and `commandname` are `EvalState` fields, and
`arg0` stays in the options table because it is `$0` before it is a
prefix and `expand.rs` reads it as one.

The threading cost 66 call sites, of which 45 already held a receiver —
`thread-context` had put one on every execution path — and the 21 that
did not were leaf helpers whose callers did.

**This is what makes `Builder::on_diagnostic` load-bearing rather than
decorative.** A `cd` that fails inside a loop under `set +e` is reported,
the loop continues, `run` returns `Ok`, and the hook is the only way an
embedder sees it as a value. The hook therefore **observes and does not
suppress**: the bytes still go where dash puts them, because that ordering
is what §3.2 is about, and an embedder that wants silence points
`Streams`' stderr somewhere quiet.

### 3.4 The variants

Ten, `#[non_exhaustive]`, with an `Other` arm. Full definitions in
`api.rs`.

`Syntax`, `Expansion`, `Redirect`, `NotFound`, `NotExecutable`, `Builtin`,
`Interrupted`, `Nul`, `Io`, `Other`.

Three choices worth defending:

* **`Expansion` carries the offending word but no `ExpansionErrorKind`.**
  `docs/idiomatization.md` §1.3 proposed a second enum. It would have six
  variants, no concrete embedder branches on it, and dash's text already
  distinguishes them. The word is kept because `expand_word` is on the
  surface and an embedder calling it wants to know which word failed.
* **`Other { line, status, message }` exists.** There are 33 `sh_error!`
  macro sites plus 25 direct calls plus one `exerror!`, and most produce
  text worth reporting and not worth a type — "`%s`: is read only" has no
  home in any of the other nine. It is also what makes the conversion
  tractable: `errors-are-values` can rewrite every raise site to `Other`
  mechanically and promote the interesting ones afterwards, instead of
  needing the final taxonomy before the first commit. `Other`'s `message`
  is documented as unstable, so promotion out of it is not a break.
* **`Interrupted` is an `Err`, and the library does not die of it.**
  `onint` re-raises SIGINT with `SIG_DFL` when the shell is not an
  interactive root shell — the library killing the process. That should
  move to the frontend: the library returns `Err(Error::Interrupted)` and
  `nsh-cli` re-raises. It is a fifth site for P8 that
  `docs/idiomatization.md` §1.7 does not count, because it is `raise`
  rather than `_exit`.

  **Half of this is done.** `errors-are-values` step F made the interrupt
  a value — `Error::Interrupted { signal }`, the first variant promoted
  out of `Other` — and `onint` returns it instead of raising. What has
  *not* moved is the `SIG_DFL`-and-`raise`, which is still in the library:
  it is a frontend-boundary question rather than an exception-mechanism
  one, and folding it in would have made step F two subjects. The
  `Signal` newtype is `public-api`'s; the variant carries a `c_int` until
  then.

### 3.5 `set -e` stays a flag on the call (§7.4, resolved)

`set -e` fires on a **status**, not on an error:

```
$ dash -c 'set -e; false; echo reached'
status=1
```

`false` produced no diagnostic, no `Err`, and nothing but a 1. So making
`EV_TESTED` a property of the `Error` is not merely inelegant — it is
impossible, because the majority of `set -e` aborts have no error value in
flight at all.

`evaltree`'s test is `eflag() && (!flags & checkexit) && status != 0`
(`eval.rs:405`), where `checkexit` is `EV_TESTED` for the node kinds whose
status `set -e` inspects and `flags` carries `EV_TESTED` down from the
enclosing syntax — a `while` or `if` condition (`eval.rs:354`), a `!`
(`eval.rs:378`), the left operand of `&&` or `||` (`eval.rs:344`). That is
a property of the *evaluation context*, and a parameter is the right
representation of it.

It stops being an integer bitmask:

```rust
pub(crate) struct EvalCtx { pub tested: bool, pub exit: bool }
```

`tested` is `EV_TESTED`, `exit` is `EV_EXIT`. The abort itself is
`Flow::Exit` in the `Ok` position, carrying the status, and `run` turns it
into `Ok(status)` with `has_exited()` true.

`Flow` collapses `evalskip`'s four bits and two of the four exception
codes:

```rust
pub(crate) enum Flow { Normal, Break(u32), Continue(u32), Return, Exit }
```

`EXEND` and `EXEXIT` both become `Exit`. `EXEND` carries no selected status;
`EXEXIT` carries the status selected by `exit` in the `Flow` value itself.
The first implementation preserved C's separate `savestatus` field, but the
Smoosh trap-status correction removed it: one ambient slot cannot represent an
EXIT action interrupted by a nested signal action.

The status itself stays a **field** rather than becoming the `Ok` payload,
because a dozen sites read `exitstatus` out of band (`eval.rs:399`,
`:1103`, `:1163`, `error.rs:334`, `exec.rs:153`, …) and turning it into a
return value is a second refactor riding on the first, which §2.2 of
`docs/idiomatization.md` forbids.

---

## 4. What `run` does when called twice (§7.6, resolved)

### 4.1 It is `eval`

`sh -c` and the `eval` built-in are already the same primitive:
`evalcmd` calls `evalstring` (`eval.rs:192`), `main`'s `-c` calls
`evalstring` (`shellmain.rs:176`), and `evalstring`
(`eval.rs:203-241`) is `setinputstring` → parse-execute loop → `popfile`.

So:

> **`Shell::run` is `eval`, at the top level. Two `run` calls compose
> exactly as two `eval` commands do.**

Everything the execution environment holds persists: variables, functions,
aliases, options, traps, the working directory, jobs, `$?`. What does not
persist is the parse. `run(b"if true; then")` is a syntax error, for the
same reason `eval 'if true; then'` is:

```
$ dash -c 'eval "if true; then"'
dash: 1: eval: Syntax error: end of file unexpected (expecting "fi")
```

A `run` that could be continued by the next one would have to block for
more input, which is what `Source::stream()` is for. `run` is a complete
parse unit; the line-continuation behaviour lives in the input stack, not
across the API boundary.

### 4.2 The parse-file stack, exactly

dash's stack is `parsefile` (the cursor), `basepf` (the statically
allocated bottom, whose `fd` is the shell's stdin, `input.rs:164`) and
`toppf` (the *floor* that `popallfiles` unwinds to, `input.rs:857-859`).
`pushfile` allocates a frame with `linno = 1` (`input.rs:790-803`);
`popfile` refuses to free `basepf` (`input.rs:822-825`);
`unwindfiles(stop)` pops until `parsefile == stop`
(`input.rs:845-849`).

`run`'s contract:

1. On entry, record `parsefile` as a mark, push the source above it, and
   move the unwind floor to the pushed frame.
2. On exit — normal, `Err`, or `Flow::Exit` — `unwindfiles(mark)` and
   restore the previous floor. The stack depth after `run` equals the depth
   before, always. That is a debug assertion, not a comment.
3. `$LINENO` starts at 1 for the pushed source, as `pushfile` already does.

Step 1's floor move is the one place this **diverges from dash**. dash's
`-c` uses `setinputstring`, which does not move `toppf`, so `reset()`'s
`popallfiles` (`input.rs:173`, reached from `shellmain.rs:229`) would unwind
past a `-c` string all the way to the shell's stdin. Making `run` its own
floor means an inner error can never terminate the embedder's `run` by
unwinding into the shell's standard input — which for a library is not an
optimisation but a correctness property, since the embedder's stdin may be
the host's terminal. The path is reachable only when
`iflag && shlvl == 0 && state != 0` (`shellmain.rs:219-227`), i.e. an
interactive shell, so the observable case is `sh -ic`. The differential
corpus runs `-c` without `-i`, so this is not covered; it is a candidate
for the sanctioned-divergence register and it should be checked against the
pty suite when `public-api` lands.

### 4.3 A `run` inside a host callback is a compile error

`[dec:nsh:public-surface]` asks that a `run` from inside a `Host` callback
*not* compose. It does better than that: it does not exist.

`run` takes `&mut self`. Every callback the shell invokes — a `Host`
method, the diagnostic hook — is invoked while that borrow is held, and no
`Host` method takes a `Shell`. So a callback cannot obtain a second
`&mut Shell`, and the re-entrant case is rejected by the compiler rather
than documented as a hazard.

This is the reason `Host` is designed as a leaf (§5.4), and it is worth
paying for. `api.rs` carries the proof in the other direction as well:
`dot_builtin` shows a built-in taking `&mut Shell` and handing it straight
back to `run`, which compiles only because its `args: &[&BStr]` borrow
nothing from the shell. §5.5.

### 4.4 `Source` is bytes or a descriptor, and nothing else

`docs/idiomatization.md` §1.2 says "from bytes, a file, or a reader". The
reader goes.

The input stack is descriptor-based, and not incidentally:
`preadfd`'s `fd == 0` test is the question "is this parse file the shell's
standard input", and it gates both line editing and the stdin tee;
`forkreset` closes `parsefile->fd` across a fork; `basepf.fd` is handed to
libedit. A `dyn Read` cannot be given to the line editor and cannot be
shared with a child across `fork`. So:

* `Source::bytes(..)` — the `-c` / `eval` shape.
* `Source::file(path)` — the `.` / `sh script` shape; the shell opens it.
  `$0` is not changed, so it is `.` without the PATH search.
* `Source::stream()` — the shell's own stdin from `Streams`; bare `sh`,
  and what an interactive frontend uses.

A caller with a reader writes it to a pipe or a file. That is the honest
cost and it is one sentence of documentation instead of a second input
path with no oracle.

---

## 5. What `Shell` owns

Field by field, at the granularity `no-ambient-state`'s `move-state` will
commit in. `crates/nsh/src/api.rs` declares exactly this list.

Baseline, **corrected**. This section originally read: "`grep -rn 'static
mut' crates/nsh/src` is **154**; excluding `gen/` it is 134, of which 4
are prose and 6 are `extern` declarations of libc globals, leaving **124
crate-owned declarations** to place." Two things were wrong with it and
`move-state` found both.

* **`crates/nsh/src/gen/` does not exist**, and never did in this tree.
  Every "excluding `gen/`" qualifier in this document is a no-op.
* **124 was a loose grep**: `grep -n 'static mut'` counts prose and
  comments as well as declarations. Counting *declarations* —
  `grep -rhE '^\s*(pub(\([a-z()]*\))?\s+)?static mut\s'` — the figure
  at the time was **107**.

Run the declaration form at both ends of any delta, in the same hour, or
the number means nothing. At `ecfd861` it is **39**.

| Field | Absorbs |
|---|---|
| `vars: VarTable` | `var.rs:297 vartab`, `:109 varinit` and its backing buffers `:80 defifsvar` / `:83 defoptindvar` / `:97 linenovar` (which `varinit[].text` aliases, so they move together — an understatement: the group was *self-referential*, and `defifsvar`, `defoptindvar` and `defpathvar` never move at all, being immutable statics), `:73 localvar_stack`, `:95 lineno` |
| `aliases: AliasTable` | `alias.rs:28 atab` |
| `commands: CmdTable` | `exec.rs:91 cmdtable`, `:92 builtinloc`, `:96 lastcmdentry` (became a slot index upstream of this table, as did `jobs.rs:104 njobs`). Function definitions live here because dash stores them in the same hash |
| `jobs: JobTable` | `jobs.rs:102 jobtab`, `:104 njobs`, `:114 curjob` (becomes an index), `:106 backgndpid`, `:109 initialpgrp`, `:111 ttyfd`, `:120 jobctl`, `:123 job_warning` |
| `options: Options` | `options.rs:114 optlist`, `:57 shellparam`, `:56 arg0` |
| `traps: TrapTable` | `trap.rs:38 trap`, `:40 ptrap`, `:42 trapcnt`, `:44 sigmode` |
| `input: InputStack` | `input.rs:120 parsefile`, `:98 basepf`, `:112 basebuf`, `:113 toppf`, `:114 stdin_state`, `:121 whichprompt`, `:122 stdin_istty`; and `parser.rs`'s eleven parser globals (`:305-315`), which are per-input-position state |
| `fds: FdTable` | The logical-to-real descriptor map, plus `redir.rs:44 redirlist` and `:47 closed_redirs` |
| `io: ShellIo` | **Done** at `ecfd861`. The row as written is stale: `output-is-a-writer` had already collapsed `output`, `errout`, `preverrout`, `out1` and `out2` into a single `SHELL_IO`, so what moved was one aggregate and not five statics |
| `eval: EvalState` | `evalskip`, `skipcount`, `loopnest`, `funcline`, `commandname`, `back_exitstatus`, `inps4`, and the per-shell nested `signal_trap_depth` catch mode |
| `streams: Streams` | **Done** at `ecfd861`. `streams.rs:98 STREAMS`, and `streams()` and `set()` with it |
| `host: Box<dyn Host>` | New. §5.4 |
| `on_diagnostic: Option<Box<dyn FnMut(&Error) + Send>>` | New. §3.3 |
| `signals: SignalSink` | `trap.rs:46 gotsig`, `:48 pending_sig`, `:50 gotsigchld`, `error.rs:92 intpending` — see §5.3 |
| `status: ExitStatus` | `eval.rs:85 exitstatus` |
| `exited: Option<ExitStatus>` | New. Replaces the `EXEND`/`EXEXIT` unwind reaching `main` |

**This table is not a complete inventory of shell state, and a reader who
finished its rows would have believed the job done with nine pieces of
per-shell state still process-global.** `move-state` found them and they
have since moved: `cd.rs`'s `curdir` and `physdir`, `mail.rs`'s
`mailtime` and `changed`, `expand.rs`'s `ifsmap`, `ncifs`, `wcifs` and
`ifsmb0len`, and `histedit.rs`'s `displayhist`. Four more —
`shellmain.rs`'s `rootpid`, `mypid`, `shlvl` and `dash_errno` — are
process *identity* rather than shell state and belong in §6 beside the
working-directory and child-reaping limits it already documents.

Two rows have been added since, for state this table did not name:
`eval.errlinno` (the line a diagnostic reports, `error.rs`'s `errlinno`)
and `eval.commandname`, which §5.2 wrongly listed as a transient alias —
see there.

`Shell` is `Send` and deliberately not `Sync`. Every method that can
observe shell state takes `&mut self`, so there is no shared-reference
concurrency to model, and `Host: Send` is what makes the box `Send`.

### 5.1 What does not become a field

Three groups, and none of them is a matter of effort.

**Pointers into a live stack frame.** `error.rs:89 handler` points at
caller-local `jmploc`s at `eval.rs:1145`, `:1192`, `expand.rs:2233`,
`parser.rs:2256`, `redir.rs:504`, `trap.rs:441`, `histedit.rs:571`. A
pointer into a frame cannot be a field of anything. It is deleted by
`errors-are-values`, which is why that step is upstream of this one —
`docs/idiomatization.md` §2.3 step 7 makes the argument and it is correct.

**Pointers into the region allocator.** `memalloc.rs:138 stackp`,
`:139 stacknxt`, `:141 sstrend` are all derived from
`addr_of_mut!(stackbase)` and are self-referential; `expand.rs:204
expdest`, `parser.rs:310 wordtext` and `jobs.rs:1580 cmdnextc` are cursors
into `stackblock()`. All of them are deleted by `delete-memalloc`, which is
also upstream.

**State written from a signal handler.** `trap.rs:46 gotsig`,
`:48 pending_sig`, `:50 gotsigchld`, `error.rs:92 intpending`. A handler has
no `&mut Shell` and cannot be given one. These become the `SignalSink`
(§5.3): an `Arc`-backed array of atomics that the shell polls where dash
reads `pending_sig`, and that the host's handler stores into.

**`suppressint` is not on that list, and stopped being signal state
before this section was written.** `error.rs:61` sits beside `intpending`
and is named with it in `docs/idiomatization.md` §step-9 — "they are
signal state" — which was true of dash and is no longer true here. Since
`errors-are-values` step F the handler does not read it: `onsig`'s
`if (!suppressint) onint();` is gone (`trap.rs:296-306`), leaving
`INTOFF`/`INTON` as a counter touched only from ordinary frames. So it is
ordinary shell state that could become a field — `errors-are-values`' or
`public-api`'s to place — and counting it against the handler
overstates what §5.3 has to carry by one. Nothing in this section
depended on it; the correction is recorded so the next inventory does not
inherit the mistake.

### 5.2 Transient aliases scope to a call

`options.rs:64 argptr` and `:66 optptr`, ~~`eval.rs:84 commandname`~~,
`exec.rs:94 pathopt`, `expand.rs:209 argbackq`, `arith_yacc.rs:99
arith_buf`, `bltin/printf.rs:104 gargv`, `bltin/test.rs:169 t_wp` are
cursors into memory a *caller* owns for the duration of one call. They are
not shell state and must not become fields; they become parameters or
locals. Making them fields would recreate the aliasing problem the refactor
exists to remove, and — worse — would put a borrow of caller data inside
`Shell`, which is exactly what §5.5 says must never happen.

### 5.3 The signal inbox

The host installs the disposition; the shell needs the delivery. The seam
is one `Arc`:

```rust
pub struct SignalSink { /* Arc<[AtomicBool; NSIG]> + an AtomicI32 */ }
impl SignalSink { pub fn raise(&self, signal: Signal); }   // one relaxed store
```

`Host::attach(sink)` hands it over once at build time; the host's
`extern "C"` handler does nothing but `sink.raise(signal)`, which is
async-signal-safe by construction. The shell polls it where `dotrap`
reads `pending_sig` (`trap.rs:370`). The sibling crate already has this
shape in `nshedit-plat/src/signal.rs`.

`attach` is a **required** method with no default body, so a host cannot
install `Disposition::Catch` and silently never deliver anything.

**The handler reads as well as writes, which this section missed.**
`raise(signal)` is not the whole of the handler's contract with the sink.
`onsig` also *asks* it two questions before storing, and it must answer
them without a receiver and without allocating:

* **Is a trap set for this signal?** `trap.rs:287` and `:295` index the
  trap table, and both are presence tests rather than reads of the
  action. So the sink carries a `[AtomicBool; NSIG]` mirror of
  "`trap[n].is_some()`" beside the arrival flags. The behavioural surface
  is two bits — `SIGCHLD` and `SIGINT` — and the array is NSIG-wide only
  because the writers index by `signo`.
* **Am I the vforked child?** `trap.rs:281`. `jobs.rs:1139` sets
  `vforked = mypid` in the *parent* before `vfork`, and the child reads
  it out of the shared address space. This is a property of an address
  space rather than of a shell, so it is an `AtomicI32` in the sink and
  **not** a `Shell` field.

**The mirror's writes must be atomic against delivery, and `INTOFF` will
not do it.** `INTOFF` defers *taking* an interrupt; it does not stop the
handler running, and since `errors-are-values` step F `INTON` is not a
delivery point at all. Nor is there a safe one-sided write order, because
the two signals want opposite ones: a mirror that reads "trapped" when
the table says none swallows a `^C` but makes `wait` answer `145` for
SIGCHLD (`bltin/wait.rs:51`), and a mirror that reads the other way takes
the interrupt instead of running the user's trap. So the table store and
the mirror store are bracketed by `sigblockall`/`sigclearmask` —
`jobs.rs:1908-1910`'s `xtcsetpgrp` is the same idiom for the same reason.
The bracket is hoisted to the two writers, `trapcmd` and `clear_traps`
(one pair per `trap` command, one per fork), and `TrapTable::set` takes a
`&SignalsBlocked` witness so a slot cannot be written outside one.

This restores a property the C has for free and a mirror destroys: in
dash `trap[signo]` is a single pointer, so a handler reads either the old
value or the new one and never an inconsistent pair.

### 5.4 The `Host` trait

```rust
pub trait Host: Send {
    fn attach(&mut self, sink: SignalSink);
    fn signal(&mut self, signal: Signal) -> io::Result<Disposition>;
    fn set_signal(&mut self, signal: Signal, to: Disposition) -> io::Result<()>;
    fn may_replace_process(&mut self) -> bool;
}
```

**No method takes a `Shell`, and that is the design.** It buys three
things at once: re-entrant `run` becomes a compile error (§4.3);
`self.host.set_signal(..)` is a field-disjoint borrow inside a
`&mut self` method, so it composes with the tables it does not touch
(`api.rs` compiles that line as proof); and `Shell` can own the host
rather than threading it through 587 signatures a second time.

**Why `signal()` and not just `set_signal()`.** dash's `setsignal` reads
the current disposition with `sigaction(signo, NULL, &act)` when `sigmode`
is unknown, and a signal that was already `SIG_IGN` on entry becomes
`S_HARD_IGN` and can never be trapped (`trap.rs:245-269`). That rule cannot
be reproduced without reading the inherited disposition, and `nsh-cli` has
to reproduce it.

**Where the line falls.** Policy is the library's, mechanism is the host's.
`trap.rs`'s `setsignal` decides *which* disposition, including the
`mflag`/SIGTSTP exception at `trap.rs:259-264`; the host performs the
`sigaction`, and to be dash it must use `sigfillset` on `sa_mask` and
`sa_flags = 0` (`trap.rs:284-287`). Those two lines are part of the trait's
contract and are documented on the method.

**`may_replace_process` is the second half of the trait, and it is new.**
`execcmd` calls `shellexec`, which `execve`s **in the current process**
(`eval.rs:1341-1350`, `exec.rs:118`). In a frontend that is the point of
`exec`. In a library it destroys the host: `sh.run(b"exec ls")` would
replace the embedding program's image. Nothing in the plan or in
`docs/idiomatization.md` names this, and it is a sharper example of "what a
library may not do on its own authority" than terminating is. A host that
refuses gets the diagnostic and status a failed `exec` produces
(`exec.rs:143-160`); `nsh-cli`'s host says yes, so dash's behaviour is
preserved exactly.

**Correction: `exec cmd` is not the only site, and the other one has no
syntax.** `process-model` counted the `shellexec` callers and there are
three. `jobs.rs:1160` is inside a vforked child and is the point of the
exercise. `builtins/exec.rs:48` is the `exec` builtin, which is the site
this paragraph found. The third is `eval.rs:1389` — `evalcommand`'s
`EV_EXIT` fast path, which `execve`s the **last command of the script** in
place, reached from `main`'s `-c` at `shellmain.rs:199`. So `dash -c 'ls'`
replaces its own image with no `exec` written anywhere, and a
`Shell::run` built naively on the `-c` path would do that to the embedder
on `sh.run(b"ls")`.

The fix is not a second `Host` method. It is a constraint on §4:
**`run` passes no `EV_EXIT`**, so the optimisation stays available to
`nsh-cli` — which passes it — and is unreachable from the API.
`[dec:nsh:host-owns-the-process]` records it, and it has a second effect
worth knowing: `evalsubshell`'s no-fork arm (`eval.rs:717-721`) is
`EV_EXIT`-only too, so from `run` the shell never runs `forkreset` in its
own process either.

**What is *not* on the trait.** Terminating the process is not, because
after this design the library never needs to: `run` returns and `nsh-cli`
calls `std::process::exit`. The one place the library still ends a process
is inside a forked child, where `_exit` is forced and correct, and that is
not the host's decision to make.

The forked child's own constraint was left to `process-model`, which has
now answered it — and the answer is narrower than the sentence this
paragraph used to carry. "Async-signal-safe work only, no destructors, no
allocator" is **unattainable after `fork` and unnecessary**: a subshell is
a shell, so it allocates because it evaluates, and `forkchild` frees the
job table before the child runs a command. What holds after `fork` is only
*it does not return*. The strong form holds after **`vfork`**, where it is
stated as "writes no location the parent reads again" and has been audited
line by line. `[dec:nsh:fork-child-is-a-terminus]` carries both, and §11
below carries what it means for an embedder.

### 5.5 The borrow problem, solved before it is discovered

A built-in gets `&mut Shell` and hands it straight back to evaluation. Ten
of them do: `.` (`shellmain.rs:429`), `eval` (`eval.rs:166`), `command`,
`fc` (`histedit.rs`), and the trap dispatcher (`trap.rs:408`).

```rust
pub(crate) type Builtin = fn(&mut Shell, &[&BStr]) -> Result<ExitStatus, Error>;
```

This works — and only works — because **`args` does not borrow from
`sh`**. If the argument vector were a slice into a `Shell` field, `&mut sh`
could not be reborrowed and no re-entrant built-in could be written at all.
So the rule, stated now rather than found later:

> Nothing the shell hands to a built-in, a callback, or the host may
> borrow from the shell.

That has three concrete consequences for `no-ambient-state`:

* The expanded argument vector is owned separately (today it is
  region-allocated, which is the same property by accident).
* `options.rs:64 argptr` and `:66 optptr` stay call-scoped (§5.2).
* `Host` methods take no `Shell` (§5.4), and the diagnostic hook takes
  `&Error` rather than `(&mut Shell, &Error)`.

`api.rs`'s `dot_builtin` is the compiling demonstration.

---

## 6. Two shells are independent, except where the process is indivisible

`[dec:nsh:no-ambient-state]` is the rule; [dec:nsh:per-shell-locale] closes
the C-library exception that this section formerly accepted. Each `Shell`
owns an explicit locale object. Locale-dependent operations borrow it, and a
short thread-locale selection is restored before control returns to the host;
neither `environ` nor process-global `setlocale` is a configuration channel.

The variable table is likewise authoritative. `Builder::inherit_env` takes an
owned snapshot when asked, and `execve` receives an envp built from the shell's
exported variables. Two Shells may therefore hold different environments and
locales without publishing either one into their host process.

The remaining limits are facilities whose underlying process or C-library
state is still indivisible:

* **`strtok`.** `cd.rs:218,237`. A process-global tokeniser cursor.
* **`getopt` / `optind`.** `histedit.rs:41-43` declare the externs and
  `:508` calls `getopt`, resetting `optind` by hand at `:497,539`.

None is counted by a `static mut` audit, because the static is in libc.
There are further process-wide facts the API has to be honest about:

* **The signal inbox.** §5.3's `SignalSink` is process-wide, and the
  `Arc` does not make it otherwise. A disposition is installed per
  *process* and the handler is called with `signo` and nothing else, so
  it cannot know which `Shell` the signal was meant for. That covers the
  arrival flags, the pending-signal scalar, `intpending`, the trap-set
  mirror and `vforked`. Two shells in one process share one inbox, and
  the second to install a handler is the one it reports to.
* **The process's own identity.** `shellmain.rs`'s `rootpid`, `mypid`,
  `shlvl` and `dash_errno` are facts about the process rather than about
  a shell, and they are listed here rather than in §5's table because
  that is what they are. `mypid` in particular is read by the vforked
  child out of the shared address space, which is why
  [dec:nsh:fork-child-is-a-terminus] audits its one write.
* **The working directory.** `chdir` is per-process. `Builder::cwd` and
  `cd` are per-instance in the sense that `$PWD` is, and process-wide in
  the sense that the syscall is. Two shells in different directories is
  not achievable without `openat`-relative resolution everywhere, which is
  out of scope. Say so in the crate docs.
* **The children of the process, which are one pool.** `waitproc` calls
  `wait3(status, flags, NULL)` (`jobs.rs:1412`) — that is `waitpid(-1)`,
  and it reaps *any* child of the process. Two `Shell`s reap each other's
  children; a `Shell` and an embedder holding a `std::process::Child` do
  the same, and the embedder is the one that loses, because its own `wait`
  then answers `ECHILD` for a status that is sitting in a job table it
  cannot see. **This is the first entry on the list that is the kernel's
  rather than the C library's**, and it is the one an embedder is most
  likely to trip over, because nothing about it looks like shared state.
  It cannot be fixed by tracking pids: reaping is destructive, so the
  ownership test has to happen before the reap, and the only primitive
  that peeks without reaping (`waitid(P_ALL, …, WNOWAIT)`) returns the
  same foreign child forever and turns a blocking wait into a spin. The
  shell would have to own `SIGCHLD` for the whole process and dispatch by
  pid, which is exactly the disposition `[dec:nsh:host-owns-signals]` says
  it may not claim. `[dec:nsh:host-owns-the-process]` records it.
* **The process group and the controlling terminal.** `setjobctl(1)`
  performs `setpgid(0, rootpid)` (`jobs.rs:482`), `tcsetpgrp` (`:483`) and,
  on the way there, possibly `killpg(0, SIGTTIN)` (`:452`) — all three on
  the *host's* own process and process group, and none of them undone by
  anything but `setjobctl(0)`. Two `Shell`s cannot each be the foreground
  process group, because there is one process. Unlike the entries above
  this one is *gated* rather than merely documented: job control is off
  unless the host grants it.

The honest statement, which belongs on the decision, is now:

> Two `Shell` values in one process have independent variables and locales.
> They still share the C library's `strtok` cursor and `getopt` state, the
> working directory, the process group and controlling terminal, the kernel's
> pool of child processes — and one thing this crate does own, the signal
> inbox, because a signal disposition and the handler that reads it are
> per-process facts that no amount of per-instance storage can divide.

The earlier form of that sentence read "share nothing this crate owns",
and the inbox is the counter-example. It is stated as a shared *fact*
rather than a shared *field* on purpose: `SignalSink` being an `Arc` is
what lets a host hold a clone, not what would make two shells
independent.

**One correction to how this section has been read.** The list is not "the
C library's globals" and never was — it acquired that shape because the
first three entries happened to be libc statics and §5's `static mut`
audit was the tool at hand. Three of the seven entries are not libc's at
all: the working directory and the child-process pool are the kernel's,
and the signal inbox is this crate's. **A process-wide fact is anything
one `Shell` can change that another observes, whoever stores it.** That is
the test to apply when the next one is found, and applying the narrower
one is how the child-process pool went unlisted through
`public-api-design`, `move-state` and `host-owns-signals`.

---

## 7. Streams, and which promise gets weakened (§7.5, resolved)

`docs/idiomatization.md` §7.5 guesses that external commands are the part
that cannot be fixed, because they "inherit real descriptors and cannot be
lied to by a per-instance table". **That is backwards: external commands
are the easy part.**

The shell forks before it execs (`forkchild`, then `shellexec`). The child is
the shell's own process. It now materialises the logical-to-real map through
`nsh_platform::ProcessFdChanges` immediately before `execve`, and the
external command sees exactly the descriptors the script asked for.
Pipelines, `exec 3>&1`, `>file` and `2>&1` are all logical table operations;
only materialisation changes exact process slots.

That is what lets `install` go (§2.1) and what retires
`[dec:nsh:host-owns-streams]`'s deferred consequence. Two gaps remain, and
they are the promises that get weakened:

1. **`/dev/stdout`, `/dev/fd/N` and `/proc/self/fd/N` name the kernel's
   table, not the shell's.** `echo hi > /dev/stdout` under
   `Streams::from_fds` reaches the process's descriptor 1, not the supplied
   one. Fixing it means special-casing those paths in `open`, which is a
   hack with its own edge cases. Documented instead.
2. **`exec cmd` cannot be honoured.** It replaces the process image, so
   the logical table would have to be forced onto the host's real
   descriptors — precisely what `from_fds` exists to avoid. It goes through
   `Host::may_replace_process` (§5.4), and an embedder's host refuses.

`Streams::inherit()` begins as an identity snapshot, but a logical
redirection can make `/dev/fd/N` differ from the still-unmodified host table;
that documented gap therefore applies to inherited and supplied streams.
External commands are unaffected because materialisation precedes `execve`.
The differential harness still exercises only the inherited frontend
configuration, so `crates/nsh/tests/streams_embed.rs` separately pins
redirection restore, pipelines, and direct external commands under
`Streams::from_fds`. Those tests were written red first and now pass.

**`Streams::capture()` is backed by an unlinked temporary file, not a
pipe.** A pipe with no concurrent reader blocks the shell as soon as the
script writes more than the pipe buffer, so a capture API built on one
deadlocks on any script with real output. The temp file is seekable, needs
no second thread, and is what the shell already does for here-documents.

---

## 8. What the example changed

`crates/nsh/examples/embed.rs` was written before `api.rs` and rejected two
signatures.

**`captured_stdout(&mut self) -> &BStr` does not work.**
`[dec:nsh:public-surface]`'s rationale and `docs/idiomatization.md` §1.1
both show `let out: &BStr = sh.captured_stdout();`. The borrow is tied to
the `&mut self` that reads the capture file, so holding the output locks
the shell, and run-look-run — the entire reason to capture — fails to
compile with four `E0499`s. It becomes
`take_captured_stdout(&mut self) -> io::Result<BString>`: owned, draining,
composable.

**`.env(std::env::vars_os())` does not compile.**
`docs/idiomatization.md` §1.1 shows it. `vars_os` yields `OsString`, which
is bytes on Unix but is not `Into<BString>`, and a bound that accepted both
`OsString` and `&BStr` would cost more than a second method. It becomes
`Builder::env` (explicit pairs) plus `Builder::inherit_env()` (the
process's own, as bytes).

A third thing the example confirmed rather than changed: `var(&self) ->
Option<&BStr>` does hold the shell immutably while the borrow lives, and
that is correct rather than a papercut — an assignment can move the table.
A caller that needs the value across a `run` copies it out.

---

## 9. Proposed edits to the decisions

Not applied here. `plan/decisions/public-surface.md` gets a diff in the
same commit as this document; the rest are listed for review.

**`[dec:nsh:public-surface]`** — should now assert rather than defer:

* Resolve the deferred consequence with §4: `run` is `eval` at the top
  level, two calls compose, the parse does not straddle the boundary, the
  stack is unwound to its entry depth on every exit path, and a `run` from
  inside a host callback is a compile error rather than a rule.
* Correct the item count: about fifty-five `pub` items, not twenty.
* Correct `captured_stdout` in the rationale's code block (§8).
* Record that `Host` methods take no `Shell`, and what that buys.
* Record `may_replace_process`: `exec cmd` `execve`s in place
  (`eval.rs:1341`), which a library may not do on its own authority. This
  is new to the decision.
* Record that `Streams::install` retires, and the two gaps `from_fds`
  keeps (§7).

**`[dec:nsh:errors-are-values]`** — resolve the deferred `set -e`
question with §3.5: `EV_TESTED` stays a property of the call, because
`set -e; false` aborts with no error value in flight. Add the two write-path
details in §3.2 (stderr unbuffered, stdout flushed *after* the message) and
the §3.3 contract that `Err` means "aborted the run", which is what makes
the diagnostic hook load-bearing.

**`[dec:nsh:no-ambient-state]`** — extend the recorded limit to the
process environment and the working directory (§6), which are the same
shape as the locale and are not listed. `process-model` adds two more and
one of them is a category error the earlier list encouraged: the
**child-process pool** (`wait3(-1)` reaps the host's children as well as
the shell's) and the **process group and controlling terminal**. Neither
is a libc static, so neither would ever have been found by looking for
one. §6 now states the test as "anything one `Shell` can change that
another observes, whoever stores it".

**`[dec:nsh:shell-as-library]`** — its first accepted consequence lists
what moves into the frontend as "exit, signals, argv, the standard
descriptors". **"Exit" understates it by a category.** Replacing the
process image is worse than ending it — the decision's own fifth
consequence says so — and moving the host's process group and taking its
controlling terminal are two more. The general form is
`[dec:nsh:host-owns-the-process]`, which subsumes the `exit` item rather
than sitting beside it, and §11 is its working.

**`[dec:nsh:host-owns-signals]`** — the seam it transferred to
`public-api` should be split before it is worked. Of the 21 disposition
call sites in the crate, **12 run in a child the library just forked** and
must stay in the library: routing them through a `Box<dyn Host>` would be
an indirect call into embedder code from a forked child, which is the
hazard `[dec:nsh:fork-child-is-a-terminus]` exists to bound. The nine
host-side sites are the trait's. `redir.rs`'s five raw `libc::signal`
calls, which the transfer lists explicitly, are all in the here-document
writer child and are five of the twelve.

**`[dec:nsh:minimal-unsafe]`** — its deferred consequence ("where the
floor actually sits… never been counted separately") is **resolved and
the edit is applied in this commit**: 255 `libc::` sites, 68 symbols, 13
more hand-declared, thirteen groups, and the finding that the floor is
not the syscalls — 85 of the 255 are `stat`, identity, limits, `errno`,
`getopt` and the C library's locale-dependent string routines. §11.3.

**`[dec:nsh:host-owns-streams]`** — the deferred consequence can be
retired by §7, and replaced with the two gaps that survive.

**`[dec:nsh:host-owns-signals]`** — record that the library's `sigaction`
count goes to literally zero rather than "zero outside a seam", because the
`Host` implementation ships in `crates/nsh-cli`. There is exactly one
consumer and the stronger property is free.

**`docs/idiomatization.md`** — §1.1's example, §1.2's "or a reader",
§1.3's `ExpansionErrorKind` and `diagnostic() -> impl Display`, and §7.5's
guess about external commands are all superseded above.

---

## 10. What this is not sure about

1. ~~**The per-instance descriptor table is the largest bet here.**~~
   **Resolved.** The `streams_embed.rs` cases for redirection, pipelines and
   external commands under `from_fds` were written first and failed against
   the ambient process-table model. They pass with the logical table. Core
   searches also find no `RawFd`, `DescriptorSlot`, or ambient exact-slot
   operation; the only raw `dup2`/`close` pair is private to the platform
   materialisation transaction.

2. ~~**Whether `EXEND` and `EXEXIT` really differ only in which status is
   taken.**~~ **Resolved, and §3.5's collapse is right.**
   `errors-are-values` did the reading before writing `Flow`, and found a
   stronger answer than the reading this entry asked for:
   `error::exception` was read in exactly *three* places in the crate —
   `evalcommand`'s built-in arm, `main`'s handler, and `init::exitreset`.
   Only the last distinguished the two codes, by deciding whether to restore
   `savestatus`. That audit correctly described the C mechanism, but the
   adopted nested-trap semantics exposed its representation as
   non-compositional. `Flow::Exit` now carries `Option<c_int>`: `Some` is the
   status selected by `exit`, while `None` leaves the shell's current status
   in force. `docs/errors-are-values.md` §0.3 records both the original audit
   and this later correction.

3. **The unwind-floor divergence in §4.2.** `run` making itself the floor
   differs from dash for `sh -ic`, and the differential corpus does not
   cover it. *Resolved by:* a pty case, or by accepting it into the
   sanctioned-divergence register with the reason above.

4. **Whether `Builder::option` is enough for `nsh-cli`.** Moving dash's
   argv parsing into the frontend is safe with respect to the oracle —
   `dscase.sh:60` invokes the binary with `$shargs -c`, so every
   differential case exercises it — but it assumes every invocation flag
   reduces to an entry in `optlist`. `-c` and `-s` do (`options.rs:33-54`);
   `--` and the operand split may not. *Resolved by:* reading `procargs`
   (`options.rs:122`) against the proposed frontend before the split.

5. **`expand_word` executes.** Command substitution is part of word
   expansion, so the API item that `[dec:nsh:public-surface]` singles out
   as the reason to have a shell library will run `$(...)` on whatever it
   is given. There is deliberately no flag to disable it, because the
   shell language has no such mode and a shell that silently expands
   `$(date)` to nothing is a different language. It is documented on the
   method. Whether that is sufficient for an embedder handling untrusted
   words is a question this design leaves open and does not think it can
   close with a type.

---

## 11. What the library does to the process (`process-model`, resolved)

§5.4 asks what a library may not do on its own authority and answers for
signals, streams and `exec`. This section answers it for the process
itself, and it is the artefact `process-model` produces alongside
`[dec:nsh:host-owns-the-process]` and
`[dec:nsh:fork-child-is-a-terminus]`.

### 11.1 The line is not the syscall, it is whose process

The tempting formulation — "these syscalls are banned in a library" —
does not survive contact with `jobs.rs`. `setpgid` appears twice in the
same fork, eleven lines apart: `jobs.rs:992` in the child, `:1079` in the
parent, deliberately raced so whichever wins puts the job in its group.
One of those is ordinary work and one of them is an operation on the
embedder's process. Same syscall, same function, same second.

So the test is **whose process is the object of the call**:

| Operation | On a child the library forked | On the host's own process |
|---|---|---|
| `fork` / `vfork` | — (this *is* the making of it) | n/a |
| `execve` | free — `jobs.rs:1160` | **grant** — `builtins/exec.rs:48`, `eval.rs:1389` |
| `_exit` | free — `exit_from_child`, `forkchild_fatal`, `redir.rs:483` | **deleted, not granted** — `trap.rs:562`; after the builder `run` returns |
| `setpgid` | free — `jobs.rs:992`, `:1079` | **grant** — `jobs.rs:482` |
| `tcsetpgrp` | **grant** — see below | **grant** — `jobs.rs:483` |
| `killpg` | free — `builtins/fg.rs:87` (SIGCONT to a job) | **grant** — `jobs.rs:452` (SIGTTIN to our own group) |
| `sigaction` / `signal` | free — 12 sites, all in a forked child | **host's** — 9 sites, `[dec:nsh:host-owns-signals]` |
| `wait3` | free | n/a — but see §6, it reaps the host's children too |
| `kill` | free | free — the *script* named the target, not the library |

`tcsetpgrp` is the one row where a child's operation still needs the
grant, and the reason is that a child taking the terminal from the host's
foreground group is the same theft performed one process away. It needs no
second gate, though: `xxtcsetpgrp` returns `Ok(())` when `ttyfd < 0`
(`jobs.rs:351-357`) and only `setjobctl` ever sets `ttyfd`. **Gating
`setjobctl` gates every terminal operation in the crate**, including the
ones in children.

### 11.2 Three grants, one of which is an absence

* **`Host::may_replace_process`** — already in §5.4, now with two call
  sites rather than one, and with the `run`-passes-no-`EV_EXIT`
  constraint that covers the second.
* **Job control** — a builder input, defaulting to off. It gates
  `setjobctl(1)`, and through `ttyfd` it gates the terminal.
* **Ending the process** — no grant, because after the builder the
  library never needs one. The capability that does not exist is the
  strongest form of the ban.

### 11.3 The syscall floor, enumerated

`unsafe-is-a-crate`'s directive names the floor as
"fork/exec/wait/signals/termios/fd ops". Measured at `410e729` over
`crates/nsh/src` (in-file `#[cfg(test)]` modules included; the command is
`grep -rhoE 'libc::[a-z_0-9]+\(' src/ --include=*.rs | sort | uniq -c`):

**255 `libc::` call sites across 68 distinct symbols, plus 13 symbols
hand-declared in 7 `extern "C"` blocks** because `libc` 0.2 does not bind
them. One vendor: there is no `nix` and no direct `rustix` in the crate.

| Group | Symbols | `libc::` sites |
|---|---|---|
| Process creation / exec | `fork` `vfork` `execve` `_exit` | 26 |
| Wait / reaping | `waitpid` `sigsuspend` — and `wait3`† | 2 |
| Process groups / terminal | `setpgid` `getpgrp` `tcgetpgrp` `tcsetpgrp` `killpg` `tcgetattr` `isatty` | 14 |
| Signals | `sigaction` `signal` `sigprocmask` `sigfillset` `sigemptyset` `sigaddset` `sigismember` `kill` `raise` `strsignal` | 34 |
| Descriptors | `close` `open64` `dup` `dup2` `pipe` `fcntl` `lseek` `read` `write` `memfd_create` `tee` `mkstemp` `unlink` `fpathconf` | 94 |
| stat family | `stat64` `lstat64` `fstat64` `faccessat` | 20 |
| Directory iteration | `opendir` `readdir64` `closedir` | 4 |
| Identity | `getpid` `getppid` `geteuid` `getegid` `getgroups` `getpwnam` | 14 |
| Limits / times / umask | `umask` `getrlimit` `setrlimit` `times` `sysconf` | 13 |
| Environment / locale | `setlocale` `getenv` `putenv` — and `environ`† | 5 |
| errno | `__errno_location` | 3 |
| `getopt` | `getopt` — and `optind`† `optopt`† `optarg`† | 1 |
| C library, not syscalls | `strerror` `strcoll` `fnmatch` `atoi` `isalpha` `isalnum` `isspace` `isdigit` — and `strtoimax`† `mbrlen`† `mbrtowc`† `mbsrtowcs`† `iswspace`† `wctype`† `iswctype`† `iswblank`† | 25 |

† hand-declared rather than bound by `libc`, so not counted in the column.
The column is `libc::` call sites and sums to exactly 255, which is the
check that the grouping lost nothing. `close` alone is 47 of them and
`_exit` 22 — most of the latter are the descriptor assertions in
`streams.rs`'s own test module, which is why the group totals are a map of
the code rather than of the shell.

Two things this makes visible that the directive's six-word summary does
not.

**The floor is not six groups, it is thirteen, and the last one is the
awkward one.** `fork/exec/wait/signals/termios/fd` is 170 of the 255
sites. The rest are `stat`, identity, limits, locale and the C library's
string and multibyte routines — not syscalls, but still FFI, and `nsh`
cannot deny `unsafe_code` while any of them is called directly. So the
floor crate is "everything below safe Rust", not "the syscalls", and its
last group is the one where a Rust replacement is a *behaviour* question
rather than a wrapping exercise: `strcoll`, `isalpha` and `mbrtowc` are
locale-dependent, and §6's first entry is why that matters.

**The wrapper layer already exists for about half of it and is missing
for exactly the parts this document has been arguing about.** Real
`Result`-returning helpers: `redir::{sh_open, sh_pipe, sh_dup2, savefd}`,
`jobs::{forkshell, vforkexec, xtcsetpgrp, xxtcsetpgrp, waitproc}`,
`trap::{setsignal, ignoresig, sigblockall}`, `system::{sigclearmask,
errno}`, `output::{write_fd_once, write_fd, xwrite}`,
`siginbox::SignalsBlocked`. Unwrapped and called raw from 90-odd sites:
`setpgid`, `getpgrp`, `tcgetpgrp`, `isatty`, `close`, `stat64`, `kill`,
`killpg`, `raise`, `_exit`. **The unwrapped list is almost exactly the
list of grant-bearing operations in §11.1** — which is not a coincidence,
it is the same observation from the other side: the operations nobody had
to think about are the ones nobody wrapped.

### 11.4 What the floor's API may not be

One constraint comes out of `[dec:nsh:fork-child-is-a-terminus]` and it
is worth stating before the crate is written, because it is cheap now and
expensive later.

**The wrappers a forked child calls must not allocate.** Between `vfork`
and `execve` the child runs in the parent's address space; between `fork`
and `execve` in a multithreaded host it may run with the allocator lock
held by a thread that no longer exists. The wrappers on that path —
`execve`, `_exit`, `dup2`, `close`, `open`, `setpgid`, `tcsetpgrp`,
`signal`, `sigaction` — must therefore return an error that is a bare
`errno`, not a `Box<dyn Error>`, not a `String`, and not something that
formats a message on construction.

That is the same shape `output-is-a-writer` and
`[dec:nsh:printf-is-parsed-not-interpreted]` arrived at for unrelated
reasons: the value carries the facts, the call site does the rendering.
Three decisions now want it, so the floor crate should be built that way
from its first commit.

### 11.5 One thing this section was not sure about, now answered

**Whether job control can be a builder input at all, or has to be a
`Host` method.** §11.2 makes it a builder flag, on the reasoning that a
grant is answered once. But `set -m` can be executed *by the script*, at
any point, and `optschanged` reaches `setjobctl` from `poplocalvars`
restoring a `local -` option set. A builder flag makes `set -m` silently
ineffective in an ungranted shell, which is a divergence the differential
corpus cannot see — it has no controlling terminal, so `setjobctl` leaves
`jobctl` at 0 there in *both* shells today.

**Answered by `public-api`: it is a `Host` method, and the ungranted
`set -m` is silent.**

`Host::may_control_terminal` sits beside `may_replace_process`, and the
argument is that it is the same kind of thing — a power over a process the
library did not create. Turning job control on is `setpgid(0, rootpid)`,
`tcsetpgrp`, and on the way there possibly a `killpg(0, SIGTTIN)` that
stops the host and every sibling with it; that belongs where the signal
dispositions and the `execve` grant already are. A builder flag would have
been a second gate in a second place, free to disagree with the first.

It is silent because `set -m` is written by the *script*, not by the
embedder, and `optschanged` reaches it from a `local -` restore — so a
warning would fire on a line nobody wrote. dash is itself silent when it
cannot get the tty in the ordinary case.

One gate covers the whole feature, and the interlock was already there:
`xxtcsetpgrp` returns `Ok(())` when `ttyfd < 0`, and `setjobctl` is the
only thing that ever sets `ttyfd`. Refusing in `setjobctl` therefore also
gates `forkchild`'s handoff, `waitforjob`'s hand-back and `fg`'s.

*Covered by:* `host::tests::set_m_without_a_grant_leaves_the_hosts_terminal_alone`,
which is a unit test rather than the ptydiff case this section asked for.
The reason is that ptydiff drives *`nsh-cli`*, which grants the terminal
because it is a frontend, so the ungranted half of the comparison has no
shell to run in — an embedded shell is not something the pty harness can
launch. What ptydiff does still carry is the granted half: its seventeen
job-control cases are the proof that granting still behaves exactly as
dash, and they pass unchanged.
