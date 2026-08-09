# Errors are values: the fixpoint, what the unwind is doing, and the order

Status: analysis and specification for the `errors-are-values` node, which
is not startable yet. Nothing here is implemented and no file under
`crates/` was touched. It builds on `[dec:nsh:errors-are-values]`,
`docs/api-design.md` §3 (which settled the taxonomy and the `set -e`
question) and `docs/idiomatization.md` §2.3 step 7.

Every number was measured on `faa2253` in the `wt/eav-prep` worktree, with
the method given in §1.1 so the claims can be re-run and can rot visibly.
Where a number is compared against `docs/idiomatization.md`'s, it was
re-measured on that document's own tree (`4463fc6`) as well, so the
comparison is like for like.

---

## 0. What this found

1. **The fixpoint is 296 of 626 functions, not 420 of 720**, and the
   earlier figure is not reproducible by any reading of the tree it was
   taken on. §1.
2. **The `raise ⊆ memalloc` claim in `docs/idiomatization.md` §2.1 is
   false.** On `4463fc6` there are 25 functions on a raise path that never
   reach the region allocator; today there are 27. The ordering conclusion
   survives; the argument given for it does not. §1.4.
3. **The crate has exactly one `impl Drop`, and it is inside
   `#[cfg(test)]`** (`output.rs:805`). Every destructor the unwind runs
   today is a memory-only `Vec`/`BString`/`Box`/`Rc` drop, and `?` runs
   those identically. Nothing observable is being done by `Drop`. §2.1.
4. **Every observable cleanup on the exception path is already explicit**,
   and it is not in the raising frame — it is in the catching one, keyed on
   marks rather than on frame nesting. §2.2 is the inventory; it includes
   two file descriptors (`eval::tpip`), the whole saved-descriptor stack,
   the input stack, the local-variable stack, and a temporary file
   `histcmd` unlinks.
5. **The interrupt does not need a non-local mechanism**, and the reason is
   already in dash: `setsignal` sets `sa_flags = 0` (`trap.rs:288`), so no
   syscall restarts, and two EINTR sites already consult `pending_sig`
   before retrying. §4.
6. **44 of the 296 are on the raise path only because `INTON` can call
   `onint`** — and they are overwhelmingly the *cleanup* functions
   (`popstackmark`, `popredir`, `unwindredir`, `popfile`, `unwindfiles`,
   `popallfiles`, `freejob`, `exitreset`, `forkreset`, `clear_traps`). A
   design in which `INTON` returns `Result` makes the shell's teardown
   fallible, which is the wrong shape. §4.3.
7. `no-ambient-state`'s recorded reason for being downstream is true but is
   not the load-bearing one, and it is escapable. The load-bearing reason
   is different and stronger. §3.1.

---

## 1. The exact fixpoint

### 1.1 How it was measured

`docs/idiomatization.md` §2.1 reports "420 of 720 transitively on a raise
path" from "a name-based call graph" that "resolves by identifier, so
same-named functions in different modules merge", and §7.1 flags that as
the weakest number in the document. This redoes it.

The tool is a `syn`-based resolver, built under the scratchpad with its own
`CARGO_TARGET_DIR` (no `cargo build` was run in this worktree). It parses
every `.rs` file under `crates/nsh/src` into a module keyed by its path, and
then:

* **Resolves by module, not by name.** A single-segment call resolves
  against the enclosing module's own items and then against that module's
  `use` aliases; `crate::m::f` resolves to `m::f`; `m::f` resolves when `m`
  is a known module. Same-named functions in different modules stay
  distinct — there are four `iflag()` shims alone (`eval`, `shellmain`,
  `histedit`, `jobs`).
* **Expands `macro_rules!`.** This matters more than the module scoping.
  `bltin/mod.rs:123`'s `error!` is dash's `#define error sh_error`, and six
  raise sites in `bltin/` spell it that way; `error.rs`'s `sh_error!`,
  `exerror!`, `INTOFF!`, `INTON!`, `FORCEINTON!` and `RESTOREINT!` are the
  rest. Macro bodies are scanned for callable paths and for nested macro
  names, resolved in the *defining* module, and closed transitively.
  Handling macros moved the fixpoint from 278 to 296.
* **Handles statement-position macros.** `sh_error!(..);` parses as
  `Stmt::Macro`, not `Expr::Macro`; visiting only the latter loses raise
  sites.
* **Is conservative about function values.** Any path expression naming a
  known function is an edge, so `let mut evalfn: unsafe fn(&Node, c_int) ->
  c_int = evalcommand;` (`eval.rs:273`) links `evaltree` to `evalcommand`
  even though the call is indirect. Same for `jobs.rs:880`'s `matchfn` and
  `expand.rs:1281`'s `scan`.
* **Adds the two dispatch tables by hand.** `evalbltin`'s
  `((*cmd).builtin.unwrap())(argc, argv)` (`eval.rs:1183`) gets an edge to
  each of the 39 entries in `builtins.rs`'s table; `var::varfunc`
  (`var.rs:407`) gets one to each `func: Some(..)` in `varinit`.
* **Excludes** `api.rs` (behind the `api-sketch` feature, all bodies
  `todo!()`), `testutil.rs` (`#[cfg(test)]`) and every `mod tests`.

Seeds are the five functions that actually raise: `error::exraise`,
`error::sh_error`, `error::exerror`, `error::exverror`,
`error::raise_longjmp`. `error::onint` is not a seed because it *calls*
`exraise` and is therefore found; §4.3 measures its contribution by cutting
that edge instead.

The fixpoint is reverse reachability from the seeds, stopping at the seven
functions that arm a handler — a frame that catches does not propagate what
it caught. Those seven are found by looking for `setjmp_catch` in the body:
`eval::evalbltin`, `eval::evalfun`, `histedit::histcmd`, `parser::expandstr`,
`redir::redirectsafe`, `shellmain::main`, `trap::exitshell`. That matches
the eight `setjmp_catch` call sites `docs/idiomatization.md` names, because
`trap::exitshell` has two (`trap.rs:447` and `:471`).

**The one gap, and why it is closed.** Method calls (`n.ncmd()`,
`node.as_ptr()`) are not resolved — the tool records them and does not
follow them. That cannot change the answer: no `impl` method anywhere in
the crate is in the fixpoint, so no missing method edge can add a caller to
it. Checked directly, by intersecting the fixpoint with the set of
`Type::method` keys: empty.

### 1.2 The numbers

