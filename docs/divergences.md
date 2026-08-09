# Where the port deliberately differs from dash

This *was* a bug-for-bug port. It is not one any more — see
[dec:nsh:we-own-the-defects]. The code is ours, dash is a reference
rather than an authority, and when the two disagree the question is
which is right, not which is the C.

What has not changed is that a difference still has to be explained.
`tests/harness/runall.sh` and the POSIX suite's `--reference` mode remain
the gate, and the default assumption on any *unexplained* difference is
still that the Rust is wrong — that is the common case by a wide margin.

This file is the exception list. A difference recorded here is
**sanctioned**: the harness will show it, and it must not be read as a
regression. A difference *not* recorded here is a defect, whichever side
it is on.

Four things can happen when the port and dash disagree, and the last
three belong in this file:

1. **The port is wrong.** Fix the port. Overwhelmingly the common case —
   see the errata commits. Nothing recorded here.
2. **dash has a bug, and the fix suits the C too.** Fix it in the C on a
   branch cut from the 0.5.13.5 release commit so it can go upstream,
   merge it, then bring the spec and the port into line. Both languages
   end up agreeing, so nothing is recorded here. Done twice: `fc -e`
   reading `optionarg` instead of `optarg` (`fix/fc-e-optarg`), and the
   missing `"[%d] %d\n"` background job announcement
   (`fix/async-job-notification`).
3. **dash has a bug we fix only in the Rust.** Because the fix does not
   suit the C, because it depends on something the port has and dash does
   not, or because the C's behaviour is not reproducible in a safe
   language at all. The two stop agreeing, and that is recorded below.
4. **dash is deliberately not doing something.** Not a bug — a decision,
   usually to stay small. If the port decides otherwise, the two stop
   agreeing on purpose, and that is recorded below too.

The distinction between 2 and 4 is a judgement about intent, not about
conformance. `fc -e` reads the wrong variable; nobody meant that. Not
managing the terminal is a design position dash has held since the
NetBSD import.

## How a category-3 divergence is registered

The harness reads a register, so a sanctioned divergence no longer spends
`FAIL=0`. The prose lives here; the executable half is
`tests/harness/divergences.sh`, and an entry is a shell function:

```
dsdiv_<id> REF_OUT PORT_OUT REF_RC PORT_RC CASE_FILE  ->  0 = this explains it
```

A case that matches is reported as `XFAIL(<id>)` and counted as passing,
with the detail in `tests/.build/xfail.out`. An entry that matches
nothing is reported too — a stale excuse is how a real regression
eventually gets waved through.

It is a function rather than a pattern in a config file because a
divergence is a claim about *behaviour*, and the only honest way to say
"the outputs differ exactly this way and no other" is code that can
inspect both sides. A glob over case names would excuse whatever else
those cases happened to break.

**The one rule: an entry must not be able to match a regression.** Three
habits keep that true, and `tests/harness/divtest.sh` enforces them by
asserting refusals rather than matches — a changed value, a dropped line,
an extra line, a duplicate, a differing exit status, a case outside the
feature:

  * *Compare, do not ignore.* `ds_same_lines` sorts both sides and
    requires equality, so it says "the same lines in a different order".
    Dropping the lines would say "anything at all", which is not a
    divergence, it is a blind spot.
  * *Scope to the feature.* An entry about `env` ordering has no business
    excusing a case that never runs `env`. Scope to what actually
    diverges, too: the first entry does not name `export -p` or `set`,
    because both already print sorted on both shells, so a permutation
    there could only be a regression.
  * *Say which side is right.* "The same lines in a different order"
    excuses *every* order, including a future one that is neither the
    reference's nor the intended one. An ordering entry also asserts the
    order — `ds_blocks_sorted` — and which lines were allowed to move —
    `ds_moved_lines_match`. Between them they turn "they differ somehow"
    into a claim that can be false.

## Register

### `jobctl.save-terminal-settings` / `jobctl.fg-terminal-settings-restore`

**Status:** agreed, not yet implemented.

POSIX XCU 2.11 requires an interactive shell to save the terminal
settings when a foreground job is stopped, move the terminal to the
settings it needs to read commands, and have `fg` restore the saved ones
before sending SIGCONT.

dash does none of it. `tcsetattr` appears nowhere in the tree; the single
`tcgetattr`, at `input.c:138`, only asks whether fd 0 is a tty. A shell
that never drives the terminal has no state to save, which is a coherent
position for a shell this size.

The observable consequence is not theoretical: `^Z` out of a program that
put the terminal in raw mode and the *shell* is left in raw mode, so the
prompt misbehaves until `stty sane`. The port will implement the POSIX
behaviour and dash will not.

Scope, so nobody mistakes this for a small change: it needs terminal
state on the job structure, save points on the stop path, restore points
in `fg` and `bg`, and correct ordering against `tcsetpgrp` and SIGCONT.
The two rules are a pair — `fg` must restore "the ones that the shell
saved" — so neither can be done alone.

