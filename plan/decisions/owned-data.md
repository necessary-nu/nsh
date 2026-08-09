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

## What this cost in the port: the expansion buffer

Written after `expand.rs`'s `expdest` builder became an owned `BString`.
The four blockers the previous pass recorded are resolved as follows: the
`memtodest` signature is a commit of its own; the `grabstackstr` seam is a
copy into the region and stays until `delete-memalloc`; the `subevalvar`
re-derivations translate one-to-one, because `Vec::reserve` reallocates
exactly where `growstackto` did; and the `pushstackmark` lengths turn out
to have been protecting against something that can no longer happen. What
follows is what the reading of those did not predict.

### `_rmescapes` leaves `expdest` pointing into a block it then frees

The tail of `_rmescapes` is

    if (flag & (RMESCAPE_ALLOC | RMESCAPE_GROW)) {
            expdest = r;
            STADJUST(q - r + 1, expdest);
    }

and `r` is inside the expansion buffer only under `RMESCAPE_GROW`. The
other live arm is `expandmeta`'s
`preglob(text, RMESCAPE_ALLOC | RMESCAPE_HEAP)`, where `r` is a
`ckmalloc`'d block that `expandmeta` `ckfree`s four lines later -- so the
C sets the expansion cursor to a pointer into memory it is about to free.
It is harmless only because of where `expandmeta` sits: at the tail of
`expandarg`, after `grabstackstr(expdest)` has taken the word, and every
entry to the expansion re-opens with `STARTSTACKSTR`. The value is written
and never read.

An owned buffer cannot hold that pointer and gains nothing by trying, so
the store is confined to the `GROW` arm. The reasoning that got there
first -- "`RMESCAPE_HEAP` has no caller, so `GROW` is the only reachable
case" -- was simply wrong, and it was a `debug_assert` on that claim that
said so, in two corpus cases out of 61,498. The claim was checkable by
grep and was not checked.

### The expansion buffer has two readers outside `expand.c`

`expandarg(n, NULL, flag)` is the call that does *not* grab its result, and
both callers read it back as `stackblock()`: `redir.c:openhere` for a
here-document, and `parser.c:expandstr` for `PS1` and `PS4`. A conversion
that reads only `expand.c` finds the first, which is two functions away,
and misses the second. What that costs is the `+ ` in front of every
`set -x` line, and the corpus observes it in exactly one case -- the rest
of an xtrace line comes from `eval.c:eprintlist`, which does not go through
this buffer.

Both pointers outlive their C equivalents rather than the reverse. The C's
are valid until the next `stalloc`, and `parser.c:setprompt` bounds
`expandstr`'s explicitly with `pushstackmark(&smark, stackblocksize())`
around the `out2str` that consumes it; the port's are valid until the next
expansion begins.

### `argstr` terminates the word with a bitwise AND that reads as a no-op

    q = stnputs(p, length, expdest);
    *(q - 1) &= (end - 1);

`end` is 1 exactly when the byte just copied closed the word -- NUL,
CTLENDVAR or CTLENDARI -- and `end - 1` is then 0, so the AND *turns the
closer into a NUL*. Otherwise `end` is 0, the mask is `-1`, and the line
does nothing. One line, doing the single most load-bearing thing in the
file: it is why the buffer can be handed to `strlen` by
`redir.c:openhere`, why `setvar(str, startp, 0)` in `subevalvar` sees a
terminated value, and why `patmatch(stackblock(), val)` in `casematch`
works. Transcribed as written it keeps working; noticed, it is what lets
the owned buffer's length be trusted as the end of the word.

### Two places write *above* the cursor and read the byte back

The region copies the whole block when it grows. `Vec::reserve` copies only
the first `len` bytes. So "bytes above the cursor survive a growth" is a
property the region has, an owned buffer does not, and the C has no reason
to mention. Both sites had to be argued rather than transcribed:

  * `expari` winds the cursor back to `begoff` and then calls
    `arith(start)` with `start` pointing *at* the arithmetic text it just
    truncated away. Safe because `arith` -- and `yylex` under it --
    allocate only from the region and never touch this buffer, so nothing
    on that path can reserve.
  * `subevalvar` closes with `*loc = '\0'; STADJUST(loc - expdest, expdest)`,
    which puts the terminator one past the length. Safe because every path
    out of `argstr` re-supplies the word's own terminator (the AND above)
    before anything reads the buffer as a string, and because `loc` is
    always strictly below the cursor there, so the byte is inside the
    initialised area until then.

### `pushstackmark(&sm, endoff)` was doing two jobs and only one survives

`expari` passes `endoff` and `expbackq` passes `startloc`, and
`docs/idiomatization.md` §5 names these two lines as the reason
`expand.rs` is dangerous -- "dash keeps offsets and pointers into the
region across calls that can move it". The length does one thing:
`grabstackblock(len)` reserves the region under the half-built word so
that `arith()`'s and `makejob()`'s `stalloc`s land above it. The
save-and-restore does the other: it releases whatever those calls
allocated.

Once the word is owned the first job has no customer, because the two
allocators can no longer collide with a buffer they cannot reach, so the
length is 0 and the save/restore stays. The hazard is not reproduced as an
index; it is made impossible. `popstackmark` is unaffected either way --
it restores `stacknxt` to the value recorded *before* `grabstackblock`.

### `expmeta`'s encoded directory entry is scratch, and that is the seam

The glob loop's `memtodest` writes the encoded form of `d_name` at
`enddir`, inside the glob buffer and past the directory prefix, purely so
that `pmatch` has something to match against; the branch that keeps the
entry then overwrites those same bytes with the raw name via
`stnputs(dname, len, enddir)`. The encoding is therefore scratch, it can go
to a buffer of its own, and the *glob* buffer can stay in the region while
the *expansion* buffer becomes owned. Without that the two are one commit,
and one commit is what §5 says not to do here.

### What the expansion buffer left behind, and why

  * **`expand.rs`'s glob buffer.** Converted since, in the pass recorded
    below: `expmeta`'s candidate path is its own owned buffer. It was
    separable from the expansion buffer because the `memtodest` commit
    made `expmeta` stop touching `expdest`, and because — unlike the
    expansion buffer — nothing outside `expmeta` and `addfnamealt` ever
    read it.
  * **`expandarg`'s result.** `grabstackstr(expdest)` is a copy into the
    region, because `ifsbreakup`, `expandmeta` and the `strlist` chain are
    still `stalloc`'d C structures holding `char *`. That copy is the seam
    with `delete-memalloc` and it disappears with those structures, not
    before.
  * **`bltin/printf.rs`'s `xasprintf` result.** `ASPF` formats through
    `output.rs:xvasprintf`, which `stalloc`s the block it writes into. The
    three buffers `printf.c` itself builds are owned; this one belongs to
    `output.rs` and goes with it.
  * **`miscbltin.rs`'s `strlist` chain.** `readcmd`'s line is owned, but
    `ifsbreakup` still `stalloc`s the `strlist` nodes that point into it —
    the same seam as `expandarg`'s result above.
  * **`output.rs`'s `xvasprintf`.** Its `stalloc` is the region's, and it
    is sized to force a fresh block; see the `X`s entry below. It goes
    with `delete-memalloc`.
  * **The two `pushstackmark(&sm, stackblocksize())` calls** in
    `input.rs:preadfd` and `parser.rs:setprompt`. `stackblocksize` is the
    region's free space, not a builder cursor; these are marks and they go
    with `popstackmark`.
  * **`parser.rs`.** Eight sites inside `getmbc` and `dollarsq_escape`,
    which write through a raw cursor into a `BString`'s spare capacity and
    commit with `set_len`. The reservation has to stay exactly the C's
    number; see the `getmbc`/`conv_escape` entry above.