| | functions | in the fixpoint |
|---|---|---|
| today (`faa2253`) | 626 | **296** (47%) |
| … if the interrupt keeps a non-local mechanism | 626 | 252 |
| … after `delete-memalloc` finishes | 600 | 261 |
| … both | 600 | **197** (33%) |

296 includes the seven catch frames, which need to *handle* a `Result` but
whose own signature need not become fallible — except `evalbltin` and
`evalfun`, which already return the caught value for `evalcommand` to
re-raise (`eval.rs:1096-1108`).

Per module, at the two ends of that range:

```
module        total   fixpoint   without the interrupt
expand           73         26        20
jobs             37         26        24
parser           50         26        26
eval             30         21        21
input            33         21        11
var              52         18        15
memalloc         26         18        17
output           29         16        14
bltin            37         15        15
exec             27         15        12
redir            17         14        11
options          13         11        11
error            17         11         8
arith_yacc       12         10        10
trap             14          9         3
shellmain        13          7         7
cd                8          6         6
alias            10          6         4
mystring         15          5         5
miscbltin         5          5         5
init              5          5         2
histedit         16          4         4
mail              2          1         1
linedit          21          0         0
nodes            31          0         0
syntax           12          0         0
streams           7          0         0
system            6          0         0
shell             6          0         0
arith_yylex       2          0         0
```

Six modules are entirely off the raise path and stay infallible:
`nodes`, `linedit`, `syntax`, `streams`, `system`, `shell`, plus
`arith_yylex` — the arithmetic lexer returns `ARITH_BAD`
(`arith_yylex.rs:197`) and `arith_yacc::yyerror` (`arith_yacc.rs:134`) is
what raises.

### 1.3 The raise sites

| form | sites |
|---|---|
| `sh_error!(..)` | 30 |
| `error!(..)` — `bltin/mod.rs:123`, dash's `#define error sh_error` | 6 |
| `crate::error::sh_error(..)` direct | 24 |
| `exerror!(..)` — one, `shellexec` (`exec.rs:158`) | 1 |
| `exraise(..)` outside `error.rs` | 3 |
| `raise_longjmp(handler, 1)` re-raise | 5 |

61 of those write a diagnostic and raise; three raise without text
(`shellmain.rs:506` `EXEXIT` from `exitcmd`, `eval.rs:431` `EXEND` from
`evaltree`'s `set -e` / `EV_EXIT` path, `eval.rs:1134` `EXERROR` after a
redirection error in a special builtin); five re-raise something already
reported (`eval.rs:1100`, `:1107`, `expand.rs:2807`, `:3415`,
`histedit.rs:733`).

`docs/api-design.md` §3.4's arithmetic — "33 `sh_error!` macro sites plus
25 direct calls plus one `exerror!`" — is close and slightly stale; the
`error!` alias is the piece it does not count.

### 1.4 The earlier number is wrong, and so is the claim built on it

Two things are being compared, so both were measured on `4463fc6`, the tree
`docs/idiomatization.md` states its numbers on. That tree still had `gen/`,
`show.rs` and `main.rs` in the crate; excluding `gen/` and `main.rs`, as
that document says it did, it has 641 functions by this tool's count and
735 by a plain textual count of `fn` definitions. 720 is consistent with a
textual count; 641 is what a resolver sees.

```
                                   idiomatization §2.1     re-measured on 4463fc6
total functions                          720                      641
transitively on a raise path             420                      325   (321 with catches)
transitively reach memalloc              424                      313
raise \ memalloc                           0                       25
```

The 420 is not reproducible. A deliberately crude reconstruction — regex
for `identifier(`, attribute to the enclosing `fn` by brace counting, merge
by bare name, which is the shape §2.1 describes — gives 321 from the raise
seeds, 322 if `INTOFF` is treated as a raiser, and 351 if *every* function
in `error.rs` is seeded. None of those is 420, and none is 424 for the
allocator. Name-merging is not the cause either: merging this tool's own
resolved edges by bare name adds zero functions to the set. So the earlier
figure came from something other than the graph as described, and the
honest statement is that its provenance is unknown and its value should not
be carried forward.

**The claim that rests on it does not survive.** §2.1 says:

> the set of functions on a raise path is a strict subset of the set that
> touches the allocator. There is no function anywhere in the crate that
> `errors-are-values` would touch and `delete-memalloc` would not.

On `4463fc6` there are 25 such functions, and they are not marginal:

```
bltin::test::testcmd, ::primary, ::binop, ::aexpr, ::oexpr, ::nexpr, ::getn
arith_yacc::yyerror, ::do_binop
error::INTON, ::FORCEINTON, ::__inton, ::onint, ::exraise, ::raise_longjmp
expand::ifsfree, ::removerecordregions, ::opendir_interruptible,
        ::restore_handler_expandarg
input::flush_input, ::pushstdin, ::reset_input, ::mkinit_postexitreset
init::postexitreset
jobs::sprint_status, output::fmtstr, output::xvsnprintf, system::strsignal
trap::onsig, ::setsignal, ::setinteractive, ::mkinit_init
var::lookupvar, ::bltinlookup, cd::cdopt
```

`bltin/test.rs`'s recursive-descent evaluator is the clearest: it raises
through `error!` and through `mystring::atomax10` → `atomax` → `badnum`, and it never
touches the region — `grep -n 'memalloc\|stalloc\|ckmalloc' bltin/test.rs`
matches one comment and no code. Today the same figure is 27.

This does not reopen `[dec:nsh:owned-data]`. The overlap is 247 of 325
(76%) on that tree and 218 of 300 today, the sequencing has already been
executed, and the *conclusion* — data before errors — is right for the
reason the decision's rationale actually gives (owned values make the error
path smaller, and `*mut c_char` signatures would be rewritten twice). What
should be corrected is the sentence claiming a strict subset, because it is
a factual claim about this codebase and it is not true.

### 1.5 Where the fixpoint comes from

Cutting individual raise sources, with the catch boundaries honoured:

| cut | fixpoint | delta |
|---|---|---|
| none | 296 | — |
| `error::onint` (interrupt non-local) | 252 | −44 |
| `memalloc::outofspace` (allocation cannot fail) | 283 | −13 |
| `mystring::badnum` (`Illegal number`) | 290 | −6 |
| `output::xvasprintf` | 295 | −1 |
| all four | 168 | −128 |

The last row is the interesting one: the individual cuts sum to 64 and the
combined cut removes 128, because most of the deep utility layer reaches a
raise through *several* of these and needs all of them gone to become
infallible. `memalloc` (18 functions) and `output` (16) are on the raise
path almost entirely because `ckmalloc` and the string builder call
`sh_error("Out of space")`. Both disappear with `delete-memalloc`.

