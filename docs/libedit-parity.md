# What the libedit port had to make true — closed

`crates/dash/src/linedit.rs` now binds `nshedit`, the Rust
re-implementation of libedit, in place of the rustyline stand-in this
file was written against. The gap it measured is closed.

| | rustyline | nshedit |
|---|---|---|
| POSIX mismatches vs C dash | 40 | 2 |
| non-editing mismatches | 0 | 0 |
| interactive (pty) | 29/31 | 31/31 |
| cases the port failed | 95 | 54 |

The two that remain are the port passing cases dash fails —
`edit-history-goto-number` and `edit-history-search-pattern-anchored` —
which is a decision rather than a defect. See `docs/divergences.md`.

Three bugs in the binding, and what found each, since none was found by
reading code:

  * `el_init` derives its descriptors through a stub `fileno` that always
    answers -1, so the editor read fd -1, took EBADF for EOF and exited
    before a key was pressed. Found by a pty probe after the suite went
    40 -> 93; fixed by using `el_init_fd`. nshedit has since hidden
    `el_init` from Rust callers.
  * `el_gets` was reimplemented here and reported the encoded slice's
    length as the count. Under EL_UNBUFFERED those differ
    (ERR-core-api-26). Every test passed; the erratum found it, not the
    tests.
  * `el_set(EL_HIST)` was a no-op because the only way to attach a
    history was a C-variadic callback stable Rust cannot define. Fixed in
    nshedit by the `EditorHistory` trait; dash adapts its own store to it
    rather than moving onto nshedit's, because `fc`, `H_ENTER` and
    `H_APPEND` need the C-shaped face of the same object.

Kept as the record of what the swap cost and how each failure was
caught. The live acceptance check is one command:

    posix/harness/run.py --shell target/debug/dash \
        --reference tests/.build/ref/src/dash
