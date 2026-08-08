# src/var.c, src/var.h

The variable table: `VTABSIZE` = 39 hash-chain buckets in `vartab`. Every
`struct var` stores its text as a single `"name=value"` string, so the
name, the `=` and the value are one allocation and `text` can be handed
straight to `putenv`/`execve`. Flags:

| Flag | Value | Meaning |
|---|---|---|
| `VEXPORT` | 0x01 | exported to the environment |
| `VREADONLY` | 0x02 | cannot be modified |
| `VSTRFIXED` | 0x04 | the `struct var` is statically allocated |
| `VTEXTFIXED` | 0x08 | the text is statically allocated |
| `VSTACK` | 0x10 | the text lives on the shell stack |
| `VUNSET` | 0x20 | the variable is not set |
| `VNOFUNC` | 0x40 | do not run the change callback |
| `VFULL` | 0x80 | pass the whole `name=value` to the callback |
| `VNOSAVE` | 0x100 | the text is already heap-allocated; adopt it |

`VSTRFIXED`, `VTEXTFIXED`, `VSTACK` and `VNOSAVE` together encode who owns
the two allocations, which is what the ownership logic in `setvareq` and
`poplocalvars` turns on.

`varinit[]` holds the statically allocated built-in variables in a fixed
order — `var.h` addresses them by index through the `vifs`, `vmail`,
`vpath`, `vps1`, … macros, and the `*val()` accessors skip the name by a
hard-coded byte count (`ifsval()` is `vifs.text + 4`). Adding a variable
anywhere but the end therefore breaks those macros; the port should
prefer named fields and drop the offset arithmetic in Wave 4, but Wave 2
must keep the order. Entries carry change callbacks: `changeifs`,
`changemail`, `changepath`, `getoptsreset`, `sethistsize`, and
`changelocale` for the `LC_*`/`LANG` group.

Defaults: `defpathvar` is
`"PATH=/usr/local/sbin:/usr/local/bin:/usr/sbin:/usr/bin:/sbin:/bin"`
and `defifsvar` is `"IFS= \t\n"`; `defpath` and `defifs` are those
strings offset past the name. `linenovar` is a fixed buffer holding
`"LINENO="` plus room for the number, rewritten on demand from `lineno`.

Local variables form a stack of stacks: `localvar_stack` is a list of
`struct localvar_list`, one per active function invocation, each holding
the `struct localvar` saves made in that invocation.

**Rules retired: the hash table.** `sorted-tables` replaced `vartab` with
a `BTreeMap` keyed by variable name, so five of the symbols below have no
counterpart in the port and their rule ids are retired: `hashval` and
`hashvar`, which existed only to choose a bucket; `varcmp` and
`varequal`, whose "compare up to the first `=`" *is* the map key, so the
comparison belongs to the container; and `vpcmp`, the `qsort` comparator
`showvars` no longer needs now that `listvars` yields its output in name
order. `struct var` loses its `next` field with them. What the port does
is what the C's own comment above `showvars` wishes for — "Maybe we could
keep them in an ordered balanced binary tree instead of hashed lists".
The five blocks are kept, without ids, because they still describe
`src/var.c`, which still has all five. `docs/divergences.md` records the
observable half.

> [spec:dash:def:var.bltinlookup-fn]
> static inline char *bltinlookup(const char *name)

> [spec:dash:sem:var.bltinlookup-fn]
> Look up a variable from a builtin's environment: simply `lookupvar(name)`.
> A separate name exists because builtins conceptually see a merged
> environment; in this implementation the two are identical.

> [spec:dash:def:var.changelocale-fn]
> static void changelocale(const char *val)

> [spec:dash:sem:var.changelocale-fn]
> Callback for `LC_ALL`, `LC_COLLATE`, `LC_CTYPE`, `LC_NUMERIC` and
> `LANG`. `putenv((char *)val)` to publish the setting to the C library,
> then `setlocale(LC_ALL, nullstr)` — an empty string, which makes the
> library re-read every category from the environment, so one callback
> correctly handles the precedence between `LC_ALL`, the specific
> categories and `LANG`. These variables carry `VFULL`, so `val` is the
> whole `"name=value"` string that `putenv` requires. Because `putenv`
> keeps the pointer rather than copying, the text must outlive the call —
> which is why they are also `VTEXTFIXED`.

