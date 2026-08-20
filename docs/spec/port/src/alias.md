# src/alias.c, src/alias.h

The alias table is a fixed array of `ATABSIZE` = 39 singly-linked bucket
chains (`atab`), keyed by `hashval(name) % ATABSIZE`. Each entry owns one
allocation holding `"name=value"`; `ap->name` points at its start and
`ap->val` points into the same block just past the `=`, so freeing
`ap->name` releases both. `flag` carries `ALIASINUSE` (1) — the alias is
currently being expanded by the parser, so its storage must not be
released — and `ALIASDEAD` (2) — deletion was requested while in use and
must happen when the expansion finishes.

**Dash source shape (`alias.alias`):**

    struct alias {
      struct alias *next;
      char *name;
      char *val;
      int flag;
    }

**Dash source shape (`alias.aliascmd-fn`):**

    int aliascmd(int argc, char **argv)

> [spec:dash:sem:alias.aliascmd-fn]
> The `alias` builtin. With `argc == 1` (no operands), walk every bucket
> `0..ATABSIZE` and every entry in each chain in list order, calling
> `printalias` on each, and return 0. Output order is therefore hash
> order, not sorted (the source carries a `TODO - sort output`).
> Otherwise iterate the operands by pre-incrementing `argv` until NULL.
> For each operand `n`: search for `=` starting at `n + 1`, not at `n`,
> so a leading `=` is part of the name rather than a separator (this
> offset is deliberate ksh compatibility inherited from 44BSD-lite). If
> `n` is the empty string, or no `=` is found, treat the operand as a
> query: look it up with `__lookupalias`; if absent write
> `"alias: <n> not found\n"` to `out2` and set the return value to 1; if
> present call `printalias`. If an `=` was found at `v`, define the alias
> with `setalias(n, v + 1)`. Return 1 if any query failed, else 0.
> Definition failures do not set the return value — `setalias` raises a
> shell error instead.

**Dash source shape (`alias.freealias-fn`):**

    alias * freealias(struct alias *ap)

**Retired C-only behavior (`alias.freealias-fn`):**
> Release one alias entry and return the pointer that should replace it
> in its chain. If `ap->flag` has `ALIASINUSE` set the entry is being
> expanded and must not be freed: set `ALIASDEAD` on it and return `ap`
> itself, leaving it linked. Otherwise save `ap->next`, `ckfree(ap->name)`
> (which releases the combined name/value block), `ckfree(ap)`, and
> return the saved next pointer. The "returns what to store in the slot"
> convention is what lets callers splice the chain without re-testing
> the in-use case.

**Dash source shape (`alias.lookupalias-fn`):**

    alias ** __lookupalias(const char *name)

**Retired C-only behavior (`alias.lookupalias-fn`):**
> Internal chain search returning the *address of the link* holding the
> match, so callers can delete or insert in place. Compute the bucket as
> `hashval(name) % ATABSIZE` and take `app = &atab[bucket]`. Walk
> `app = &(*app)->next` while `*app` is non-NULL, stopping at the first
> entry for which `varequal(name, (*app)->name)` is true — `varequal`
> compares up to a terminating `=` or NUL on either side, which is what
> allows a bare name to match a stored `"name=value"` block. Return the
> address of the matching link, or the address of the trailing NULL link
> if there is no match; the result is never NULL, and `*result == NULL`
> is the "not found" test.

**Dash source shape (`alias.lookupalias-pub-fn`):**

    struct alias * lookupalias(const char *name, int check)

> [spec:dash:sem:alias.lookupalias-pub-fn]
> Public lookup. Dereference `__lookupalias(name)` to get the entry or
> NULL. If `check` is non-zero and an entry was found that has
> `ALIASINUSE` set, return NULL instead of the entry — this is what stops
> the parser recursing infinitely on a self-referential alias while that
> alias is still being expanded. Otherwise return the entry (possibly
> NULL).
>
> The historical extractor folded this symbol into
> `alias.lookupalias-fn` after stripping the leading underscores from the
> distinct static function `__lookupalias`. The Rust implementation keeps
> only the public lookup behavior; the chain-address topology is retired.