## What this cost in the port: the builtins' buffers

Written after `miscbltin.rs`'s `read` line and `bltin/printf.rs`'s three
buffers became owned. Same rule as above: each entry is a place where the
C's *structure* was doing work its *text* did not admit to.

### `readcmd_handle_line`'s `s` is the cursor, not the line

The doc comment above it says `@param line complete line of input`. It is
not. `readcmd` calls it with `p + 1` — the cursor one past the terminator
`STACKSTRNUL` just wrote — and the first statement of the body is

    s = grabstackstr(s);

which is `stalloc(s - stackblock())`, so the parameter arrives as a
*length* and comes back as the line's base. The call does the other half
too: it reserves those bytes so that the `stalloc` `ifsbreakup` performs
for each `strlist` node lands above them rather than on top of the line it
is splitting. An owned line is already its own base and has nothing to
reserve, and the `strlist`s that point into it are kept alive by the
caller holding it rather than by the enclosing `popstackmark`.

### `conv_escape` writes two bytes past what `CHECKSTRSPACE` promises

`printf.c` guards both of its `conv_escape` calls with
`CHECKSTRSPACE(4, cp)`, and 4 is not the bound. The `\U` arm is

    USTPUTC(CTLMBCHAR, out);
    USTPUTC(len, out);
    STADJUST(mboff, out);
    memcpy(out, &value, 4);
    STADJUST(len, out);
    USTPUTC(len, out);
    USTPUTC(CTLMBCHAR, out);
    STADJUST(mboff, out);

with `mboff` `-2` when `mbchar` is false. So the closing pair lands at
`out0[len]` and `out0[len + 1]` — bytes 5 and 6 for `\U0001F600`, whose
`len` is 4 — and `out` is then wound back to `out0 + len`, which is what
the function returns. The two bytes are above the returned length and the
next write overwrites them, and in a 504-byte stack block nobody notices.

Spare capacity is exactly as long as it is reserved to be, so the port has
to reserve what the C *writes* rather than what the C *says*: 8, which
also covers the `mbchar` case's `4 + len`. This is the one place in the
pass where the C's number is wrong rather than merely generous, and the
opposite of the `getmbc` entry above, where it is generous and must not be
narrowed. A `debug_assert` in the arm names the highest byte it touches.

### `print_escape_str`'s `X`s fit only because the mask is zero there

    p = makestrspace(len, q);
    memset(p, 'X', total);
    p[total] = 0;

`makestrspace(len, q)` guarantees `len` bytes at `q`, and `q` is itself
`len` from the base — so the block is `2 * len` and `p[total]` is its last
byte only while `total == len - 1`. It is, but not locally: `q[-1]` is
`(!!((f[1] - 's') | done) - 1) & f[2]`, whose mask is 0 whenever `f[1]` is
not `'s'`, and `f[1] == 's'` is precisely the `goto easy` that skips these
three lines. Read separately they ask for a buffer that is one byte short
on every `printf '%<width>b'`, and never on `echo`'s.

### The run of `X`s is scratch, and `xvasprintf` is why it can be

`print_escape_str` formats a run of `X`s through the real conversion to
get the field width right, then memcpies the real bytes over them —
because the real bytes can contain NUL and `printf` would stop at it. That
is three cursors into one region: the converted string at the base, the
`X`s above it, and the formatted result.

The third is not in the same block, and that is not luck. `xvasprintf`
asks `stalloc` for `max(len, stackblocksize()) + 1`, which exceeds the
free space by construction, so it always chains a fresh block and the `X`s
survive being read as the conversion's argument while it writes. Once that
is established the `X`s have no reason to sit above the converted string
at all — nothing ever reads the two as one string — so they become a
buffer of their own, and the converted string can be owned. Same shape as
`expmeta`'s encoded directory entry above.

### `mklong` returns a lifetime, the way `grabstackblock` did

`mklong` builds the widened conversion at `stackblock()` and returns the
pointer without grabbing it, so the value is live exactly until the next
`stalloc` — and the caller's next act is the `printf` that reads it. What
the region call communicates is when the bytes stop being valid, not where
they are; the owned form is a buffer the caller holds across the `printf`.
## What this cost in the port: the glob buffer

Written after `expmeta`'s candidate path became an owned `BString`. This
closes the first entry of the list above: the previous pass said the glob
buffer was separable because `expmeta`'s encoded `d_name` is scratch, and
it was. Nothing outside `expmeta` and `addfnamealt` ever touched it, which
is the opposite of what the expansion buffer turned out to be, and the code
that changed is small enough that the arguments below are most of the diff.

The C never names this buffer. It is the stack block, addressed through
two locals: `cp`, which `growstackto` returns, and `enddir`, which is
`cp + expdir_len` plus whatever has been appended. `expmeta` recurses one
frame per path component, every frame owns `[0, expdir_len)` -- the
directory prefix its parent wrote, ending in `/` -- and writes the next
component above it. So the property to hold is

> at every point where the glob buffer can grow, its length is the current
> frame's `expdir_len`

which is the same sentence as the expansion buffer's "bytes past the
cursor do not survive a growth", said for a buffer whose cursor belongs to
whichever frame is running.

### `addfnamealt`'s re-seed is the allocator's price, not the algorithm's

    name = grabstackstr(enddir);
    addfname_common(name);
    STARTSTACKSTR(enddir);
    return stnputs(name, expdir_len, enddir) - expdir_len;

Read as work, the last two lines rebuild the directory prefix so the next
candidate has something to append to. They do not. `grabstackstr` gave the
block away to `strlist`, `addfname_common` then `stalloc`s a `struct
strlist` on top of it, and the prefix has to be copied because the C has
nowhere to copy it *from* any more. Owned, nothing was given away: the
prefix is still the first `expdir_len` bytes and the re-seed is
`set_len(expdir_len)`. Same shape as `grabstackblock` in `readtoken1` --
an allocator move that reads as an allocation.

### `stnputs` takes its length in its cursor, and the recursion needs that

`makestrspace(n, p)` opens with `len = p - stacknxt`, so `stnputs(s, n, p)`
appends *at `p`* and discards whatever was above it. In a builder that only
ever appends at the end that is invisible. Here it is the mechanism by
which a frame recovers from its own recursion: the child `expmeta` returns
having left the buffer at *its* deeper `expdir_len`, and the parent's next
`stnputs` at `cp + expdir_len` is what cuts it back. A conversion that
appended at `Vec::len()` -- the obvious reading of `stnputs` -- concatenates
the child's prefix onto the parent's and emits `a/b/c/b/…`. It is one
subtraction in `memalloc.c` and it is load bearing three functions away.

### The reservation is exact, and a 504-byte minimum block was covering it

`growstackto(expdir_len + name_len + 1)` is the only bound on
`expmeta_rmescapes`, which then writes `strcpy(enddir, name)` through a raw
pointer that carries no bound of its own. That is safe only if
`name_len == strlen(name)`, and the C never says so. It holds by induction:
the top-level call passes `strlen(p)`, and the recursion passes
`name_len - (endname - name)` for a `name` whose temporary NUL --
`*zeroedp = '\0'` -- sits at `p - esc`, strictly *below* `endname`, so it
cannot shorten the string being measured.

In the C an off-by-something here is invisible, because a region block is
never smaller than 504 bytes and doubles. So the same arithmetic is now the
difference between a correct glob and a heap overflow, and both
`expmeta_rmescapes` call sites assert it -- against the reservation and
*not* against `Vec::capacity()`. That distinction was not obvious and had
to be found by mutation: `Vec::reserve` over-allocates as well, so shaving
a byte off the request leaves the capacity unchanged and an assertion on
the capacity says nothing. An assertion has to name the number the C
computed, not the allocator's answer to it.