> [spec:dash:def:var.exportcmd-fn]
> int exportcmd(int argc, char **argv)

> [spec:dash:sem:var.exportcmd-fn]
> Implements both `export` and `readonly`; which one is decided by
> `argv[0][0] == 'r'`, selecting `VREADONLY` or `VEXPORT` as `flag`.
> `notp` is `nextopt("p") - 'p'`, so it is zero exactly when `-p` was
> given. With no `-p` and at least one operand, process each operand: if
> it contains an `=`, set the variable to the text after it with the
> flag; if it does not and the variable already exists, just OR the flag
> into its existing flags, leaving the value untouched; if it does not
> exist, `setvar(name, NULL, flag)` creates it unset but flagged, so a
> later assignment is exported. Otherwise (with `-p`, or with no
> operands) print the matching variables with `showvars(argv[0], flag, 0)`,
> using the command name as the prefix so the output is re-executable.
> Always returns 0.

> [spec:dash:def:var.findvar-fn]
> var ** findvar(const char *name)

> [spec:dash:sem:var.findvar-fn]
> Return the address of the link holding `name`'s entry, or the address
> of the trailing NULL link if absent — never NULL itself, so callers
> test `*result`. Start at `hashvar(name)` and walk `&(*vpp)->next`,
> stopping at the first entry for which `varequal((*vpp)->text, name)`
> holds. Returning the link address is what lets `setvareq` unlink an
> entry without a second traversal.

> static inline unsigned int hashval(const char *p)

> Hash a variable name, stopping at `=` so `"name"` and `"name=value"`
> hash alike. Seed with the first byte shifted left 4, then add each byte
> as it is consumed, breaking once the *next* byte is `=`. Note the
> lookahead means the character before the `=` is included and the `=`
> itself is not. Bytes are taken as `unsigned char`. Overflow of the
> `unsigned int` accumulator is defined wraparound and part of the
> function. The caller reduces the result modulo the table size.

> var ** hashvar(const char *p)

> Return the bucket head address for `p`: `&vartab[hashval(p) % VTABSIZE]`.

> [spec:dash:def:var.initvar-fn]
> void initvar(void)

> [spec:dash:sem:var.initvar-fn]
> Link every entry of `varinit[]` into `vartab`. Walk the array and, for
> each, `hashvar(vp->text)` for its bucket and push it onto the front of
> that chain. The loop is do/while, so the array must be non-empty.
> Then, if `geteuid()` is 0, replace `vps1.text` with `"PS1=# "` so root
> gets a distinguishing prompt. Because these entries are `VSTRFIXED` and
> `VTEXTFIXED` they are never freed, and because they are linked before
> the environment is imported, an environment assignment to the same name
> finds and updates them rather than creating a duplicate.

> [spec:dash:def:var.listvars-fn]
> char ** listvars(int on, int off, char ***end)

> [spec:dash:sem:var.listvars-fn]
> Build a NULL-terminated array of the `text` pointers of all variables
> whose flags match: `(vp->flags & (on | off)) == on`, i.e. every bit in
> `on` set and every bit in `off` clear. Accumulate into a stack string
> used as a pointer array, growing with `growstackstr` whenever the write
> position reaches `stackstrend()`. Walk every bucket and every chain, so
> the order is hash order. If `end` is non-NULL, store the position of
> the terminator through it before writing the NULL, giving the caller
> the element count without a second scan. Commit with `grabstackstr` and
> return the array. `environment()` is the macro
> `listvars(VEXPORT, VUNSET, 0)` — everything exported and set — which is
> what is passed to `execve`.

> [spec:dash:def:var.localcmd-fn]
> int localcmd(int argc, char **argv)

> [spec:dash:sem:var.localcmd-fn]
> The `local` builtin. Raise `sh_error("not in a function")` when
> `localvar_stack` is empty, since there is nothing to restore to. Then
> call `mklocal(name, 0)` on each operand from `argptr`. Return 0.
> Operands may be bare names or `name=value`; `mklocal` handles both.

