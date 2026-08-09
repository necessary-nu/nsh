# src/memalloc.c, src/memalloc.h

Two allocators. The checked heap wrappers (`ckmalloc`, `ckrealloc`,
`savestr`, and the `ckfree` macro which is plain `free`) turn allocation
failure into a shell error instead of a NULL return. Everything else
implements the *shell stack*: a LIFO arena used for parse trees and for
strings built a character at a time, so that an interrupt in the middle
of a parse can be recovered from by resetting a mark rather than by
unwinding individual frees.

**Rules retired.** The `delete-memalloc` change removes the target allocator
module after the shell-stack arena became unreachable and its checked heap
wrappers' final callers moved to owned Rust values. The C sources remain
unchanged as reference material. The blocks below retain their C signatures
and semantics, but carry no `[spec:dash:...]` IDs because there is no longer a
target implementation to claim.

The arena is a chain of `struct stack_block`, each holding a `prev` link
and a `space[]` payload, headed by the statically allocated `stackbase`
so the allocator works before any heap allocation. Four globals track the
current position and must always be consistent:

- `stackp` — the block being allocated from.
- `stacknxt` — the next free byte within it.
- `stacknleft` — bytes remaining after `stacknxt`.
- `sstrend` — `stacknxt + stacknleft`, cached so the hot string-building
  path tests a single pointer comparison.

`MINSIZE` is `SHELL_ALIGN(504)`; every request is rounded up with
`SHELL_ALIGN` so returned pointers stay suitably aligned. Because
`stackbase` is static and never freed, `popstackmark` can free every
block above a mark unconditionally.

The `stackblock()`/`stackblocksize()` macros expose the unallocated
remainder of the current block as a scratch buffer: code writes into it,
then either commits with `grabstackstr`/`stalloc` or abandons it. The
`ST*` macros (`STPUTC`, `USTPUTC`, `STADJUST`, `STUNPUTC`, `STTOPC`,
`STACKSTRNUL`, `CHECKSTRSPACE`) build strings there.

> static void *checknull(void *p)

> Return `p`, but call `outofspace()` first when it is NULL. The single
> choke point that converts a failed allocation into a shell error.

> __attribute__((__noinline__)) void *ckmalloc(size_t nbytes)

> `malloc(nbytes)` passed through `checknull`, so it either returns
> usable memory or does not return at all. Callers never test the result.
> `noinline` keeps the error path out of hot callers; no behavioural
> effect.

> __attribute__((__noinline__)) void *ckrealloc(void *p, size_t nbytes)

> `realloc(p, nbytes)` passed through `checknull`. Note that on failure
> the original block leaks, since the error path unwinds past the caller
> that still held the only other reference — acceptable because the
> shell is about to abandon that allocation context anyway.

> static inline void grabstackblock(size_t len)

> Commit the first `len` bytes of the current stack block as a permanent
> allocation: simply `stalloc(len)`, discarding the returned pointer.
> Used to reserve scratch space that has already been written into.

> static char *growstackblock(size_t min)

> Enlarge the current scratch block to hold at least `min` bytes,
> preserving its contents, and return a pointer to the (possibly moved)
> block. Compute `newlen = stacknleft * 2`, treating wraparound as
> `outofspace()`. Round the requested minimum up with
> `SHELL_ALIGN(min | 128)` — the `| 128` sets a floor so tiny requests
> still grow the block usefully — and if doubling was not enough, add
> that rounded minimum to `newlen`.
>
> Then take one of two paths. If the scratch area starts exactly at the
> beginning of the current block (`stacknxt == stackp->space`) and that
> block is not the static `stackbase`, the block holds nothing but
> scratch and can be resized in place: with interrupts suspended, save
> `sp->prev`, `ckrealloc` the whole block to
> `newlen + sizeof(struct stack_block) - MINSIZE`, restore the `prev`
> link (which `realloc` may have moved), and reset `stackp`, `stacknxt`,
> `stacknleft` and `sstrend` to describe the enlarged block.
>
> Otherwise the block also holds committed allocations that must not
> move. Remember the old scratch address and length, `stalloc(newlen)` —
> which allocates a fresh block, since `newlen` exceeds what is left —
> then `memcpy` the old contents to the new space and set `stacknxt` to
> it, adding `newlen` back to `stacknleft`.
>
> Note the saved length is `int oldlen = stacknleft;` — an `int`, while
> `stacknleft` is a `size_t`. A scratch block larger than `INT_MAX`
> therefore copies a truncated (and, once sign-extended at the `memcpy`
> call, wrong) length. Reproduce the truncation; it is unreachable in
> practice but it is observable behaviour, and a port that silently
> widens it has changed the program. The effect is to un-commit
> the space `stalloc` just took, so the new block's scratch area starts
> at the copied data.

> void * growstackstr(void)

> Grow the string under construction by at least one byte and return the
> write position within the moved buffer. Record the current length
> `stackblocksize()`, call `growstackblock(0)` (which doubles, so passing
> a minimum of 0 is safe), and return the new block base plus that
> length. Called by `STPUTC`/`STACKSTRNUL` when the write pointer has
> reached `sstrend`.