---

## 2. What unwinding is currently doing for us

This is the part the conversion can get wrong silently, so it is stated as
an inventory rather than a principle.

### 2.1 `Drop` is doing nothing observable, and that is checkable

```
$ grep -rn 'impl Drop\|fn drop(' crates/nsh/src
output.rs:805:    impl Drop for Sink {
output.rs:806:        fn drop(&mut self) {
```

One hit, and `output.rs:749` is `#[cfg(test)]` — `Sink` is a test fixture.
**The library defines no `Drop` impl at all.** Every destructor that runs
between a raise and its catch today belongs to `Vec`, `BString`, `Box<Node>`
or `Rc<Node>`, and does nothing but free memory. There is no `File`, no
`OwnedFd`, no guard type: descriptors are `c_int` and are closed by hand.

The consequence for the conversion runs the opposite way to the obvious
fear: **`?` runs the same destructors an unwind runs.** An early `return Err(..)` drops every local
of the returning frame, and the caller's `?` drops every local of its own,
all the way out. So the memory reclamation the unwind performs today is
preserved for free, and there is no list of `Drop`s to reproduce manually.

What is *not* preserved for free is everything between the `?` and the
catch that is not a local — and in this codebase that is everything that
matters, because the shell's resources are globals.

### 2.2 The manual cleanup, which is the real list

Each catch frame performs, by hand, what the unwind skipped. This is the
list that must keep happening, and none of it is `Drop`.

**`shellmain::main`, the top-level handler (`shellmain.rs:210-247`)**

| what | where | why it is observable |
|---|---|---|
| `init::exitreset()` | `shellmain.rs:214` | below |
| — restore `exitstatus` from `savestatus`; clear `savestatus`, `evalskip`, `loopnest`, `inps4` | `init.rs:67-78` | `$?` after a trapped exit |
| — `close(eval::tpip[0])`, `close(tpip[1])` | `init.rs:79-82` | **two file descriptors**, the command-substitution pipe left open by a raise between `sh_pipe` and `forkshell` (`eval.rs:775-779`) |
| — `expand::ifsfree()` | `init.rs:87` | frees the IFS region list; a stale region mis-splits the *next* word |
| — `redir::unwindredir(NULL)` | `init.rs:91` → `redir.rs:525` | discards **every** saved descriptor: `popredir` `dup2`s each back and `close`s the save (`redir.rs:413-452`) |
| `init::reset()` (skipped when exiting) | `shellmain.rs:229` | below |
| — `input::popallfiles()` then drain input to the next newline | `input.rs:mkinit_reset` | the parse-file stack, and discarding the rest of the bad line |
| — `var::unwindlocalvars(NULL)` | `var.rs:mkinit_reset` | pops every `local` scope |
| `outcslow('\n', out2)` when `exception == EXINT` | `shellmain.rs:234` | a byte on stderr |
| `popstackmark(smark_p)` | `shellmain.rs:236` | §2.3 |
| `FORCEINTON()` | `shellmain.rs:237` | §2.4 |

**`trap::exitshell` (`trap.rs:439-478`)** — `exitreset()` then
`postexitreset()` (`init.rs:122-125` → `input::flush_input()`), then a
second `setjmp_catch` around `setjobctl(0)` so a raise inside the job-control
teardown cannot prevent the `_exit`.

**`eval::evalbltin` (`eval.rs:1192-1196`)** — `freestdout()`, restore
`commandname`, restore `handler`.

**`eval::evalfun` (`eval.rs:1236-1245`)** — `INTOFF`, restore `loopnest` and
`funcline`, `freefunc(func)` (the `Rc` decrement that pairs with the
`reffunc` inside the closure), `freeparam(&shellparam)`, restore
`shellparam` and `handler`, `INTON`, then `evalskip &= !(SKIPFUNC |
SKIPFUNCDEF)`.

**`redir::redirectsafe` (`redir.rs:506-521`)** —
`restore_handler_expandarg(savehandler, err)` then `RESTOREINT(saveint)`.

**`parser::expandstr` (`parser.rs:2293-2349`)** —
`restore_handler_expandarg`, restore `doprompt`, `unwindfiles(file_stop)`,
restore `heredoclist`.

**`expand::restore_handler_expandarg` (`expand.rs:3411-3421`)** is shared by
the two above and is the sharpest single item in this list:

```rust
crate::error::handler = savehandler;
if err != 0 {
    if crate::error::exception != crate::error::EXERROR {
        crate::error::raise_longjmp(crate::error::handler, 1);
    }
    ifsfree();
}
```

It restores the handler, **re-raises anything that is not `EXERROR`**, and
otherwise frees the IFS regions. In a `Result` design this is a `match` on
the error kind with an explicit `return Err(e)` arm, and the `ifsfree()` has
to stay on the swallowing arm only.

**`histedit::histcmd` (`histedit.rs:727-735`)** — `active = 0`,
**`unlink(editfile)`**, restore `handler`, re-raise. Deleting the temporary
file is the only filesystem side effect on any catch path.

**`eval::evalcommand`'s `out:` (`eval.rs:1140-1152`)** — `popredir`,
`unwindredir(redir_stop)`, `unwindfiles(file_stop)`,
`unwindlocalvars(localvar_stop)`, `setvar("_", lastarg)`. This runs on the
normal return and after a *swallowed* builtin error, and is skipped by an
unwind past the frame — which is exactly why the top-level `exitreset` does
the same work with a `NULL` stop.

That last point is the structural observation: **the unwind functions are
keyed on a saved mark, not on frame nesting, and they are idempotent.**
`unwindredir(stop)` pops until `redirlist == stop`; `unwindfiles(stop)`
until `parsefile == stop`; `unwindlocalvars(stop)` likewise. A `?` that
returns through such a frame skips its epilogue, exactly as the unwind
does, and the outer frame's unwind-to-mark still cleans up. So this whole
family converts unchanged, and it converts *safely*: the failure mode of
getting it wrong is a leak that the outer mark then reclaims, not a
double-free.

### 2.3 `setstackmark` / `popstackmark`

Live pairs outside `memalloc.rs`'s own tests:

| pair | file:line | crossed by a raise? |
|---|---|---|
| `evalstring` | `eval.rs:222` / `:250`, `:252` | yes |
| `evaltree` | `eval.rs:278` / `:426` | yes — and `:431`'s `exraise(EXEND)` is **after** the `popstackmark`, so the `set -e` and `EV_EXIT` paths deliberately do not pop |
| `main` startup | `shellmain.rs:140` / `:169`, and `:236` in the handler | yes |
| `cmdloop` | `shellmain.rs:307` / `:344` | yes |
| `mail` | `mail.rs:33` / `:80` | yes |
| `expandarg` | `expand.rs:3342` / `:3352` | yes |
| `expari` | `expand.rs:1051` / `:1053` (`pushstackmark(.., 0)`) | yes |
| `expbackq` | `expand.rs:1093` / `:1095` (`pushstackmark(.., 0)`) | yes |
| `preadfd` around `el_gets` | `input.rs:455` / `:457` | yes |
| `setprompt` around `out2str` | `parser.rs:2286` / `:2288` | yes |
| `sstrdup` helpers | `mystring.rs:448`/`:463`, `:473`/`:480` | yes |

**Is `?` returning through them equivalent to the unwind?** For releasing
the region, yes: both skip the `popstackmark`, and the enclosing handler's
`popstackmark(smark_p)` (`shellmain.rs:236`) releases to the outermost mark.
For `evaltree` it is *not* equivalent unless the placement is preserved:
`eval.rs:426` pops and returns, `eval.rs:431` raises **without** popping.
Turning that raise into `Ok(Flow::Exit)` puts a normal return where a
non-local jump was, and a naive rewrite runs the `popstackmark` on a path
the C never runs it on — releasing the region under a caller that may still
hold a pointer into it.

That is a reason, independent of the ones already recorded, why
`errors-are-values` must not start before `delete-memalloc` finishes.
`[dec:nsh:owned-data]`'s closing section says `memalloc.rs` survives that
node in two pieces (`struct strlist`, and the checked-malloc wrappers with
the hash tables). While `setstackmark` still exists, the `Flow` conversion
has to reason about mark placement on every converted return; once it is
gone, the question disappears.

### 2.4 `INTOFF` / `INTON`, and the counter

82 `INTOFF` and 86 `INTON` references across 17 files. Only **three**
`FORCEINTON` call sites (`shellmain.rs:237`, `eval.rs:781`, `eval.rs:1126`)
and exactly **one** `SAVEINT`/`RESTOREINT` pair (`redir.rs:513`/`:519`),
matching the C (`main.c:132`, `eval.c:654`, `eval.c:937`,
`redir.c:496`/`:502`).

What happens on an unwind today:

* `exraise` does `INTOFF()` **before** raising (`error.rs:220`), so the jump
  is atomic against SIGINT.
* The unwind skips every `INTON` between the raise and the catch. The
  counter is left arbitrarily high.
* `shellmain::main`'s handler discards the leak with `FORCEINTON()` —
  `suppressint = 0` unconditionally, then `onint()` if one is pending
  (`error.rs:129-136`). It does not balance the counter; it resets it.
* `redir::redirectsafe` restores the *exact* saved value instead
  (`RESTOREINT`), because it is a catch that returns to the middle of
  `evalcommand` rather than to a top level.
* `evalbltin` and `evalfun` do **neither**. So an `EXERROR` from a
  non-special builtin — the case `eval.rs:1096-1097` deliberately swallows —
  returns to `evalcommand` with `suppressint` one higher than it started.
  That is dash's behaviour, not a port artefact (`eval.c`'s `cmddone` label
  restores `commandname` and `handler` and nothing else), and it must be
  reproduced.

What must happen on an early return: **the same thing, and no more.** A `?`
through an `INTOFF`…`INTON` bracket leaks the counter exactly as the unwind
does, and the same three `FORCEINTON`s and one `RESTOREINT` clear it at
exactly the same points. The hazard is the opposite of the obvious one: an
implementer who "fixes" the leak by pairing `INTOFF`/`INTON` with a guard
type changes *which instruction a pending SIGINT is delivered at*, which is
observable. `[dec:nsh:owned-data]` already recorded this rule once, for
`recordregion`:

> The INTOFF/INTON brackets are kept where the C has them … They are not
> protecting the list any more; they are fixing the instruction at which a
> pending SIGINT is delivered, and that is not this commit's to move.

It is not this node's to move either.

### 2.5 The two paths that correctly skip destructors

**`exraise` under `vforked` (`error.rs:215-218`)** calls `_exit` before it
does anything else. `vforkexec` (`jobs.rs:1270-1280`) sets `vforked` around
a `vfork()`, and the child runs `forkchild` and then `shellexec`
(`exec.rs:118`), which raises `EXEND` if every `execve` fails
(`exec.rs:158`). The child **shares the parent's address space and stack**.
Unwinding there would run destructors on the parent's objects and return
into the parent's frames.

For a `Result` design this is not a wrinkle, it is a hard boundary:
**`shellexec` cannot return `Err` to `vforkexec` in the child.** The `Err`
would propagate up through frames the parent also owns. So the vforked arm
of the raise must remain a terminating operation at the raise site, and the
right shape is for `report`-and-`_exit` to be the *first* thing the error
constructor does, before it ever becomes a value:

```rust
// still diverging, still first, still not a value
if jobs::vforked != 0 { flush_coverage(); libc::_exit(eval::exitstatus); }
```

This is also the one surviving `_exit` that `[dec:nsh:public-surface]`
records as "forced", and it is the reason `P8`'s target is "0 outside a
forked child" rather than 0.

**`shellexec` reaching `execve`** is the second, and it is milder: the
address space is replaced, so nothing is skipped that anyone can observe.
It does impose one requirement that the current code already meets and a
refactor could break — `shellexec`'s `envv: Vec` (`exec.rs:126-128`) must
outlive every `tryexec`, because `envp` points into it. A `?` inserted
between the `environment()` call and the `execve` would drop it.

### 2.6 Two hazards the inventory turned up

**`dotcmd`'s buffer is freed by the unwind while `commandname` still names
it.** `shellmain.rs:456-486`: `dotcmd` holds `dotfile: Vec<u8>`, points
`eval::commandname` at its bytes, runs `cmdloop(0)`, and on the normal
return *moves* the `Vec` into a static slot (`dotfile_kept`,
`shellmain.rs:398`) so the pointer outlives the frame — because
`evalbltin`'s epilogue reads `commandname` before restoring it. On the
exception path the move is skipped and the `Vec` is dropped by the unwind,
leaving `commandname` dangling until `evalbltin` restores it
(`eval.rs:1194`). It is safe today only because nothing reads `commandname`
in that window: `exvwarning2` (`error.rs:278-291`) reads it to build the
`sh: 1: ...: ` prefix, but that runs *before* `exraise`, while the buffer is
still alive. A `?` reproduces this exactly — and it is the sort of thing
that stops being true if the diagnostic ever moves after the raise, which
is precisely what `[dec:nsh:errors-are-values]` forbids for a different
reason. Both constraints point the same way.

