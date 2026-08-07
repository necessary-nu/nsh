---
id [dec:nsh:owned-data]
epitome "The shell's data is owned Rust values, not blocks in a hand-rolled allocator; `memalloc` ceases to exist."
state @decided
category @ban
scope {
    elements ([arch:nsh:shell-core])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep the stack allocator and make it safe -- a typed arena with lifetimes."
        rejected_because "It preserves the thing that made the allocator necessary. dash's region allocator exists because C has no destructors and `longjmp` skips cleanup; Rust has both, so an arena here buys nothing and keeps every value a raw pointer into it."
    }
    {
        option "Convert the error mechanism to `Result` first, then the data."
        rejected_because "It changes the signature of every function on a raise path, and then the data representation changes those signatures again. The measurements say the data is upstream: `memalloc` is why the values are `*mut c_char`, and `*mut c_char` is why the libc string calls and most of the `unsafe` exist."
    }
)
consequences {
    accepted (
        "`crates/dash/src/memalloc.rs` is deleted, not rewritten. 384 call sites across 24 files go with it."
        "Roughly 90 libc string calls -- `strlen`, `strcmp`, `strchr`, `strcpy`, `strspn` -- disappear as a consequence rather than as work, because they only exist to operate on `*mut c_char`."
        "`union node` becomes an owned Rust enum. `calcsize`, `copynode`, `nodesize[]`, `funcblocksize` and `funcstring` -- the manual deep-copy-into-one-block that `copyfunc` performs -- are replaced by `Rc<Node>` and a derived `Clone`."
        "[dec:nsh:minimal-unsafe] mostly falls out of this rather than being separate work."
    )
    deferred ("The syscall surface stays. A shell makes syscalls -- `close`, `dup2`, `fcntl`, `sigaction`, `stat64`, `_exit` -- and those are not what this is about. Whether they go behind `std` or a safe wrapper is a later, smaller question.")
}
edges {
    requires ([dec:nsh:shell-as-library])
    enables ([dec:nsh:errors-are-values] [dec:nsh:no-ambient-state] [dec:nsh:minimal-unsafe])
}
---

## Rationale

`memalloc.rs` is dash's stack allocator: a bump allocator over malloc'd
blocks with mark-and-release (`setstackmark` / `popstackmark`), plus
`ckmalloc` / `ckrealloc` and a string builder (`STPUTC`, `stputs`,
`growstackstr`).

It is a faithful port of something that should not survive the port. It
holds two distinct jobs, and Rust has a better answer to each:

  * **Building a string of unknown length.** `STPUTC` / `growstackstr` /
    `grabstackstr` is `Vec<u8>`.
  * **Bulk deallocation at a known point.** `setstackmark` /
    `popstackmark` exists because C has no destructors *and* because
    `longjmp` skips any cleanup an error path would have done. Both
    halves of that reason are absent in Rust.

## Why this comes before the error mechanism

The first attempt at idiomatization started with
[dec:nsh:errors-are-values] and that was the wrong end. Converting the
exception mechanism means changing the signature of every function on a
path from a raise to a catch. The data representation change then
rewrites those same signatures a second time -- and it is the data change
that makes the error change smaller, because owned values clean themselves
up and there is far less for an error path to unwind.

The measurements that decided it, taken on the tree at `c607bdd`:

```
memalloc call sites   384 across 24 files (parser.rs 98, expand.rs 79)
libc string calls     ~90 (strlen, strcmp, strchr, strcpy, strspn, …)
libc malloc/free       17
```

The string calls are not independent work. They exist only because the
values are `*mut c_char`, and they are `*mut c_char` because the allocator
hands out untyped blocks. Removing the allocator removes them
transitively. That is the shape of the whole argument: `memalloc` is
upstream of most of what makes this code un-idiomatic.

## Order

1. `union node` becomes an owned enum. It is the arena's main tenant, it
   is the spine the parser, evaluator and `show` all hang off, and the
   differential harness covers `parser.rs` and `eval.rs` at 100% of
   functions -- so it is the large change with the best net under it.
2. The string builder becomes `Vec<u8>`, which is most of what is left in
   `parser.rs` and `expand.rs`.
3. Whatever still allocates from the region moves to ownership, and
   `memalloc.rs` is deleted.

Nothing here crosses an FFI boundary -- nodes are internal and `funcnode`
is internal -- so the C layout is not load bearing on anything outside the
crate. The comment at the top of `nodes.rs` explaining why the port kept
`#[repr(C)] union` describes a constraint of the *port*, not of the shell.