### The buffer that no other function reads

The expansion buffer's conversion turned on finding `redir.c:openhere` and
`parser.c:expandstr`, two callers in other files reading `stackblock()`
back. The glob buffer was checked for the same thing and has none: its only
export is `addfnamealt`'s `grabstackstr`, and after that the bytes are a
`strlist` entry. `expandmeta` holds `INTOFF` across the whole glob and
nothing on the path -- `pmatch`, `opendir`, `readdir`, `lstat` -- re-enters
expansion, so there is never a second candidate path in flight either. That
is why it is a `static` and not a parameter: the cursors are raw pointers
that outlive the borrow that produced them, and there is nothing for a
parameter to disambiguate.

### What the top-level call was getting by accident

`expmeta(p, len, 0)` starts on whatever block the region happens to be on,
and gets away with it because `expdir_len` is 0 -- it writes from the base
and never reads what was there. An owned buffer's length is not 0: the
previous glob's `addfnamealt` left it at *that* glob's `expdir_len`. Every
consequence of the stale length turns out to be benign, which is precisely
why `expandmeta` clears explicitly -- the invariant above is worth stating
as an equality that can be asserted, and an equality is what the assertion
on entry to `expmeta` checks.

### What the corpus cannot aim at, and what does

Every property above is invisible to a corpus case that globs a short path
in a shallow tree, because the buffer only has to survive a growth once it
is longer than its first reservation, and a frame only has to cut itself
back once a sibling has recursed before it.
`crates/nsh/tests/glob_buffer.rs` is §5's targeted suite for that: three
cases over deliberately deep, wide and long-named fixtures, one per
property, each confirmed to fail when its property is broken. Expected
output was checked against the reference C.

### What the glob buffer left behind, and why

`expand.rs` keeps no string builder over the region after this. What is
left of `memalloc` in the file is the region allocator itself, and all of
it belongs to `delete-memalloc`:

  * `stalloc` for `struct strlist`, and the two grab seams
    (`grabexpdest`, `addfnamealt`) that copy an owned buffer into it.
  * `ckmalloc` for `ifsregion`, for `wcifs`, and in `_rmescapes`'
    `RMESCAPE_HEAP` arm.
  * `setstackmark` / `pushstackmark` / `popstackmark`.

## What this cost in the port: the last region allocations

Written during `delete-memalloc`, which did not finish -- see "What is
left, and what it is waiting on" at the end.  Same rule as the sections
above: each entry is a place where the C's *structure* was doing work its
*text* did not admit to.

### `ifslastp == NULL` is not "the list is empty" until you check that it is

`recordregion` reuses the static head node `ifsfirst` when `ifslastp` is
NULL and `ckmalloc`s a node otherwise, so the head is both a list element
and a sentinel.  Modelling the chain as a `Vec` needs the equality
"`ifslastp == NULL` iff no region is live", and that is a claim about
three separate writers rather than a local fact: startup, `ifsfree`, and
`removerecordregions`' first branch.  All three free the chain behind the
head *before* nulling `ifslastp`, so the equality holds -- and
`ifsfirst`'s stale contents are then unreachable, which is what permits
emptying the `Vec` to throw them away.  Had any one of the three nulled
the tail pointer while leaving `ifsfirst.next` set, the `Vec` would have
silently lost regions and no assertion inside `recordregion` would have
noticed.

The INTOFF/INTON brackets are kept where the C has them, one pair per
`ckmalloc` and one per `ckfree`, even though an owned `Vec` cannot be
left half-linked by an interrupt.  They are not protecting the list any
more; they are fixing the instruction at which a pending SIGINT is
delivered, and that is not this commit's to move.

### `wcschr` matches the terminator, and `ifsisifs` can ask it to

    isifs = wcschr(wcifs, wc) != NULL;

`wc` is the byte under the cursor, widened.  When that byte is NUL,
`wcschr` returns a pointer to `wcifs`' own terminator -- non-NULL -- so
the C treats a NUL inside an IFS region as an IFS character.  `contains`
over the converted wide string does not, and the difference only shows up
on input that puts a NUL inside a region, which is exactly what `nulonly`
regions are for.  The port keeps the C's scan, terminator and all.  This
is the second time in this decision that a libc string call turned out
not to be the obvious Rust method; the first was `padvance` returning a
size rather than a length.

### `_rmescapes`' `stalloc` arm is unreachable, and the reason is a constant

Three arms take the `RMESCAPE_ALLOC` branch: `GROW` writes into the
expansion buffer, `HEAP` `ckmalloc`s, and the fall-through `stalloc`s.
`RMESCAPE_ALLOC` is set in exactly two places -- `preglob`, inside
`if (FNMATCH_IS_ENABLED)`, and `subevalvar`'s literal `ALLOC | GROW`.
`FNMATCH_IS_ENABLED` is 0, so `preglob` only ever adds `RMESCAPE_GLOB`,
and the only caller that both allocates and is not `GROW` is
`expandmeta`'s `preglob(text, RMESCAPE_ALLOC | RMESCAPE_HEAP)` -- written
out in full at the call site, which is why it survives the constant.  The
previous pass got the neighbouring claim about this same flag wrong, so
this one is a `debug_assert` on the arm rather than a sentence, and the
mutation that proves it live is `echo "a"*`.

The size bound is the other half.  `expandmeta`'s buffer is written
through a raw cursor that carries no bound of its own, so `fulllen` --
`strlen(p) + (p - str) + 1` -- is the only thing between the C's
arithmetic and a heap overflow.  It is exact rather than generous:
`echo a\**` writes precisely `fulllen` bytes, so shaving one fires the
assertion.  Asserted against `fulllen` and not `Vec::capacity()`, per the
glob buffer's entry above.

### `xvasprintf`'s `stackblocksize()` had one customer, and it had already left

`stalloc(MAX(len, stackblocksize()) + 1)` asks for more than the region's
free space by construction, so it always chains a fresh block.  The
builtins' pass established why that mattered: `print_escape_str`'s run of
`X`s sat above the converted string in the *same* block and had to
survive being read as this call's `printf` argument.  Both of those are
owned buffers now, so the term has no customer left and the request is
`len + 1` -- which is all `xvsnprintf` ever writes and all any caller ever
reads.  Removing it is not a simplification of the C; it is the C's
reason expiring, and the difference matters because the same expression
would still be load bearing if the `X`s had not moved first.

### `commandname` outlives `dotcmd` by the width of `evalbltin`'s epilogue

`dotcmd` sets `commandname = fullname` and never restores it.
`evalbltin` does, but only after `flushall()` and
`if (outerr(out1)) sh_warnx("%s: I/O error", commandname)` -- so there is
a window, after `dotcmd` has returned, in which the global still names
the path `find_dot_file` built and something reads it.  The region covers
that window because `dotcmd`'s block belongs to `evalcommand`'s mark; a
local `Vec` does not, and would be freed one statement early.

So the `Vec` is *moved* into a static slot on the way out rather than
dropped.  Moving a `Vec` moves the header and not the bytes, so the
pointer stays valid -- asserted, because that is the whole reason the
line works.  The slot's previous occupant is provably unreferenced: every
`evalbltin` restores `commandname` before any other `dotcmd` can run, so
the only window in which a path is still named is the one just described,
and it admits no second `dotcmd`.

That assertion also caught what the first draft had wrong.
`find_dot_file` returns its argument unchanged for a name containing `/`,
without ever touching the out-buffer, so `fullname` is then the expanded
word and the buffer is empty -- which is what the C does too, and what
the emptiness test now discriminates on.

### `yylval.name` is per-evaluation, not per-token