> [spec:dash:def:var.localvar]
> struct localvar {
>   struct localvar *next;
>   struct var *vp;
>   int flags;
>   const char *text;
> }

> [spec:dash:def:var.localvar-list]
> struct localvar_list {
>   struct localvar_list *next;
>   struct localvar *lv;
> }

> [spec:dash:def:var.lookupvar-fn]
> char * lookupvar(const char *name)

> [spec:dash:sem:var.lookupvar-fn]
> Return a pointer to the value of `name`, or NULL if it does not exist
> or is `VUNSET`. On success return `strchrnul(v->text, '=') + 1` — the
> text just past the `=`. Under `WITH_LINENO`, when the entry is
> `vlineno` and still holds the static `linenovar` buffer, first rewrite
> that buffer from the current `lineno` so `$LINENO` reads as the line
> being executed rather than a stale value.

> [spec:dash:def:var.lookupvarint-fn]
> intmax_t lookupvarint(const char *name)

> [spec:dash:sem:var.lookupvarint-fn]
> `atomax(lookupvar(name) ?: nullstr, 0)` — look the variable up,
> substituting the empty string when unset, and parse it in base 0 so
> `0x` and leading-`0` forms are honoured. Base 0 also means an empty or
> blank value yields 0 instead of raising, which is what makes an unset
> variable usable as a number.

> [spec:dash:def:var.mklocal-fn]
> void mklocal(char *name, int flags)

> [spec:dash:sem:var.mklocal-fn]
> Save a variable's current state so the enclosing function's return can
> restore it, and give it a fresh local binding. With interrupts
> suspended, allocate a `struct localvar`.
>
> The name `-` is special: it saves the shell option settings rather than
> a variable. Copy the whole `optlist` into fresh memory, store it in
> `lvp->text`, and leave `lvp->vp` NULL as the marker for this case.
>
> Otherwise find the variable and note whether `name` carries an `=`.
> When it does not yet exist, create it — `setvareq(name, VSTRFIXED|flags)`
> if an `=` was supplied, otherwise `setvar(name, NULL, VSTRFIXED|flags)`
> which leaves it unset — and record `lvp->flags = VUNSET` so the restore
> removes it entirely. When it does exist, save its `text` and `flags`,
> then set `VSTRFIXED|VTEXTFIXED` on the live entry so that neither the
> struct nor the saved text can be freed while the save holds a pointer to
> it, and apply the assignment with `setvareq(name, flags)` if one was
> given.
>
> Push the save onto `localvar_stack->lv` and restore interrupts. Note
> that `lvp->text` is left uninitialised in the newly-created branch,
> which is safe only because `lvp->flags == VUNSET` makes the restore
> path ignore it.

> [spec:dash:def:var.poplocalvars-fn]
> static void poplocalvars(void)

> [spec:dash:sem:var.poplocalvars-fn]
> Undo one function's worth of `local` declarations. With interrupts
> suspended, pop the top `struct localvar_list` off `localvar_stack`,
> take its save list, and free the list node. Then for each save, in
> reverse order of creation (the list is LIFO):
>
> - `vp == NULL` — the `$-` case: copy the saved `optlist` back, free the
>   copy, and call `optschanged()` so derived state follows the options.
> - `lvp->flags == VUNSET` — the variable did not exist: clear
>   `VSTRFIXED|VREADONLY` (which `mklocal` had set, and which would
>   otherwise block removal) and `unsetvar(vp->text)`.
> - otherwise — restore: free the current text unless it is `VTEXTFIXED`
>   or `VSTACK`, put back the saved `flags` and `text`, and run the change
>   callback via `varfunc` unless `VNOFUNC` is set.
>
> Free each save. Restore interrupts.

> [spec:dash:def:var.pushlocalvars-fn]
> struct localvar_list *pushlocalvars(int push)