> __attribute__((__noinline__)) char *growstackto(size_t len)

> Ensure the scratch block holds at least `len` bytes and return its
> base. If `stackblocksize()` is already at least `len` return
> `stackblock()` unchanged; otherwise `growstackblock(len)`.

> __attribute__((__noinline__)) char *makestrspace(size_t newlen, char *p)

> Ensure `newlen` more bytes are writable at `p`, where `p` points into
> the current scratch block, and return the equivalent position in the
> possibly moved block. Compute the offset `len = p - stacknxt`, call
> `growstackto(len + newlen)`, and add `len` back to the returned base.
> Returning a *rebased* pointer is what makes the `ST*` macros safe
> across reallocation — the caller must always adopt the result.

> static __attribute__((__always_inline__)) inline void outofspace(void)

> Raise `sh_error("Out of space")`, which unwinds and does not return.

> void popstackmark(struct stackmark *mark)

> Release everything allocated since `mark` was set. With interrupts
> suspended, free blocks from the top of the chain until `stackp` equals
> `mark->stackp`, then restore `stacknxt` and `stacknleft` from the mark
> and recompute `sstrend = mark->stacknxt + mark->stacknleft`. Restore
> interrupts. Every allocation above the mark becomes invalid at once,
> which is the whole point: no per-object bookkeeping.

> __attribute__((__noinline__)) void pushstackmark(struct stackmark *mark, size_t len)

> Record the current stack position into `mark` — `stackp`, `stacknxt`
> and `stacknleft` — then `grabstackblock(len)` to commit `len` bytes
> above it. Committing the leading bytes protects data that lives at the
> mark boundary from being reclaimed by the matching `popstackmark`.

> char * savestr(const char *s)

> `strdup(s)` through `checknull`: a heap copy that must be released with
> `ckfree`, as opposed to `sstrdup` which copies onto the shell stack.

> void setstackmark(struct stackmark *mark)

> The ordinary way to take a mark: `pushstackmark(mark, len)` where `len`
> is 1 exactly when the current position sits at the very start of a
> dynamically allocated block (`stacknxt == stackp->space` and
> `stackp != &stackbase`), and 0 otherwise. Reserving that one byte stops
> `growstackblock` from later deciding the block is pure scratch and
> `realloc`ing it out from under the mark, which would leave
> `mark->stackp` dangling.

> struct stack_block {
>   struct stack_block *prev;
>   char space[MINSIZE];
> }

> struct stackmark {
>   struct stack_block *stackp;
>   char *stacknxt;
>   size_t stacknleft;
> }

> void *stalloc(size_t nbytes)

> Allocate `nbytes` from the shell stack. Round up with `SHELL_ALIGN`. If
> that exceeds `stacknleft`, add a block: take `blocksize` as the aligned
> request but no less than `MINSIZE`, compute the gross allocation
> `sizeof(struct stack_block) - MINSIZE + blocksize`, and treat overflow
> (`len < blocksize`) as `outofspace()`. With interrupts suspended,
> `ckmalloc` it, link it in front via `prev`, and repoint `stacknxt`,
> `stacknleft`, `sstrend` and `stackp` at the new block. Any space left
> in the old block is abandoned, not reused.
>
> Then take the pointer at `stacknxt`, advance `stacknxt` and decrease
> `stacknleft` by the aligned size, and return it. Note `sstrend` is not
> updated here — it is only meaningful for the scratch-block macros,
> which reset it via the paths that move blocks.

> __attribute__((__noinline__)) char *stnputs(const char *s, size_t n, char *p)

> Append exactly `n` bytes of `s` to the stack string at `p`: reserve
> space with `makestrspace(n, p)` (adopting the rebased pointer), copy
> with `mempcpy`, and return the position just past the copied bytes. No
> NUL is written.

> static inline char *_STPUTC(int c, char *p)

> Body of the `STPUTC` macro. If `p` has reached `sstrend` there is no
> room, so replace it with `growstackstr()`. Store `c` at `p`, advance,
> and return the new position. `USTPUTC` is the unchecked form — a bare
> `*p++ = c` — which callers may use after `CHECKSTRSPACE` has
> guaranteed room.

> char * stputs(const char *s, char *p)

> `stnputs(s, strlen(s), p)` — append a NUL-terminated string without its
> terminator, returning the new write position.

> void stunalloc(void *p)

> Give back the most recent allocation(s) down to `p`, which must lie
> within the current block: add `stacknxt - p` back to `stacknleft` and
> set `stacknxt = p`. Under `DEBUG`, first validate that `p` is non-NULL
> and lies between `stackp->space` and `stacknxt`, writing `"stunalloc\n"`
> to fd 2 and `abort()`ing otherwise. Only unwinds within the current
> block — it cannot pop back into a previous one, which is what
> `popstackmark` is for.