`arith_yylex` `stalloc`s each variable name.  Read as a token value it
looks like a per-token lifetime, and a `Vec<u8>` returned by value would
compile.  `arith_yacc:assignment` copies `yylval` into a local, recurses
-- which calls `yylex` again and overwrites `yylval` -- and only *then*
reads `val.name`, so `a=b=1` has two names live at once.  The lifetime
the C picked is the whole arithmetic evaluation, which is what `expari`'s
enclosing mark releases, and the owned form has to say the same thing: a
list cleared by `arith` on entry.  `arith` is already non-reentrant -- it
seeds the `arith_buf` cursor from its argument -- so one list suffices,
and pushing to it moves the inner `Vec` headers but not their bytes, so
names handed out earlier stay valid.

### `padvance` returns a size, and three `stalloc` callers were using it as one

Recorded in the string-builder section as a property of `padvance`; here
it is the property of its callers.  `cdcmd`, `find_dot_file` and
`shellexec`'s `%func` arm each ask for `len` bytes and `strcpy` into
them, and `len` is one *more* than the string's length when the PATH
component is empty.  A `Vec` sized from `strlen` is correct for every
case the corpus has and one byte short on `PATH=:`.  All three size from
`len` and assert `strlen < len`; the assertion is tight -- it fires on an
ordinary `cd` through `CDPATH` when shaved by one.

### `popstackmark` versus `Drop`, under the mechanism that is actually there

This decision claims both halves of the region's reason are absent in
Rust: no destructors, and `longjmp` skipping cleanup.  The first half is
unconditional.  The second is *true here but not for the reason the
sentence gives*, and the difference is worth writing down because
[dec:nsh:errors-are-values] has not landed and the mechanism could still
change.

`error.rs:raise_longjmp` is `std::panic::panic_any`, `eval.rs:setjmp_catch`
is `catch_unwind`, and both Cargo profiles set `panic = "unwind"`.  So an
exception in this port runs every destructor between the raise and the
catch -- which is precisely what `popstackmark` in the catching frame was
doing for the region.  That is not a property of Rust; it is a property of
this port's choice, and `error.rs` records that the choice was forced: a
real `longjmp` over a `catch_unwind`-armed `jmploc` is undefined and
segfaulted on every fork and exit path until it was removed.  Had the port
kept libc's `setjmp`/`longjmp`, `Drop` would not run and every mark would
still be load bearing.

Two paths still skip destructors and both are correct to:
`exraise` under `vforked` calls `_exit` directly, and `shellexec` reaches
`execve`.  In each the address space is about to stop being this shell's,
so there is nothing to release.

### What the last region allocations left behind

`crates/nsh/src/memalloc.rs` still existed after this pass, held up by
`struct strlist` and by the checked-malloc wrappers.  The section below
records how the first of those went; the second is still open.

## What this cost in the port: `struct strlist`

Written after the argument list became a `Vec` of owned fields and the
region's marks came out.  Same rule as the sections above: each entry is a
place where the C's *structure* was doing work its *text* did not admit to.

The shape of the step is that `strlist` is two things bolted together and
only one of them is hard.  `next` is a cons cell and becomes a `Vec` with
no argument at all; `text` is a `char *` into the region whose lifetime is
"until the enclosing `popstackmark`", and every reader in three files
leans on that.  Separating the two is what made the step bisectable: the
list converted first, with the text still a region pointer and no lifetime
moving, and the text converted second.

### `nfile.expfname` is a field with a different owner

`expredir` does `redir->nfile.expfname = fn.list->text` and returns; the
node then travels to `redir.c:openredirect`, which reads the pointer back.
That works in the C because `fn` is a local `struct arglist` whose *text*
is in the region, and the region belongs to `evalcommand`'s mark.  `fn` is
declared inside the loop over the redirections, so the moment a field owns
its bytes the stored pointer is dangling one statement later.

It is the only part of the conversion separable from it, and it went
first.  The node owns a `BString`; `None` is the C's null, which is not the
same as an empty file name, because `> ""` is a real redirection.  The
`char *` the four reads in `openredirect` want comes out of a borrow that
ends immediately, and that being sound needs two facts worth stating: a
`BString`'s bytes do not move when its header does, and nothing between
the read and the use -- `stat64`, `sh_open`, `sh_open_fail` -- re-enters
expansion, so no second `expredir` can write the field while the pointer
is live.

### `arglist->list` is the list *and* the cursor

`parse_command_args` ends with `arglist->list = sp`, advancing the head
past the `command` and `-p` words it consumed, while `evalcommand` keeps
the original head in `osp` and hands *that* to `eprintlist`.  So `set -x`
traces `command -p foo` as it was written and `argv` starts at `foo`, out
of one field meaning two things.  A `Vec`'s start does not move, so the two
separate: the list is the `Vec`, the head is an index.  Draining the front
instead -- the obvious reading -- loses the `command -p` from the trace.

### `msort`'s merge takes the first half on a tie, so it is stable

`q = msort(list, half)` is the *first* half and `p = msort(p, len - half)`
the second, and the merge takes `p` only on `strcoll(p->text, q->text) < 0`.
Strictly less, so equal elements come from `q` -- the earlier run -- and
the sort is stable.  `slice::sort_by` is too, and the difference is not
academic: `strcoll` returns 0 for byte-different strings under a collating
locale, which is exactly when a glob's order would visibly change.

Three of the four lines around the call exist only to re-find the tail of a
list the sort has reordered (`*savelastp = sp; while (sp->next) sp =
sp->next; exparg.lastp = &sp->next;`).  A slice's tail does not move.

### `argstr`'s bitwise AND is what terminates the word, not the source NUL

The expansion buffer is read back as a C string by `ifsbreakup` and by
`redir.c:openhere`, so "the last byte is a NUL" is a property the whole
conversion rests on.  The obvious reading is that the word's own
terminator is copied along with it -- `length` counts the closing byte and
`stnputs` copies it.

That reading is wrong, and the mutation says so: making `length` *not*
count the closing byte still leaves a terminated buffer, because
`*(q - 1) &= (end - 1)` then masks the last byte that *was* copied to zero.
The AND is not a line that occasionally fires; it is the only thing
guaranteeing the terminator.  The entry above -- "one line, doing the
single most load-bearing thing in the file" -- understated it.  A
`debug_assert` in `grabexpdest` names the property, and the mutation that
proves it live is a word arriving one byte short.

### `grabexpdest` is a move; `addfnamealt` cannot be

Both were recorded above as "a copy of an owned buffer *into* the region,
which goes when `strlist` does".  Only one of them is that.

`grabexpdest` is `mem::take`: the expansion buffer's allocation *becomes*
the field's, and the `STARTSTACKSTR` at the head of the next `expandarg`
clears the empty one left behind.  The C's `grabstackstr` was not a copy
either -- it was a bump-pointer move meaning "these bytes outlive the next
builder", which is what a move says.

`addfnamealt` cannot be, and the reason is the algorithm and not the
allocator.  The field wants `[0, n)` of the glob buffer and the *next*
candidate wants `[0, expdir_len)` -- the same bytes -- so exactly one of
the two has to copy.  The C copies the prefix back
(`STARTSTACKSTR(enddir); stnputs(name, expdir_len, enddir)`) because
`grabstackstr` had already given the block away; the port copies the field
out and keeps the buffer, which costs the same order of bytes and leaves
the glob buffer's capacity and its `len == expdir_len` invariant alone.
What has gone is the region: the copy lands in the field's own allocation
and not in a block a `popstackmark` has to free.  The glob pass's entry --
"owned, nothing was given away, so the re-seed is `set_len`" -- is still
true, and it is true *because* this copy stays.

