# What the standard library already does, and what dash meant instead

Status: proposal. Nothing here is applied. It is supplementary to
`docs/idiomatization.md`: that document says what the end state is and in
what order to reach it; this one says, for each step it names, **what can
be deleted while you are already in the file**.

Nothing here is a step of its own. Every entry attaches to a node in
`nplan_tree` under `shell-as-library`, and the reason is economic: a
deletion that rides along with a step already planned costs one commit's
review, and the identical deletion as its own project costs a plan node, a
differential sweep and a bisect. Two entries do not attach anywhere, and
§7 proposes a node for them.

Every number was measured on `crates/nsh/src` — 27,312 lines across 37
modules — after the `crates/dash` → `crates/nsh` rename. Commands are
given so the claims can rot visibly. Behavioural claims were measured
against `tests/.build/ref/src/dash` and, where Rust semantics are the
question, against an `rustc`-compiled probe outside the build tree.

---

## 1. The lens: three reasons a reimplementation exists

The code is C89. dash's first commit is 1997; the Almquist/Berkeley
material it derives from is 1991-1993. The useful question is therefore
not "what did C lack" but **"what could portable C not assume in 1997,
and where is dash instead doing something deliberate"** — because the
answer decides the risk, and it decides it before any code is read.

### (a) Portability workaround — nothing to preserve

The facility existed; dash could not assume it. These sit behind
`#ifndef HAVE_…` in `src/system.h` and `src/system.c`, and the port
compiled all of them unconditionally because the port rules treat a
`#ifndef` arm as "a contract the target must satisfy"
(`crates/nsh/src/system.rs:1-14`).

`snprintf` is the headline case, and the brief guessed the other way, so
it is worth stating precisely: **the C dash does call `vsnprintf`** —
`src/output.c:426`, inside `xvsnprintf`, whose only non-portable content
is a `#ifdef __sun` comment about Solaris returning `-1` for a zero
length. dash did not hand-roll a formatter. `fmtstr`, `doformat`,
`xvasprintf` and `xasprintf` are buffer management *around* libc's
formatter.

Also here: `mempcpy`, `stpcpy`, `strchrnul`, `memrchr`, `strsignal`,
`strtoimax`/`strtoumax`, `bsearch`, `killpg`, `sysconf`, `tee`,
`memfd_create`, `fnmatch`, `glob64` — every one a `#ifndef HAVE_…`
fallback. And `SHELL_ALIGN` (`src/machdep.h:41-45`), which is `align_of`
spelled as `sizeof(union {int i; char *cp; double d;})` because C89 had
no `alignof`.

**Deleting these preserves nothing, because there is nothing there to
preserve.** They are the safest subtractions in the crate.

### (b) Port artefact — the C is not the oracle for it

The C uses the library facility; the *Rust* had to reimplement it because
Rust cannot express the C construct. There is one large member:
`output.rs`'s `c_vsnprintf` (`output.rs:194-298`) and the `VaArg` enum
standing in for `va_list` (`output.rs:96-113`). The module comment says
so itself (`output.rs:15-27`).

This code has **no counterpart in dash at all.** Its correctness is not
underwritten by the differential harness having compared it against
anything — only by it happening to agree, on the conversions the corpus
exercises, with the libc formatter it re-derives. Deleting it removes
risk rather than adding it.

A second, smaller member: `error.rs:44-50` is an `extern "C"` block
containing nothing but a comment — the `setjmp` declaration it once held
was removed when `setjmp_catch` became a `catch_unwind`. The `jmp_buf`
stand-in above it (`error.rs:41-42`, `[u8; 512]` at 16-byte alignment) is
*not* dead: it is a field of every `jmploc` (`:66,74`). It is dead
*weight* — 512 bytes per handler that nothing ever reads — and it goes
with `errors-are-values`, not before.

### (c) Deliberate — the traps

dash is doing something the obvious library call does not do. Each row
is measured in §5.

| dash | the obvious swap | why it is not |
|---|---|---|
| `var.rs:297`, `alias.rs:28`, `exec.rs:91` hash tables | `HashMap<BString, V>` | bucket order reaches `environ`, `alias` and `hash` output |
| `expand.rs:2479 msort` | `Vec::sort()` | `strcoll`, and the shell calls `setlocale(LC_ALL, "")` |
| `cd.rs:184 updatepwd` | `fs::canonicalize` / `Path` | logical (`-L`), textual, no filesystem access, `//` preserved |
| `expand.rs:2551 pmatch` | `glob`/`fnmatch` crate | matches in-band `CTLESC`/`CTLMBCHAR` bytes, not backslashes |
| `syntax.rs:121 is_alpha` | `u8::is_ascii_alphabetic` | `isalpha` is locale-dependent after `setlocale` |
| `bltin/printf.rs` conversions | `write!` | `%a`, `%e`, `%g`, `%#o` have no `std::fmt` equivalent |
| `var.rs:1024 varcmp` | `<[u8]>::cmp` | signed-char subtraction, and `=` compares as `\0` |
| `redir.rs:285 libc::dup` | `OwnedFd::try_clone` | `dup` does not set `CLOEXEC`; `try_clone` does |
| `redir.rs:479 savefd` | `try_clone()` | treats `EBADF` as **success**; wants a descriptor floor of 10 |
| `mystring.rs:155 atomax` | `str::parse::<i64>()` | saturates + `ERANGE` where Rust errors; base 0; leading blanks |
| `input.rs:493 preadfd` | `io::Read` / `BufReader` | must **not** retry some `EINTR`; `std::io` always retries |
| `trap.rs:331 onsig` | `signal-hook` | unwinds out of the kernel signal frame; `sa_flags = 0` deliberately |

---

## 2. The ledger, ranked by lines deleted per unit of risk

Risk is scored on what decides that the replacement is complete and
correct:

* **R0 — the compiler decides.** The code is unreachable, or the change
  is a rename. A differential sweep cannot move.
* **R1 — the type system decides, plus one local argument.** Behaviour
  identity follows from an argument that fits in a comment.
* **R2 — a human must read an invariant.** Behaviour identity follows
  from something the code does not say.
* **R3 — behaviour differs somewhere.** Only correct if the difference is
  shown unobservable, or sanctioned under `docs/divergences.md` — which
  means it cannot land before `sanctioned-divergences`.

Score is body lines removed divided by the risk weight (R0=1, R1=2, R2=6,
R3=20). It is a ranking device, not a measurement.

| # | Candidate | Lines | Risk | Score | Rides on |
|---|---|---|---|---|---|
| 1 | `gen/` (§4.4) | 2,006 | R0 | 2,006 | `delete-gen` |
| 2 | `system.rs` dead portability layer (§4.1) | ~200 body + ~250 test | R0 | ~450 | **new node** |
| 3 | `show.rs` (§4.3) | 442 | R0 | 442 | **new node** |
| 4 | `output.rs` `c_vsnprintf` / `VaArg` / macro layer (§4.2) | ~230 | R1 | 115 | `output-is-a-writer` |
| 5 | `mystring.rs` (§4.5) | ~230 of 282 | R1 | 115 | `owned-strings` |
| 6 | `bsearch` / `findstring` / `find_builtin` (§4.7) | ~95 | R1 | 48 | `builtins-take-args`, `owned-strings` |
| 7 | `bltin/mod.rs` stdio shim (§4.13) | 200 | R1 | 100 | `builtins-take-args` |
| 8 | `memalloc.rs` (§4.8) | 821 | R2/R3 | — | `delete-memalloc` |
| 9 | `jobs.rs` `cmdtxt`/`cmdputs` (§4.12) | 364 | R2 | 61 | `owned-strings` |
| 10 | `options.rs` `nextopt` (§4.14) | 47 | R2 | 8 | `builtins-take-args` |
| 11 | `redir.rs` fd ownership (§4.9) | ~0 net | R2 | 0 | `process-model` |
| 12 | `input.rs` buffered reading (§4.10) | ~0 net | R3 | 0 | `move-state` |
| 13 | `trap.rs` signal handling (§4.11) | ~0 net | R3 | 0 | `host-owns-signals` |
| 14 | `jobs.rs` fork/wait/job table (§4.12) | ~0 net | R3 | 0 | `process-model` |
| — | The three hash tables | ~250 | **R3** | 12 | §5.1 — a trap, not a win |
| — | `expand.rs` `msort` / `pmatch` / `expmeta` | — | — | **do not** | §5.2, §5.4 |
| — | `cd.rs` `updatepwd` | — | — | **do not** | §5.3 |
| — | ctype via `syntax.rs` / `system.rs` | — | — | **do not** | §5.5 |

