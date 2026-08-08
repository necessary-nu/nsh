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