### `ifsbreakup`'s fields stop aliasing the string they split

In the C a field *is* an offset into the grabbed word, and `*q = '\0'`
terminates it in place; the word therefore has to outlive every field cut
from it, which is what the enclosing mark provided and what
`readcmd_handle_line`'s `grabstackstr` was reserving room above.  Owned,
each field copies out at exactly the instant the C wrote its terminator, so
`expandarg`'s word is a local that dies at the end of the call and `read`'s
line only has to survive one call.

Taking the copy at that instant and no later is what makes them equal, and
it rests on an ordering the C never states.  The one write into the string
that happens after `ifsbreakup_slow` has stopped emitting fields is
`*ifst.r = '\0'`, the trailing-IFS-whitespace truncation, and it has to
land in the field that does not exist yet -- the one `add:` takes from
`ifst.start`.  It does, because `r` is only ever set once `maxargs` has
reached 0 and both branches that set it return without emitting.  That is
a `debug_assert` rather than a sentence.  Honesty about its evidence: the
site is confirmed reachable (negating the test fires it on
`printf "a b c   \n" | read x y`) but no local mutation was found that
makes the inequality false, because the branches that set `r` are the same
branches that stop emitting.

### `addfname`'s `sstrdup` was `glob`'s buffer, not the region's

`addfname_common(sstrdup(name))` reads as the region's usual "keep this
one".  It is not: `name` is a `gl_pathv` entry and `expandmeta_glob` calls
`globfree64` four lines later, so the copy is escaping *glibc's*
allocation.  The field owning it says the same thing without the region.
The arm is unreachable in this build (`GLOB_IS_ENABLED` is 0), which is
why it is worth reading rather than testing.

### No field escapes into the variable table, and `setvareq` had to be read

`evalcommand`'s assignment loop hands a field straight to
`setvareq(s, vflags)` and `mklocal(name, VEXPORT)`, and `setvareq` stores
`s` into `vp->text` *without copying* when `flags & VNOSAVE` is set.  Had
either caller passed it, the variable table would hold a pointer into
`evalcommand`'s frame and the region would have been what kept it valid.
Neither does -- `vflags` is 0 or `VEXPORT`, `mklocal` adds `VSTRFIXED` --
so both take the `s = savestr(s)` path.  Reading this cost five minutes; a
wrong answer is a use-after-free on every `FOO=bar cmd`.

### The marks are no-ops, and that is checked rather than argued

With the fields owned, `stalloc` has no caller on any shell path.
`mystring.rs:sstrdup` still contains one and nothing calls *it*; the only
other reference is `grabstackblock`, which is what `pushstackmark`
performs.  So every mark in the shell saved a cursor, grabbed nought or
one byte, and restored the cursor.

That is a claim about 61,498 corpus cases rather than about the source, so
it is asserted.  `memalloc::region_untouched()` is `stackp == &stackbase
&& stacknxt == stackbase.space && stacknleft == MINSIZE`, and three places
check it: `eval::evaltree`, which runs at the head of every command;
`main.c:cmdloop` where its `popstackmark` was, which covers the last
command a script runs; and `eval::evalstring`'s tail, for `eval` and `.`.
Nothing winds `stacknxt` back now, so one `stalloc` anywhere fails every
check after it -- which is what the mutation shows: a `stalloc(1)` in
`expandarg` fires `evaltree`'s on `echo hi; echo there` and `cmdloop`'s on
a one-line script.

Removing `popstackmark` does remove an `INTOFF`/`INTON` bracket, and that
bracket is an interrupt *delivery* point -- `INTON` runs `onint()` when the
counter reaches zero with a signal pending.  What is lost is delivery for a
SIGINT arriving inside the bracket itself, a dozen instructions wide, which
is then delivered at the next `INTON` or `CHECKINT`.  A signal arriving
outside it is delivered by the handler directly, because `suppressint` is
zero there, so the window is the bracket and nothing wider.

### What is left, and who owns it

`crates/nsh/src/memalloc.rs` still exists, and what is left in it is two
things with two owners:

  * **The region has no caller.**  `stalloc`, `stunalloc`, the
    `stack_block` chain, `growstackblock`/`growstackto`/`makestrspace`/
    `stnputs`/`stputs`/`_STPUTC`, and
    `setstackmark`/`pushstackmark`/`popstackmark` are reached from nothing
    the shell runs.  Two references survive and both are test-shaped:
    `mystring.rs:sstrdup`, whose only caller (`addfname`) is gone but which
    keeps a spec rule and a unit test, and the marks in `memalloc.rs`'s and
    `mystring.rs`'s own tests.  Deleting the code is a spec-rule question
    -- fifteen of `memalloc.rs`' twenty `def` rules and their `sem` pairs,
    plus nine of its thirteen unit tests and `mystring.rs:sstrdup`'s --
    rather than a call-site one, and it is the last thing this node has to
    do.

  * **The checked-malloc wrappers.**  `ckmalloc`/`ckrealloc`/`ckfree`/
    `savestr` have 13/2/28/6 call sites outside `memalloc.rs` across
    `var.rs`, `alias.rs`, `jobs.rs`, `options.rs`, `trap.rs`, `input.rs`,
    `redir.rs` and `exec.rs`, and not one of them changed in this pass --
    the section above counted the same thing another way.  These are the hash tables and the process-state structs,
    not the region, and `var.rs`/`alias.rs` are out of bounds for this node
    twice over: another pass owns them, and docs/idiomatization.md 2.3
    step 4 records that changing their iteration order is a category-3
    divergence over thirty corpus cases that cannot land before
    `sanctioned-divergences`.  `exec.rs`'s `tblentry` -- a `ckmalloc`'d
    struct with a flexible array member holding `Rc::into_raw` of a
    function node -- belongs with them.

`crate::memalloc::` references went 47 -> 26 across the crate in this
pass, and three of the 26 are `region_untouched`, which exists only to
check that the rest is dead.  `expand.rs`, `parser.rs` and `mail.rs` reach
`memalloc` not at all.

## What this cost in the port: the variable table

Written after `var.rs`'s fourteen `ck*`/`savestr` sites and its one raw
`libc::free` became owned Rust values: `struct var`'s node and its text,
and the two nested intrusive lists behind `local`.  Same rule as the
sections above -- each entry is a place where the C's *memory layout* was
doing work its *text* did not admit to.

The step split three ways because the failure modes are three different
things.  The lists become `Vec`s and fail as wrong teardown order.  The
node becomes a `Box` the table owns and fails as a dangling
`localvar.vp`.  The text becomes owned bytes and fails as a lifetime bug
in `execve`'s `envp`, in `putenv`, or in the alias `mklocal` creates.

### `VTEXTFIXED` is not a fact about the buffer, it is a `Drop` impl

`vp->text` is one `char *` and four different owners: a `static` in
`var.c`, an `environ` entry the process was started with, a `ckmalloc`'d
`NAME=value` from `setvar`, or a `savestr` copy.  Which one it is is
recorded nowhere except in `flags & (VTEXTFIXED|VSTACK)`, and every read
of those two bits is a decision about whether to `ckfree` a buffer or
`savestr` one.  So the flag bits *are* the ownership, written as data.

The Rust makes the type carry it -- `VarText::Fixed` or
`VarText::Owned(Box<[u8]>)` -- and keeps the flag bits, because
`setvareq`'s `bits` arithmetic and `poplocalvars`' restore still copy them
around and a stored variable's `flags` are observable through `local`.
Two records of one fact can drift, so both places that write them assert
they agree: `Owned` exactly when `flags & (VTEXTFIXED|VSTACK)` is clear.
The assertion fires on the first line of `mkinit_init` if the wrapper
stops honouring `VTEXTFIXED`, and on the first `local` if `mklocal` saves
a flag word that does not match the text it saved beside it.