**Dash source shape (`alias.printalias-fn`):**

    void __attribute__((noinline)) printalias(const struct alias *ap)

> [spec:dash:sem:alias.printalias-fn]
> Write one alias definition to `out1` as `single_quote(ap->name)`
> followed by a newline (`snlfmt` is `"%s\n"`). Because `ap->name` is the
> combined block, this prints `name=value` in one go, with the whole
> `name=value` string shell-quoted so the output can be re-read as input.
> Declared `noinline` purely to keep the caller's code size down; that
> has no observable behavior.

**Dash source shape (`alias.rmaliases-fn`):**

    void rmaliases(void)

> [spec:dash:sem:alias.rmaliases-fn]
> Delete every alias. With interrupts suspended (`INTOFF` .. `INTON`
> around the whole sweep), for each bucket `0..ATABSIZE` set
> `app = &atab[i]` and loop while `*app` is non-NULL: remember
> `ap = *app`, store `freealias(*app)` back through `app`, and if the
> stored value is the same entry (`ap == *app`, meaning `freealias`
> refused because the alias was in use and merely marked it `ALIASDEAD`)
> advance `app = &ap->next` to step past it. Otherwise leave `app` where
> it is, since the slot now holds the next unexamined entry. In-use
> aliases survive the sweep as dead entries and are reclaimed when their
> expansion completes.

**Dash source shape (`alias.setalias-fn`):**

    STATIC void setalias(const char *name, const char *val)

> [spec:dash:sem:alias.setalias-fn]
> Define or redefine an alias. `name` points at a `"name=value"` string
> and `val` points into that same string just past the `=`. First
> validate the name: scan characters from `name` and raise
> `sh_error("Invalid alias name: %s", name)` on the first whose
> `BASESYNTAX` class is not `CWORD`; the loop is do/while on `*++p != '='`
> so the `=` itself terminates the scan and an empty name is rejected by
> testing the first character before advancing. Then `__lookupalias(name)`
> for the chain slot. With interrupts suspended: if an entry exists,
> release its old storage with `ckfree(ap->name)` unless `ALIASINUSE` is
> set (in which case the block is still being read and is intentionally
> leaked), and clear `ALIASDEAD` so a pending deletion is cancelled by
> the redefinition. If no entry exists, `ckmalloc` one with `flag = 0`
> and `next = 0` and link it into the slot. Either way compute
> `namelen = val - name`, set `ap->name = savestr(name)` (a fresh copy of
> the whole `"name=value"` block), and `ap->val = ap->name + namelen` so
> the value points into the new copy at the same offset. Restore
> interrupts.

**Dash source shape (`alias.unalias-fn`):**

    int unalias(const char *name)

> [spec:dash:sem:alias.unalias-fn]
> Delete one alias by name. `__lookupalias(name)` for the slot; if it
> holds an entry, then with interrupts suspended replace the slot's
> contents with `freealias(*app)` — which unlinks and frees it, or marks
> it `ALIASDEAD` and leaves it linked if it is in use — and return 0.
> If the slot is empty return 1. The return value is "not found", the
> inverse of the usual success convention.

**Dash source shape (`alias.unaliascmd-fn`):**

    int unaliascmd(int argc, char **argv)

> [spec:dash:sem:alias.unaliascmd-fn]
> The `unalias` builtin. Parse options with `nextopt("a")`; on `-a` call
> `rmaliases()` and return 0 immediately, ignoring any operands. With no
> `-a`, walk the post-option operands via the global `argptr`, calling
> `unalias` on each; for every one that reports not-found write
> `"unalias: <name> not found\n"` to `out2` and set the result to 1.
> The result variable `i` is reused from the option loop and is
> initialized to 0 by the `for` statement, so a run with no failures
> returns 0. Return that result.