**`evalfun`'s `Rc` balance depends on where the raise can happen.**
`eval.rs:1226`'s `reffunc(func)` is inside the closure; `eval.rs:1240`'s
`freefunc(func)` is in the epilogue, which runs on both paths. That is
balanced only because nothing between the closure's entry and `reffunc` can
raise. The `INTON()` at `eval.rs:1229` can, and it is after. A conversion
that reorders the prologue turns this into a use-after-free that only shows
up when a function redefines itself while running — the failure mode
`[dec:nsh:owned-data]` already recorded for `funcnode.count`.

---

## 3. Order and staging

### 3.1 Is `no-ambient-state` really blocked on this?

The recorded reason (`docs/api-design.md` §5.1, `docs/idiomatization.md`
§2.3 step 7) is that `error.rs:89`'s `handler: *mut jmploc` points into a
live stack frame and a pointer into a frame cannot be a field of a `Shell`.

The pointer targets are real. Six sites install a caller-local `jmploc`:
`parser.rs:2324`, `redir.rs:515`, `eval.rs:1176`, `eval.rs:1222`,
`trap.rs:450`, `histedit.rs:530`. Two install the static `main_handler`
(`shellmain.rs:133`, `init.rs:106`). `expand.rs:2609` saves the handler and
never installs one, bug-for-bug with the C.

**Confirmed as a fact, refuted as the load-bearing reason.** The pointer is
only ever compared, never dereferenced as a jump buffer — `setjmp_catch`
tests `lj.loc == loc` (`eval.rs:140`) and that is the whole of its use. A
`u64` handler id on `Shell`, incremented per armed scope, would serve
identically, so `no-ambient-state` *could* be done first at the cost of
inventing that id. Inventing it is `errors-are-values` work performed inside
`no-ambient-state`, which §2.2 of `docs/idiomatization.md` forbids for the
usual reason, but it is not a physical block.

Two better reasons, in order of strength:

1. **`catch_unwind` over a `&mut Shell` that is then reused is exactly what
   `UnwindSafe` exists to flag.** It compiles — the reborrow ends when
   `catch_unwind` returns — so the compiler's objection is delivered as a
   trait bound that has to be silenced with `AssertUnwindSafe` at seven
   sites, over a `Shell` whose tables may be half-updated at the point the
   panic was raised. §2.2's inventory is a list of invariants that are
   broken between the raise and the catch (`redirlist` mid-pop,
   `localvar_stack` mid-push, `parsefile` mid-unwind), so the assertion
   would be false in the precise technical sense the trait is about.
2. **Step size.** `no-ambient-state` is a 587-signature rewrite and
   `errors-are-values` is a 296-signature rewrite over an overlapping set.
   `[dec:nsh:differential-is-the-oracle]` records that the harness names the
   case, not the function, so a commit changing both has two candidate
   causes for every red case.

The recorded reason should be replaced by these rather than merely
supplemented, because as written it is falsifiable in one sentence and
someone will falsify it.

### 3.2 The staging

Every step keeps the harness green and changes one property. The adapter
that makes this possible is one function:

```rust
/// The bridge, deleted by the last step. Takes an already-reported error
/// and performs the legacy non-local jump. It does NOT write a diagnostic:
/// the value was reported when it was constructed.
pub(crate) unsafe fn raise_reported(e: Error) -> ! {
    eval::exitstatus = e.status();
    error::exception = e.exception_code();   // EXERROR / EXEND / EXEXIT / EXINT
    if jobs::vforked != 0 { shell::flush_coverage(); libc::_exit(eval::exitstatus); }
    error::INTOFF();
    error::raise_longjmp(error::handler, 1)
}
```

An unconverted caller of a converted function writes
`f(..).unwrap_or_else(|e| raise_reported(e))`, and the wavefront moves
outward from the raise sites toward the seven catch frames with a green
harness at every commit.

**A. The diagnostic funnel, before any signature changes.**
Introduce `pub(crate) enum Error` with `Other { line, status, message }`
only, `Error::message() -> BString`, and `report(e) -> Error` that does
exactly what `exverror` does minus the raise: `exvwarning2`'s prefix from
`arg0`/`errlinno`/`commandname`, the body, the newline, then `flushall()`
(`error.rs:324-326`). Re-route `exverror` through `report`. Nothing else
changes and no signature moves. This single commit is where the 61,498
cases decide whether the text and its interleaving with `stdout` survive —
including the two details `docs/api-design.md` §3.2 names: `errout` is
unbuffered (`output.rs:61-68`) so a diagnostic is three raw `write(2)`s, and
`flushall()` runs *after* the message so a builtin's buffered stdout appears
*after* its own diagnostic.

*Reversible. The highest-value commit in the node, and the cheapest to
bisect.*

**B. `raise_reported`, and `exraise` split.**
`exraise` keeps its `vforked` and `INTOFF` behaviour; `raise_reported` is
the same path taking an `Error`. Still no signature changes. Assert, in a
`debug_assert`, that no `Error` is reported twice.

**C. Leaf modules, cheapest first.** Ordered by how many cross-module
fixpoint edges point *into* the module, which is the number of adapters the
commit needs:

```
module       fixpoint fns   cross-module callers
mail                 1            1
arith_yacc          10            1
histedit             4            2
miscbltin            5            3
cd                   6            3
alias                6            4
shellmain            7            4
bltin               15            4
eval                21            6
parser              26            6
init                 5            7
trap                 9            8
mystring             5           10
exec                15           11
expand              26           15
redir               14           17
jobs                26           22
options             11           23
var                 18           26
input               21           37
memalloc            18           47
output              16           62
error               11          107
```