`VSTACK` is set by nobody, in the C or in the port.  It is carried
because it appears in three masks and removing it from those is a
separate, checkable act.

### `mklocal` aliases the text and says so with a flag

`lvp->text = vp->text; vp->flags |= VSTRFIXED|VTEXTFIXED` leaves two
pointers to one buffer and resolves the ambiguity by marking the variable
as not owning it.  The save owns it; the variable reads it until
`poplocalvars` hands it back or an assignment replaces it.

The Rust moves the buffer into the save and leaves `VarText::Fixed` in
the variable pointing at the same bytes, which is the same arrangement
with the ownership in the type rather than in a bit.  That is only sound
while the save is reachable, and it forces one ordering change: the save
is pushed onto the frame *before* `setvareq(name, flags)` rather than
after.  `setvareq` raises on a read-only variable -- `readonly x; f() {
local x=2; }` reaches it -- and a save still sitting in a local would be
dropped by the unwind, leaving the variable borrowing freed bytes.  The C
leaks the `localvar` on that path instead and leaves the variable
`VSTRFIXED|VTEXTFIXED` for the rest of the process; neither is observable,
because `VREADONLY` makes every later `setvareq` on that name fail before
it reads either bit.

### The saved option vector is distinguished by a null, not a tag

`poplocalvars` asks `vp == NULL` to mean "this save holds `optlist`, not a
variable", then asks `lvp->flags == VUNSET` to mean "the variable did not
exist, and `text` was never written".  Two fields doing type dispatch, one
of them for a record whose third field is deliberately uninitialised.

Three variants replace it, which removes the null check and makes the
uninitialised field unrepresentable.  The `VUNSET` sentinel had to be
shown sound before it could be dropped: a stored variable can never have
flags exactly `VUNSET`, because `setvareq` only files an entry when the
incoming or inherited flags carry `VEXPORT`, `VREADONLY` or `VSTRFIXED`
as well -- every other combination reaches the removal path.  So the C's
test could not misfire, and the enum is a representation change rather
than a fix.

### `envp` is pointers into the table, and the table must not move

`listvars` collects `vp->text` and `exec.rs:shellexec` hands the array to
`execve`.  The strings are not copied at any point: the child's
environment is read straight out of the parent's variable table.  Nothing
between `environment()` and `execve` touches `vartab`, which is what makes
it safe in the C and in the port -- but the port has one extra
requirement, that the buffer behind a `VarText::Owned` cannot move while
the array is live.  `Box<[u8]>` cannot reallocate, which is why the text
is a boxed slice and not a `BString`; a growable buffer would make the
`envp` array depend on nobody appending to a variable's bytes.

`localvar::Saved.vp` needs the same guarantee for the node rather than the
text: it holds a `*mut var` across a whole function invocation, during
which `setvareq` may file or remove other names.  A `BTreeMap` that owned
its values inline would move them on a split, so the map's value is
`Box<var>` for a variable it owns and a bare pointer for one of
`varinit`'s sixteen statics.  This is asserted rather than argued --
`var.rs`'s `a_saved_entry_does_not_move` files two hundred names that sort
below the saved one and checks `findvar` still answers the same address.

### `putenv` holds a pointer the shell can still free

`changelocale` is called from the `LC_ALL`, `LC_COLLATE`, `LC_CTYPE`,
`LC_NUMERIC` and `LANG` entries of `varinit`, all of which carry `VFULL`,
so the argument is the whole `NAME=value` -- and glibc's `putenv` stores
that pointer in `environ` without copying.  `setlocale(LC_ALL, "")` on the
next line reads `environ` back.

Assigning to one of those names again is fine: the old buffer is dropped
and `putenv` overwrites the slot with the new one, and the window between
the two contains no libc call.  The port's window is *shorter* than the
C's, which frees `vp->text` before the flag arithmetic rather than at the
store.  `unset LC_ALL` is the case that is wrong in both: the buffer is
freed, `varfunc` no longer sees `VFULL` -- `bits` does not inherit it on
that path -- so `changelocale` gets the empty string, `putenv("")` fails
with `EINVAL` because there is no `=`, and the slot in `environ` keeps
pointing at freed memory that `setlocale` then reads.  That is dash's, not
the port's, and it is reproduced rather than fixed: `docs/std-replacements.md`
section 7 records that the `putenv` can only go when the locale question is
answered.

### `setvar`'s buffer is one byte longer than it looks

`p = mempcpy(nameeq, name, namelen + 1)` copies the name *and the byte
after it*, which is the `=` or the NUL that `endofname`/`strchrnul` stopped
at, and `p[-1] = '='` writes over that byte only when there is a value.
So an unset variable's buffer is `NAME\0\0` and `varnull` -- the accessor
every reader of an unset variable's value goes through -- returns a
pointer to the second NUL.  Rebuilding that as "name, then `=`, then
value, then NUL" gets the set case right and the unset case one byte
short, and the failure is a read past the end of the allocation rather
than anything a test would print.  `var.rs`'s
`setvar_files_a_name_equals_value` pins both layouts.

### `VNOSAVE` never crosses the module boundary, checked twice

`setvareq(s, flags | VNOSAVE)` means "the table adopts this allocation
without copying", and the previous pass established that neither of
`evalcommand`'s two call sites passes it.  Re-checked here, on this tree:
`vflags` is 0 or `VEXPORT`, `mklocal` adds `VSTRFIXED`, and `mkinit_init`
passes `VTEXTFIXED` -- so `setvar` is the only caller that has ever set
it, and it now hands its buffer over as a `VarText::Owned` instead.  The
flag survives, because a stored variable's `flags` keep it and
`poplocalvars` restores them, but the public `setvareq` asserts it is
absent: the signature it has cannot express adoption, so a caller passing
it would be asking for a leak.

### What the variable table left behind

`var.rs` has no `ck*`, `savestr` or `libc::free` call site left, and no
`crate::memalloc` import.  `struct var`, `struct localvar` and
`struct localvar_list` are owned Rust values; `varinit`'s sixteen entries
are still a `static mut` array, because `vifs`/`vps1`/... address them
positionally and `lookupvar` compares against `vlineno()` by address.
## What this cost in the port: five small containers

Written after `redir.rs`'s saved-descriptor stack, `trap.rs`'s action
table, `alias.rs`'s nodes, `options.rs`'s positional parameters and
`cd.rs`'s two working directories became owned values.  The five are
unrelated to each other and landed as five commits; what they have in
common is that each one's C representation was carrying a fact the Rust
has to carry deliberately.

### `curdir` and `physdir` are one allocation, and a guard says so

`cd.c:setpwd` frees `physdir` only `if (physdir != oldcur)`.  That test
is not defensive: after `setpwd(NULL, …)` -- which is what `var.c`'s
`INIT` does when `$PWD` does not name the current directory -- `curdir`
and `physdir` are the *same* `getpwd()` result, because the C writes
`physdir = s` and then `dir = s`.  The guard exists to stop the double
free and for no other reason.  Two owned copies say the same thing about
the shell and remove the question; the extra copy is one path per `cd`.

Its sibling, `oldcur == val`, is a different kind of thing: it is the C
asking "is my caller `pwdcmd`, handing me `curdir` itself?", and it is
the only way that call can be distinguished from `setpwd(p, …)` with a
path that happens to compare equal byte for byte.  Owned values have no
pointer to compare, so the three calls the two tests separate became
three arms of an enum.  `Pwd::Current` is `pwdcmd`'s.

### `nullstr` is a sentinel `cd.c` reads by identity, never by content

