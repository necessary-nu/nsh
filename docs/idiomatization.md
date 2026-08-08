# Idiomatization: the end state, and the order that gets there

Status: proposal. Nothing here is decided. Where it disagrees with
`plan/main.styx` or with a file in `plan/decisions/`, it says so and says
why; applying any of it is a separate act.

Every number in this document was measured on the tree that became
`4463fc6` ("fix: Record the line number on a backgrounded command's
wrapper"), with `owned-nodes` complete and `owned-strings` not started.
The commands are given so they can be re-run and the claims can rot
visibly.

---

## 0. The problem this document exists to solve

The sequence of idiomatization has been decided incrementally and
corrected twice, both times by an "X is upstream of Y" argument made
after the fact. Nobody has written the upstream graph down, and nobody
has written down what "done" is. Both corrections were right. Neither was
derivable from anything in the repository.

So: section 1 is the end state, section 2 is the order and the argument
for it, section 3 is what the plan does not contain at all, section 4 is
the oracle, section 5 is risk.

---

## 1. What the end state is

### 1.1 What an embedder calls

```rust
use nsh::{Shell, Streams, ExitStatus};

let mut sh = Shell::builder()
    .arg0("myapp")
    .env(std::env::vars_os())
    .streams(Streams::capture())      // or ::inherit(), or ::from_fds(..)
    .build()?;

let status: ExitStatus = sh.run(b"for f in *.txt; do wc -l \"$f\"; done")?;
let out: &BStr = sh.captured_stdout();
let home = sh.var(b"HOME");
sh.set_var(b"PATH", b"/usr/bin:/bin");
```

The three properties that make this different from spawning `/bin/sh`:

* **No second process.** `sh.run` parses and executes in the caller's
  process. External commands still fork; the shell itself does not.
* **No quoting round-trip.** The input is a byte slice, the output is a
  byte slice, and neither passes through a `String`
  ([dec:nsh:bytes-not-text]).
* **Errors arrive as values.** `run` returns `Result<ExitStatus,
  nsh::Error>`, not an exit status and some text on a pipe.

And one that is easy to promise and hard to deliver:

* **Two shells in one process are independent.** Two `Shell` values, on
  one thread or two, share nothing. This is the whole content of
  [dec:nsh:no-ambient-state] and it is what makes the type a type rather
  than a handle onto a global.

### 1.2 The public API surface

Today the public API is the entire transliteration:

    $ grep -c 'pub mod' crates/dash/src/lib.rs          # 35
    $ grep -rc '^pub \|^    pub ' crates/dash/src | awk -F: '{s+=$2} END{print s}'
    1129

Thirty-five public modules, roughly 1,129 public items, and no
distinction anywhere between what an embedder may touch and what is
internal. `crate::eval::exitstatus` is `pub`. So is `memalloc::stalloc`.
`shell-as-library` cannot be checked against a surface like that, because
every internal detail is already part of it.

The end state is a small named surface:

| Item | Purpose |
|---|---|
| `Shell` | The instance. Owns variables, aliases, functions, the command table, jobs, options, traps, the parse-file stack, the descriptor table. |
| `Shell::builder() -> Builder` | argv, environment, streams, options, signal policy, working directory. |
| `Shell::run(&mut self, impl Into<Source>) -> Result<ExitStatus, Error>` | Run a script from bytes, a file, or a reader. |
| `Shell::run_command(&mut self, &[&BStr])` | The `-c` shape, with positional parameters. |
| `Shell::var / set_var / unset_var / vars` | The variable table, as bytes. |
| `Shell::status() -> ExitStatus` | `$?`. |
| `Shell::expand_word(&mut self, &BStr)` | Word expansion without execution — the thing every embedder actually wants and no `Command` API gives. |
| `Streams` | Where the shell's own three streams come from. Already exists. |
| `Error`, `ExitStatus`, `Source`, `Signal` | The value types. |
| `Host` (trait) | The things a library may not do on its own authority: install a signal disposition, terminate the process. Implemented by the frontend. |

Everything else becomes `pub(crate)`. The check is mechanical: count of
`pub` items reachable from `lib.rs` under a `#![deny(missing_docs)]`.

`show.rs` (the `#ifdef DEBUG` tree printer) and `gen/` (the build-time
generators) were not on this list; section 3.2 argued they should not be
in the crate at all, and `delete-gen` removed both.

### 1.3 The error taxonomy

`errors-are-values` says "Result propagation" and names no type. This is
the type.

The exception mechanism conflates three unrelated things behind four
integers (`EXINT`, `EXERROR`, `EXEND`, `EXEXIT`, `error.rs:77-81`). They
have to come apart, because only one of them is an error:

1. **Diagnostics.** "not found", "Bad substitution", a syntax error.
   The script did something wrong. These are `Error`.
2. **Control flow.** `exit`, `return`, `break`, `continue`, the `set -e`
   abort, `EXEND`, `EXEXIT`. These are not errors and must not be in the
   `Err` position — a `Result` whose `Err` includes "the script called
   `exit 0`" makes every caller wrong by default.
3. **Interrupt.** `EXINT`. Asynchronous, signal-delivered, and the host
   must be able to tell "your script failed" from "the user hit ^C".

```rust
pub enum Error {
    Syntax   { line: u32, message: BString },
    Expansion{ line: u32, kind: ExpansionErrorKind, word: BString },
    Redirect { fd: RawFd, source: io::Error },
    NotFound { name: BString },        // status 127
    NotExecutable { name: BString },   // status 126
    Builtin  { name: &'static BStr, status: u8, message: BString },
    Interrupted(Signal),
    Nul(std::ffi::NulError),           // the CString::new edge from bytes-not-text
    Io(io::Error),
}

impl Error {
    pub fn status(&self) -> ExitStatus;              // dash's exit code, exactly
    pub fn diagnostic(&self) -> impl fmt::Display;   // dash's stderr text, exactly
}
```

`status()` and `diagnostic()` are what let [dec:nsh:errors-are-values]'s
"the frontend keeps the current behaviour" be true: the *content* is in
the value, and the frontend can render it byte-for-byte as dash does.

**A constraint on this design that the decision does not record, and
which will otherwise be discovered by 61,498 failing cases.**
`tests/harness/dscase.sh:64-71` runs each case with `2>&1` and compares
the merged stream. So the *interleaving* of the shell's diagnostics with
command output is under test everywhere. A design in which the library
returns an error and the frontend prints it will emit every diagnostic at
the end of the run instead of at the point of failure, and will fail
thousands of cases at once.

Therefore: **a diagnostic is written where dash writes it, and the error
value is returned as well.** The value is for the embedder and for
control flow; it is not the delivery mechanism for the text. Embedders
who want structure instead of bytes get `Builder::on_diagnostic(hook)`.
This is a real constraint on the shape of the refactor and it belongs on
the decision.

Control flow does not go in `Error`. It goes in the `Ok` position, or in
a private `Flow` type internal to `eval`:

```rust
pub(crate) enum Flow { Normal, Break(u32), Continue(u32), Return, Exit(ExitStatus) }
```

`Shell::run` turns `Flow::Exit` into `Ok(status)`. A `Shell` that has run
`exit` is not poisoned — that is a per-instance flag, not a process
event, and an embedder can keep using the shell or not as it chooses.

### 1.4 What `main.rs` retains

`main.rs` is 156 lines today and about 100 of them survive, because they
are the parts that only a process may do:

* The `.init_array` constructor capturing the inherited SIGPIPE
  disposition and which of fds 0/1/2 were already closed, and the code in
  `main` that undoes Rust's runtime's SIGPIPE, SIGSEGV/SIGBUS and
  `sanitize_standard_fds` behaviour. All of it is a property of the Rust
  runtime, not of the shell, and it must not move into a library.
* argv as `Vec<Vec<u8>>` via `args_os`.
* `Streams::install(Streams::INHERIT)` — the frontend is entitled to the
  process's descriptors, so it is the thing that lends them out.
* Installing the signal dispositions the shell asks for, via the `Host`
  trait ([dec:nsh:host-owns-signals]).
* `std::process::exit(status)` at the end, and `_exit` in a forked child.

Two things go:

* The panic hook that filters `Longjmp` payloads (`main.rs:113-128`).
  It exists only because the exception mechanism is an unwind. It is
  deleted by `errors-are-values`, and its deletion is a check on that
  step.
* `main_fn(argc, argv, streams) -> !`. The `-> !` is the reason no
  embedder can call the library today.

### 1.5 `no_std`

No, and the question should be closed rather than deferred.

A POSIX shell needs `fork`, `execve`, `waitpid`, `pipe`, `dup2`,
`sigaction`, `open`, and a heap. `no_std` + `alloc` gives up
`std::io::Error`, `std::os::unix`, `std::ffi::CString`, `std::process`
and `std::path` — every one of which is the correct vocabulary type for
something the shell does — and buys nothing, because libc is still
linked. Portability to a non-POSIX target is not a goal; the product is
a POSIX shell.

What *is* worth having is the weaker, checkable property that motivates
the `no_std` question:

* the library never terminates the process (`_exit`, `process::exit`,
  `abort`),
* the library never panics on input (`unwrap`, `expect`, indexing),
* the library never writes to a stream it was not given.

Those are checkable with `grep` and belong in section 1.7.

### 1.6 Crate names and crate structure

**Rename `dash` to `nsh`, and do it first.** The rename is the only step
whose cost strictly grows with delay: every commit message, module path,
doc reference and decision written between now and the rename has to be
written twice. Its cost today is bounded and known:

* `crates/dash/` → `crates/nsh/`, and `name = "dash"` → `"nsh"` in
  `Cargo.toml`. `crate::` paths are unaffected.
* `dash::` appears in exactly two files outside the crate root:
  `crates/dash/src/main.rs` and `crates/dash/tests/streams_embed.rs`.
* `target/debug/dash` is the default `PORT` in
  `tests/harness/dsdiff.sh:30` and `covrust.sh:53`, plus prose in
  `tests/README.md`.
* It is **free with respect to the oracle.** `dscase.sh:51,60` symlinks
  each shell under test to `<dir>/.bin/sh` and invokes it through that
  path precisely so `argv[0]` is identical for the two sides. Renaming
  the binary cannot change a single byte of compared output. The
  reference build (`tests/.build/ref/src/dash`) is the C and keeps its
  name.

The one scheduling constraint: renaming the crate directory invalidates
`target/debug/dash`, so it must not land during a differential sweep.

**Split the frontend into its own crate.** Today `main.rs` is a `[[bin]]`
inside the library crate, which means it can reach `crate::` internals
and nobody would ever find out. With `crates/nsh` (lib) and `crates/nsh-cli`
(bin), the binary can only use the public API, so **anything the frontend
needs that is not public is a compile error rather than an inspection
finding.** That is the mechanism that makes [dec:nsh:shell-as-library]
checkable, and there is no other one. The split costs a workspace member
and a `nsh = { path = "../nsh" }`.

**The `nshedit` dependency is a path outside the repository**
(`crates/dash/Cargo.toml`: `path = "../../../../git/libedit/crates/nshedit"`).
A crate with an out-of-tree path dependency cannot be published, cannot
be cloned and built, and cannot be built in CI. Resolving this — vendor,
submodule, workspace merge, or a published version — is a prerequisite
for calling the thing a library at all, and it appears nowhere in the
plan.

### 1.7 "Idiomatic", reduced to checkable properties

[dec:nsh:shell-as-library] reduced idiomatic to four properties. There
are now seven decisions in scope, and four is not enough — it leaves out
everything about the *surface*, which is the part an embedder sees. Here
are eleven, each with the command that decides it, today's value, and the
target.

| # | Property | Check | Today | Target |
|---|---|---|---|---|
| P1 | No ambient state | `grep -rc 'static mut' src` ; no `thread_local!` | 154 decls, 204 names | 0 |
| P2 | Errors are values | `grep -rn 'catch_unwind\|panic_any\|resume_unwind' src` ; both profiles build with `panic = "abort"` and the harness passes | 8 catch sites, 1 raise, profiles pinned to `unwind` | 0; pins removed |
| P3 | Host owns signals | `libc::signal\|sigaction` outside the `host` seam | signal calls in `trap.rs`, `jobs.rs`, `error.rs`, `main.rs` | 0 in the library |
| P4 | Host owns streams | literal `0`/`1`/`2` as an fd outside `streams.rs`; `Streams::set` passes the same suite as `install` | `install` full fidelity, `set` partial (recorded as deferred on the decision) | parity |
| P5 | Owned data | `memalloc.rs` exists; `grep -c '\*mut c_char'` | 99 memalloc sites / 22 files; 1,257 `c_char` pointer occurrences | file deleted; 0 |
| P6 | Bytes, not text | no `String`/`&str` in any signature carrying shell data; `bstr` is a dependency | `bstr` not yet a dependency | `BStr`/`BString` throughout |
| P7 | Minimal unsafe | `unsafe fn` count; `#![deny(unsafe_op_in_unsafe_fn)]`; every `unsafe` block has `// SAFETY:` | 611 of 800 `fn` are `unsafe` (76%) | budget: syscall wrappers, the signal handler, redirection's fd work. Order 30, not 611. |
| P8 | The library does not end the process | `libc::_exit\|process::exit\|libc::abort` in the library | 4 (`error.rs:217`, `trap.rs:475`, `redir.rs:393`, `shellmain.rs:287`) | 0 outside a forked child |
| P9 | Re-entrant | two `Shell`s, one thread and two, pass the suite; `testutil::lock()` deleted | one shell per process, `lock()` required | both pass, *except the locale* — see below |
| P10 | Publishable | no unpublished deps; `cargo package` succeeds; lint allows removed | `nshedit` is a git dep (was a path four directories out); `lib.rs` has 6 blanket `allow`s including `clippy::all` and `dead_code` | clean |

**P9 has a limit that cannot be refactored away.** `var.rs:103-106
changelocale` calls `putenv` and then `setlocale(LC_ALL, "")`, and the C
library's locale is process-global. One `Shell` assigning `LC_COLLATE`
changes how another sorts a glob, however cleanly the rest of the state
is separated. Two more libc globals are the same shape and are invisible
to P1 because the static lives in libc rather than here: `strtok`
(`cd.rs:218,237`) and `getopt` with `optind = 0` (`histedit.rs`). P9's
honest target is "independent except for the C library's own globals",
and the exceptions belong on [dec:nsh:no-ambient-state] as recorded
limits rather than being discovered by an embedder.
| P11 | The API is a surface, not the source | count of `pub` items reachable from `lib.rs`; `#![deny(missing_docs)]` | 35 `pub mod`, ~1,129 `pub` items | order 20 items, all documented |

P11 is the one most obviously missing from the current four, and it is
the one that makes the difference between "the crate has a lib target"
and "the crate is a library". P8, P9 and P10 are corollaries nobody has
written down. P7's target being a number rather than "small" contradicts
[dec:nsh:minimal-unsafe]'s deferred consequence, which says the count is
not worth taking yet; the count is worth taking precisely because the
property has to be checkable, and 611/800 is a fine baseline to measure
against even though it measures the transliteration.

---

## 2. The order, and what goes wrong if a step is late

### 2.1 The measurements

Three footprints, computed over 720 functions (excluding `gen/` and
`testutil.rs`) with a name-based call graph. The graph is approximate —
it resolves by identifier, so same-named functions in different modules
merge — but the sets are large enough that the shape is not sensitive to
that.

```
total functions                                 720
transitively on a raise path                    420   (58%)
transitively reach a memalloc primitive         424   (59%)
name a static                                   587   (82%)

raise-path  ∩  memalloc-reachable               420   ← raise ⊆ memalloc
raise-path  ∩  static-touching                  403   (96% of raise-path)
memalloc    ∩  static-touching                  406
all three                                       403
raise-path  \  memalloc-reachable                 0
```

The first of those is the ordering argument for `owned-data`, and it is
stronger than the argument [dec:nsh:owned-data] actually makes. The
decision says "the measurements say the data is upstream". The
measurement says something sharper: **the set of functions on a raise
path is a strict subset of the set that touches the allocator.** There is
no function anywhere in the crate that `errors-are-values` would touch
and `delete-memalloc` would not. Converting errors first rewrites 420
signatures, every one of which the allocator work then rewrites again.
Zero of them would have been spared.

Module fan-in, as distinct files naming `crate::<m>::` and total
references:

```
error      26 files / 169 refs        parser     15 / 66
output     24 / 134                   var        15 / 60
memalloc   22 /  99                   eval       13 /  52
shell      18 /  39                   input       9 /  36
options    17 / 149                   streams     9 /  22
mystring   17 /  63                   jobs        8 /  34
```

`output` (24 files, 134 refs) and `options` (17 files, 149 refs) are the
second and third most coupled modules in the crate and neither appears
anywhere in the plan. Section 3 returns to this.

Corpus reach, from `tests/.build/cov/report.txt` (LLVM instrumentation,
whole corpus), restricted to the `dash` crate:

```
functions entered   529 / 621   85.19%
regions covered              89.12%

parser.rs   100.00% fn / 99.12% reg      jobs.rs      86.49% / 75.33%
eval.rs     100.00% / 97.41%             error.rs     82.35% / 73.10%
exec.rs      96.00% / 97.08%             output.rs    81.08% / 82.42%
var.rs       90.91% / 95.15%             histedit.rs  76.47% / 76.12%
memalloc.rs  92.31% / 91.14%             mystring.rs  66.67% / 77.20%
expand.rs    89.83% / 92.82%             syntax.rs    58.33% / 54.00%
input.rs     90.91% / 84.62%             linedit.rs   26.09% / 15.11%
redir.rs    100.00% / 95.03%             system.rs    15.38% / 34.34%
```

Twelve files do not appear in the report at all, which for a binary-linked
measurement means nothing in them is reachable from `main`:
`show.rs`, `gen/mkinit.rs`, `gen/mknodes.rs`, `gen/mksignames.rs`,
`gen/mksyntax.rs`, `gen/mod.rs`, `builtins.rs`, `signames.rs`,
`streams.rs`, `bltin/mod.rs`, `lib.rs`, `testutil.rs`. (`builtins.rs`,
`signames.rs` and `bltin/mod.rs` are tables and macros with almost no
instrumentable code; `show.rs` is 442 lines and 11 functions of live
Rust that nothing calls.)

### 2.2 A principle the sequence has to obey

[dec:nsh:differential-is-the-oracle] records as deferred: "Per-function
attribution. When a differential case fails after a refactor it says the
shell changed, not which function did."

That is not a nice-to-have. It is the constraint that sets **step size**.
An oracle with no attribution can only bisect, and bisection over a
commit that changed two things at once tells you the commit, not the
thing. So:

> **Every step changes one property. A step that changes two is not
> smaller work, it is an unbisectable failure.**

This is the reason not to merge `owned-strings` with `errors-are-values`
even though they touch the same 420 functions, and the reason not to
merge `errors-are-values` with `no-ambient-state` even though they touch
the same 403. The double signature rewrite is the price of a debuggable
failure, and it is worth paying.

It is also the reason to prefer changes that are *mechanical* (the
compiler decides whether they are complete) over changes that are
*semantic* (only the harness does).

### 2.3 The sequence

Steps marked **[new]** are not in `plan/main.styx` today.

---

**0. `crate-rename` [new] — `dash` → `nsh`, and split `nsh-cli` out.**

*If it is later:* every commit message, module path, doc line and
decision written in the meantime names the wrong product, and the split
that makes P11 checkable does not exist while the whole API is being
designed — so the API is designed against a surface where every internal
is public and no mistake is caught.

*Cost:* bounded and measured (§1.6). Free with respect to the oracle.
Only scheduling constraint: not during a sweep.

---

**1. `sanctioned-divergences` — teach `dsdiff.sh` the register.**

Currently in the plan with **no dependencies at all**, which means it is
unsequenced. It should be a dependency of everything that follows.

*If it is later:* every step from here on can produce a divergence that
is *forced* rather than chosen. `owned-nodes` already hit one — `list()`
never writes `linno` on the `NBACKGND` wrapper, and reading uninitialised
memory is not a behaviour a safe language can reproduce, so there was no
bug-for-bug option to take. That one happened to be unobserved by the
corpus. The next will not be: `delete-memalloc` touches 424 functions in
code the corpus covers at 89-99%, and `no-ambient-state` touches 587.
When a forced divergence lands in covered code and the mechanism does not
exist, the step stalls until it is built — or FAIL=0 is spent, which is
worse.

*Design, since the plan does not say:* `dsdiff.sh` case identity today is
`basename` of a `%06d` position index (`dscase.sh:13`), which changes
whenever a corpus is edited. A register keyed on that is unusable. Key
instead on `(corpus basename, sha256 of the normalised case body)`, and
require the entry to record **both sides' expected normalised output**,
not merely "this one is allowed to differ" — so a divergence that
*changes* is still a FAIL. The POSIX harness already has exactly this
shape and it should be copied rather than reinvented:
`posix/harness/dispositions.json` plus `dispositions.d/*.json`, per-rule,
mandatory `reason`, validated at load (`catalog.py:81-96`).

*Report:* `PASS=n FAIL=0 SANCTIONED=k FLAKY=j`. FAIL=0 stays the legible
number; `SANCTIONED` is a second one that should only ever be inspected
deliberately.

---

**2. `delete-gen` [new] — decide whether `gen/` belongs in the crate.**

2,006 lines across four modules, not linked into the binary, zero corpus
coverage, and `nodes.rs` stopped being `mknodes`'s output when
`owned-nodes` landed (`gen/mod.rs` says so).

*Hypothesis to test, not a conclusion:* in a Rust-first world the
generators are not needed at all. `syntax.rs` and `signames.rs` are
tables that a `const fn` or a `build.rs` can produce directly, and the
reason dash generates them — C has no compile-time evaluation — does not
apply. What the generators *are* still good for is being an oracle for
the tables (commit `77822d2` records `mksyntax` emitting the C
generator's files exactly, and `73bf36d` the same for `mksignames`), and
that is a test-time need, not a library need.

*Recommendation:* move `gen/` to `crates/nsh/tests/` or to a
`nsh-tablegen` dev-only crate, keeping the parity tests. Do not delete
the parity property; delete the presence in the shipped library.

*If it is later:* it is 2,006 lines and 5 modules that every subsequent
mechanical refactor has to carry through, for code that does not run.
`gen/mkinit.rs` alone has 14 `unsafe fn` and 7 `static mut` that count
against P1 and P7 and mean nothing.

*What was done, and where it departs from the recommendation:* `gen/` was
deleted rather than moved, because the parity property it carried is not
the one the recommendation assumes. The generators emit **C**, not Rust —
`mksyntax::main_fn` writes `syntax.c`/`syntax.h` and `mksignames::main_fn`
writes `signames.c` — so `77822d2` and `73bf36d` assert that the Rust port
of the *generator* agrees with the C generator. Neither ever touched
`crates/nsh/src/syntax.rs` or `signames.rs`, which are hand transcriptions
of the C generator's output, so the generators were never the authority
for the checked-in tables and deleting them loses no ability to regenerate
anything. The property worth keeping is the one nothing asserted: that the
tables the shell indexes *are* that output. It is now asserted directly,
in `syntax.rs::tests::the_tables_are_the_c_generators_output` and
`signames.rs::tests::the_table_is_the_c_generators_output`, against the
`syntax.c` and `signames.c` the reference build generates — 155 lines in
place of 2,019, checking the artefact the shell uses instead of a second
implementation of the generator. The C reference is unaffected:
`src/Makefile.am:33,55-67` builds the four `src/mk*.c` helpers with
`COMPILE_FOR_BUILD` and they are in `CLEANFILES`, so they are host
programs that never link into `dash`.

`show.rs` and the twenty uncalled `system.rs` wrappers went with it, on
one shared criterion: **none of the 74 symbols contributes an instruction
to `tests/.build/ref/src/dash`.** `show.c` is entirely inside
`#ifdef DEBUG` and the reference `config.h` does not define it; the
`system.c`/`system.h` fallbacks are `#ifndef HAVE_…` arms whose `HAVE_*`
that same `config.h` defines. A symbol absent from the reference binary
has no oracle and never had one, which is what makes the deletion R0 and
what justifies retiring its rules rather than leaving them claimed.

---

**3. `owned-strings` — the stack string builder becomes `Vec<u8>`/`BString`.**

Already in the plan, correctly placed after `owned-nodes`. Two additions:

* This is where `bstr` enters and therefore where
  [dec:nsh:bytes-not-text] is actually implemented. The decision exists;
  no WBS node references it.
* The 116 remaining libc string calls (`strlen` 28, `strcmp` 27,
  `strchr` 18, `memcpy` 10, `strcpy` 9, `strspn` 5, `memmove` 5,
  `memset` 4, `strncmp` 3, `strcspn` 3, `strstr` 2, `strtod` 1,
  `strdup` 1) go here, transitively, exactly as the decision predicts.

*If it is later:* `errors-are-values` and `no-ambient-state` would run
over 420 and 587 signatures that still say `*mut c_char`, and then
`owned-strings` rewrites the same signatures a third time.

---

**4. `delete-memalloc` — the last region allocations, and the file goes.**

Already in the plan. It has one tenant the plan does not name:
`exec.rs`'s command hash table, `tblentry`, is still a `ckmalloc`'d C
struct with a flexible array member
(`exec.rs:803: ckmalloc(size_of::<tblentry>() - ARB + strlen(name) + 1)`),
and it stores `Rc::into_raw` of the function node
([dec:nsh:owned-data] says so in prose). `var.rs`'s `vartab: [*mut var;
VTABSIZE]` and `varinit: [var; 16]` are the same shape. Both are hash
tables, and neither appears in the WBS.

**Correction, from `docs/std-replacements.md` §4.1 and verified here: they
do NOT want to be `HashMap<BString, _>`, and this was the most dangerous
sentence in this document.** dash's bucket walk order is observable
output. `var.rs:640-675 listvars` walks the 39 buckets and the result *is*
`execve`'s `envp`, so it is what `env`, `export -p` and a bare `set`
print. Measured against the C and the port together:

```
env -i sh -c 'export AA=1 BB=2 CC=3 DD=4 EE=5 FF=6; env'
  -> AA FF DD BB PWD EE CC        both shells, byte-identical
sh -c 'alias bb=1 zz=2 mm=3 aa=4; alias'
  -> bb zz mm aa
```

Neither sorted nor insertion order. It is a hash table with a fixed,
weak, seed-free hash --

```c
hashval = (*p << 4);  while (*p) { hashval += *p; if (p[1] == '=') break; }
```

first byte shifted left four, plus the sum of the bytes, modulo 39
chained buckets. There is nothing deep here to preserve reverently.

What that means for the refactor is narrower than "do not use a map".
`std::HashMap` with a fixed hasher is deterministic -- the randomisation
is `RandomState`, not hashing -- but it still will not reproduce *this*
order, because hashbrown uses power-of-two capacity and open addressing,
so 39 chained buckets iterate differently whatever the hash. The Rust
equivalent is roughly forty lines: `Vec<Vec<(BString, V)>>`, 39 buckets,
`hashval` carried over. Write those rather than reaching for `HashMap`
and hoping.

**Decided: the tables become `BTreeMap` and the order becomes sorted.**
POSIX specifies neither `env`'s output order nor `export -p`'s, so this
is unspecified behaviour that the differential harness happens to pin.
Preserving it is a harness constraint, not a correctness one, and
reproducing a weak hash's bucket walk forever to keep a number green is
the tail wagging the dog. `export -p | sort` is what everyone types
anyway.

It is a category-3 divergence under `docs/divergences.md` and the corpus
does observe it -- ten files run a bare `env`, ten run `export -p` or a
bare `set`, ten run a bare `alias`. So it cannot land before
`sanctioned-divergences`, which is the head of the critical path in any
case. That is a good pairing rather than an obstacle: a mechanism built
with no customer is usually the wrong mechanism, and this one arrives
with thirty cases and three built-ins to be right about.

*If it is later:* everything downstream keeps working through raw
pointers into a region, so `no-ambient-state` would be moving a *pointer
into an allocator* onto an instance, which is not a move at all.

---

**5. `output-is-a-writer` [new].**

`output.rs` is 1,066 lines, is referenced from 24 files and 134 sites —
second only to `error` — and holds four of the process globals
(`output`, `errout`, `preverrout`, and the `out1`/`out2` pointers at
`output.rs:54-84`). It contains a hand-rolled `printf` (`VaArg`,
`doformat`, `fmtstr`, `xasprintf`) with a macro layer on top, and
`bltin/mod.rs` remaps BSD stdio names onto it for the imported builtins.

*Why it is its own step:* `host-owns-streams` moved *which descriptors*
the shell uses. It did not move *what writes to them*. The deferred
consequence on that decision — that under `Streams::set` the shell's own
writes follow but the language's descriptor numbers do not — is a
consequence of the buffers being global, not of the fds being global.
Making `output` a per-instance writer is the other half of
`host-owns-streams`, and it is prerequisite to P4 parity.

*If it is later:* it is 134 call sites and four statics, and it is on the
critical path of `no-ambient-state` anyway. Doing it inside
`no-ambient-state` makes that step bigger and less bisectable, which
violates §2.2.

---

**6. `builtins-take-args` [new].**

Forty builtins behind `type BuiltinFn = unsafe fn(c_int, *mut *mut c_char)
-> c_int` (`builtins.rs:34`), dispatched through a table whose order is
load-bearing for a binary search in `exec.c:find_builtin`. Two of them
(`bltin/printf.rs` at 829 lines, `bltin/test.rs` at 669) are ports of
standalone BSD utilities carrying their own macro shim.

The end state is `fn(&mut Shell, &[&BStr]) -> Result<ExitStatus, Error>`,
and the sorted table becomes a `match` or a `phf`-style static map.

*If it is later:* the builtins are the largest single block of
`*mut *mut c_char` left after `owned-strings`, and they are on the raise
path (10 of the 34 `sh_error!` sites are in builtins) and touch statics.
Every one of the three remaining large steps would run over them.

*If it is earlier than 3:* the argument vector is still region-allocated,
so the signature cannot be `&[&BStr]` yet.

---

**7. `errors-are-values`.**

Already in the plan, correctly placed after `delete-memalloc`. What
`delete-memalloc` buys it is visible directly in the top-level handler at
`shellmain.rs:205-240`: on catching an exception the code runs
`init::exitreset()`, `init::reset()`, `popstackmark(smark_p)` and
`FORCEINTON()` — four pieces of manual cleanup that exist because
`longjmp` skips destructors. Owned data does that cleanup itself, so the
error path has less to do, which is the decision's own argument made
concrete.

Also collected here: the 170 `INTOFF`/`INTON`/`FORCEINTON` sites across
17 files. `suppressint` and `intpending` live in `error.rs` but they are
signal state, and the macro pair exists to make allocator and table
mutations atomic against SIGINT. Once the allocations are owned and the
tables are Rust collections, most of the 170 have nothing left to
protect. What survives is real and belongs with step 9.

*Incremental, not big-bang.* A function converted to
`-> Result<T, Error>` can still be called from an unconverted raiser via
`f(..).unwrap_or_else(|e| raise(e))`, so the wavefront moves from the 34
raise sites outward to the 8 catch sites
(`trap.rs:446,470`, `parser.rs:2258`, `redir.rs:507`, `eval.rs:1146,1193`,
`histedit.rs:764`, `shellmain.rs:128`) with a green harness at every
commit. The step is complete when `error::Longjmp`, `raise_longjmp`,
`setjmp_catch`, the `jmp_buf` stand-in and the panic hook in `main.rs`
are all deleted and the `panic = "unwind"` pins come off both profiles.

*If it is later than `no-ambient-state`:* `error::handler` is a
`*mut jmploc` pointing into a live stack frame (`error.rs:86`), and a
pointer into a frame cannot be a field of a `Shell`. So the exception
mechanism physically blocks moving its own state onto the instance. And
`catch_unwind` over a closure capturing `&mut Shell`, followed by
continued use of that `Shell`, is the exact hazard `UnwindSafe` exists to
flag — the code would have to write `AssertUnwindSafe` around an
instance it then keeps using, eight times.

**The plan's relative order of `errors-are-values` before
`no-ambient-state` is right, and this is the argument for it.** It has
not been written down anywhere.

---

**8. `no-ambient-state`.**

The largest step: 154 `static mut` declarations, 204 distinct static
names, 587 of 720 functions naming one.

*Do it in two sub-steps, and the plan should say so.* Threading a context
parameter and moving state into it are different kinds of change, and
merging them makes the largest step in the project unbisectable:

* **8a. Thread `&mut Shell` through, with `Shell` empty.** Purely
  mechanical; the compiler decides completeness; the harness should not
  move by a byte. This is the 587-signature rewrite, done once, with
  nothing semantic in it.
* **8b. Move statics into `Shell`, one table at a time.** Each move is
  local, reviewable and independently bisectable: `vartab` and
  `localvar_stack` (`var.rs:73,297`), `cmdtable` (`exec.rs:91`),
  `optlist`/`shellparam` (`options.rs`), the trap table, the job table,
  `parsefile` and the input stack, `out1`/`out2`. No signature moves.

This also unblocks the deferred consequence on
[dec:nsh:host-owns-streams]: the per-instance descriptor table that makes
`Streams::set` and `Streams::install` agree.

*If it is earlier than 7:* see step 7.
*If it is later than 9:* a signal handler needs to find the shell it
belongs to, and with the state in globals there is exactly one, so the
handler design would be built against an assumption that is about to be
deleted.

---

**9. `host-owns-signals`.**

Correctly last in the plan. Three reasons, only one of which the plan
gives:

1. It needs `no-ambient-state`, because a per-instance shell needs a
   per-instance route from the handler back to itself. The sibling crate
   already shows the shape: `nshedit-plat/src/signal.rs` has
   `set_signal_slot(slot: *const AtomicI32)` and `set_signal_ops`.
2. **It has the weakest oracle of any step.** The differential corpus
   does not test signal dispositions — `tests/README.md` records that it
   actively *hid* one for the length of the port, because the harness
   imposed its own signal state on both shells and the SIGPIPE
   divergence (~99,930 spurious `I/O error` lines) showed up only as a
   count difference that read as a scheduling flake. The fix
   (`env --default-signal`) means the harness no longer lies, but it
   still only samples one configuration. The real net is 31 pty cases,
   against 61,498 batch cases for everything else, over `jobs.rs` at
   70.97% line coverage — the thinnest coverage of any large module.
3. Signal dispositions are process-global and inherited across fork
   *and* exec, so a mistake escapes into every child. That is not a
   property any other step has.

---

**10. `public-api` [new] — construct the surface.**

Everything above is subtraction. Nothing in the plan constructs the thing
the project is for. This step is `Shell`, `Builder`, `Source`, `Host`,
`Error` as public types; `pub(crate)` on everything else; docs; and the
API-level test suite that replaces what the differential harness cannot
reach (§4).

*Design early, implement late.* The *shape* of `Shell` determines what
`no-ambient-state` builds, so it must be settled before step 8, or step 8
will produce a `Shell` that is a bag of 204 fields and the API step will
rewrite it. Add a design node early with the implementation node here.

---

**11. `posix-nonconformance` — the 69 `fix-*` nodes.**

These are in the plan and have **no ordering relationship to
`shell-as-library` at all.** They should all be sequenced after it. Each
one is a deliberate behaviour change; each one spends some of the
oracle's authority; and the structural work needs that authority at full
strength. Two of them are already observed by the POSIX suite
(`edit-history-goto-number`, `edit-history-search-pattern-anchored`,
recorded in `docs/divergences.md` as undecided).

---

### 2.4 Summary of the change to the plan's order

Current (`plan/main.styx:1865-1952`):

    owned-nodes → owned-strings → delete-memalloc
                → errors-are-values → no-ambient-state → host-owns-signals
    sanctioned-divergences (unsequenced)
    host-owns-streams (done)

Proposed:

    crate-rename ─┐
    sanctioned-divergences ─┬→ owned-strings → delete-memalloc
    delete-gen ─┘           │        ↑
                            │   owned-nodes (done)
                            │
        → output-is-a-writer → builtins-take-args
        → errors-are-values
        → no-ambient-state (8a threading, 8b moves)
        → host-owns-signals
        → public-api  [design node sequenced before no-ambient-state]
        → posix-nonconformance (all 69)

The spine — nodes, strings, delete, errors, state, signals — is
unchanged, and the argument in §2.1 says it is right. What is added is
everything the spine does not cover.

---

## 3. What the plan does not contain

`mcp__nplan__nplan_tree` returns 105 nodes. Sixty-nine are `fix-*`
behaviour changes, twenty-six are POSIX chapters, and ten are under
`shell-as-library`. Those ten are: `host-owns-streams` (done),
`errors-are-values`, `no-ambient-state`, `host-owns-signals`,
`owned-data` and its three children, and `sanctioned-divergences`.

Checked against the hypotheses in the brief:

**3.1 The builtins — confirmed missing.** Forty entries, an
`unsafe fn(c_int, *mut *mut c_char) -> c_int` signature, 1,761 lines
across `bltin/`, plus `miscbltin.rs` (661) and the builtin halves of
`var.rs`, `cd.rs`, `alias.rs`, `jobs.rs`, `exec.rs`, `trap.rs`. Section
2.3 step 6.

**3.2 `gen/` — confirmed missing, and the doubt is justified.** 2,006
lines, not linked into the binary, zero coverage, and one of the four is
already not the source of the thing it generates. Section 2.3 step 2,
which now records what `delete-gen` did with it.

**3.3 `var.rs` and the variable table — confirmed missing.**
`vartab: [*mut var; VTABSIZE]` (`var.rs:297`), `varinit: [var; 16]`
(`var.rs:109`), the `localvar`/`localvar_list` stack (`var.rs:58-73`),
9 `static mut`, 56 `unsafe fn`, 15 files depending on it. It is covered
implicitly by `no-ambient-state` and `delete-memalloc` and named by
neither.

**3.4 `exec.rs`'s command table — confirmed missing from the WBS,
present in the AKM.** [dec:nsh:owned-data] says "the `Rc` is stored in
`exec::tblentry`, which is still a `ckmalloc`'d C struct with a flexible
array member … it goes when `memalloc` does." No node says so.

**3.5 Job control and the process model — partly missing.**
`host-owns-signals` names job control in passing ("needs pty cases for
job control first"). Nothing covers the process model itself:
`jobs.rs` is 2,055 lines with 10 `static mut`, 39 `unsafe fn`, 6 of the 9
fork/exec/wait sites, and the `vforked` flag that `error.rs:216` special-
cases on the raise path. `forkshell`/`forkparent`/`forkchild` are where a
library differs most sharply from a program: a forked child of an
embedded shell inherits the host's entire address space, and what it may
do before `execve` is a real design question with a real answer
(`_exit`, never `exit`; no destructors; no allocator). None of that is
written down. `jobs.rs` also has the lowest coverage of any large module
(75.33% regions, 70.97% lines).

**3.6 The public error taxonomy — confirmed missing.** Section 1.3.

**3.7 Crate/module structure — confirmed missing.** Section 1.6: rename,
frontend split, and the out-of-repo `nshedit` path dependency.

Three the brief did not list:

**3.8 `output.rs`.** Second-highest fan-in in the crate (24 files, 134
refs), 1,066 lines, four process globals, a hand-rolled variadic printf,
and the other half of `host-owns-streams`. Section 2.3 step 5.

**3.9 `options.rs`.** Third-highest fan-in (17 files, 149 refs).
`optlist[iflag]`, `optlist[xflag]` and friends are read from everywhere
as ambient booleans, and `shellparam` holds the positional parameters as
`*mut *mut c_char`. Every option read is a `Shell` field access in the
end state. It is inside `no-ambient-state`'s scope and is large enough to
name.

**3.10 The public API itself.** The plan's ten nodes under
`shell-as-library` are all removals. Nothing builds `Shell`. Section 2.3
step 10.

---

## 4. When the oracle stops working

### 4.1 It already has, and the project already knows

The best evidence is in a decision that has already landed.
[dec:nsh:host-owns-streams] lists three places where the C's `0`, `1` and
`2` were load-bearing in a way a rename would miss — `forkreset`'s
`fd > 0`, `preadfd`'s `fd == 0`, `forkparent`'s reliance on `open`
returning the lowest free descriptor — and then says:

> Each is invisible under the default streams, which is exactly why they
> are worth naming: the differential harness cannot catch them, because
> it only ever runs the identity case.

That is the whole answer, generalised. **The differential harness tests
one configuration: `Streams::INHERIT`, one shell per process, signals
claimed by the shell, argv from the process, exit by `_exit`.** Every one
of the library properties adds a configuration axis, and the harness
samples exactly the point on that axis where nsh must equal dash. So the
oracle's coverage of the new surface is zero, by construction, from the
moment the axis exists — not at some later step.

The failure mode is therefore not "the oracle dies at step N". It is:
**each step that adds a degree of freedom creates a blind spot the same
day, and the blind spot is exactly the thing the step was for.**

### 4.2 What the oracle does keep doing, all the way to the end

It guards the shell *language*, and it is extremely good at that.
`parser.rs` at 100% of functions and 99.12% of regions, `eval.rs` at 100%
and 97.41%, `redir.rs` at 100% and 95.03%, `exec.rs` at 96% and 97.08%,
`expand.rs` at 89.83% and 92.82% — against 61,498 cases comparing merged
stdout and stderr byte for byte. Nothing else the project could build
would be as good a net under `owned-strings`, `delete-memalloc`,
`errors-are-values` or `no-ambient-state`, all of which are refactors
that must not change one byte of observable behaviour.

So the oracle survives to the end **for the identity configuration**, and
that is the right thing for it to do. The conclusion is not "replace it".
It is: the differential harness's contract becomes *"the frontend, in the
configuration dash runs in, is dash"*, and that contract is worth keeping
forever. It attaches to the **frontend crate**, not to the library —
which is another argument for the split in §1.6.

### 4.3 What has to replace it, and when

For each axis, the replacement and the step it must exist by:

| Axis | Harness coverage | Replacement | Needed by |
|---|---|---|---|
| Supplied streams | zero (identity only) | `crates/nsh/tests/streams_embed.rs` — exists, 4 tests, and currently pins a *limitation* rather than a capability | already overdue |
| Two shells in one process | zero | API tests: two `Shell`s, same thread and two threads, interleaved | step 8 |
| Errors as values | zero | API tests asserting the `Error` variant and `status()` for each raise site; plus the `panic = "abort"` build | step 7 |
| Host-owned signals | 31 pty cases | pty cases for job control (the plan says so); plus `/proc/self/status` `SigCgt`/`SigIgn` assertions across fork and exec, which is how the SIGPIPE bug was found | step 9 |
| Per-instance fd table | zero | API tests: `echo hi >file` under `Streams::set` must reach the supplied stream | step 8 |
| Unreached code | `system.rs` 15.38% of functions, `linedit.rs` 26.09%, `mystring.rs` 66.67%, `syntax.rs` 58.33%, `show.rs` 0% | unit tests, or deletion | continuous |

The last row is the one that will be skipped and should not be.
[dec:nsh:differential-is-the-oracle] rejected the per-function unit suite
for good reasons, and those reasons apply to code the corpus *reaches*.
For code it does not reach, there is no oracle at all and the rejected
alternative is the only one available. 92 functions in the crate are in
that state today.

### 4.4 The points of no return, and when to sequence them

Three, in increasing severity:

1. **The first sanctioned divergence a corpus case observes.** FAIL=0
   never comes back; from then on the number is `FAIL=0 SANCTIONED=k` and
   somebody has to trust `k`. Mitigated, not avoided, by the mechanism in
   §2.3 step 1 recording both sides' expected output so a *changed*
   divergence still fails. Sequence: the mechanism now, the first
   divergence whenever it is forced.

2. **`host-owns-signals`.** After it, the shell no longer installs its
   own dispositions, and whether the frontend reproduces dash's exactly
   is decided by 31 pty cases and by nothing else. This is correctly last
   in the plan and the reason should be recorded as *oracle weakness*,
   not only as *dependency*.

3. **`posix-nonconformance`, all 69 nodes.** Each is a deliberate
   divergence from dash. Run in bulk they convert the differential
   harness from an identity check into a diff against a register of 69
   entries, at which point its authority is the register's authority.
   **These must be sequenced entirely after `shell-as-library`**, and
   today nothing in the plan says so.

There is a fourth candidate that is *not* a point of no return, and
saying so is useful: the crate rename. `dscase.sh` invokes both shells
through the same `.bin/sh` symlink specifically so `argv[0]` is identical
by construction, so renaming the port's binary cannot change a compared
byte. It is free. Do it first.

---

## 5. Risk register

### Reversible, incremental, green harness at every commit

| Step | Why it is safe |
|---|---|
| `crate-rename` | Pure rename; `git revert` restores it; provably invisible to the oracle. |
| `delete-gen` | Deletes code that is not linked and has zero coverage. |
| `sanctioned-divergences` | Harness-only. Cannot change the shell. |
| `owned-strings` | Module by module; parser 99.12% and expand 92.82% region-covered under it. |
| `output-is-a-writer` | 134 call sites, mechanical; output at 82.42%. |
| `builtins-take-args` | 40 entries, one table, one signature; the builtin corpora are large. |
| `no-ambient-state` **8a** (threading) | Mechanical; the compiler decides completeness; the harness must not move. |
| `no-ambient-state` **8b** (moves) | One table per commit; each independently bisectable. |
| `errors-are-values` | Incremental via the raise adapter (§2.3 step 7), leaves to catch sites. |

### Not incrementally landable

* **`delete-memalloc`'s final commit.** The moment `stackblock()` stops
  being a live region, everything still reading it breaks at once. The
  plan already mitigates this correctly by making it last, after
  `owned-nodes` and `owned-strings` have emptied the region. The residual
  is whatever `owned-strings` missed.
* **`public-api`'s `pub` → `pub(crate)` sweep.** 1,129 items; either the
  crate compiles or it does not. Reversible, but not partial. Do it in
  the frontend-split commit so the compiler enumerates the holes.

### The two that are genuinely dangerous

**A. `delete-memalloc`, specifically `expand.rs`.**

`expand.rs` is the largest module (3,022 lines), holds 28 of the 99
remaining `memalloc::` sites, has 59 `unsafe fn`, and is where the
shell's semantics live. The hazard is not the allocation — it is that
dash keeps *offsets and pointers into the region across calls that can
move it*: `expand.rs:794` and `:831` call `pushstackmark` with an
explicit length (`endoff`, `startloc`) precisely because the region may
have grown underneath. Converting that to a `Vec<u8>` is not mechanical.
An index into a `Vec` is safe where a pointer into a region was not, but
every place that re-derived a pointer after a growth encodes an
invariant, and reproducing it as an index requires reading the invariant
rather than the code.

The corpus covers `expand.rs` at 92.82% of regions, so roughly 7% of the
module — 154 regions — is unguarded, and expansion is the part of a shell
whose bugs are silent value corruption rather than crashes.

*Mitigation:* convert `expand.rs` alone, in its own commit, after every
other consumer of the region is done, and hand-audit every site that
holds an offset across a call. Consider unit tests for those specific
functions, notwithstanding [dec:nsh:differential-is-the-oracle] — the
decision rejected a *complete* per-function suite, not a targeted one.

**B. `host-owns-signals`.**

Signal dispositions are process-global, inherited across fork *and*
exec, and this project has already been bitten by exactly that, twice:
Rust's runtime setting SIGPIPE to `SIG_IGN` produced ~99,930 spurious
`I/O error` lines and was invisible for the length of the port because
the harness imposed its own signal state on both sides; and the harness
itself once "passed" 6,964 cases without executing one. The oracle here
is 31 pty cases over the module with the thinnest coverage in the crate.

*Mitigation:* the pty corpus for job control that the plan already names
as a prerequisite, plus direct assertions on `/proc/self/status`
(`SigCgt`, `SigIgn`, `SigBlk`) taken in the shell and in a forked child
and in an exec'd child, compared against the C. That is how the SIGPIPE
bug was eventually found (`main.rs` says so), and it is a stronger
instrument than behavioural comparison because it observes the state
directly instead of a consequence of it.

---

## 6. Proposed changes to `plan/`

Not applied. Listed so they can be reviewed as a set.

**WBS (`plan/main.styx`, under `shell-as-library`):**

* Add `crate-rename` (rename to `nsh`, split `nsh-cli`), no deps.
* Give `sanctioned-divergences` no deps but make `owned-strings`,
  `errors-are-values` and `no-ambient-state` depend on it.
* Add `delete-gen`, no deps.
* Add `output-is-a-writer`, deps `delete-memalloc`.
* Add `builtins-take-args`, deps `owned-strings`.
* Split `no-ambient-state` into `thread-context` and `move-state`.
* Add `public-api-design` (deps: none; must precede `no-ambient-state`)
  and `public-api` (deps: `no-ambient-state`, `host-owns-signals`).
* Add `process-model` covering fork/exec/wait in a library, deps
  `no-ambient-state`.
* Add `nshedit-in-tree` — resolve the out-of-repo path dependency.
* Make `posix-nonconformance` depend on `shell-as-library`.

**AKM (`plan/decisions/`):**

* `[dec:nsh:errors-are-values]` — record the stderr-interleaving
  constraint from §1.3. A design that defers diagnostics to the frontend
  fails thousands of corpus cases, and that fact should be in the
  decision rather than discovered.
* `[dec:nsh:errors-are-values]` — record the taxonomy split
  (diagnostic / control flow / interrupt) and that control flow is not
  `Err`.
* `[dec:nsh:differential-is-the-oracle]` — promote the deferred
  "per-function attribution" consequence to a constraint on step size
  (§2.2), and record that the harness tests one configuration and
  therefore has zero coverage of every axis idiomatization adds (§4.1).
* `[dec:nsh:minimal-unsafe]` — the deferred audit says the count is not
  worth taking. Take it as a baseline (611 of 800) and state a target,
  because P7 is otherwise not checkable.
* `[dec:nsh:shell-as-library]` — the four properties are necessary and
  not sufficient. Add the surface property (P11) and the corollaries
  (P8, P9, P10) from §1.7.
* New decision: the public API shape and the crate split, since
  "one crate or two" is an existence question with a compiler-enforced
  consequence.

**`plan/architecture.styx`:** if the frontend splits, `[arch:nsh:shell-bin]`
exposes `crates/nsh-cli/src/main.rs`, and `[arch:nsh:conformance]` should
record that the differential harness's contract attaches to the frontend.

---

## 7. What this document is not sure about

Stated rather than smoothed over.

1. **The call-graph numbers are name-based.** `raise ⊆ memalloc` (420 of
   420) is the strongest claim here and it rests on a resolver that
   merges same-named functions across modules. The direction of the
   result is not in doubt — the two sets are 420 and 424 out of 720 — but
   the exact subset relation could be an artefact. *Resolved by:* a real
   call graph from `cargo call-stack` or from rustc's MIR, or by
   spot-checking the 4 functions in `memalloc \ raise`.

2. ~~**Whether `gen/` should be deleted, moved, or kept.**~~ *Resolved:*
   deleted. The generators emit C, not Rust, so they were never the
   authority for `syntax.rs` or `signames.rs`; those tables are
   transcriptions of the C generator's output, and their provenance is
   now asserted directly against the reference build's `syntax.c` and
   `signames.c`. Section 2.3 step 2 carries the full argument.

3. **Whether `output-is-a-writer` is really separable from
   `no-ambient-state`.** The argument is fan-in (24 files) and the
   deferred consequence on `host-owns-streams`. The counter-argument is
   that four statics is small and doing it separately means touching 134
   sites twice. *Resolved by:* trying the first ten call sites and seeing
   whether the change is local.

4. **Whether the `Error` taxonomy in §1.3 survives contact with
   `set -e`.** `set -e` decides whether an error terminates based on
   syntactic context (a command in a `while` condition, a `!`, a `&&`
   left operand), and dash implements that with `EV_TESTED` flags through
   `evaltree`. Whether that stays a flag on the call or becomes a
   property of the `Error` is not settled here.

5. **Whether `Streams::set` parity is achievable at all** without an
   fd-remapping layer that intercepts the language's descriptor numbers.
   [dec:nsh:host-owns-streams] says a per-instance descriptor table
   suffices. That is plausible for the shell's own bookkeeping and it is
   not obvious for an external command, which inherits real descriptors
   and cannot be lied to. P4's target may have to be weakened to
   "`install` is full fidelity; `set` is full fidelity for everything
   except external commands", and that would be a divergence worth
   registering. *Resolved by:* writing the test before writing the
   feature.

6. **What `Shell::run` does about the parse-file stack when it is called
   twice.** dash's input stack is global and `-` reads. Two `run` calls
   on one `Shell` should compose like two lines of a script, and one
   `run` inside a `Host` callback should not. This is a semantics
   question the API design has to answer and this document does not.