Entries 11-14 delete roughly nothing and are here because the question
was asked and the answer is "the standard library does not have this".
Saying so is the deliverable; a reader who does not find it here will
spend an afternoon rediscovering it.

**The top of the table is boring on purpose. The largest safe deletions
in this crate are of code that does not run,** and only one of them
(`gen/`) is in any document today.

---

## 3. Riders, per step of `docs/idiomatization.md` §2

What to delete while you are already in there.

**0. `crate-rename` [doing].** Nothing. Do not carry a deletion in a
rename commit; the rename's whole value is being provably invisible to
the oracle (`dscase.sh:51,60`).

**1. `sanctioned-divergences`.** Nothing to delete. But this node is the
gate for §5.1 (hash tables), §5.2 (`strcoll`), §5.3 (`updatepwd`) and
§5.5 (ctype). Each of those, if changed, produces a divergence a corpus
case can observe — §5.1 demonstrably so. They are the concrete reason
the register has to exist before anyone rewrites a table or a path
builder, and they are a stronger argument for sequencing this node first
than the one `docs/idiomatization.md` §2.3 step 1 gives.

**2. `delete-gen`.** 2,006 lines, already scoped. Three riders:
* `show.rs` — 442 lines, 11 functions, `DEBUG = false` at `shell.rs:25`,
  0% corpus coverage. Same kind of thing, same answer. §4.3.
* `gen/mksignames.rs:94,103,117,233`, `gen/mknodes.rs:562` and
  `gen/mkinit.rs:553` hold 6 of the 7 `libc::malloc` sites outside
  `memalloc.rs`. Deleting `gen/` shrinks `delete-memalloc`'s surface
  before it starts.
* `error.rs:44-50` — an `extern "C"` block containing only a comment.

**3. `owned-strings`.** The largest rider. When a value stops being
`*mut c_char`:
* `mystring.rs` (§4.5) — ~230 of 282 body lines.
* the 116 libc string calls the decision already predicts, plus
  `system.rs`'s `mempcpy` (10 callers) and `strchrnul` (5), which are
  `copy_from_slice` and `find_byte(..).unwrap_or(len)`.
* `expand.rs:2203 memrchr` — `#ifndef HAVE_MEMRCHR`; `bstr`'s
  `ByteSlice::rfind_byte`.
* `jobs.rs:1602-1982 cmdtxt`/`cmdputs` — 364 lines of manual string
  building into a fixed 64-byte buffer with a hand-rolled flush. §4.12.
* `cd.rs:218,237 libc::strtok` — **not a cleanup, a bug fix.** `strtok`
  holds its parse position in a libc static. It is the last piece of the
  shell whose state is not merely process-global but *libc*-global, and
  it makes `updatepwd` non-reentrant against any other `strtok` caller in
  the host process — a hazard that only exists because the shell is
  becoming a library. `split_str(b"/")` is byte-identical here; §5.3
  explains why the rest of `updatepwd` is not.

**4. `delete-memalloc`.** Riders:
* `mystring.rs:289 sstrdup` and `memalloc::savestr` → `BString::from`.
* `shell.rs:57 max_int_length` — the C89 `log10(2)` digit-count trick; a
  `const` in Rust.
* `SHELL_ALIGN` / `SHELL_SIZE` (`memalloc.rs:26-41`) — `align_of` and
  `Layout`. `Vec<u8>` needs neither.
* the three hash tables are **not** a rider here. See §5.1.

**5. `output-is-a-writer`.** The rider is §4.2 and it is the best
non-dead-code entry in the ledger. What makes it cheap: **the internal
format strings are trivial.** Every format literal outside
`bltin/printf.rs` is one of `%s` (119 occurrences), `%d` (30), `%c` (7),
`%.*s` (4), `%jd` (3), `%*c` (2), `%ld`, `%5d`, `%.4o`, `%-20s`,
`%-16s`. Every one is a `write!`. What must survive is §5.6.

**6. `builtins-take-args`.** Riders:
* `exec.rs:654-663 find_builtin` (§4.7), which also removes a
  representation hazard: the code casts `&'static CStr` — a *fat*
  pointer — to `*const *const c_char` and relies on the data pointer
  being the first word, which Rust does not guarantee.
* `bltin/mod.rs` (§4.13) — 200 lines of BSD-stdio remapping macros, six
  of which have no expansion anywhere.
* `options.rs:394-460 nextopt` (§4.14) — dash's own `getopt`.
* `bltin/test.rs:203-217 getop` scans an **unsorted** table linearly.
  Sorting it would be a behaviour change: `t_lex` resolves ambiguity by
  first match. Leave it linear; it is 15 entries.

**7. `errors-are-values`.** Riders:
* `error.rs:41-42` — `jmp_buf` is 512 bytes on every `jmploc` (`:66,74`)
  that nothing reads, because `setjmp_catch` is a `catch_unwind`.
  `error.rs:423-443` (`Longjmp`, `raise_longjmp`) go by construction.
* `trap.rs:331 onsig` stops being `extern "C-unwind"` — §4.11.
* `error.rs:363-377 errmsg` **stays**: it overrides `ENOENT`/`ENOTDIR`
  with dash's own strings and falls back to `libc::strerror`. §5.9
  explains why std cannot supply that.
* `error.rs:386-391 __inton` is `#ifdef REALLY_SMALL`, never called.

**8. `no-ambient-state` (`thread-context`, `move-state`).** Riders:
* `input.rs`'s `parsefile` (§4.10) is one of the named tables and the
  place to record that `BufRead` is not the model.
* `redir.rs:44 redirlist` and `:47 closed_redirs` (§4.9).
* `exec.rs:96 lastcmdentry` — a `*mut *mut tblentry` cursor that
  `cmdlookup` writes and `delete_cmd_entry` reads. A side channel between
  two calls; it becomes a return value, not a field.
* **A hard limit this step will hit and the plan does not record:**
  `var.rs:103-106 changelocale` calls `libc::putenv` and
  `libc::setlocale(LC_ALL, "")`. Both are process-global and neither can
  be per-`Shell`. §7.

**9. `host-owns-signals`.** §4.11. No deletion; the finding is that
`signal-hook` is the wrong shape, and that `trap.rs:132 onsig` is
declared `extern "C-unwind"` and *unwinds out of a kernel-delivered
signal frame*, which is the most fragile construct in the crate and is
not on any risk register.

**10. `public-api`.** `system.rs`'s survivors, `mystring.rs`'s survivors,
`shell.rs:38-47`'s `likely`/`unlikely` (identity functions) and
`syntax.rs`'s five table accessors are all `pub` today; none belongs on
the surface.

**11. `posix-nonconformance`.** Nothing here.

---

## 4. The candidates in detail

### 4.1 `system.rs` — a portability layer for a portability problem nobody has

**What it is.** 650 lines: 350 non-blank outside the test module
(`system.rs:1-398`), ~250 lines of tests. It is `src/system.c` +
`src/system.h` — the `#ifndef HAVE_…` fallbacks.

**The measurement that decides it.** Twenty of twenty-eight exported
items have **zero callers in the crate**:

```
$ for f in <every pub item of system.rs>; do
    n=$(grep -rn "system::$f\b" crates/nsh/src --include=*.rs \
        | grep -v '/system.rs:' | wc -l); echo "$n $f"; done | sort -rn
10 mempcpy    1 strtoimax   0 isalnum  0 islower   0 memfd_create  0 strtod
 5 strchrnul  1 glob64      0 isalpha  0 isprint   0 stpcpy        0 sysconf
 3 sigclearmask 1 bsearch   0 isblank  0 ispunct   0 strsignal     0 tee
 2 globfree64             0 iscntrl  0 isspace   0 killpg        0 fnmatch
                         0 isdigit  0 isupper
                         0 isgraph  0 isxdigit
```

