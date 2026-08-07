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