Expect `suspend-shell-restores-its-own-terminal-settings` to pass on the
port and fail on the reference once this lands. That mismatch is this
entry.

**Caveat on the requirement itself.** The corpus records these rules as
unconditional, but that is what the extractor saw: it detects `[UP]`,
`[XSI]` and `[OB]` only when the marker appears in the rule body, so a
marker sitting on the XCU 2.11 section heading would have been lost.
Worth checking the chapter front matter before treating "POSIX requires
it" as settled.

### `list()` leaves `linno` unwritten on a backgrounded command's wrapper

**Status:** fixed in the Rust; dash unchanged. Category 3.

When `list()` sees `cmd &` and `cmd` is neither an `NPIPE` nor an
`NREDIR`, it wraps it in a synthesised `NREDIR` node and sets that node's
`type` to `NBACKGND`. It never writes the wrapper's `linno`, so the field
holds whatever `stalloc` handed back. `evalsubshell` copies it into both
`errlinno` and `var::lineno`, and both are observable — through
`$LINENO`, and through the `sh: N:` prefix on a diagnostic.

The reason nobody has noticed is that the value is immediately
overwritten on every path that survives: the forked child re-sets it from
the command inside. What is left is the fork-failure path, where the
`Cannot fork` diagnostic carries the uninitialised number. No case in
`tests/corpus` reaches it, which is what makes this fixable now under the
constraint above.

This is category 3 rather than category 2 because there is no
bug-for-bug option to choose between. Reading uninitialised memory is not
a behaviour a safe language reproduces — an owned node has to name a
value — so the C's behaviour was already unavailable, and the only open
question was which correct value to write. The port writes the line the
backgrounded command starts on, captured at the same point `command()`
and `pipeline()` capture theirs, so the wrapper records the line its
contents record.

An upstream fix would be welcome and is not blocked by anything here; it
just is not a precondition. See [dec:nsh:we-own-the-defects].

### `env` and `alias` print in sorted order

**Status:** fixed in the Rust; dash unchanged. Category 3. Registered as
`sorted_tables` in `tests/harness/divergences.sh` -- the register's first
real entry.

dash keeps variables and aliases in 39-bucket chained hash tables and
walks the buckets to produce output. `var.rs listvars` is what builds
`execve`'s `envp`, so the walk order is the environment every child sees;
`alias.rs aliascmd` walks the buckets to print. The hash is
`(first_byte << 4)` plus the sum of the bytes, which puts
`export AA=1 BB=2 CC=3 DD=4 EE=5 FF=6` on the wire as
`AA FF DD BB EE CC` -- neither sorted nor insertion order, just an
artefact of a weak hash over a prime bucket count.

`vartab` and `atab` are `BTreeMap`s keyed by name now, so both print
sorted. Upstream wants the second half of that: `alias.c` has carried a
one-line request for sorted output above `aliascmd` since the NetBSD
import.

**The heading used to name four builtins, and two of them were wrong.**
`export -p` and a bare `set` both go through `showvars`, and `showvars`
already `qsort`s with `vpcmp` before printing -- the C comment above it
says so, and wishes out loud for "an ordered balanced binary tree instead
of hashed lists". So those two printed sorted on both shells before this
change and print sorted on both after it; the only observable difference
is `env` (and anything else that reads `environ`, such as `printenv`) and
a bare `alias`. Measured, not assumed:

```
$ env -i sh -c 'export AA=1 BB=2 CC=3 DD=4 EE=5 FF=6; env'
dash: AA FF DD BB EE CC        port: AA BB CC DD EE FF
$ env -i sh -c 'alias AA=1 BB=2 CC=3 DD=4 EE=5 FF=6; alias'
dash: AA FF DD BB EE CC        port: AA BB CC DD EE FF
$ env -i sh -c 'export AA=1 BB=2 CC=3 DD=4 EE=5 FF=6; export -p'
both: AA BB CC DD EE FF
```

What is left of the four-builtin claim is that `set`'s order stops
depending on a sort at print time and starts being a property of the
container, which is the arrangement POSIX's "lexicographic order"
requirement wants. Neither shell honours the *locale's* collating order
that the requirement actually names: dash's `varcmp` compares bytes, and
so does a `BStr` key. That gap is unchanged and still owed.

POSIX specifies no ordering for `env` or `alias`, so nothing here is a
conformance question. What the order had was a differential harness that
pinned it, and keeping a weak hash's bucket walk forever so that a number
stays green is the tail wagging the dog.

**What the corpus actually observes.** One case, in
`tests/corpus/aud_state_var.txt`:

```
export XV=1 YV=2; echo "$XV$YV"; env | grep -E '^(XV|YV)='
```