The twelve ctype wrappers are dead because every caller goes to `libc::is*`
directly (`syntax.rs:122,128,134`, `mystring.rs:170`, `miscbltin.rs:303`).
`killpg` is dead because `jobs.rs:277,521` call `libc::killpg`. `sysconf`
is dead because `bltin/times.rs:23` calls `libc::sysconf`. `strsignal` is
dead because `jobs.rs` calls `libc::strsignal`. `glob64`/`globfree64` are
reached only from `expand.rs:2001 expandmeta_glob`, behind
`GLOB_IS_ENABLED = 0` (`mystring.rs:67`), which cannot run.

The corpus agrees independently: `system.rs` is the lowest-covered module
in the crate at 15.38% of functions (`docs/idiomatization.md` §2.1).

**The replacement.** For the four live items: `mempcpy` →
`dst[..n].copy_from_slice(&src[..n])`; `strchrnul` →
`find_byte(c).unwrap_or(len)`; `bsearch` → §4.7; `sigclearmask` → the
three `libc::sigprocmask` lines it already is. `strtoimax` keeps its
`#[link_name = "__isoc23_strtoimax"]` binding — §5.8, that link name is
load-bearing.

**Risk R0, and the cost is not behavioural.** Nothing calls them. The
cost is that `system` carries **91 `[spec:dash:…]` rules**, the
second-highest of any module (`expand` 106, `system` 91, `jobs` 70).
Deleting the code retires the rules and `nplan_spec_*` will report it.
That is a plan transaction, to be decided deliberately rather than
discovered when the coverage report changes.

**Which node.** None exists. §7.

### 4.2 `output.rs` — the `vsnprintf` that dash never had

**What it is.** `output.rs:86-298` is 197 non-blank lines, plus a 73-line
macro layer at `:647-719`:

| lines | item | fate |
|---|---|---|
| 96-113 | `VaArg` enum, `va_list` alias | delete |
| 115-125 | `snp!` macro | **keep**, ~11 lines |
| 130-155 | `take_arg`, `take_int` | delete |
| 160-187 | `format_one` | **keep**, ~31 lines |
| 194-298 | `c_vsnprintf` — the format-string walker | delete, 105 lines |
| 647-719 | nine `From<T> for VaArg` + four `macro_rules!` | delete, 73 lines |

**Why the walker can go.** Two independent facts.

First, the internal formats are trivial (§3 step 5); the widest thing in
the whole inventory is `%.*s`.

Second, and this is the part that is not obvious: **the `printf` builtin
never passes a multi-conversion format.** `printfcmd` walks the user's
format itself. At `bltin/printf.rs:300` it takes `start = fmt - 1`
pointing at the `%`; at `:333-334` it saves `*(fmt+1)` and writes a NUL
there. The string handed to `PF!`/`ASPF!` is therefore exactly
`%[flags][width][.prec]<conv>` and nothing else. `echocmd`'s three
formats (`"%s "`, `"%s\n"`, `"%s"`, `:796,802,806`) take the `easy` path
at `:199` and never reach the formatter.

So the builtin needs "render one conversion, C-exactly" — `format_one` +
`snp!`, 42 lines calling libc `snprintf` with a rebuilt single-conversion
format. It does not need a walker.

**Risk R1.** The 134 internal call sites are mechanical and the compiler
finds them all. Three details want checking by hand:

* `%.4o` (`miscbltin.rs`, `umask`) — `{:04o}` is right; `{:#o}` is
  **not**, it renders `0o10` where C's `%#o` renders `010` (measured).
* `%*c` and `%5d` (`jobs.rs`, `show.rs`) — `{:>width$}`, `{:5}`.
* `fmtstr` (`output.rs:427-436`) **clamps** its return to `length`
  rather than reporting what would have been written, unlike `snprintf`.
  The unit test at `:882-909` records that a first draft got this wrong.
  Its five call sites must keep the clamp.

**Which node.** `output-is-a-writer`.

### 4.3 `show.rs` — 442 lines behind a `const false`

`shell.rs:25` is `pub const DEBUG: bool = false`. `TRACE!`
(`show.rs:290-296`) and `TRACEV!` expand to `if DEBUG { … }`. Three
`TRACE!` sites exist (`cd.rs:158`, `var.rs:892`, `trap.rs:444`) and one
commented-out `showtree` (`shellmain.rs:317`). The only live entry point
is `options.rs:200 opentrace()`, reached by `set -o debug`, itself
`#ifdef DEBUG` in the C.

[dec:nsh:owned-data] already records that `show.rs` "was converted
against the C by reading, not by testing", including a defect (`sharg`
never advancing `bqlist`) that nothing can observe. That is a precise
statement that the module has no oracle.

`jobs.c:cmdtxt` is the *other* tree printer and it is live — it builds
the text the `jobs` builtin shows. Do not confuse them. §4.12.

**Risk R0. Which node.** None; §7.

### 4.4 `gen/` — already planned

2,006 lines across five modules, not linked into the binary, zero
coverage. Covered by `delete-gen`; riders in §3 step 2. One fact for that
node's decision: **there is no `build.rs` anywhere in the workspace.**
`syntax.rs` and `signames.rs` are committed source, not generated output,
so `gen/` is already not on any build path.

### 4.5 `mystring.rs` — 282 lines of `<string.h>`

| lines | item | replacement | risk |
|---|---|---|---|
| 21-53 | `to_cchar` + eight `[c_char; N]` statics | `&'static BStr` literals | R1 |
| 71-73 | `equal` | `a == b` on `&BStr` | R1 |
| 77-79 | `scopy` | `copy_from_slice` | R1 |
| 101-120 | `scopyn` | `#if 0` in the C; **never called** | R0 |
| 128-142 | `prefix` | `ByteSlice::strip_prefix` | R1 |
| 210-223 | `is_number` | see below | R1 |
| 232-281 | `single_quote` | **stays** — shell quoting, not a std facility | — |
| 289-292 | `sstrdup` | `BString::from` | R1 |
| 299-324 | `pstrcmp`, `findstring` | §4.7 | R1 |
| 155-202 | `atomax`, `atomax10`, `number` | **§5.8** | R3 |

`is_number` has a wrinkle a rewrite gets wrong. The C loop
(`:212-222`) is a `do/while`: it reads `*p` **before** checking for the
terminator, so on the empty string it tests `is_digit(0)`, which is
false, and returns 0 — the right answer by accident. A straight `all()`
returns `true` for empty. The unit test at `:363` pins it.

**Which node.** `owned-strings`, except the number parsers, which belong
with their callers (`eval.rs:1279,1317`, `options.rs:423`,
`jobs.rs:402,428,430,814`, `var.rs:624`).

### 4.6 The three hash tables — read §5.1 first

| | table | size | hash | entry |
|---|---|---|---|---|
| variables | `var.rs:297 vartab: [*mut var; 39]` | 39 (not prime) | `var.rs:1012 hashvar` → `hashval` | `struct var`, `next` chain |
| aliases | `alias.rs:28 atab: [*mut alias; 39]` | 39 | shared `var::hashval` | `struct alias`, `next` chain |
| commands | `exec.rs:91 cmdtable: [*mut tblentry; 31]` | 31 (prime) | `exec.rs:787-792`, inline | `tblentry` + flexible array member |

`exec.rs:803` is the sharp edge, and [dec:nsh:owned-data] names it:

```rust
cmdp = ckmalloc(core::mem::size_of::<tblentry>() - ARB + libc::strlen(name) + 1)
```

`ARB` is 1 (`exec.rs:44`) and `cmdname: [c_char; ARB]` (`:84`) is C's
flexible array member. The entry stores `Rc::into_raw` of a function node.

Four further structural facts a `HashMap` swap has to answer for:

1. **The variable key is a prefix of the value.** `struct var` has one
   `text` field holding `"NAME=value"`; `hashval` stops at `=`
   (`var.rs:1012`) and `varequal` compares up to it. There is no separate
   key. `HashMap<BString, BString>` splits one allocation into two.
2. **`alias` does the same trick harder.** `alias.rs:64-66`:
   `ap->name = savestr(name)` then `ap->val = ap->name + namelen`. Name
   and value are one allocation with the value pointing into its middle.
3. **Insertion is asymmetric.** `var.rs:423-430 initvar` inserts at the
   bucket **head**; `setvareq` appends at the **tail**. That asymmetry
   is what puts the `varinit` entries where they are in `env` output.
4. **Aliases have a deferred free.** `ALIASINUSE`/`ALIASDEAD`
   (`alias.rs:14-15`) keep an alias alive while it is being expanded and
   free it afterwards. `Rc<Alias>` expresses this; a bare `HashMap` does
   not.