`cd.h`'s contract, restated in `[spec:dash:sem:cd.getpwd-fn]`: `getpwd`
returns `nullstr` on failure and "callers detect it by pointer identity,
not by content, and must never free".  `curdir == nullstr` in `updatepwd`
and `physdir == nullstr` in `pwdcmd` are both that test.

`Option<BString>` is exact here only because no reachable value collides
with the sentinel: `getcwd` cannot return an empty string on success, and
`updatepwd` always emits at least the leading slash before its `lim`
floor stops the `..` pops.  If either could produce an empty path the
mapping would silently merge "failed" with "the empty path", so the
argument, not the type, is what makes this safe.

### `strtok` put `updatepwd`'s parse position in a libc static

The last piece of shell state that was not merely process-global but
*libc*-global.  Two `strtok` calls in `updatepwd` shared the hidden
cursor with every other `strtok` caller in the process, which does not
matter for a shell that owns its process and does matter for a shell
that is becoming a library: any host code calling `strtok` between the
two would have redirected the walk.

`split_str(b"/")` filtered for empty fields is byte-identical, and the
filter is why: `strtok` never yields an empty token, so runs of slashes
collapse the same way and the two `cdcomppath` advances past the leading
slashes -- pure `strtok` bookkeeping -- could go with it.  The rest of
`updatepwd` is untouched, because `cd -L` is textual and
`docs/std-replacements.md` §5.3 measures four ways `std::path` would get
it wrong.

### `onsig` reads the trap table from a kernel signal frame

`trap[NSIG]` is written by `trapcmd` under `INTOFF` and read by `onsig`,
which the kernel can call between any two instructions.  Whatever the
table is made of, `onsig`'s two questions -- is `trap[SIGCHLD]` set, is
`trap[SIGINT]` set -- must compile to what `!= NULL` compiled to.
`Option<BString>` does: `Vec`'s pointer is `NonNull`, so the `None`
discriminant is the null pointer word and `is_none()` is that load.  No
allocation, no lock, nothing that is not async-signal-safe.

The write side needed the ordering thought about, and it is the reverse
of what a first reading suggests.  The C frees the old action and *then*
stores the new one, so the slot spends a few instructions holding a
dangling non-NULL pointer.  That is not a bug for `onsig`, which never
dereferences it: the slot reads "a trap is set" for the whole window,
which is the same answer before and after.  A `take` followed by a store
would have made the slot read `None` in the middle -- and under `INTOFF`
that is the difference between `intpending` being set and not.
`mem::replace` keeps the C's answer and leaves nothing stale.

### `dotrap` hands `evalstring` the slot's own pointer, and the C reads it freed

`dotrap` does `p = trap[i + 1]` and then `evalstring(p, 0)`, which parses
directly out of that buffer.  A trap action is allowed to run `trap`,
including on its own signal, and `trapcmd` frees the slot's old action.
`trap 'trap - INT; echo gone; echo more' INT` reaches it: reference dash
prints both lines, from memory it has already freed.

Copying the action before running it is what makes ownership work at all
here, and it is also what lets `clear_traps` stop leaking.  The C's
`*tp = NULL; … ckfree(*tp)` frees NULL and leaks the action
(`trap.c:189`); dropping the taken value frees it instead, which is only
safe because no reader holds a pointer into a slot any more.  `exitshell`
takes `trap[0]` for the same reason the C sets it to NULL without
freeing.

### `strpush.string` is where an alias's buffer changes owner

The one place in these five where the C's ownership could not be moved
into Rust, and it is worth stating precisely because it looks like it
should be easy.

`alias.c:setalias` skips `ckfree(ap->name)` when `ap->flag & ALIASINUSE`
and then re-points `ap->name` at a fresh `savestr`.  That reads as a
deliberate leak so the in-flight reader keeps a valid pointer.  It is not
a leak: `input.c:popstring` ends with

    if (sp->string != sp->ap->name)
            ckfree(sp->string);

and `sp->string` is the `ap->name` that was current when `pushstring`
ran.  So the test means "did `setalias` re-point this alias under me?",
and when it did, ownership of that buffer transfers from `alias.c` to
`input.c`, which frees it with `free`.  An owned `BString` there would be
a Rust allocation freed by libc.

The path is reachable in two lines, because `parsecmd` returns at the
newline inside an alias body, so the redefinition *executes* while the
body is still being read:

    alias a='alias a=zzz
    echo hi'
    a

Reference dash prints `hi` from the old buffer and then lists `a=zzz`,
and valgrind reports no leak -- which is only possible if `popstring`
did the free.  Deferring the re-point until the reader finishes is not a
way out either: `alias` run from *inside* the same body prints the new
value, so the C's re-point is immediate and observable.

What did convert is the node.  `input.rs` holds an entry's address in a
`parsefile` for as long as the alias is being read from, so the map's
values are `Box<alias>`; a `BTreeMap` moves its values when it
rebalances, and the `Box` is what stops that being observable.  The name
buffer waits for `input.rs:popstring` to stop freeing it.

### A redirection frame's address is stable; its index is what survives

`redir.c`'s `sv` is a `redirtab *` held across the whole redirection
loop, and the loop calls `openredirect`, which for a here-document
reaches `expandarg`, which reaches command substitution, which runs
`evalcommand`, which pushes and pops redirection frames of its own.  In
the C that is free: `sv` points at its own `ckmalloc` block and nothing
moves it.  A `Vec` reallocates.

So `redirect` holds the frame's *index* and re-indexes on every use
rather than borrowing across the loop.  Nested activity is balanced --
`evalcommand`'s `unwindredir(redir_stop)` pops back to its own depth --
so the index still names the same frame when control returns, and an
exception that unwound past it would have abandoned the loop anyway.
Indexing also converts what would have been a dangling frame pointer
into a panic.

`pushredir`'s return value was a `redirtab *` used only as a mark for
`unwindredir` to compare against; a stack in a vector says the same thing
with a depth, which is `eval.rs:931`'s one-line change.

The slots stay `c_int`.  `docs/std-replacements.md` §4.9 item 5 is the
reason and it is unchanged by this: `popredir` restores with `dup2` and
then closes, it is reached from the unwind path through
`mkinit_exitreset`, and giving a slot a destructor moves descriptor
closes to points the C never had them.  `mkinit_forkreset`'s
`redirlist = NULL` abandons frames without restoring or closing anything,
and `Vec::clear` over integer slots abandons them the same way; over
`OwnedFd`s it would not.

### `savefd`'s EBADF-as-success is reachable from two tokens

`docs/std-replacements.md` §8 item 5 listed this as inferred rather than
traced.  Traced now.  `dash -c 'exec 9>&-; { :; } 9>&1'` under `strace`:

    fcntl(9, F_DUPFD_CLOEXEC, 10)    = -1 EBADF (Bad file descriptor)
    dup2(1, 9)                       = 9
    close(9)                         = 0

`savefd`'s `if (err != EBADF)` guard skips both the `close(ofd)` and the
`sh_error`, so duplicating a closed descriptor returns -1 and the caller
carries on; `renamed[9]` stays `CLOSED` and `popredir` closes 9.  The
port produces the identical three syscalls.  Any future `OwnedFd`
conversion has to reproduce it, and `try_clone()` cannot -- it returns
`Err` and a `?` propagates.

### `shellparam.malloc` and `shellparam.p` are one fact said twice

A flag saying who owns the words and a pointer to them.  They became one
field: `owned` is `Some` exactly where `malloc` was 1, `borrowed` is the
`argv` `p` pointed into when it was 0.