`memalloc` and `output` are at the bottom for a reason that removes them:
they are on the raise path almost entirely through `outofspace` and
`xvasprintf`, and `delete-memalloc` deletes both. If this node starts after
that one finishes, those 34 functions never need converting (§1.2's 261).

**D. The catch frames.** Seven `setjmp_catch` calls become a `match` on a
`Result`. Each is its own commit, because each carries a different epilogue
from §2.2. `redir::redirectsafe` and `parser::expandstr` share
`restore_handler_expandarg` and should land together.

**E. `Flow` out of the `Err` position.** `EXEND` and `EXEXIT` become
`Flow::Exit`, `evalskip`'s four bits become `Flow::Break/Continue/Return`,
and `EV_TESTED`/`EV_EXIT` become `EvalCtx` per `docs/api-design.md` §3.5.
This is the step that has to answer `docs/api-design.md` §10.2 — whether
`EXEND` and `EXEXIT` differ in anything but which status is taken — by
reading `init.rs:64-92`'s `exitreset` and `shellmain.rs:219-227`. Do that
reading *before* writing `Flow`, as that document asks.

*This is also the step that must not run while `setstackmark` exists,
per §2.3.*

**F. The interrupt.** §4.

**G. Deletion, and the pins come off.** `error::Longjmp`,
`raise_longjmp`, `raise_reported`, `setjmp_catch`, `jmp_buf`, `jmploc`,
`handler`, `exception`, the panic hook at `crates/nsh-cli/src/main.rs:116-133`,
and `panic = "unwind"` from both profiles in the workspace `Cargo.toml`.
The node is complete when
`grep -rn 'catch_unwind\|panic_any\|resume_unwind' crates/nsh/src` is empty
and the harness passes with `panic = "abort"`.

### 3.3 Proposed WBS structure

`errors-are-values` currently has no children (`plan/main.styx:1964-1969`,
`deps (delete-memalloc sanctioned-divergences public-api-design)`). Seven,
matching §3.2:

```
errors-are-values
  error-value-and-report     A — the Error type and the diagnostic funnel
  raise-adapter              B — raise_reported; exraise split
  fallible-leaves            C — modules in the order above, one per commit
  catch-sites                D — the seven setjmp_catch frames
  control-flow-is-not-error  E — Flow, EvalCtx; deps: delete-memalloc complete
  interrupt-is-polled        F — §4; deps: catch-sites
  delete-longjmp             G — deletion and the profile pins
```

Applying this is one `nplan_add` run and has not been done; the node is not
startable and adding children would change another agent's frontier.

---

## 4. The interrupt

### 4.1 The mechanism, exactly

`trap::onsig` (`trap.rs:331-352`) is `extern "C-unwind"` and, for an
untrapped SIGINT:

```rust
gotsig[signo - 1] = 1;
pending_sig = signo;
if signo == SIGINT && trap[SIGINT].is_null() {
    if error::suppressint == 0 { error::onint(); }   // does not return
    error::intpending = 1;
}
```

So there are **two delivery modes**, and only one of them is asynchronous:

* **Asynchronous.** `suppressint == 0` at the moment of delivery: `onint()`
  runs inside the signal handler and unwinds out of it. `error.rs:250-263`
  clears `intpending`, unblocks the mask, and — unless this is an
  interactive root shell — sets `SIGINT` to `SIG_DFL` and `raise`s it, so
  the process dies there and `exraise(EXINT)` is never reached. The `EXINT`
  unwind is therefore reachable **only in an interactive root shell**.
* **Deferred-synchronous.** `suppressint != 0`: the handler sets
  `intpending` and returns. The next `INTON` or `FORCEINTON` that brings
  `suppressint` to zero calls `onint()` from ordinary shell code
  (`error.rs:118-136`).

The second mode needs no non-local mechanism at all: it already happens at a
point the shell chose.

### 4.2 The answer: no, and dash says why

**The interrupt does not need to keep a non-local mechanism**, provided two
things, both of which the codebase already half-does.

**(i) `onsig` stops calling `onint` and only stores.** This is
`[dec:nsh:host-owns-signals]`'s `SignalSink` (`docs/api-design.md` §5.3) —
the handler does one relaxed store and returns, which is
async-signal-safe by construction. `error::intpending` and
`trap::pending_sig` are already exactly that inbox; what changes is that
nothing is *called* from the handler.

**(ii) Every blocking syscall's `EINTR` path polls the inbox instead of
retrying blindly.** The reason this works, and the reason it is not a
guess, is `trap.rs:288`: `act.sa_flags = 0`. dash never sets `SA_RESTART`,
so **every** interruptible syscall the shell makes returns `EINTR` when a
signal arrives. There is always a synchronous point at which to notice.

dash already uses this idiom, in two of the five `EINTR` sites:

```
redir.rs:181   open64   retry only while  pending_sig == 0
input.rs:493   read     retry unless      basepf.prev != NULL && pending_sig != 0
```

The other three retry unconditionally and are where the work is:
`output.rs:542` (`write`), `jobs.rs:1516` (`wait3`) and `expand.rs:1113`
(`read` from a command substitution). Each becomes "if an interrupt is
pending, return `Err(Error::Interrupted(SIGINT))`; otherwise retry" — three
sites, all mechanical, all in code the corpus reaches. (`exec.rs:474` and
`:598` carry the C's `/* SYSV: retry on EINTR */` comment and no retry, in
both languages; they are not `EINTR` sites.)

What this costs, stated rather than smoothed over:

* **The delivery point moves for the asynchronous case.** Today an
  untrapped SIGINT during a blocked `read` in `preadfd` unwinds out of the
  handler and abandons the read at the instruction it arrived. Polled, it
  abandons the read at the `EINTR` return — one syscall later, no
  observable difference, because the handler's own `longjmp` also lands
  after the kernel has returned from the signal frame.
* **`onsig` stops needing `extern "C-unwind"`.** That is a property worth
  having on its own: the comment at `trap.rs:315-330` records that this was
  a real bug (`SIGABRT`, status 134, on `kill -INT $$`) and that the fix
  depends on the unwinder walking a kernel signal frame through
  `__restore_rt`'s CFI. Depending on that is not something a library should
  ask of an embedder.
* **`error.rs:250-263`'s `SIG_DFL`-and-`raise` moves to `nsh-cli`.**
  `docs/api-design.md` §3.4 already says so; it is the fifth `P8` site that
  `docs/idiomatization.md` §1.7 does not count, because it is `raise` rather
  than `_exit`.
* **`P2` becomes achievable in full.** If any part of the interrupt stayed
  an unwind, `panic = "abort"` would still break it, and the decision's
  headline consequence — "the Cargo profile constraint goes away" — would be
  false in the one case a user is most likely to hit.

### 4.3 Why the alternative is worse than it looks

Making `INTON` return `Result` — the shape that falls out of treating the
interrupt as an ordinary error — costs 44 functions in the fixpoint (296 vs
252). That is not the problem. The problem is *which* 44:

```
memalloc::popstackmark   redir::popredir      redir::unwindredir
input::popfile           input::popallfiles   input::unwindfiles
input::freestrings       input::flush_input   input::popstring
init::exitreset          init::forkreset      init::postexitreset
trap::clear_traps        jobs::freejob        expand::ifsfree
expand::removerecordregions   exec::clearcmdentry   exec::delete_cmd_entry
alias::rmaliases         var::lookupvar       ...
```

These are the teardown functions from §2.2 — the ones the *error* path
calls. A design in which `unwindredir` can fail with an interrupt while
unwinding an error is a design in which cleanup is fallible, and every call
site has to decide what to do with an error raised while handling an error.
`error.rs`'s existing answer is better and should be kept: `INTOFF` around
the mutation, and delivery at a point of the shell's choosing.

So the recommendation is narrow and concrete: **`INTON` stays infallible and
non-raising; `onint` stops being reachable from it.** `INTON` decrements
the counter and, when it reaches zero with an interrupt pending, leaves
`intpending` set for the next poll site. The poll sites are the five `EINTR`
returns above plus `dotrap` (`trap.rs:361`), which is already called from
`evaltree`'s `out:` (`eval.rs:415`) on every command.

**This is a behaviour change and it needs the divergence register.** Today
`INTON` delivers the interrupt at the instruction where the counter reaches
zero; polled, it is delivered at the next poll site. In an interactive shell
that difference is a few instructions and is unobservable; the corpus cannot
see it at all (§5). It should be entered in the sanctioned-divergence
register with that reasoning rather than assumed invisible.

---

## 5. What the harness cannot see

`[dec:nsh:differential-is-the-oracle]` records that the harness runs one
configuration and therefore has zero coverage of every axis this work adds.
For this node specifically:

| what the harness does cover | how |
|---|---|
| Every diagnostic's bytes and its interleaving with stdout | `dscase.sh:64-71` merges the streams with `2>&1`, 61,498 cases |
| Which diagnostics abort the run and which do not | the same, via `$?` and subsequent output |
| `exit`, `return`, `break`, `continue`, `set -e` | the same |
| A *trapped* SIGINT (`trap 'echo INT' INT; kill -INT $$`) | `tests/corpus/curated_signals.txt:3` |

| what it cannot see | why | the test it owes |
|---|---|---|
| The `Error` value itself — variant, `status()`, which raise site produced it | the harness compares bytes on a stream, and the value never reaches it | `crates/nsh/tests/errors_are_values.rs`: for each of the 61 reporting raise sites, drive the shell to it and assert the variant and status. Pattern: `expansion_buffer.rs` |
| The diagnostic hook — errors dash reports and carries past (`docs/api-design.md` §3.3) | those produce `Ok` and identical bytes | same file: `nosuchcmd; echo done` reports twice, returns `Ok(0)`, and the hook observes two errors |
| `panic = "abort"` | both profiles are pinned to `unwind` (workspace `Cargo.toml`) | a CI job that builds and runs the suite with `RUSTFLAGS=-Cpanic=abort`; it is the whole of P2's second half and it is a one-line job |
| The `EXINT` unwind reaching a handler | reachable only when `rootshell && iflag` (`error.rs:254-256`), and the corpus runs `-c` without `-i` | a pty case, alongside the 31 that exist |
| `onsig`'s asynchronous path at all | requires a real SIGINT at a chosen instruction | a test that raises SIGINT during a blocked read and asserts `Err(Interrupted)` rather than a retry |
| `suppressint`'s value after a swallowed builtin error (§2.4) | not observable from outside | a unit assertion in the same file: `suppressint` after `cd /nonexistent` inside a function equals its value before, plus one |
| The vforked `_exit` path (§2.5) | the child's behaviour is identical either way; only the *mechanism* differs | an assertion that `shellexec`'s failure in a vforked child never returns — a `debug_assert!(vforked == 0)` at the `Err` construction site is the cheapest form |
| Re-entering `exitshell` from a subshell in an EXIT trap | fixed once already (`shellmain.rs:112-124` records it); the corpus reaches it, but the *mechanism* is what changes | keep the case that found it and name it in the test file |

The last row generalises into the one rule this node should adopt: the two
bugs `[dec:nsh:errors-are-values]`'s rationale remembers — a subshell in an
EXIT trap unwinding past `main`, and `onint` aborting because an unwind
cannot leave an `extern "C"` frame — were both found by the corpus *after*
the mechanism changed. The corpus will find the third one too. It will not
find the value.

---

## 6. Risk

Following `docs/idiomatization.md` §5's shape.

### Reversible, green at every commit

| step | why it is safe |
|---|---|
| A — the `Error` type and `report` | Additive until `exverror` is re-routed, and that one line is guarded by all 61,498 cases at once. A red harness here names the diagnostic, which is the most legible failure in the node |
| B — `raise_reported` | No behaviour: the same three writes to the same globals, then the same `raise_longjmp` |
| C — the leaf modules | Adapters at the boundary; the compiler decides completeness of each module; the harness must not move by a byte |
| D — the catch frames | One frame per commit, each with its own epilogue from §2.2 |
| G — deletion | Either it compiles or it does not; nothing partial |

### Not incrementally landable

**E, `Flow`.** `EXEND` and `EXEXIT` are raised from `evaltree`
(`eval.rs:431`), `exitcmd` (`shellmain.rs:506`) and `shellexec`
(`exec.rs:158`) and caught in `shellmain::main` and `trap::exitshell`. The
moment they stop being exceptions, `shellmain.rs:219-227`'s `if (e == EXEND
|| e == EXEXIT || s == 0 || !iflag || shlvl)` has nothing to test and the
entire startup state machine is rewritten in one commit. There is no
half-way state in which `exit` is both.

### The two that are genuinely dangerous

**A. `parser::expandstr` and `redir::redirectsafe`, because of
`restore_handler_expandarg`.**

`expand.rs:3411-3421` is nine lines that decide, on the exception path,
whether to re-raise or to swallow-and-`ifsfree`. It is shared by two callers
whose surrounding state is completely different: `redirectsafe` returns into
the middle of `evalcommand` with a redirection half-applied and the
interrupt counter saved by hand; `expandstr` returns into `PS1`/`PS4`
rendering with the parse-file stack pushed and `heredoclist` swapped out.
Get the re-raise condition wrong in either direction and the failure is a
*silently swallowed* error — the shell carries on with a half-built
redirection or a half-expanded prompt, and the corpus sees a plausible wrong
answer rather than a crash. `expand.rs` is covered at 92.82% of regions, so
roughly 7% of it is unguarded, and this is error-path code, which is the
part corpora reach least.

*Mitigation:* convert these two together, in their own commit, after every
other catch frame, and write the swallow/re-raise decision as a `match` on
`Error` with an explicit arm per exception code rather than as a negated
comparison. Add a `debug_assert` that the swallowed arm is only ever reached
with what is today `EXERROR`.

**B. The interrupt, because its oracle is 31 pty cases and it is the one
step that changes when a signal is delivered.**

`docs/idiomatization.md` §5 already names `host-owns-signals` as dangerous
for this reason, and step F is the same hazard arriving one node early. The
specific exposure: `suppressint` is a counter with 168 `INTOFF`/`INTON` references across 17 files, its
value is invisible to every test that exists, and the failure mode of
getting it wrong is not a crash but a shell that stops responding to `^C` —
which no batch harness can observe, because a batch harness never sends one.

*Mitigation:* the direct instrument, not the behavioural one. Assert
`suppressint` and `intpending` at the seven catch frames in debug builds,
and add the pty cases before step F rather than after. `[dec:nsh:owned-data]`
records the same discipline paying off twice already — a `debug_assert` on
a claim about `RMESCAPE_HEAP` was what proved the claim wrong, in two corpus
cases out of 61,498.

---

## 7. Proposed amendments to `plan/decisions/`

Not applied.

**`[dec:nsh:errors-are-values]`** — the deferred `set -e` question is
answered by `docs/api-design.md` §3.5 and should be resolved on the
decision: `EV_TESTED` stays a property of the call, because `set -e; false`
aborts with no error value in flight. Beyond that, four additions this
analysis produced:

1. **The interrupt does not keep a non-local mechanism.** `onsig` stores
   into the signal inbox and returns; `onint` becomes unreachable from
   `INTON`; delivery is polled at the six `EINTR` sites and at `dotrap`. The
   enabling fact is `trap.rs:288`'s `sa_flags = 0` — dash never restarts a
   syscall — and dash already uses the idiom at `redir.rs:181` and
   `input.rs:493`. Without this, `panic = "abort"` still breaks the
   interrupt path and the decision's first accepted consequence is false.
2. **`INTON` stays infallible.** Making it fallible puts 44 functions into
   the fixpoint and they are the shell's teardown — `popstackmark`,
   `unwindredir`, `unwindfiles`, `popallfiles`, `exitreset`. Cleanup that
   can fail while handling a failure is the wrong shape.
3. **`exraise`'s `vforked` arm has no `Result` equivalent.** The child
   shares the parent's stack; an `Err` returned from `shellexec` propagates
   through frames the parent owns. `_exit` at the raise site is forced, and
   it is the surviving `P8` site.
4. **What the unwind is doing is not `Drop`.** The crate has one `Drop`
   impl and it is `#[cfg(test)]`; `?` reproduces the memory reclamation for
   free. What must be preserved is the explicit, mark-keyed cleanup in the
   seven catch frames, listed in `docs/errors-are-values.md` §2.2 — including
   two descriptors (`eval::tpip`), the saved-descriptor stack, the input
   stack, the local-variable stack, the IFS regions, and a temporary file.