## What this cost in the port

Written after step 1, `union node` -> owned enum. Each entry is a place
where the C's *structure* was doing work that its *text* did not admit to,
so a mechanical conversion would have compiled and been wrong. The
differential harness catches what it exercises; several of these it would
have caught only by luck, and two it would not have caught at all.

### `copyfunc` copies the strings, not the pointers

The one that actually bit. `nodes.c` looks like a deep copy of a *tree*,
and `Rc::new(n.clone())` looks like the same thing. It is not:
`calcsize` accumulates `funcstringsize` alongside `funcblocksize`, and
`copynode` calls `nodesavestr` for every `string` field in
`src/nodetypes`. A node's `char *` points into the stack allocator, and a
function definition outlives the `popstackmark` that its text was parsed
under -- so the copy has to own the bytes.

Leaving the string fields as bare `*mut c_char` and deriving `Clone`
produces a function definition whose name and every word is a dangling
pointer into a freed block. The symptom is not a crash. It is

    $ f() { echo hi; }; f
    SH: 1: <two-dozen-bytes-of-garbage>: not found

which is to say: the shell executes whatever was written over that block
since. This is why `nodes::NodeText` exists -- a two-arm type that is
`Borrowed` in a parse tree and becomes `Owned` when cloned. It is the
narrowest thing that keeps step 2 (strings) a separate slice, and it is
the exact place the C copied.

### `nhere.doc` is back-patched after the node is already in the tree

`parseredir` builds the redirection node; `parseheredoc`, running at the
next newline, writes the body into it through `heredoc->here->nhere.doc`.
By then the node is buried inside a command inside a list. A back pointer
into an owned tree is the one thing Rust will not give you, so the *slot*
is shared instead (`Rc<OnceCell<Node>>`, one handle in the node and one
in the pending `struct heredoc`).

The trap is the second half: `copynode` *deep-copies* `nhere.doc`. A
derived `Clone` shares the `Rc`, which would leave a function's
here-document text pointing at the stack allocator -- the same failure as
above, but only for functions containing here-documents. `nhere` needs a
hand-written `Clone`, and the reason has to be written down next to it,
because "share the Rc" is what every other arm wants.

`struct heredoc` also carried `union node *here` only to read
`here->type` in `parseheredoc` (NHERE vs NXHERE picks `SQSYNTAX` vs
`DQSYNTAX`). That is one bit, settled by `parsefname` immediately before
the append, so it travels as a `bool` and the back pointer disappears.

### `parsefname` reads the globals at two different times, on purpose

`union node *n = redirnode;` is the *first* line of `parsefname`, but
`struct heredoc *here = heredoc;` comes *after* the `readtoken()`. That
asymmetry is load bearing in one direction: `readtoken` can reach
`parseredir` and set `redirnode` again, so the node must be copied out
first. (It cannot reach it for `heredoc`, because the eofmark word is read
with `CHKEOFMARK`, which makes `parsebackq` push a syntax level instead of
recursing into `list()`.) Taking ownership of the node is the same
guarantee as the C's pointer copy -- but only if it is taken at the same
point, which is why `parsefname` now takes the node as an argument
instead of reading the global itself.

### `simplecmd` type-puns an NARG into an NDEFUN

`n->type = NDEFUN; n->ndefun.text = n->narg.text;` reads offset 16 and
writes offset 8 of the same block, and `ndefun.linno` lands in what was
`narg`'s padding. The read has to precede the write, and the NARG's
`backquote` list is silently discarded because `ndefun` has no such field.
Rebuilding the node reproduces all of that, but only if you notice that
the two `text` fields are at different offsets and that the discard is
deliberate.

### The tree is written to at run time, on a tree that may be shared

`expredir` writes `nfile.expfname` and (via `fixredir`) `ndup.dupfd` into
the node it is about to perform. When the command is inside a function
body, that node is inside an `Rc` that the command table also holds. This
is the whole reason `evaltree` takes `&Node` and those two fields are
`Cell`s rather than the tree being `&mut`.

`src/nodetypes` already says so for one of them -- `expfname` is declared
`temp`, "a field that doesn't have to be copied when the node is copied"
-- which on inspection means it is not part of the node's value at all.
`dupfd` is not marked, but `fixredir` with `err != 0` writes only that
field, so it behaves the same way.

