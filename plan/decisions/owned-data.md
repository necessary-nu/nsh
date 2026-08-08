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
        "`crates/nsh/src/memalloc.rs` is deleted, not rewritten. 384 call sites across 24 files go with it."
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

## What this cost in the port: the string builder

Written after step 2, the `STPUTC`/`grabstackstr` string builder becoming
owned byte vectors. Same rule as above: each entry is a place where the
C's *structure* was doing work its *text* did not admit to.

The shape of the whole step is one sentence. The C's builder is a `char *`
cursor into a region whose base, `stackblock()`, can move; every offset the
code carries -- `markloc`, `typeloc`, `savelen`, `startloc`, `strloc`,
`patloc` -- exists *because* the base can move. An owned buffer has no base
to move, so the cursor is the length, `STADJUST` backwards is `truncate`,
`grabstackblock` is a move, and the offsets that survive are the ones that
were doing something other than surviving a reallocation. Telling those two
apart is the work.

### `grabstackblock` was not an allocation, it was a lifetime

`readtoken1` ends with `grabstackblock(len); wordtext = out;`. Reading it as
"allocate the word" gets the wrong answer, because the bytes are already
written and the call returns nothing. What it does is stop the *next*
string builder from writing over them -- `wordtext` has to outlive the token
that produced it, and in a bump allocator the only way to say that is to
move the bump pointer. It is `mem::take`, and reading it as an allocation
would have produced a copy that is correct and pointless.

The same call appears in `parsebackq` under a different name and doing a
different job: `str = stackblock(); savelen = out - stackblock();
grabstackblock(savelen); STARTSTACKSTR(out);` parks the half-built word so
the *recursive* `list(2)` can use the region, and `out = stnputs(str,
savelen, stackblock())` copies it back afterwards. Move out, move back. The
copy back is not the C being careful, it is the C having no way to move.

### `nhere.eofmark`'s bytes outlive three levels of the parser

`parsefname` does `rmescapes(wordtext); here->eofmark = wordtext;` and the
delimiter is then read by `checkend` on every line of the here-document,
which is *after* an arbitrary number of further tokens have been read. In
the C that works because the word was grabbed and the enclosing mark has
not popped. `wordtext` is a single buffer that the next token overwrites,
so the delimiter has to be taken out of it -- `heredoc.eofmark` owns a
`BString`. A mechanical conversion that left it borrowing `wordtext` gets a
here-document that terminates on whatever word was read last.

### `eofmark` is three types wearing one

`readtoken1`'s `eofmark` is NULL, `FAKEEOFMARK` (`(char *)1`) or a real
delimiter, and the three are distinguished by two different predicates
(`eofmark == NULL`, `realeofmark(eofmark)`) in eleven places. Only the third
carries bytes. Making the parameter own a `BString` would have forced
`expandstr` to invent one for `FAKEEOFMARK`, which is not a string and must
not behave like one -- it exists to make `readtoken1` take the
here-document branches without there being a here-document.

### `parsesub` writes a byte it stepped over three hundred lines earlier

    typeloc = out - stackblock();
    STADJUST(chkeofmark == 0, out);
    ...                                     /* the variable name goes here */
    p = stackblock();
    p[typeloc - 1] = CTLVAR;
    p[typeloc] = subtype | VSBIT;

The `STADJUST` reserves one byte whose *value is not yet known*: the subtype
is only settled after the name and the operator have been parsed. In a
region that is free -- the cursor moves and the byte keeps whatever the
block held. In a `Vec` the byte has to exist before the length passes it, so
it is a `resize` with a placeholder, and the placeholder is unobservable
only because nothing reads between the two points. That is a claim about
the whole of `parsesub`, not a local one, and it is the reason the C could
be sloppy and this cannot.

`p = stackblock()` being re-read on the last line is the other half: the
block may have moved while the name was being appended, so `typeloc` is an
offset and not a pointer. Every such re-read is a marker for "a growth can
happen here", and they are worth reading as documentation rather than as
noise.

### A NUL from `$'\0'` ends the word, and the C says so by re-deriving the cursor

    if (dollarsq) {
            char *p = stackblock();
            *out = '\0';
            out = p + strlen(p);
    }

Three lines that look like they terminate the string. They do the opposite:
they *shorten* it, to the first NUL anywhere in the word, because `$'\0'`
put one in the middle. `strlen` from the base is the search, and the write
at `out` is only there so the search terminates when there is no embedded
NUL. Transcribing it as "write a terminator" leaves `$'a\0b'` as `a\0b`
instead of `a`.

### `getmbc` and `conv_escape` write past the cursor and the caller commits