**`[dec:nsh:owned-data]`** — the rationale's second bullet says
`setstackmark`/`popstackmark` exists because C has no destructors *and*
because `longjmp` skips cleanup, and that "both halves of that reason are
absent in Rust". The decision's own last section already corrects the second
half. The correction should be promoted out of the "What this cost" prose
into the rationale, because the sentence as written is what a reader takes
away.

**`docs/idiomatization.md` §2.1** — the three numbers (`420`, `424`,
`raise ⊆ memalloc`) are not reproducible on the tree they were measured on
and the subset claim is false. §1.4 has the re-measurement and the method.
The ordering conclusion is unaffected and should be re-grounded on the
argument `[dec:nsh:owned-data]` actually makes.

**`[dec:nsh:no-ambient-state]` / `docs/api-design.md` §5.1** — replace "a
pointer into a frame cannot be a field of anything" as the reason
`errors-are-values` is upstream. It is true and it is escapable: the pointer
is only ever compared (`eval.rs:140`), so a handler id would do. The reasons
that hold are the `AssertUnwindSafe`-over-a-reused-`Shell` hazard and step
size. §3.1.

---

## 8. What this is not sure about

1. **The fixpoint is an over-approximation, and by an unknown margin.** Any
   path expression naming a function is an edge, so a function pointer that
   is assigned and never called still counts, and a local variable sharing a
   name with a function in the same module produces a false edge. The
   direction is safe — 296 is an upper bound on what must become fallible —
   but the true number is somewhat lower. *Resolved by:* running the same
   reachability over rustc's MIR once the crate can be built without the
   out-of-tree `nshedit` dependency, which `docs/idiomatization.md` §1.6
   names as a prerequisite for calling this a library at all.