`aud_state_gen3` runs `env` forty-odd times and every one of them is
`env | grep -c '^NAME='`, which counts rather than lists; the `alias`
cases print one alias, or none after `unalias -a`. So the sweep goes
`XFAIL=1`, not the thirty this section used to predict.

**What the entry refuses.** Five conditions, each one a regression class
the entry must not be able to reach: the exit status must match; the case
must run `env`, `printenv` or `alias`; the two outputs must hold the same
lines, so a changed value, a dropped line, an extra line or a duplicate
still fails; only assignment-shaped lines may have moved, so a
diagnostic that raced stdout is not this; and the port's blocks of those
lines must be *sorted*, without which the entry would excuse any
environment order at all rather than this one.

`export -p` and `set` are deliberately absent from the scoping pattern
even though the old heading named them. Both already print sorted on both
shells, so a permutation there could only be a regression, and an entry
listing them could excuse it.

It has one known limit, pinned by a `divtest.sh` case so it cannot drift:
the sortedness test reads each maximal run of assignment-shaped lines as
one block, so a case printing two environments back to back with nothing
between them would be refused and reported as a failure rather than an
`XFAIL`. Nothing in the corpus does. For an entry whose job is to not
excuse too much, that is the right way to be wrong.

### `hash` prints cached commands in name order

**Status:** fixed in the Rust; dash unchanged. Category 3. Registered as
`sorted_cmdtable` in `tests/harness/divergences.sh`.

dash's command cache is a 31-bucket chained hash table. A bare `hash`
walks buckets and chains, so its output order is an artefact of the inline
hash in `cmdlookup`, just as `env` and `alias` used to expose their table
orders. The port owns the cache as a `BTreeMap<BString, Box<tblentry>>`
keyed by command-name bytes. The box keeps an entry address stable across
map rebalancing for the still-C-shaped resolver interface; the map owns
both the NUL-terminated key and the entry, and a function entry owns its
body as `Rc<Node>`.

Consequently, a no-operand `hash` prints entries in bytewise command-name
order while dash retains bucket order. POSIX does not specify `hash`'s
listing order. This is the third table-order divergence anticipated by
`docs/std-replacements.md` section 5.1 and is accepted for the same reason:
preserving a weak hash's internal walk would preserve an allocator-driven
representation as observable policy.

The executable entry is scoped to a no-operand `hash`; `hash name` and
`hash -r` cannot match it. It requires equal exit statuses and the same
line multiset, and only lines with `printentry`'s shape may move: a
reconstructed path with an optional trailing `*` rehash marker. It then
strips that marker and the directory prefix and asserts that each block on
the nsh side is sorted by command name. Sorting the whole printed line
would be wrong because two commands may resolve through different `PATH`
elements.

The line grammar deliberately accepts only the pathname characters used
by the corpus. A valid but exotic command path containing whitespace or
other shell punctuation is refused and remains a loud differential. That
is preferable to broadening an ordering exception until it could mistake
unrelated output or a diagnostic for a hash-table line. `divtest.sh` pins
the matching case, content/drop/add/duplicate/status failures, the line
shape and feature scope, the optional `*`, basename rather than full-path
sorting, and an unsorted nsh permutation.

## Candidates not yet decided

### The port is *more* conformant than dash in two line-editing cases

`edit-history-goto-number` (`edit.history-goto`, the `[number]G` command)
and `edit-history-search-pattern-anchored`
(`edit.history-search-pattern`) pass on the port and fail on the C. The
port's line editor is nshedit; dash's is libedit; nshedit satisfies these
two and libedit does not.

Nobody chose this. It is what fell out of attaching the history, and it
is the same shape as the `edit.undo` entry that used to sit here —
which resolved itself, since nshedit reproduces dash's behaviour there
and both now fail together.

Under the bug-for-bug contract a port that is *better* than its original
is still a divergence, so the choice is the usual three:

  1. Reproduce libedit's failure, keeping the port faithful.
  2. Register these as sanctioned improvements and let the port lead.
  3. Fix libedit, so both conform and nothing is registered — the route
     `fc -e` and the background job announcement took.

Option 3 is not available in the same way here: the defect is in
libedit, not in dash, and nshedit is already the fixed implementation.
So it reduces to 1 or 2, and 2 costs nothing to hold — no maintenance,
no divergence in the shell language, only in the editor.

Left undecided deliberately. Recorded so that a future parity run
showing "2 mismatches, port ahead" is read as this entry and not as a
regression.

**Since resolved in principle, not yet in writing.**
[dec:nsh:we-own-the-defects] makes option 2 the default: dash is a
reference rather than an authority, so a port that is better than its
original is not thereby wrong. What is left is to move this section into
the register above and say so, which is bookkeeping rather than a
decision. It is held back only because these two *are* observed by the
POSIX suite, so promoting them is exactly the case that wants the
sanctioned-divergence mechanism first.