**Risk R3, not R2 — see §5.1.** The tables are the most-cited "obvious
`HashMap`" in the whole idiomatization and they are the single biggest
trap in this document.

**Which node.** The allocation belongs to `delete-memalloc`; the
ownership to `move-state`; the *observable order* belongs to
`sanctioned-divergences`, and none of the three names it today.

### 4.7 `bsearch` over sorted tables — three copies, one line each

* `system.rs:154-182` — the `#ifndef HAVE_BSEARCH` fallback, 29 lines.
* `mystring.rs:299-324` — `pstrcmp` + `findstring`, 26 lines.
* `exec.rs:654-663 find_builtin` — `libc::bsearch` over the 40-entry
  `builtins::builtincmd`.
* `parser.rs:2310 findkwd` — `findstring` over `parsekwd`.

There are two `bsearch` implementations in the crate and they are used
asymmetrically: `findstring` calls `crate::system::bsearch` (the ported
fallback), `find_builtin` calls `libc::bsearch`. Both are
`<[T]>::binary_search_by`.

The comparison is `strcmp` — unsigned byte order — and `mkbuiltins` sorts
with `LC_COLLATE=C` (`builtins.rs:12`), so `<[u8]>::cmp` is **exactly**
right here. This is the one sorted-table case in the crate where the
obvious swap is the answer, and given §5.1 and §5.7 it is worth saying so
explicitly.