### Redirection lists are walked through whichever arm is convenient

`nfile.next`, `ndup.next` and `nhere.next` are the same offset, and so are
`nfile.fd`, `ndup.fd` and `nhere.fd`. `expredir`, `redir.c:redirect`,
`parser.c:command` and `jobs.c:cmdtxt` all read `n->nfile.next` and
`n->nfile.fd` without checking what the node is. The sharpest case is
`jobs.c:cmdlist`, which is called with `ncmd.args` (NARG nodes) *and* with
`ncmd.redirect` (redirections) and walks both with `np->narg.next`. Once
the lists are `Vec`s all of this is iteration, but the accessor for `fd`
has to answer for all three arms (`Node::redir_fd`) or those call sites
stop compiling in a way that suggests they were wrong, which they were
not.

### `list()` leaves a node half-initialised, and it is observable in principle

Backgrounding a command that is neither NPIPE nor NREDIR wraps it:

    n3 = stalloc(sizeof(struct nredir));
    n3->nredir.n = n2;
    n3->nredir.redirect = NULL;
    n2 = n3;
    n2->type = NBACKGND;

`n3->nredir.linno` is never written -- it is whatever `stalloc` returned.
`evalsubshell` then does `errlinno = lineno = n->nredir.linno`, so `$LINENO`
and the `sh: N:` prefix of a diagnostic can both see it. Every path out of
there either forks and has the child's `evalcommand` overwrite it
immediately, or is a `Cannot fork` error, which is why no corpus case sees
it. An owned node has to name a value; it is 0. This is a deliberate
divergence from "whatever was in the block", which the port could not have
reproduced anyway.

### `funcnode.count` is `Rc`'s count minus one

`copyfunc` sets `count = 0` meaning *one* owner, and `freefunc` frees when
`--count < 0`. That is `Rc` starting at 1 and dropping at 0, exactly.
`evalfun`'s `func->count++` before it evaluates the body is the reference
that keeps a function alive across being redefined while it runs, and it
is `Rc::increment_strong_count`. Checking this against `exec.c` rather than
assuming it was worth the five minutes: an off-by-one here is a
use-after-free that only appears when a function redefines itself.

The `Rc` is stored in `exec::tblentry`, which is still a `ckmalloc`'d C
struct with a flexible array member, so what the table holds is
`Rc::into_raw`. That is not the end state; it goes when `memalloc` does.

### `NEOF` is a pointer that is not a node

`#define NEOF ((union node *)&tokpushback)`. It is assigned in exactly one
place, guarded by `chknl == 0`, and `chknl` is zero only when `nlflag & 1`
-- which only `parsecmd` passes. So the sentinel can never appear inside a
tree, and `ParseResult::Eof` is safe to confine to `parsecmd`'s return
type rather than being a third state of every node.

### Two `switch` defaults fall through into the next case

`eval.c:evaltree`'s `default:` has no body outside `DEBUG`, so it falls
into `case NNOT:` and reads `n->nnot.com`; `jobs.c:cmdtxt`'s falls into
`case NPIPE:` and reads `n->npipe.cmdlist`. Both are reinterpretations of
whatever node arrived. Enumerating what can actually arrive: `evaltree`
sees only the fifteen types its cases name, and `cmdtxt` sees everything
except NCLIST -- which never reaches it, because the NCASE arm hands over
an NCLIST's `pattern` and `body` and never the NCLIST. So both
fallthroughs are unreachable, and with a tagged union there is nothing
left for them to reinterpret. The arms are kept, with the reasoning next
to them, because "unreachable" is a claim someone will want to re-check.

### What the harness would not have caught

`show.c` is the tree printer and it is entirely `#ifdef DEBUG`;
`showtree` is called from one commented-out line in `main.c`. Nothing in
`tests/corpus` reaches it. The `set -x` output the differential harness
does compare comes from `eval.c:eprintlist`, not from `show.c`, and the
xtrace-bearing corpora (13 of them, including `everything.txt` and
`aud_parser_ps4.txt`) exercise that path. So `show.rs` was converted
against the C by reading, not by testing -- including the detail that
`sharg` never advances `bqlist`, so every `CTLBACKQ` in a word prints the
*first* backquoted command.

`jobs.c:cmdtxt` is the other printer, and that one is live: it builds the
text the `jobs` builtin shows. It is covered, but only for the shapes a
corpus case actually backgrounds.