> [spec:dash:sem:var.pushlocalvars-fn]
> Begin a new local-variable scope and return the previous top of
> `localvar_stack`, which the caller passes back to `unwindlocalvars` to
> unwind exactly this scope. When `push` is zero, do nothing but return
> that value — used where a scope is nominally entered but locals should
> land in the caller's scope. Otherwise, with interrupts suspended,
> allocate an empty `struct localvar_list`, link it in front, and make it
> the new top.

> [spec:dash:def:var.setvar-fn]
> struct var *setvar(const char *name, const char *val, int flags)

> [spec:dash:sem:var.setvar-fn]
> Set or unset a variable, building the combined `"name=value"` text.
> `name` may itself be `"name=value"` or a bare name. Validate first:
> `endofname(name)` gives the end of the valid identifier and
> `strchrnul` gives the position of the `=`; if the name is empty, or the
> two disagree — meaning an invalid character appears before the `=` —
> raise `sh_error("%.*s: bad variable name", namelen, name)`.
>
> A NULL `val` means unset: add `VUNSET` and use a zero-length value.
> With interrupts suspended, allocate `namelen + vallen + 2` bytes and
> copy the name plus one more byte; if there is a value, overwrite that
> byte with `=` and append the value. Write a NUL. The extra byte is what
> makes an unset variable's text end in two NULs, which `varnull` relies
> on. Hand ownership to `setvareq(nameeq, flags | VNOSAVE)` and return
> its result.

> [spec:dash:def:var.setvareq-fn]
> struct var *setvareq(char *s, int flags)

> [spec:dash:sem:var.setvareq-fn]
> Install the already-built `"name=value"` string `s` into the table.
> Must be called with interrupts off. `s` may be adopted rather than
> copied, so it must not be transient.
>
> First force `VEXPORT` when `allexport` is on: `flags |= VEXPORT &
> (((unsigned)(1 - aflag)) - 1)`, which yields `VEXPORT` when `aflag` is
> 1 and 0 when it is 0, branchlessly.
>
> Find the entry. If it exists:
> - Refuse if `VREADONLY`: free `s` when `VNOSAVE` says we own it, then
>   raise `"%.*s: is read only"` naming the variable.
> - Free the old text unless it is `VTEXTFIXED` or `VSTACK`.
> - Decide which of the old flags to keep. If the new flags are anything
>   other than exactly `VUNSET` within the
>   `VEXPORT|VREADONLY|VSTRFIXED|VUNSET` group — that is, this is a real
>   assignment or an attribute change — keep every old flag except the
>   ownership/unset bits (`bits = ~(VTEXTFIXED|VSTACK|VNOSAVE|VUNSET)`).
>   Otherwise this is a pure unset: if the entry is `VSTRFIXED` it is
>   statically allocated and must stay linked, so keep only that bit; if
>   it is not, unlink and free the entry, free `s` when we own it and
>   nothing else claims it, and return.
> - OR the retained old flags into the new ones.
>
> If it does not exist: a pure unset has nothing to do, so free `s` if
> owned and return. Otherwise allocate an entry, link it into the bucket,
> and give it a NULL callback.
>
> Then, unless the text is already `VTEXTFIXED`, `VSTACK` or `VNOSAVE`,
> `savestr(s)` so the table owns a copy. Store `text` and `flags`, and
> unless `VNOFUNC` is set run the variable's change callback via
> `varfunc`. Return the entry. On the assignment paths that is a live
> entry; on the pure-unset path that unlinks and frees the entry, the
> function returns the **freed** `vp` — a dangling pointer. No caller
> dereferences it, but a port must not "helpfully" substitute NULL;
> reproduce the dangling return.

> [spec:dash:def:var.setvarint-fn]
> intmax_t setvarint(const char *name, intmax_t val, int flags)

> [spec:dash:sem:var.setvarint-fn]
> Set `name` to the decimal rendering of `val`. Size a VLA with
> `max_int_length(sizeof(val))`, format with `%` `PRIdMAX`, call
> `setvar`, and return `val` unchanged so the caller can use it as an
> expression.

> [spec:dash:def:var.showvars-fn]
> int showvars(const char *prefix, int on, int off)