2. **Where the earlier 420 came from.** Three reconstructions were tried and
   none produced it. It is possible the figure counted something else
   entirely — functions naming any symbol in `error.rs`, say, which gives
   351. Recording "not reproducible" is honest; recording "wrong by 95" is
   more precise than the evidence supports.

3. **Whether five `EINTR` sites are all of them.** They are all the sites
   that *name* `EINTR`. A syscall whose `-1` return is handled without
   inspecting `errno` would not appear, and `jobs.rs` — the module with the
   thinnest coverage in the crate — is where one would hide. *Resolved by:*
   auditing every `libc::` call in `jobs.rs` and `redir.rs` for an
   unchecked `-1`, before step F.

4. **Whether `evalbltin`'s and `evalfun`'s `suppressint` leak is really
   dash's behaviour and not a port bug.** The C reads the same way
   (`eval.c`'s `cmddone` restores `commandname` and `handler` and nothing
   else, and `exraise` does `INTOFF` unconditionally), and the harness is
   green, but the harness cannot see the counter. *Resolved by:* an
   assertion, or by an interactive experiment — a `^C` after a swallowed
   builtin error in an interactive dash, which is the one configuration
   where a stuck counter is observable.

5. **Whether step E can really wait for `delete-memalloc`.** That node did
   not finish; `[dec:nsh:owned-data]` records it surviving in two pieces
   with two different owners, one of which is blocked on
   `sanctioned-divergences`. If `errors-are-values` has to start first, §2.3
   is the list of mark placements that must be preserved by hand, and
   `eval.rs:426`/`:431` is the one that is easy to get wrong.