What the words alone cannot carry is `getopts`, which is real pointer
arithmetic over a `char **` -- `optnext[-1]`, `optnext - optfirst` -- and
whose `optfirst` is *either* `shellparam.p` or a pointer into the
builtin's own `argv` (`getoptscmd` picks by argument count).  One uniform
array is not a convenience there, it is the interface.  So the owned form
keeps a `ptrs` vector beside `words`, holding each word's address and a
NULL, rebuilt whenever `words` changes; each word carries its own
terminator because every reader reads it as a C string.  The invariant
that falls out: a word may be dropped or the list rebuilt, but a word
must never be resized in place.

`shift` needs both halves.  With owned words it drains the front and
reindexes.  With borrowed ones the C shifts the array down *in place*,
which rewrites the caller's `argv` -- that is not an accident, it is how
`shift` inside a function works, and `evalcommand` discards the array
when the call returns.

### `saveparam = shellparam` is a copy the C immediately disarms

`evalfun` copies the whole struct and then, two lines later and inside
the protected region, sets `shellparam.malloc = 0`.  The second statement
is what makes the first safe: without it the epilogue's
`freeparam(&shellparam)` would free the words `saveparam` still points
at, and `shellparam = saveparam` would restore the freed pointers.

That is a move written as a copy plus a disarm, and it is why `shparam`
being `Copy` was load-bearing rather than incidental.  `takeparam` is
both statements at once.  The one behavioural difference is the window
between it and `borrowparam`: the C leaves the outer parameters readable
there and the port leaves none.  Nothing in that window -- `INTOFF`,
`handler = jl`, `reffunc`, two assignments, `INTON` -- reads a positional
parameter, and on the C's side the same window is the one its `malloc = 0`
exists to make safe.
## What this cost in the port: the job table and the process array

Written after `jobtab` became a `Vec<Job>` and `job.ps` a `Vec<ProcStat>`.
`jobs.c` is the one place in dash whose *comments* admit that its container
moves under its readers, so most of this section is the C's own warning
re-read against the Rust rather than something the pass discovered.

### `growjobtab` names its three kinds of interior pointer, and they are the whole step

`growjobtab` `ckrealloc`s `jobtab` and then, if the block moved, walks the
table backwards adjusting everything that pointed into it: each entry's
`prev_job`, each entry's `ps` *but only where it pointed at that same
entry's inline `ps0`*, and finally `curjob`.  The `joff`/`jmove` macro pair
exists to spell "the same field of the entry at offset `l`, in the new
array", because at that moment the old and new arrays are two different
addresses for the same logical table.

`Vec<Job>` has exactly this hazard on `push`, and the C's enumeration is
also the fix list: `curjob` and `prev_job` become `Option<usize>` and `ps`
becomes owned, after which `growjobtab` is four `push`es and the relocation
pass has nothing to relocate.  `jobno`, which recovered a job number by
subtracting `jobtab` from a pointer, becomes `jp + 1`.

The set of pointers the C repairs is not the set of pointers that exist.
`makejob` returns a `struct job *` that `evalsubshell`, `evalpipe`,
`evalbackcmd`, `evalcommand` and `vforkexec` each keep in a local across
one or more `forkshell`s, and `growjobtab` cannot reach a local.  Those
survive only because nothing between a `makejob` and its `waitforjob`
calls `makejob` again: `evalpipe`'s loop forks `pipelen` times without
creating a second job, `expbackq` holds `backcmd.jp` across a `read` and
not across any evaluation, and `evalcommand` waits on the job `vforkexec`
has just made.  That is a real invariant the C never states, and it would
have to be re-established every time one of those five callers changed.
Holding an index means not having to know it.

### `job.ps` aliasing `job.ps0` is a self-referential struct, and the port had it

`makejob` sets `ps = &jp->ps0` for a single-process job and `ckmalloc`s
only for a pipeline, and the port reproduced this faithfully -- `ps0` was a
field and `ps` pointed at it.  A `struct job` therefore could not be moved
without repair, which is what `growjobtab`'s `ps` arm is for.  It was
*correct* in the port, because the port also carried the relocation; but
the moment `jobtab` became any container that moves its elements without
running that pass, every single-process job's `ps` would have dangled into
the old allocation, silently, with `nprocs == 1` and no null to trip on.
Deleting the optimisation is what makes `Vec<Job>` expressible at all; the
inline slot bought one `malloc` per foreground command and cost the whole
representation.

`nprocs` goes with it.  The C sizes the array from `makejob`'s argument and
counts the processes it has actually forked in a separate 16-bit bitfield;
a `Vec` is both at once, so `ps.len()` is `nprocs` and `forkparent` pushes
where it used to write through `&jp->ps[jp->nprocs++]`.

### A job reaches the current-job chain before it can have any processes

Three readers index `ps[0]` unconditionally and two index `ps[nprocs - 1]`,
which for `nprocs == 0` is `ps[-1]`.  The C gets away with it because a job
with no processes is not supposed to be reachable -- and it is.  `evalpipe`
calls `makejob(pipelen)`, which links the job into the chain with `used`
set, and only then opens the pipe; a failing `pipe(2)` raises `"Pipe call
failed"` out of `evalpipe` without freeing it.  What is left on the chain
is a `used`, `JOBRUNNING`, zero-process job that `jobs`, `kill %n`,
`wait %n` and `%string` can all find, and whose `ps0` is the zeroed one
`memset` left.  In the C, `jobs` hands `ps0.cmd == NULL` to `%s` and
`getjob`'s name match hands it to `prefix`.

The Rust answers for that job rather than reading past the end: `ps_pid`
gives the zero the C reads out of `ps0` and `ps_cmd` gives the null string
where the C reads a null pointer, `wait %n` on it reports 0, and it matches
no name and no pid.  This is the only place in the file where the behaviour
is *chosen* rather than ported, and it is chosen because what the C does
there is undefined.  The underlying leak is `evalpipe`'s, not `jobs.c`'s,
and it is still there.

### `freejob` unlinks a job and deliberately leaves its own `prev_job` alone

`set_curjob(jp, CUR_DELETE)` rewrites the link that *arrived at* `jp`; it
never touches `jp->prev_job`.  Two loops depend on that and neither says
so: `showjobs` calls `showjob`, which calls `freejob` on a finished job,
and then steps to `jp->prev_job`; `forkchild` frees every job on the chain
with `for (jp = curjob; jp; jp = jp->prev_job) freejob(jp)`, walking
through each entry after freeing it.  A `freejob` that cleared `prev_job`
-- the obvious thing to do when a job becomes unused -- would truncate the
first walk to one job and the second to one iteration, and the corpus
cannot see either, because both need job control.

The Rust keeps `prev_job` and empties `ps` instead, which also makes a
second `freejob` on the same job a no-op where the C would free every
`ps[i].cmd` twice.

### What `forkchild` may do in a `vfork` child, and what a `Vec` nearly added

`vforkexec` is the one path in the file that runs in a shared address space:
`vfork`, then `forkchild`, then `shellexec` to `execve`, with nothing in
between allowed to allocate, free or drop ([dec:nsh:owned-data] records
`exraise` under `vforked` and `shellexec` as the two paths that correctly
skip destructors).  `forkchild` guards this with `lvforked`, and everything
above that guard reads the job table without writing to it: `jp->jobctl`,
`jp->nprocs`, `jp->ps[0].pid`.  All three stay reads under the new
representation, and `freejob` -- the only thing in `forkchild` that frees --
is below the guard, where the child is a real `fork` and dash already calls
`ckfree`.  The `FORK_BG` arm, which opens `/dev/null`, is not on the `vfork`
path either: `vforkexec` passes `FORK_FG`.

The trap to avoid was making the job table's readers take `&mut` and having
a `Job` move, or giving `forkchild` a local `Vec`; neither is present, and
the count of allocator calls between `vfork` and `execve` is still zero.