> [spec:dash:sem:var.showvars-fn]
> Print matching variables in collating order, in re-executable form.
> Collect them with `listvars(on, off, &epend)` and `qsort` with `vpcmp`
> — POSIX requires `set` to output in the locale's collating order, which
> the hash table does not provide. Choose the separator: a space when
> `prefix` is non-empty (so `export x='1'`), and the prefix itself — i.e.
> the empty string — when it is empty (so `x='1'`).
>
> For each entry, split at the `=`. When there is a value, single-quote
> it; when the variable is unset there is no `=`, so the quoted part is
> `nullstr`. Print
> `"%s%s%.*s%s\n"` — prefix, separator, the name *including* its `=` when
> present, then the quoted value. Return 0.

> [spec:dash:def:var.unsetcmd-fn]
> int unsetcmd(int argc, char **argv)

> [spec:dash:sem:var.unsetcmd-fn]
> The `unset` builtin. Parse `-v`/`-f` with `nextopt("vf")`, keeping only
> the last one seen in `flag`. Then for each operand: unless `-f` was
> given, `unsetvar` it; and unless `-v` was given, `unsetfunc` it. With
> neither option both happen, and the function is removed first — the
> comment notes this ordering lets a function be unset even when a
> read-only variable of the same name would make the variable removal
> fail. Return 0.

> [spec:dash:def:var.unsetvar-fn]
> void unsetvar(const char *s)

> [spec:dash:sem:var.unsetvar-fn]
> `setvar(s, 0, 0)` — assigning a NULL value is how unsetting is spelled.

> [spec:dash:def:var.unwindlocalvars-fn]
> void unwindlocalvars(struct localvar_list *stop)

> [spec:dash:sem:var.unwindlocalvars-fn]
> `poplocalvars()` repeatedly until `localvar_stack` equals `stop`.
> Passing the value `pushlocalvars` returned unwinds exactly the scopes
> opened since; passing 0 unwinds everything, which is what the `RESET`
> event does after an error.

> [spec:dash:def:var.var]
> struct var {
>   struct var *next;
>   int flags;
>   const char *text;
> }

> [spec:dash:def:var.var.func-fn]
> void (*func)(const char *)

> [spec:dash:sem:var.var.func-fn]
> The change-callback member of `struct var`, not a function of its own —
> the extractor lifted the member declaration out of the struct. It is
> invoked whenever the variable is set or unset, unless `VNOFUNC` is
> given. The argument is the value alone, or the whole `"name=value"`
> when the variable carries `VFULL`; see `varfunc`. NULL means no
> callback. The callbacks in use are `changeifs`, `changemail`,
> `changepath`, `getoptsreset`, `sethistsize` and `changelocale`.

> int varcmp(const char *p, const char *q)

> Compare two variable strings up to the first `=` or NUL on either
> side, so `"PATH=x"` and `"PATH"` compare equal. Walk while the current
> characters match, stopping at a NUL; after each advance, map a `=` to
> `'\0'` on either side so it compares as a terminator. Return
> `c - d`, the usual sign convention.

> static inline int varequal(const char *a, const char *b)

> `!varcmp(a, b)` — true when the two names are equal, ignoring anything
> from an `=` onward.

> [spec:dash:def:var.varfunc-fn]
> static void varfunc(struct var *vp)

> [spec:dash:sem:var.varfunc-fn]
> Invoke a variable's change callback, if it has one. Pass the whole
> `text` when `VFULL` is set — the form `putenv` needs — and otherwise
> just the value, obtained with `varnull`. Do nothing when `func` is NULL.

> [spec:dash:def:var.varnull-fn]
> static char *varnull(const char *s)

> [spec:dash:sem:var.varnull-fn]
> Return the value part of a `"name=value"` string: `strchrnul(s, '=') + 1`.
> For an unset variable there is no `=`, so this lands one past the
> terminating NUL — which is why `setvar` always allocates a trailing
> extra byte and leaves two NULs there, making the result a valid empty
> string rather than a read past the end.

> STATIC int vpcmp(const void *a, const void *b)

> `qsort` comparator over an array of `char *` variable texts:
> dereference both and return `varcmp` of the two, so ordering is by name
> only.