Two bonuses. `findstring` returns a pointer *into* the array and callers
recover the index by subtraction; `binary_search_by` returns the index.
And `find_builtin`'s `pstrcmp` reads `*(a as *const *const c_char)` where
`builtincmd`'s first field is `name: &'static CStr`, a **fat** pointer —
so it works only because the data pointer happens to be the first word,
which Rust does not guarantee. The swap deletes a latent representation
dependence, not just lines.

**Risk R1. Which node.** `builtins-take-args` for `find_builtin`,
`owned-strings` for the rest.

### 4.8 `memalloc.rs` — already planned

821 lines: the region allocator ~245, the string builder ~160, the malloc
wrappers ~54, tests 257. Covered by `delete-memalloc`. Riders in §3
step 4. One correction to the shape: `SHELL_ALIGN` (`memalloc.rs:26-41`)
is not part of the allocator's contract with anything, it is C89's
missing `alignof`, and nothing that replaces the region needs it.

### 4.9 `redir.rs` — what `OwnedFd` can and cannot do

**What it is.** 546 lines. The saved-descriptor table is
`redirtab { next: *mut redirtab, renamed: [c_int; 10] }`
(`redir.rs:38-44`), `ckmalloc`'d at `:536`, freed at `:446`.

**What `OwnedFd` buys.** A slot holds one of three things: `EMPTY = -2`
(`:29`), `CLOSED = -1` (`:30`), or a real saved descriptor ≥ 10.
`Option<OwnedFd>` is two states, not three, so the slot becomes a
three-arm enum. That is a genuine improvement — the sentinels stop being
magic integers — and it is worth roughly zero lines.

**What breaks, precisely.**

1. **`savefd` (`:474-495`) wants `F_DUPFD_CLOEXEC` with a minimum of
   10.** `OwnedFd::try_clone()` is `F_DUPFD_CLOEXEC` with a minimum of 0.
   **std cannot express this.** `libc::fcntl` can, and does.
2. **`savefd` treats `EBADF` as success.** `:482` — `if err != EBADF`
   guards both the `close` and the error raise, so duplicating a
   *closed* descriptor returns `-1` and the caller carries on. That is
   how `exec 9>&-` inside a redirection works. `try_clone()` returns
   `Err` and a `?` propagates it. This is the subtler of the two and it
   is the one a mechanical rewrite loses.
3. **`sh_dup2(ofd, -1, cfd)` (`:284-288`) calls `libc::dup`, which does
   *not* set `CLOEXEC`;** `try_clone()` does. `sh_pipe`'s memfd path
   (`:334`) is `sh_dup2(pip[0], -1, pip[0])`, so the write end of a
   here-document memfd is currently inheritable across `exec`.
4. **`popredir` (`:427-442`) does `dup2` then `close`, in that order, on
   descriptors it does not own.** With `OwnedFd`, drop order decides
   whether the restore happens before or after the close.
5. **Drop cannot be interrupt-atomic.** `redirect` (`:90-131`) and
   `popredir` (`:410-447`) run inside `INTOFF`/`INTON` because a
   half-updated table is unrecoverable. `popredir` is also reached from
   the unwind path (`unwindredir`, `:518-522`, called from
   `mkinit_exitreset`). Giving the slots destructors puts descriptor
   closes at unwind points the C never had, which is a change to *when*
   fds close, not just to who owns them. This lands in
   `errors-are-values`, not in a cleanup.

**One thing std does have:** `std::io::pipe` (stable since 1.87) returns
`(PipeReader, PipeWriter)` as `OwnedFd`s and would replace
`redir.rs:339 libc::pipe`. The `memfd_create` branch (`:328-336`) has no
std equivalent.

`redir.rs:393 libc::_exit(0)` is in the here-document writer child. P8
exempts a forked child; it must stay `_exit`, and it must stay *before*
anything that could run a destructor.

**Risk R2. Which node.** `process-model` for ownership types;
`move-state` for `redirlist`/`closed_redirs`.

### 4.10 `input.rs` — why `BufRead` is not the model

**What it is.** 892 lines, 9 `static mut`. `struct parsefile`
(`input.rs:75-90`) carries `lleft`/`nleft` (two independent counts),
`nextc`, `buf`, `strpush`, `basestrpush`.

**Four things `BufRead`/`BufReader` cannot do.**

1. **Negative-offset pushback.** `IBUFSIZ` is
   `BUFSIZ + PUNGETC_MAX + 1` = 8209 (`input.rs:20,23`), and `preadfd`
   (`:406`) reserves `PUNGETC_MAX` = 16 bytes of headroom *in front of*
   the region it reads into. `pungetc` walks backwards into it, clamped
   at `:417-418`. A refill then `memmove`s the already-consumed tail
   down to preserve that window. `BufReader` has no concept of a byte
   before the current read position, and `Seek` cannot help — the source
   may be a pipe.
2. **`EINTR` must sometimes not be retried.** `input.rs:493-495`:

   ```rust
   if errno() == libc::EINTR
       && !(!basepf.prev.is_null() && crate::trap::pending_sig != 0)
   { continue 'retry; }
   ```

   A read interrupted by a signal is retried *unless* there is a pending
   trap and the input stack is not at its base — in which case it
   returns the error so `dotrap` can run before more input is read.
   Every `std::io` read helper retries `EINTR` unconditionally, with no
   hook for a predicate. This is trap-dispatch timing, which the pty
   suite observes and the batch corpus does not.
3. **The parse position is a stack, not a stream.** `pushstring` /
   `popstring` push alias bodies and `-c` strings *in front of* the
   current file, and `pgetc` checks the string stack before the buffer
   on every character. A `BufRead` chain would have to be rebuilt per
   alias expansion, and `popstring` has to restore the exact byte
   position in the *outer* source.
4. **`flush_input` `lseek`s the real descriptor backwards** by the
   unconsumed byte count, so a child `exec`d from a script reads from
   where the parser stopped. That is a property of the fd, not of a
   buffer, and it is why the buffer is `BUFSIZ` and not larger.

Plus one path with no std analogue: `preadfd` (`:406-500`) uses `tee(2)`
into a private pipe to peek at a terminal without consuming, falling back
to one-byte reads. That is a Linux syscall, not a stream abstraction.

**Risk R3.** Every one of the four is observable. `Vec<u8>` as the
buffer and explicit indices is the correct end state; `BufReader` is not.

**Which node.** `move-state` (`parsefile` is one of the named tables),
with the `EINTR` behaviour written down where `errors-are-values` will
find it.

### 4.11 `trap.rs` — why not `signal-hook`

**What it is.** 522 lines, 8 `static mut`. `trap: [*mut c_char; NSIG]`
indexed by signal number with slot 0 as `EXIT` (`trap.rs:60`);
`sigmode: [c_char; NSIG-1]` indexed `signo - 1` (`:63`) — two different
indexing conventions in adjacent arrays.

**Three reasons a signal crate is the wrong shape.**

1. **The disposition policy is the behaviour.** `setsignal` decides
   `SIG_DFL` / `SIG_IGN` / handler from the trap string, the interactive
   flag, job-control state, *and* the disposition the shell inherited —
   which it discovers lazily, by calling `sigaction` with a NULL action
   the first time it sees a signal. "Was this ignored when we started"
   is load-bearing: POSIX says a signal ignored at entry stays ignored.
   No crate models this.
2. **`sa_flags = 0` means no `SA_RESTART`, deliberately.** Every
   blocking syscall in the shell is written to see `EINTR` (§4.10
   item 2, `redir.rs:181`, `jobs.rs:1514`). A crate that sets
   `SA_RESTART` — most do; it is the friendly default — silently changes
   the shell's interruptibility everywhere.
3. **[dec:nsh:host-owns-signals] puts installation in the frontend.** A
   crate whose purpose is installing dispositions belongs in `nsh-cli`
   if anywhere, and by then the frontend needs exactly `sigaction`.

**The finding that is not about crates at all.** `onsig`
(`trap.rs:331`) is declared `pub unsafe extern "C-unwind"` and, when
`suppressint == 0`, calls `onint()` (`:348`), which raises — a
`panic_any` (`error.rs:236`) — **from inside a kernel-delivered signal
frame.** The unwinder walks back through `__restore_rt`'s CFI.

The construct is fully documented where it lives (`trap.rs:315-330`
explains the ABI choice and the trampoline), so this is not an
undocumented hazard. What is missing is that it appears in no
*cross-cutting* document: not in `docs/idiomatization.md` §5's risk
register, and on no decision. It is the single most target-specific
construct in the crate, it is the reason `panic = "unwind"` is pinned in
both profiles, and it disappears the moment `errors-are-values` lands and
the handler merely sets a flag. That makes it an argument *for* that step
that the step does not currently claim, and the one `unsafe` under
[dec:nsh:minimal-unsafe] that is not a syscall wrapper, a signal-handler
flag write, or fd work.

**Risk R3. Which node.** `host-owns-signals`, with the `onsig`
observation moved onto `errors-are-values`.

### 4.12 `jobs.rs` — what `std::process` can actually do

**What it is.** 2,055 lines, 10 `static mut`, the lowest coverage of any
large module (75.33% of regions).

**`std::process::Command` cannot express the fork.** `forkchild`
(`:1094-1178`) runs 85 lines *between* `fork` and `execve`:
`setpgid(0, pgrp)` then `tcsetpgrp` (`:1120-1130`), a full signal-mode
reset, `closescript`, `redirlist = NULL`, `handler` reset. `Command`
offers `pre_exec` (unsafe, async-signal-safety is the caller's problem)
and `process_group`, but it has no `tcsetpgrp`, and — decisively — the
shell needs the **parent** to also call `setpgid` on the child
(`forkparent`, `:1182-1231`) so that neither side races. `Command` gives
the parent a `Child` only after the race is already lost. And
`vforkexec` (`:1255-1283`) has no `Command` analogue at all.

**Nor the wait.** `waitproc` (`:1490`) declares `wait3` itself at
`:1504-1507`, because the `libc` crate has no binding:

```rust
/* `wait3` has no binding in the `libc` crate, so it is declared here. */
extern "C" { fn wait3(status: *mut c_int, options: c_int,
                      rusage: *mut libc::rusage) -> pid_t; }
```

Flags are `0` when the caller passed `DOWAIT_BLOCK` and `WNOHANG`
otherwise (`:1492-1496`), with `WUNTRACED` added under job control. The
blocking path is a `sigsuspend` between a blocked-signal check and the
non-blocking `wait3` at `:1514`, so `SIGCHLD` cannot arrive in the gap.
`std::process::Child::wait` blocks, reaps one *specific* child, and has
no `WUNTRACED` — a shell must reap *any* child, including ones it never
spawned, and must see stops as well as exits.

**The job table wants a `Vec` and has one obstruction.** `jobtab: *mut
job` (`:99`) is `ckrealloc`'d by `growjobtab` (`:1022`), which then
**walks the table fixing interior pointers** — `jp->ps` may point at
`jp->ps0`, and `curjob` is a raw pointer into the table. A `Vec<Job>`
must replace both with indices, and `jobno` (`:451`) recovers a job
number by pointer subtraction, which becomes the index directly. This is
mechanical once the pointers are gone and impossible before.

**What is a clean deletion.** `cmdtxt` (`:1602-1821`, 220 lines) and
`cmdputs` (`:1839-1982`, 144 lines) build the command text the `jobs`
builtin prints, into a fixed 64-byte buffer with a manual flush. That is
364 lines of `Vec<u8>` and `write!`. It is live and covered, so R2 rather
than R0, and it rides on `owned-strings`. `sprint_status` (`:543`) writes
into a caller-supplied buffer with no length parameter; it becomes a
`BString` return.

**No sorting anywhere in the file, and no `ioctl`/`TIOCGWINSZ`** — the
terminal size question does not arise; `nshedit` owns that.

**Risk R3. Which node.** `process-model` for fork/wait/table,
`owned-strings` for `cmdtxt`/`cmdputs`.

### 4.13 `bltin/mod.rs` — 200 lines of `#define`

Eighteen `macro_rules!` remapping BSD stdio names onto the shell's output
layer: `stdout`, `stderr`, `printf`, `putc`, `putchar`, `FILE`,
`fprintf`, `fputs`, `fflush`, `fileno`, `ferror`, `INITARGS`, `error`,
`warn`, `warnx`, `exit`, `setprogname`, `getprogname`, `setlocate`,
`getenv`. Six have no expansion anywhere in the crate (`warn`, `exit`,
`setprogname`, `setlocate`, `getenv`, `INITARGS`) and `bltin/mod.rs`
documents two of them as vestigial in the C as well (`:131-137,146-153`).

Once the builtins take `&mut Shell` and `&[&BStr]`, the shim has no
reason to exist. **Risk R1.** **Which node.** `builtins-take-args`.

### 4.14 `options.rs` — three option parsers, one of them libc's

`nextopt` (`options.rs:666-712`, 47 lines) is dash's own `getopt`, and
the C says so. It differs from `getopt(3)` in ways the builtins depend
on: no
`optind`/`optopt` globals (it advances `argptr` instead), no `--`
handling, no permutation, no `optstring` leading `:` mode, and it
`sh_error`s rather than returning `?`. It is not replaceable by
`getopt` — but it is also not replaceable by any crate, because the
error text is under test.

The finding is the inconsistency: the crate contains **three** option
parsers. `nextopt`; the `getopts` builtin (`options.rs`), which
maintains POSIX `OPTIND`/`OPTARG` in shell variables; and
`histedit.rs`, which calls **`libc::getopt` directly** and resets it with
the glibc-specific `optind = 0`. That third one is process-global libc
state inside what is becoming a library, and it is the same class of
problem as `strtok` (§3 step 3) and `putenv` (§7).

`shellparam` (`options.rs`) stores positional parameters as a
NULL-terminated `*mut *mut c_char` *plus* a separate `nparam` count, and
counts by hand in two places. `Vec<BString>` makes `$#` a `.len()`.

**Risk R2. Which node.** `builtins-take-args`.

### 4.15 Smaller items, listed so they are not rediscovered

* `syntax.rs` — 333 lines, 306 non-blank, of which the five tables run
  from `:150` to the end. No `build.rs`. The five accessors exist because
  Rust cannot form a pointer into the middle of an array and index it
  negatively (`syntax.rs:12-19`); `[T; 257]` plus `+ SYNBASE` is what it
  already does and is correct. `is_digit` (`:115`) is the unsigned-wrap
  trick `((unsigned)(c - '0') <= 9)` and is **locale-free**, unlike its
  three neighbours (§5.5).
* `signames.rs` — 105 lines, almost all table, zero real functions.
  Nothing to replace.
* `bltin/times.rs` — four hand-unrolled minute/second splits and a
  `%dm%fs` format. `%f` is the one C float conversion Rust reproduces
  exactly (§5.6), so this one is a real `write!`.
* `histedit.rs:673-689` — a hand-rolled `mkstemp` prefix built with an
  **unbounded `libc::sprintf`** into a fixed buffer. `libc::mkstemp` does
  the rest. A `BString` + `format!` removes the only unbounded `sprintf`
  in the crate.
* `expand.rs:433-447 getpwhome` — `libc::getpwnam`, whose result points
  into a static buffer. std has no `getpwnam`; keep it, and note it is
  another non-reentrant libc global.
* No random number generation anywhere in the crate. No time formatting
  beyond `times`.

---

## 5. The traps

Ordered by how convincing the wrong answer looks.

### 5.1 `HashMap<BString, V>` changes `env` output — measured

This is the most-cited "obvious" replacement in the whole idiomatization
(`docs/idiomatization.md` §2.3 step 4 and §3.3-3.4 both name it), and it
is the biggest trap in this document.

`var.rs:640-675 listvars` walks `vartab` **bucket by bucket, chain by
chain**, and returns the array that `exec.rs:125` hands to `execve` as
`envp`. So the variable table's internal order *is* the environment's
order in every child process. Measured against the reference build:

```
$ env -i dash -c 'export AA=1 BB=2 CC=3 DD=4 EE=5 FF=6; env'
AA=1
FF=6
DD=4
BB=2
PWD=...
EE=5
CC=3
```

Not sorted. Not insertion order. Bucket order, and therefore a function
of `VTABSIZE = 39` and of `hashval`'s exact arithmetic
(`var.rs:1012-1016`).

The same is true of two builtins that print directly:

```
$ dash -c 'alias zz=1 aa=2 mm=3 bb=4; alias'
'bb=4'
'zz=1'
'mm=3'
'aa=2'
```

`alias.c` carries its own `/* TODO - sort output */`. `hash`
(`exec.rs:336`) is the third.

For contrast, `set`, `export -p` and `readonly -p` **do** sort, via
`libc::qsort` + `vpcmp` (`var.rs:693-698`) — and that comparator is its
own trap, §5.7.

**Consequences.**

* `HashMap` is wrong twice over: its order differs from bucket order,
  and it is randomised per process, so the divergence would not even be
  stable across two runs of the same case.
* `BTreeMap` is wrong once: sorted is not bucket order either.
* An `IndexMap`-shaped insertion-order map is wrong too — `initvar`
  inserts at the bucket head and `setvareq` at the tail
  (`var.rs:423-430` vs `:521`), so bucket order is not insertion order.

**The only order-preserving replacement is a hash table with dash's
table size and dash's hash function** — i.e. keep the structure, change
the memory management. `Vec<Vec<Entry>>` of length 39 with `hashval`
ported literally is byte-identical and still deletes every `ckmalloc`.

**Risk R3.** Any other choice is a deliberate divergence and must wait
for `sanctioned-divergences`. It is also a *good* divergence to consider
— sorted `env` and sorted `alias` are what every other shell does, and
`alias.c`'s own TODO says dash agrees — but it is a `posix-nonconformance`
decision, not a refactor.

*Resolution:* attach the order question to `sanctioned-divergences` and
decide it there. Until then, port the table shape literally.

### 5.2 `msort`/`strcoll` is not `Vec::sort()` — measured

`expand.rs:2453-2498 msort` is a linked-list merge sort whose comparison
is `libc::strcoll` (`:2479`). The C's own comment says why
(`expand.rs:2430-2434`, and `var.rs:677-682` for the variable case):
POSIX requires the locale's collating order.

`var.rs:105` calls `libc::setlocale(libc::LC_ALL, "")`. **The locale is
live.** Measured, in a directory containing `a B c D`:

```
LC_ALL=C            dash -c 'echo *'   ->  B D a c
LC_ALL=en_US.UTF-8  dash -c 'echo *'   ->  a B c D
```

and a Rust `vec![b"a", b"B", b"c", b"D"].sort()` gives `["B","D","a","c"]`
— the C-locale answer, always. So `Vec::sort()` is a silent behaviour
change for every user whose `LANG` is not `C`, in the most-used feature
of the shell. It is not a crash; it is `ls` order.

**What makes this dangerous rather than merely wrong:** whether the
differential harness catches it depends on the ambient locale.
`tests/harness/sandboxed.sh:55-70` sets only `TMPDIR` and `PATH` with
`--setenv` and does not clear the environment, so `LANG` reaches both
shells. This box has `LANG=en_US.UTF-8`, so a sweep run here *would*
catch the swap. A sweep run under `LC_ALL=C` — which is what CI usually
does, and what a careful person sets to make output stable — would not.
**The oracle's coverage of this replacement is an undeclared property of
whoever runs it.**

*Resolution:* keep `strcoll`.
`Vec::sort_by(|a, b| strcoll(a, b))` is correct and much shorter —
`msort` takes the first half on ties and so is stable, and
`slice::sort_by` is stable, so that substitution is safe. What is not
safe is dropping `strcoll`. Separately: pin the locale in
`sandboxed.sh`, and record which locale, because either choice is a
decision.

### 5.3 `cd -L` is not `fs::canonicalize`, and `updatepwd` is not `Path`

`cd.rs:184-244 updatepwd` builds the logical path and never touches the
filesystem — `docd` (`:154-175`) calls `updatepwd` *first* and `chdir`
second. Four measured differences:

```
$ mkdir -p real/sub; ln -s real link
$ dash -c 'cd link/sub && cd .. && pwd; pwd -P'
.../link      <- logical: the textual parent
.../real      <- physical
```

1. **`..` is textual, so it does not resolve symlinks.**
   `fs::canonicalize` is physical and gives `.../real`.
2. **The path need not exist.** `cd nosuchdir/../b` succeeds when `b`
   exists (measured). `canonicalize` fails with `ENOENT`; it requires
   every component to exist.
3. **`//` is preserved and `///` is not.** `cd.rs:212-216` special-cases
   exactly two leading slashes — POSIX XBD 4.13. Measured: `cd //; pwd`
   → `//`; `cd ///; pwd` → `/`. And
   `Path::new("//").components().collect::<PathBuf>()` → `/` (measured
   with `rustc`). `Path` normalisation loses this.
4. **`Path::components()` removes `.` but not `..`** — measured,
   `/a/./b/../c` → `/a/b/../c`. It cannot remove `..`, because for a
   physical path that would be wrong, which is exactly the point.

`cd.rs:206 lim` is a floor below which `..` cannot pop, which is why
`cd /; cd ..; pwd` prints `/`.

*Resolution:* `updatepwd` is shell semantics, not path handling. Port it
to `BString` operations. The only std replacement inside it is
`libc::strtok` → `split_str(b"/")` (§3 step 3), which is both safe and a
re-entrancy fix. `std::path` should not appear in this function at all.

### 5.4 `pmatch` is not `fnmatch`, and the `glob` crate is further still

`expand.rs:2551-2747 pmatch` is 197 lines. `mystring.rs:66-67` sets
`FNMATCH_IS_ENABLED = 0` and `GLOB_IS_ENABLED = 0`, so `libc::fnmatch`
(`expand.rs:2558`) and `expandmeta_glob` (`:2001`) are both dead paths.
Four independent reasons it cannot be swapped:

1. **The pattern is not the user's text.** `preglob`/`rmescapes` turns
   backslash escapes into `CTLESC` (0x81) *inside* the pattern
   (`:2507`). `pmatch` matches on `CTLESC` (`:2569`) and `CTLMBCHAR`
   (`:2712`), not on `\`. A crate taking `&str` cannot receive this
   input at all — [dec:nsh:bytes-not-text].
2. **`[[:class:]]` goes through `wctype`/`iswctype` (`:2514-2547
   ccmatch`)**, which is locale-dependent. The `glob` crate's classes are
   not.
3. **Multibyte handling is dash's own.** `mbnext` (`:402`) returns a
   packed length pair and `pmatch` advances by `(mb >> 8) + (mb & 0xff)`
   (`:2579`). Nothing reproduces this encoding.
4. **There is a reproduced bug in the bracket arm.** `:2642-2647`
   documents it: `mbs` starts as the address of the local `c` and the
   `strncmp` at `:2695` reads `mb` bytes from a single-byte local. Any
   correct implementation diverges here.

`expmeta` (`:2226-2424`) is likewise not `glob`: it walks directories
itself with `opendir`/`readdir64`, applies leading-dot rules, and calls
`pmatch` per entry.

*Resolution:* do not touch. When `owned-strings` reaches `expand.rs`,
`pmatch` becomes a function over `&[u8]` and otherwise stays as it is.
`docs/idiomatization.md` §5 already names `expand.rs` the most dangerous
module; this is a second reason.

### 5.5 `isalpha` is not `is_ascii_alphabetic` — the locale again

`syntax.rs:119-135` defines `is_alpha`, `is_name` and `is_in_name` over
`libc::isalpha`/`libc::isalnum`. These decide what a **variable name** is
(`parser.rs:2146 endofname`, `arith_yylex.rs:67`). `system.rs:314-388`
wraps twelve more. After `setlocale(LC_ALL, "")` they are
locale-dependent for bytes 0x80-0xFF; `char::is_ascii_alphabetic` is not.

`system.rs:626-648`'s unit test asserts the two agree across 0-255 — and
it passes because a Rust test binary never calls `setlocale`, so it runs
in the `C` locale. **The test establishes the equivalence in exactly the
configuration where it cannot fail.**

Note the asymmetry that makes this easy to miss: `is_digit`
(`syntax.rs:115`) is the unsigned-wrap trick and is locale-free, so one
of the four neighbours genuinely *is* replaceable. Three are not.

*Unresolved on this box.* Demonstrating the divergence needs a
single-byte locale (ISO-8859-1), where `isalpha(0xE9)` is true; only
`en_US.utf8` is generated here, and in a UTF-8 locale no single byte
0x80-0xFF is alphabetic, so the two agree. *Resolved by:*
`localedef -i en_US -f ISO-8859-1 en_US.ISO-8859-1` then
`LC_ALL=en_US.ISO-8859-1 dash -c $'\xe9=1; echo $\xe9'`.

*Resolution regardless:* keep `libc::isalpha`. It costs one `unsafe`
call; the alternative is a divergence visible only to users whose locale
is not the developer's.

### 5.6 The `printf` builtin's conversions have no `std::fmt` equivalent

`bltin/printf.rs:335-369` dispatches
`%c %s %d %i %o %u %x %X %a %A %e %E %f %F %g %G`. Measured against the
reference build and against a `rustc` probe:

| | C dash | Rust |
|---|---|---|
| `%a` 1.5 | `0x1.8p+0` | no equivalent |
| `%e` 12345.678 | `1.234568e+04` | `{:.6e}` → `1.234568e4` |
| `%g` 0.0001 | `0.0001` | no equivalent |
| `%#o` 8 | `010` | `{:#o}` → `0o10` |
| `%f` NaN | `nan` | `{:.6}` → `NaN` |
| `%f` 12345.678 | `12345.678000` | `{:.6}` → `12345.678000` ✓ |

`%f` is the only agreement, and `bltin/times.rs:52` is its only internal
user.

*Resolution:* keep `format_one` + `snp!` (§4.2) — 42 lines calling libc
`snprintf` with a rebuilt single-conversion format. This is not a wart:
POSIX specifies the `printf` utility in terms of C's `fprintf`, so
calling C's `fprintf` *is* the specification.

### 5.7 `varcmp` is signed-char subtraction, not `<[u8]>::cmp`

`var.rs:1024-1043` compares `*p as c_int` where `p: *const c_char`. On
x86-64 Linux `c_char` is `i8`, so a byte ≥ 0x80 is negative and sorts
*before* ASCII; `<[u8]>::cmp` is unsigned and sorts it after. The
comparator also maps `=` to `\0` inside the loop (`:1035-1040`) so that
`A=1` sorts as `A` — but **not** for the first character
(`:1025-1026`). That asymmetry is unreachable (a name cannot start with
`=`) and is exactly the kind of thing a tidy-up removes without noticing.

`libc::qsort` (`var.rs:693`) is unstable where `slice::sort_by` is
stable. Names in `vartab` are unique so there are no ties, but that
argument has to be made, not assumed.

*Resolution:* `sort_by` with `varcmp` ported literally, including the
`as i8 as i32` cast, and a comment saying the cast is the behaviour.

### 5.8 `strtoimax` saturates; `str::parse` errors

`mystring.rs:155-179 atomax` calls `crate::system::strtoimax`, bound as
`__isoc23_strtoimax` (`system.rs:134`). Measured:

```
printf '%d\n' " 42"                  -> 42       ("42".parse() would be Err)
printf '%d\n' 99999999999999999999   -> 9223372036854775807 + warning,
                                        status 1 (parse -> Err(PosOverflow))
echo $((0b101))                      -> 5        (the __isoc23_ binding; the
                                        plain strtoimax symbol has no 0b)
```

`atomax` also accepts base 0 (`0x`, `0`, `0b`), allows trailing
whitespace (`:170-172`), and raises `Illegal number: %s` through
`badnum`. `str::parse::<i64>()` does none of it.

*Resolution:* keep the libc call. A hand-written parser reproducing
saturation, `ERANGE`, base 0 and `0b` is more code than the binding and a
new place for a defect.

### 5.9 `libc::strerror` has no std equivalent

`error.rs:365-377 errmsg` returns `libc::strerror(e)` except for
`ENOENT`/`ENOTDIR`, which it overrides with `"No such file"`,
`"Directory nonexistent"` or `"not found"` depending on the operation.
Seven other sites call `libc::strerror` directly (`cd.rs:263`,
`redir.rs:298,487`, `jobs.rs:436,2012`, `miscbltin.rs:654`,
`bltin/printf.rs:788`).

`std::io::Error::from_raw_os_error(e).to_string()` appends
`" (os error N)"`. No std API yields the bare `strerror` string;
`rustix::io::Errno`'s `Display` differs too.

*Resolution:* keep `libc::strerror`, and record that it returns a static
buffer and is not thread-safe — `streams.rs:87` already says so. If P9
(two shells, two threads) is to hold, this becomes `strerror_r`, which is
a change of *function*, not of crate.

---

## 6. Dependencies: the case for adding almost nothing

Current: `libc` and `nshedit` (`crates/nsh/Cargo.toml`). Decided but not
yet added: `bstr` ([dec:nsh:bytes-not-text]).

This is a library, so every crate is imposed on every embedder. The test
applied below is not "is the crate good" but **"does it delete lines the
standard library cannot, and does it delete more than it adds to the
graph"**.

One fact changes the arithmetic and should come first: **`bstr` and
`rustix` are already in `Cargo.lock`**, both transitively through
`nshedit` (`bstr` ← `nshedit`; `rustix` ← `nshedit-plat` ← `nshedit`).
Adding either to `[dependencies]` adds **zero** new crates to the build
graph. `memchr` (← `bstr`), `bitflags`, `errno` and `linux-raw-sys`
(← `rustix`) are likewise already compiled. So is `thiserror`
(← `postcard` ← `nshedit`). Toolchain here is rustc 1.97.1; no
`rust-version` is declared in any manifest, which is itself a gap for a
publishable library.

| Crate | Verdict | Why |
|---|---|---|
| `libc` | **stays** | `strerror` (§5.9), `strcoll` (§5.2), `setlocale`, `isalpha` (§5.5), `mbrtowc`/`wctype`/`iswctype` (§5.4), `snprintf` (§5.6), `getpwnam`, `fork`/`execve`/`wait3`, `sigaction`, `tcsetpgrp`, `times`, `getrlimit`, `fcntl(F_DUPFD_CLOEXEC, 10)` (§4.9). std has no answer for the first six. |
| `bstr` | **add** | Already decided, already in the lock. Deletes hand-rolled `<[u8]>` work across ~116 libc string calls, and supplies `find_byte`/`rfind_byte`/`split_str` for §4.1 and §3 step 3. |
| `rustix` | **no; revisit at `process-model`** | Free in the graph, and it is the only crate that types `fcntl_dupfd_cloexec(fd, 10)` — but so does `libc::fcntl`, which the code already calls. It deletes zero lines; it changes which crate the same syscall goes through. Adopting it now means two syscall vocabularies in one crate, which is worse than one. If `process-model` moves the whole fd surface at once, it becomes a real candidate. |
| `nix` | **no** | Strictly larger than `rustix`, more churn, duplicates `libc` without deleting anything. |
| `signal-hook` | **no, and the shape is wrong** | §4.11. [dec:nsh:host-owns-signals] puts installation in the frontend, so a crate for installing dispositions belongs in `nsh-cli` if anywhere; and the `sa_flags = 0` / inherited-`SIG_IGN` policy is behaviour under test that no registry abstraction models. |
| `glob` / `globset` | **no** | §5.4. Different language. |
| `thiserror` | **no** | Free in the graph and still wrong: the `Error` variants must render dash's exact diagnostic text (`docs/idiomatization.md` §1.3), which means a hand-written `Display` reading a `BString`, not an `#[error("…")]` attribute holding a format string. Nine variants, forty lines. |
| `memchr` | **no** | `bstr` re-exports what is needed. |
| `bitflags` | **maybe, at `public-api`** | Free in the graph. `VEXPORT`/`VREADONLY`/`VSTRFIXED`, `EXP_*`, `REDIR_*`, `BUILTIN_*` are all bit sets, and `flags & mask == on` (`var.rs:652`) currently type-checks against any `c_int`. It deletes no lines; the win is that a wrong mask stops compiling. Decide when the flags stop being `pub`. |

**Where std plus fifty lines wins outright:** §4.7 (`binary_search_by`),
§4.1's four live items, `memrchr`, `strtok` → `split_str`, `libc::pipe` →
`std::io::pipe`, and the entire internal formatting layer (§4.2). None
needs a crate; all delete more than they add.

**Where std loses and libc stays:** §5.2, §5.5, §5.6, §5.9, §4.9's
descriptor floor and `EBADF` semantics, §4.10's selective `EINTR`, and
`wait3`. In each case the gap is not ergonomics — std does not model the
concept.

**Net recommendation: add `bstr`, add nothing else, and revisit exactly
one crate (`rustix`) at exactly one node (`process-model`).**

---

## 7. What this contradicts, and what needs a new node

### Stale in `docs/idiomatization.md`

* **§1.6 on `nshedit`.** It says the dependency is a path outside the
  repository. It is now a git dependency (`crates/nsh/Cargo.toml`), and
  `nplan_tree` already carries `nshedit-in-tree` — "the git dep only
  fixes clone-and-build". §1.7's P10 ("no out-of-repo path deps") should
  be restated as "no unpublished deps", because a git dep still blocks
  `cargo package`. Add `rust-version` while that node is open.
* **§1.6 and §2.3 step 0 on the crate split.** Both are done;
  `crates/nsh` and `crates/nsh-cli` exist, `crate-rename` is `doing`.
* **§2.3 step 4's characterisation of the hash tables.** It says they
  "want to be `HashMap<BString, _>`". §5.1 measures that they do not.
  This is the one place where this document disagrees with that one on
  substance rather than currency.
* **§2.3 step 2's `gen/` argument** mentions "a `const fn` or a
  `build.rs`". There is no `build.rs` in the workspace and `syntax.rs` /
  `signames.rs` are committed source, so `gen/` is already off every
  build path — which strengthens the case rather than weakening it.

### Against a decision, or against a stated property

* **P9 ("two shells in one process are independent") cannot be made true
  for the locale.** `var.rs:103-106 changelocale` calls `libc::putenv`
  and `libc::setlocale(LC_ALL, "")` in response to an assignment to
  `LC_ALL`, `LC_COLLATE` or `LC_CTYPE` (`var.rs:181-193`). Both are
  process-global. Because `strcoll` (§5.2), the ctype functions (§5.5)
  and `mbrtowc` (§5.4) all read that global, one `Shell` setting
  `LC_COLLATE` changes how another sorts a glob. This is a property of
  the C library, not of the refactor, and no amount of `move-state` fixes
  it. It belongs on [dec:nsh:no-ambient-state] as a recorded limit, and
  P9's check needs an exception clause naming `LC_*` — otherwise the
  property is stated, believed, and false.
* **Two more libc globals of the same class.** `libc::strtok` in
  `cd.rs:218,237` (§3 step 3) and `libc::getopt` + `optind = 0` in
  `histedit.rs` (§4.14). Both are process-global parser state inside what
  is becoming a library. Neither is in P1's `static mut` count, because
  the static is in libc.
* **`libc::putenv` gives libc a pointer into shell-owned storage**
  (`var.rs:104`; glibc's `putenv` does not copy). `exec.rs:125,171`
  builds its own `envp` and never reads `environ`, so the only consumer
  is `setlocale` — meaning the `putenv` can go when the locale question
  is answered, and not before.
* **`trap.rs:331 onsig` unwinds out of a kernel signal frame** (§4.11).
  Documented at the site (`trap.rs:315-330`), absent from every
  cross-cutting document. It belongs on [dec:nsh:errors-are-values] as an
  additional argument for that step — it is why `panic = "unwind"` is
  pinned in both profiles — and on [dec:nsh:minimal-unsafe] as the one
  `unsafe` that is not a syscall wrapper.
* **The differential harness does not pin the locale**
  (`tests/harness/sandboxed.sh:55-70`). Not a contradiction of anything
  written down, because nothing is written down — which is the point.
  [dec:nsh:differential-is-the-oracle] says the harness "tests exactly
  one configuration"; the locale is a second axis of that configuration
  and it is currently set by whoever types the command.

**Nothing here contradicts a decision's substance.**
[dec:nsh:owned-data]'s deferred consequence — "the syscall surface stays
… whether they go behind `std` or a safe wrapper is a later, smaller
question" — is answered by §6: mostly `libc`, and the one place a wrapper
would add capability is §4.9.

### The node that does not exist

Three entries in the ledger (§4.1 `system.rs`, §4.3 `show.rs`, and the
empty `extern "C"` block at `error.rs:44-50`) are deletions of
unreachable code, and none attaches anywhere. `delete-gen` is the same
*kind* of work — "remove what is dead" — and its title already says so.

*Proposed:* widen `delete-gen` to `delete-dead-code`, covering `gen/`
(2,006 lines), `show.rs` (442), the twenty unreferenced `system.rs` items
(~200 body + ~250 test), `mystring.rs:101-120 scopyn` (`#if 0` in the C),
and the two dead stubs (`error.rs:44-50`, `error.rs:386-391` —
`#ifdef REALLY_SMALL`). One node, ~2,900 lines, R0 throughout. The only
cost is the spec-rule transaction in §4.1, which is a reason to do it as
one deliberate act rather than five incidental ones.

---

## 8. What this document is not sure about

1. **The ctype/locale divergence (§5.5) is argued, not measured.** Only
   `en_US.utf8` is generated on this box and a UTF-8 locale hides the
   difference. *Resolved by:* generating a single-byte locale and running
   the two-line probe in §5.5. If the divergence is not real,
   `is_alpha`/`is_name`/`is_in_name` become `is_ascii_*` and §5.5 leaves
   the trap list.

2. **§4.2's "42 lines survive" assumes the printf builtin's
   single-conversion property holds for every arm.** `%b` goes through
   `print_escape_str` (`bltin/printf.rs:174-226`), which rewrites the
   conversion character twice and calls `ASPF!` on the result; that path
   was read, not run. *Resolved by:*
   `printf '%-5b|%5.2b\n' 'a\tb' 'xy'` against both shells.

3. **Whether any corpus case observes the orders in §5.1.** The
   behaviour is measured and certain; whether the 61,498 cases *reach*
   it is not. If none does, the hash-table swap can land before
   `sanctioned-divergences` under [dec:nsh:we-own-the-defects]'s rule.
   *Resolved by:* `grep -l 'env$\|^env \|alias$\|^hash$' tests/corpus/*`
   and re-running those cases only.

4. **Whether `system.rs`'s 91 spec rules can be retired or must be
   migrated.** The port rules treat a `#ifndef HAVE_…` arm as a contract
   on the target. Deleting the code either retires the rule or leaves it
   uncovered, and which is right is a question about
   `docs/spec/port/src/system.md`'s intent, not about the code.
   *Resolved by:* reading that file and deciding whether "the target must
   supply `mempcpy`" is a claim anyone still needs asserted.

5. **Whether `savefd`'s `EBADF`-as-success (§4.9 item 2) is reachable
   from the language.** It is plainly deliberate in the C, but the path
   that produces a closed `from` descriptor was inferred, not traced.
   *Resolved by:* `dash -c 'exec 9>&-; { :; } 9>&1'` under `strace`.

6. **The risk weights in §2 are a ranking device, not a measurement.**
   R0=1/R1=2/R2=6/R3=20 was chosen so that a 400-line R0 deletion
   outranks a 40-line R1 one, which is the brief's stated preference. Any
   monotone weighting gives the same top five; the middle of the table is
   sensitive to the choice.