`CHECKSTRSPACE(MAX(MB_LEN_MAX, 16) + 7, out)` at the head of `readtoken1`'s
loop is not a hint. `getmbc` writes the multibyte character's bytes at
`out + 2` (or `out + 3`) *before* it knows whether it will keep them, then
either fills in the markers around them and returns the length, or returns
0 and leaves the scribble for the next write to overwrite. `conv_escape`
does the same with a four-byte `memcpy` for a character that may be two
bytes long. Both are writes into what a `Vec` calls spare capacity, so the
reservation has to stay exactly the C's number and the commit has to be a
`set_len` of what the function returned. Shrinking the reservation to "what
the character needs" would be correct for every input the corpus has and
wrong for the one it does not.

### `padvance` returns an allocation size, not a string length

    qlen = len + strlen(name) + 2;   /* "2" is for '/' and '\0' */
    q = growstackto(qlen);
    if (len) { q = mempcpy(q, start, len); *q++ = '/'; }
    strcpy(q, name);
    return qlen;

When `len` is 0 -- an empty `PATH` component, meaning the current directory
-- the bytes written are `strlen(name) + 1`, one *fewer* than the value
returned. `qlen` is what the caller passes to `stalloc` to take the
candidate out of the block, so it is a size and not a length, and three of
the six callers use it that way. A conversion that treats the return value
as a string length is off by one on exactly the `PATH=:` case.

The callers split into two kinds, and the split is invisible in the C
because both spell it `stackblock()`. `shellexec`, `printentry` and
`typecmd` read the candidate and are done with it before the next
`padvance`. `find_command`, `find_dot_file` and `cdcmd` call `stalloc` --
which is not an allocation either, it is "keep this one", because
`readcmdfile`, `docd` and the caller's own later work can all search the
path again. Those three now copy; the other three do not.

### `commandtext` reads a byte the C never wrote

`cmdtxt` emits nothing at all for a command with no words. `x=1 &` is one.
`commandtext` then does `savestr(stackblock())` over a block whose first
byte was never written -- the reference reads a NUL there and prints an
empty command text for the job, and the port did too, by the same accident.
An owned buffer has to name the value; it is `""`. This is the same shape
as `list()`'s uninitialised `nredir.linno` recorded above: a deliberate
divergence from "whatever was in the block", which the port could not have
reproduced anyway.

### `updatepwd` reads before the start of its own buffer

`if (*(new - 1) != '/')` runs when the buffer holds `curdir`, and reads one
byte before the block when `curdir` is empty. It cannot be empty -- it is
either `nullstr`, which the line above returns on, or a path `updatepwd`
itself produced -- so the read is unreachable rather than benign. An index
of `-1` is not available, so the port asks `last() != Some(&b'/')`, which
agrees with the C everywhere the C is defined.

### What is *not* converted, and the reason

`expand.rs` still builds its string in the region. The blocker is
structural rather than a matter of effort, and it is worth writing down
before the next attempt:

  * `expdest` is not the expansion's cursor, it is *a* cursor, and
    `expmeta` borrows it. At `expand.c`'s glob loop the code does
    `expdest = enddir; memtodest(dname, len, ...); cp = stackblock();
    enddir = cp + expdir_len;` -- `enddir` points into the *glob* buffer,
    not into the expansion being built, and `memtodest` is being used as a
    generic "encode these bytes at this cursor" routine. So `expdest`
    cannot become an index into one owned buffer until `memtodest`,
    `chtodest` and `mbtodest` take their destination as a parameter.
  * `expandarg` ends with `grabstackstr(expdest)` and hands the result to
    `ifsbreakup`, `expandmeta` and the `strlist` chain, all of which are
    `stalloc`'d C structures holding `char *`. Those belong to
    `delete-memalloc`. The clean seam is for the *builder* to become owned
    and the *result* to still be copied into the region, which keeps this
    node's property separable from that one's.
  * `_rmescapes` with `RMESCAPE_GROW` writes into the builder and moves
    `expdest`, and `subevalvar` re-derives `startp`, `str` and `rmescend`
    from `stackblock()` afterwards precisely because of it. Those
    re-derivations are the invariant, and each one needs reading against
    the C rather than transcribing.
  * The offsets that `expari` and `expbackq` protect with
    `pushstackmark(&sm, endoff)` are protecting against `arith()` and
    `makejob()` allocating from the region, not against the string builder
    moving. `evalbackcmd` always forks, so no nested `expandarg` runs in
    the parent -- that is what makes a single global builder safe at all,
    and it is not obvious from the code.
