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
`tests/harness/divergences.sh`. A decision-style entry is a shell function:

```
dsdiv_<id> REF_OUT PORT_OUT REF_RC PORT_RC CASE_FILE  ->  0 = this explains it
```

A case that matches is reported as `XFAIL(<id>)` and counted as passing,
with the detail in `tests/.build/xfail.out`. An entry that matches
nothing is reported too — a stale excuse is how a real regression
eventually gets waved through.

There is a second form for corrections that can occur together in one
generated case. A `dsnorm_<id>` function performs one narrow transformation
from dash's observed output to the port's specified output and records that it
acted. All enabled normalizers run, and the case is sanctioned only if the two
complete outputs and statuses are then identical. The report carries every ID
that participated, comma-separated. This is composition, not partial credit:
one unexplained byte still makes the case fail.

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

### `alias` displays `name=quoted-value`

**Status:** fixed in the Rust; dash unchanged. Category 3. Registered as
`alias_stdout_format` in `tests/harness/divergences.sh`.

POSIX specifies alias output as `name=value`, with quoting applied to the
value so the definition can be fed back to the shell. dash instead passes the
complete `name=value` string through its quoting helper and prints
`'name=value'`. Both forms can be re-entered, but only the first has the
required unquoted name and equals sign. The port now prints `name='value'`.

The byte-level relationship is deliberately narrow: moving dash's first quote
from before the name to immediately after the equals sign produces the port's
line exactly, including values that contain quotes. The executable register
performs only that transformation, requires equal statuses and equal line
multisets afterwards, refuses definition-only commands, and permits only
alias-shaped lines to move. It also proves that multi-entry port listings are
sorted, because quote placement and the existing alias-table ordering
divergence occur in the same output and cannot be sanctioned independently.

### `env` and `alias` print in sorted order

**Status:** fixed in the Rust; dash unchanged. Category 3. Registered as
`sorted_tables` for environment output and by `alias_stdout_format` for alias
output in `tests/harness/divergences.sh`.

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

**What the entries refuse.** Both require matching exit status and equal line
multisets after their one stated normalization, so a changed value, dropped
line, extra line or duplicate still fails. Only assignment-shaped lines may
move, a diagnostic that raced stdout is not excused, and the port's blocks
must be *sorted*. `sorted_tables` is scoped only to `env` and `printenv`;
`alias_stdout_format` is scoped to alias display and additionally requires the
exact quote move described above.

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
by the corpus and requires a directory separator. A valid bare command,
or an exotic path containing whitespace or other shell punctuation, is
refused and remains a loud differential. Requiring the separator also
keeps surrounding status text such as `rc=0` out of the sorted block.
That is preferable to broadening an ordering exception until it could
mistake unrelated output or a diagnostic for a hash-table line.
`divtest.sh` pins the matching case, surrounding fixed lines,
content/drop/add/duplicate/status failures, the line shape and feature
scope, the optional `*`, basename rather than full-path sorting, a bare
name refusal, and an unsorted nsh permutation.

### POSIX corrections retained over dash

**Status:** implemented in the Rust; dash unchanged. Category 3. Every ID
below is registered in `tests/harness/divergences.sh`. Its 238 focused checks
exercise the intended matches and adversarial content, status and scope
boundaries.

These are the deliberate results of the POSIX conformance pass. Most are
normalizers because the generated state corpus routinely observes more than
one in the same shell process.

* `getopts_optarg_unset`: when an option has no argument, `getopts` unsets
  `OPTARG`; dash leaves it set to an empty value. The entry changes only a
  complete `${OPTARG-U}` result record.
* `getopts_diagnostic_prefix`: a `getopts` diagnostic identifies the invoking
  program (`SH` or `./script.sh`). dash omits it. Only the two exact option
  diagnostics and those two invocation forms qualify.
* `getopts_optind_reset`: assigning `OPTIND=1` restarts the scan. dash retains
  a hidden cursor and continues or stops; nsh makes `OPTIND` the authoritative
  state and re-observes the first `-a` operand.
* `set_hashall_option`: `set -h` is implemented, so `set -o` and `set +o`
  contain the corresponding disabled `hashall` record. The normalizer removes
  only those exact report lines before comparing the rest.
* `ignoreeof_noninteractive_eof`: `ignoreeof` applies only to an interactive
  input source. A command file or stdin script still terminates at physical
  EOF even if it executes `set -i` and enables `ignoreeof`; dash prints fifty
  retry diagnostics first. nsh captures the input-source classification when
  the command loop begins, rather than allowing an option mutation to
  reclassify the source.
* `fc_listing_format`: `fc -l` separates the event number and command with a
  tab, uses a leading tab with `-n`, and indents continuation lines. dash uses
  four spaces before the number, one after it, and no continuation indent.
* `fc_substitution_status`: `fc -s true=false` returns the status of the
  resulting `false` command. dash reports success.
* `fc_recursion_error_status`: exhausting the recursive `fc -s` guard is a
  utility error and returns 2. dash prints the same diagnostic but leaves the
  interactive command status at zero.
* `ulimit_all_format`: every `ulimit -a` row names the resource, units, option
  and value in the POSIX.1-2024 form. The register maps all twelve exact nsh
  labels to dash's older labels while leaving every value untouched.
* `ulimit_default_soft_report`: with neither `-H` nor `-S`, a no-operand query
  reports the soft limit. dash suppresses one such query in the exercised
  set/query sequences; the entry accepts exactly one additional line equal to
  the value just set.
* `jobs_command_text`: the default `jobs` record contains its `<command>`
  field. dash prints a padded status with that field empty. The entry removes
  a suffix only when the complete command text occurs in the case itself.
* `jobs_waited_removal`: a successfully waited job is removed from the known
  jobs. dash retains it and a later `jobs` prints a stale `Done` record.
* `wait_consumed_status`: after a successful wait consumes a PID's saved
  status, waiting for it again returns 127. dash returns the stale zero status.
  The normalizer changes only the final result of `wait $! $!` or the one
  `second=0` record in the named-PID probe.
* `wait_consumed_jobspec`: the same consumption rule removes a job ID. After a
  bare `wait`, `wait %1` is an unknown-job utility error in nsh; dash returns
  the stale success. The exact `%1` diagnostic and its one `rc=2` record are
  paired before either is normalized.
* `kill_jobspec`: `kill` resolves a job-control job ID such as `%1`. dash hands
  the string to `kill(2)` as a PID and diagnoses `No such process`.
* `trap_subshell_listing`: POSIX.1-2024 preserves the inherited trap commands
  for an initial no-operand `trap` listing in a subshell, even though live
  dispositions were reset. Added lines must be byte-identical to the outer
  listing and are bounded by the number of lexical subshell listings.
* `exit_trap_final_status`: the adopted Smoosh compatibility rule uses a
  normally completed EXIT action's final command status as the shell or
  subshell status. dash restores the status that entered the action. The
  register pins the nested-child witness exactly: `inner` remains unchanged
  and only the reported child status changes from 2 to 0.
* `trap_p_option`: POSIX.1-2024 `trap -p PIPE` prints the selected trap. dash
  rejects `-p`; the decision entry pins both its exact diagnostic and nsh's
  exact listing.
* `utf8_pattern_characters`: in a UTF-8 locale, pattern literals, `?`, bracket
  expressions, character classes, and parameter trims operate on decoded
  characters. dash's inherited matcher rejects or mis-trims the thirteen
  pinned multibyte witnesses by treating bytes as characters. The entry
  accepts only their complete output pairs and successful statuses.
* `c_locale_multibyte_ifs`: in the C locale, the two bytes encoding `é` are
  two non-whitespace `IFS` separators. dash incorrectly keeps the sequence as
  one character. The entry pins all twelve generated witnesses, including
  every surrounding field and diagnostic; the nsh side also agrees with Bash
  in POSIX mode on the byte-wise split.
* `parameter_operand_quote_preservation`: quoting and backslash protection in
  a `${parameter op word}` operand survives when that operand supplies the
  result. dash loses the protection while moving the encoded bytes, which can
  split one field, glob an escaped metacharacter, or discard a quoted empty
  field. Six complete generated results are registered.
* `empty_quote_field_anchors`: an empty quote fragment anchors one field; it
  neither creates a second empty field nor suppresses splitting in an adjacent
  unquoted substitution. Three complete generated results pin both sides of
  that rule.
* `case_fallthrough_diagnostic`: nsh tokenizes the POSIX.1-2024 `;&` case
  operator as one token, so its unsupported-operator diagnostic names `;&`;
  dash stops at `;` or `&`. No other syntax diagnostic is changed.
* `missing_command_file_status`: failure to open the command-file operand to
  `sh` has status 127. dash routes it through its generic shell-error status 2.

Three registered differences are consequences of the safe logical-descriptor
model rather than POSIX corrections:

* `closed_input_read_error` reports EBADF and status 128 when `read` is asked
  to consume a logically closed input slot, where dash's stdio path treats it
  as ordinary EOF and returns 1. Both fail; nsh preserves the cause.
* `closed_output_dup_diagnostic` diagnoses the closed source in
  `1>&- 2>&1`; dash exits 2 silently. Redirections are still applied in the
  same left-to-right order.
* `logical_fd_introspection` and `logical_fd_low_nofile_survival` keep host
  descriptor numbers out of shell semantics. Consequently `/dev/stdin`
  cannot reveal whether a here-document uses a pipe or anonymous file, and a
  shell can still query a logical `RLIMIT_NOFILE` of zero or one after the host
  backing descriptors already exist. The content, limits and redirection
  behavior remain compared exactly.

The implementation still honors real exhaustion. Pipe ends are moved into the
hidden backing range before a pipeline forks; if `RLIMIT_NOFILE` leaves no
number in that range, construction fails once as `Pipe call failed`. Linux's
`F_DUPFD_CLOEXEC` encodes that lower-bound failure as `EINVAL`; the platform
boundary translates it to `EMFILE`, so ordinary redirection exhaustion retains
the useful `Too many open files` diagnostic. Ownership stays with `OwnedFd` and
`SharedFd`, so partial construction closes every acquired endpoint by drop.

### `error.interrupt-delivery-point`

**Status:** decided and implemented by `errors-are-values` step F.
Category 3 — a change the port makes and the C does not.

dash delivers an untrapped SIGINT at one of two instructions. If
`suppressint` is zero when the signal arrives, `onsig` calls `onint`
*inside the signal handler* and leaves it by a non-local jump
(`trap.rs:331-352`, `error.rs:250-263`). If `suppressint` is non-zero, the
handler sets `intpending` and returns, and the interrupt is delivered by
whichever `INTON` next brings the counter to zero — that is, at the
instruction where the counter reaches zero.

The port delivers it at the next **poll site** instead. `onsig` stores
into the signal inbox and returns, in both cases; `INTON` decrements the
counter and, when it reaches zero with an interrupt pending, leaves
`intpending` set; and the interrupt is noticed at the next place the shell
looks, which is one of the `EINTR` returns or `dotrap`.

**Why this is a divergence and not an implementation detail.** The
delivery point is an instruction address, and between the old one and the
new one the shell executes real instructions. A `^C` arriving during an
`INTOFF` bracket used to be delivered the moment the bracket closed;
now it is delivered when the shell next reaches a poll site. Nothing in
between is observable *in the shell language* — no output is produced,
no syscall is issued that a script can see — but the claim "unobservable"
is a claim, and this register is where claims of that shape are written
down rather than assumed.

**Why the port does it.** The C's asynchronous path depends on an
unwinder walking a kernel signal frame out of `onsig` through
`__restore_rt`'s CFI. `trap.rs:315-330` records that this was a real bug
in this port once — `SIGABRT`, status 134, on `kill -INT $$` — fixed only
by declaring the handler `extern "C-unwind"`. Depending on that is not
something a library may ask of an embedder, and it is incompatible with
`panic = "abort"`, which is the Cargo profile constraint
`[dec:nsh:errors-are-values]` exists to remove. If any part of the
interrupt stayed an unwind, `panic = "abort"` would still break the shell
in the one case a user is most likely to hit.

**Why it works at all**, rather than being a hope: `setsignal` sets
`act.sa_flags = 0` (`trap.rs:288`). dash never sets `SA_RESTART`, so every
interruptible syscall the shell makes returns `EINTR` when a signal
arrives, and there is always a synchronous point at which to notice. dash
already uses this idiom at two of its five `EINTR` sites.

**No executable register entry, and that is deliberate.** An entry in
`tests/harness/divergences.sh` is a function that explains a *difference
the corpus observed*. This divergence produces none: the corpus runs
`-c` without `-i`, so it never sends a signal at a chosen instruction, and
`error.rs:254-256` makes the `EXINT` path reachable only in an interactive
root shell. An entry that can never match is a stale excuse by
construction, and this file's own rule is that a stale excuse is how a
real regression eventually gets waved through.

The executable half is the pty suite instead, which is where this
divergence is actually observable. Six cases were added to
`tests/harness/ptydiff.py` **before** the change, each blocking in a
different syscall so that a delivery point that moved too far shows up as
a shell that stops answering `^C`:

    ^C during a blocked read       the `read` builtin, `input.rs`
    ^C during a slow child         an external command
    ^C during wait                 `wait3`, `jobs.rs`
    ^C during a substitution       the command-substitution read, `expand.rs`
    ^C after a builtin error       the `suppressint` leak, docs 2.4
    ^C after a nested error        the same leak, one frame deeper

`docs/errors-are-values.md` 6B is the reasoning: the failure mode here is
not a crash but a shell that stops responding to `^C`, which no batch
harness can observe because a batch harness never sends one.

### `unset_readonly_diagnostic`

**Status:** implemented in the Rust; dash unchanged. Category 3.
`crates/nsh/src/builtins/unset.rs`.

`unset` refusing a read-only name is a special built-in's failure, so
POSIX.1-2024 XCU 2.8.1 ends a non-interactive shell. The status is left
unspecified, which is why dash answers 2 and GNU Bash's POSIX mode answers
1 and both conform.
`[spec:nsh:req:compat.bash.error-boundary]` picks 2 for the default
dialect, and as of 2026-09-04 the shell does:

```
$ dash -c 'readonly R=1; unset R; echo "rc=$?"'
dash: 1: unset: R: is read only          # rc 2, no `rc=` line
$ nsh  -c 'readonly R=1; unset R; echo "rc=$?"'
unset: R is read-only                    # rc 2, no `rc=` line
```

What is left is the diagnostic. dash writes it through its `$0: line: `
spine; this shell writes the prefix-less `unset: NAME is read-only` that
`[spec:nsh:req:compat.smoosh.error-contracts]` fixed, and keeps that
spelling in both dialects so a script does not have to know which one it
is running under to read the message. Bash's own wording is a third
answer again -- `unset: R: cannot unset: readonly variable` -- so there is
no spelling that matches both references.

The entry rewrites exactly that one complete reference line, in a case
that both makes a name read-only and unsets one, and carries the name
across so a diagnostic about a different variable is not excused. The
statuses and every other byte are still compared exactly, which is what
holds the fatality: the shell ending at that command is not part of what
this entry excuses, and a port that carried on to print `rc=2` fails.

Two corpus cases in `tests/corpus/salvage.txt` route the diagnostic through
`sed 's|^[^:]*: ||'`, which strips one colon field from each side. The entry
rewrites that filtered shape too, but only for a case containing that exact
filter, so an unfiltered diagnostic that lost its command name is still a
difference.

Measured 2026-09-04. Seventeen cases in the whole 36,097-case corpus contain
both a `readonly` and an `unset` and so can reach the changed line; run as
one corpus they went from `PASS=13 FAIL=4 XFAIL=5` to `PASS=17 FAIL=0
XFAIL=9`. The four that moved are the four where the *status* differed, which
this entry cannot excuse and only the code change fixes.
`tests/corpus/aud_state_var.txt` went `PASS=39 FAIL=1 XFAIL=2` to
`PASS=40 FAIL=0 XFAIL=3`, `aud_state_flags.txt` `34/1/1` to `35/0/2`,
`aud_exec_struct.txt` `64/7/2` to `65/6/3`, and `salvage.txt` `6913/50/18` to
`6914/49/19`.

### `dot_missing_file_diagnostic` / `parameter_error_diagnostic` / `nounset_error_diagnostic`

**Status:** implemented in the Rust; dash unchanged. Category 3.
`crates/nsh/src/builtins/dot.rs`,
`crates/nsh/src/expand/typed/parameter.rs`.

Three siblings of `unset_readonly_diagnostic`, and the same residue: once
`bash.divergences.error-boundary-status-collisions` gave these failures
dash's status 2, the statuses agree and nothing is left but the
diagnostic's shape. dash writes every one of them through its
`$0: line: ` spine -- with the sourced script's name as a second field
when the failure happened inside a `.` script -- and this shell keeps the
prefix-less spelling `[spec:nsh:req:compat.smoosh.error-contracts]` fixed:

```
$ dash -c '. ./nonesuch'          dash: 1: .: cannot open ./nonesuch: No such file
$ nsh  -c '. ./nonesuch'          .: ./nonesuch: not found
$ dash -c ': ${x?boom}'           dash: 1: x: boom
$ nsh  -c ': ${x?boom}'           x: boom
$ dash -c 'set -u; echo $x'       dash: 1: x: parameter not set
$ nsh  -c 'set -u; echo $x'       x: parameter not set
```

Each entry rewrites exactly one complete reference line and nothing else.
`dot_missing_file_diagnostic` fires only in a case that runs a `.` at a
command position and carries the operand across, so a diagnostic about a
different file, or about a different `open` failure, is still a
difference. `parameter_error_diagnostic` reads the names out of the case
file -- the parameters the script actually wrote a `?` expansion for --
and rewrites a line only for one of those, which is what stops it from
becoming a blanket excuse for a missing spine.
`nounset_error_diagnostic` is scoped to a case that enables `nounset` and
to the single message that option produces. The statuses and every other
byte stay compared exactly, so a shell that carried on past one of these
still fails.

`tests/harness/divtest.sh` holds 28 cases across the three, every one of
them a difference the entry must refuse.

Measured 2026-09-04 over the whole corpus, before and after the status
change and these entries together: `TOTAL PASS=60881 FAIL=891 FLAKY=93
XFAIL=999` became `TOTAL PASS=61093 FAIL=679 FLAKY=56 XFAIL=1205`. Of the
212 cases that stopped failing, 206 are explained by a register entry and
six now match dash exactly. Twenty-three corpora improved and four reached
`FAIL=0`: `aud_exec_fuzz1` (`1465/35` to `1500/0`, almost all of them
`set -u` reads), `aud_parser_expand` (`137/5`), `aud_parser_discard`
(`53/1`) and `aud_state_input` (`31/1`). None of the three entries is
reported stale, and the ten entries that are were stale before this too.

### `re_entered_prompt_substitution`

**Status:** implemented in the Rust; dash unchanged. Category 3.
`crates/nsh/src/parser.rs::expand_string`.

A prompt's text is re-parsed at expansion time, and `expand_string` used to
start that parse on top of whatever the caller was reading. A pushed-back
token belongs to the caller's source, so with a string-fed shell -- where the
outer parse has already pushed `Eof` back -- a `$( )` in the prompt reached
end of file at once:

```
$ dash -c "PS4='\$(echo P)+ '; set -x; echo hi"
dash: 1: Syntax error: end of file unexpected (expecting ")")
$(echo P)+ echo hi
hi
$ dash -c "PS4='\`echo P\`+ '; set -x; echo hi"
+ echo hi                    # the backquote form, expanded to nothing
hi
```

The backquote row is the sharper one: no diagnostic, and a prompt that
silently became empty. Measured 2026-09-04 against dash 0.5.12-12, eight
`$( )` spellings and one backquote spelling diverge; `$(( ))` and `${ }` in
the same position never did, and the same scripts on standard input are
correct in both shells, because the outer parse has not reached its end
there.

`[spec:posix:req:param.ps4]` makes command substitution in `PS4`
explicitly unspecified, so *declining* to expand would conform and this
would be nobody's bug. Emitting a parse diagnostic for text that parses does
not: that is a stale token replayed, not a choice, and
`[dec:nsh:we-own-the-defects]` makes the question which shell is right
rather than which is the C.

**Both dialects, and that is the point.** `expand_string` has five callers --
`PS4` (`evaluation/command.rs`), the interactive prompt (`parser.rs`), an
array subscript (`variables/arrays.rs`), `(( ))`
(`evaluation/bash_arithmetic.rs`) and the profile/`ENV` file name
(`runtime.rs`). The subscript one was
repaired first, gated on Bash mode, which left one shared function answering
two ways for a reason neither dialect names. The other three were measured
under `bash.divergences.re-entered-parse` before the gate came off: the
profile name is expanded during startup, before any command has been parsed,
so no token is pushed back and `ENV='$(echo FILE)'` already worked; the
prompt is rendered only from a terminal-fed source, and `PS1='[$(echo s)] '`
already resolved to `[s] ` through a pty in dash and in both dialects; and
`(( ))` is Bash grammar the default dialect never reaches. So `PS4` was the
whole of the observable defect, and the repair belongs to the function
rather than to a dialect.

The entry pins four complete result pairs, one per generated witness, both
sides byte for byte -- including where in the script dash's outer parse
happens to have reached. In `xtrace nested PS4` its *first* traced command
expands correctly and only the last one does not, and in `ps4 error in
cmdsub` it recovers on its second prompt because the first failed expansion
consumed the stale token. Neither is something a looser entry could describe.

Costs four cases, unchanged in number before and after: `aud_parser_ps4`
holds `ps4 with cmdsub`, `ps4 with backquote` and `ps4 error in cmdsub`, and
`aud_exec_edge` holds `xtrace nested PS4`. Both corpora returned to their
prior tallies -- `PASS=43 FAIL=0` and `PASS=90 FAIL=2` -- with the four
reported as `XFAIL(re_entered_prompt_substitution)`.

## POSIX survey improvements outside the differential register

### `edit.history-goto` / `edit.history-search-pattern`

**Status:** sanctioned improvements. The Rust behavior leads.

`edit-history-goto-number` (`edit.history-goto`, the `[number]G` command)
and `edit-history-search-pattern-anchored`
(`edit.history-search-pattern`) pass on the port and fail on the C. The
port's line editor is nshedit; dash's is libedit; nshedit satisfies these
two and libedit does not.

The defect is in libedit, not in Dash's shell-language implementation, and
nshedit already implements the conforming result. Deliberately regressing the
native editor would have no compatibility benefit, so these outcomes are
accepted under `[spec:nsh:sem:idiom.specified-defects+1]`.

They do not get functions in `tests/harness/divergences.sh`: that register
classifies byte/status differences in the generated differential corpus, while
these are named POSIX survey cases driven through a terminal. Their executable
evidence remains those survey cases. A future result showing the port ahead in
exactly these two cases is therefore expected; a different editor mismatch is
not covered by this entry.

## Smoosh divergences taken under `[dec:nsh:we-own-the-defects]`

### An EXIT trap action does not decide the shell's exit status

**Status:** deliberate. `crates/nsh/src/trap.rs::exit_shell`.

POSIX states the rule outright:

> The value of `"$?"` after the trap action completes shall be the value
> it had before the trap action was executed.
> — `[spec:posix:req:builtin.trap.action-overrides-and-exit-status]`

So an EXIT action's own last command does not become what the shell exits
with. `trap ":" EXIT; false` leaves 1, and `trap "false" EXIT; true`
leaves 0. An `exit` *inside* the action still names the status, and an
outer `exit n` still outranks both.

This shell took the action's status instead, which is worse than losing
one: it *invented* one. `set -e; trap cleanup EXIT` -- the standard
defensive-script idiom -- reported success no matter how the script
failed, because `cleanup`'s last command usually succeeds. Found by the
adversarial POSIX cases in `posix/harness/cases_adversarial.py`, not by
the differential harness.

Five Smoosh cases expect the opposite and now fail:
`builtin.trap.subshell.false.exit`, `.loud`, `.loud2`, `.true.ec1`, and
`semantics.return.trap`. They are the outlier rather than the oracle --
GNU bash 5.2.37 and dash 0.5.13 were both run against all five and both
agree with this shell, byte for byte on output and exactly on status.
Two of the five cite dash bug threads in their own comments. Recorded in
`tests/surveys/smoosh/RESULTS.toml` under `nonpassing`, which is why the
recorded total is 181/186 rather than 186/186.

Corrected 2026-09-04, original kept verbatim above: the recorded total is
now 180/186. `builtin.unset.test` joined these five under the entry below,
which is a second, unrelated Smoosh divergence and not a change to this one.

Corrected again the same day: 173/186. Seven further cases joined under
the entry after that one, for the same reason and by the same argument.
The five named in this entry are unaffected by either; the total is shared
and the divergences are not.

Corrected a third time the same day: 172/186. `builtin.exec.badredir`
joined under the entry after that one, which is a plain divergence from
dash rather than a Smoosh-versus-contract collision. The five named in
this entry are unaffected by any of the three.

Not covered by this entry: an EXIT action that fails to *parse*. There
bash keeps the pre-trap status and dash reports its own syntax-error
status; this shell follows dash. The action never completed, so the rule
above does not reach it, and nothing in the corpus decides it.

### A refused `unset` ends the shell with dash's status, not Smoosh's

**Status:** deliberate. `crates/nsh/src/builtins/unset.rs`.

Smoosh's `builtin.unset` case records status 1 for `unset` on a read-only
name -- `tests/surveys/smoosh/shell/builtin.unset.ec` holds `1`, and
`[spec:nsh:req:compat.smoosh.error-contracts]` wrote it down. The default
dialect answers 2.

This is not two oracles disagreeing; it is two of this repository's own
rules disagreeing, and one of them was being followed by accident.
Measured 2026-09-04, `readonly R=1; unset R; echo CONTINUED`:

    dash 0.5.12-12         rc=2  `unset: R: is read only`      no CONTINUED
    pinned bash 5.3.15     rc=0  `cannot unset: readonly ...`     CONTINUED
    pinned bash --posix    rc=1  `cannot unset: readonly ...`  no CONTINUED
    nsh -o bash            rc=0  `unset: R is read-only`          CONTINUED
    nsh, before this one   rc=1  `unset: R is read-only`       no CONTINUED

The third and fifth rows are the point. Status 1 *and* exit is GNU Bash's
POSIX mode, which nothing here ever chose; this shell was reaching it by
taking Smoosh's status and dash's fatality. Meanwhile
`[spec:nsh:req:compat.bash.error-boundary]` already required "status 2 and
a non-interactive shell that exits" for the default dialect, in the
paragraph whose preceding sentence is about a special built-in's refusal
of a read-only name. So the shell was in violation of a rule, not sitting
on an open question.

`[spec:nsh:sem:idiom.specified-defects+1]` does not break the tie -- it
ranks POSIX above an explicit nsh rule above a documented divergence, and
says nothing about two nsh rules. POSIX does not either: XCU 2.8.1
requires the exit and leaves the status unspecified. The tie is broken on
provenance instead, which is the same ground that decision uses against
dash. Smoosh's bytes are imported evidence of what another shell did;
`error-boundary` is a contract this repository wrote about its own dialect
boundary, citing the standard. Evidence does not outrank a contract.

Only the status moves. The stdout `unset\nfoo\nunset\n`, the diagnostic
`unset: x is read-only`, and the shell ending at that command are all
still Smoosh's, and `crates/nsh/tests/smoosh_errors.rs` asserts every one
of them. `builtin.unset.test` therefore fails the Smoosh survey on the
status alone -- "expected 1, got 2" -- and is recorded in
`tests/surveys/smoosh/RESULTS.toml` under `nonpassing`, which moves the
recorded total from 181/186 to 180/186. The Bash dialect is untouched
and still matches 5.3.15 exactly. The remaining difference from dash --
the diagnostic spelling -- is registered above as
`unset_readonly_diagnostic`.

### Four more refusals end the shell with dash's status, not Smoosh's

**Status:** deliberate. `crates/nsh/src/builtins/dot.rs`,
`crates/nsh/src/builtins/export.rs`,
`crates/nsh/src/evaluation/command.rs::classify_abandoned_command`,
`crates/nsh/src/error.rs::expansion_error_value`.

The entry above resolved this collision for `unset` and named it as the
first. These are the rest of the class. Every one of them already ended
the shell, so the *boundary* was never in question; it was the number,
and 1 was the imported Smoosh byte where 2 is the dialect's.

Measured 2026-09-04 against `tests/.build/ref/src/dash` 0.5.12-12, the
pinned Bash 5.3.15, and a release build of this change, load 18-54. The
first two columns are the two references; the last two are the two
dialects, and no Bash-dialect answer moved.

| script (`-c`, `; echo R` appended) | dash | bash | bash --posix | nsh | nsh -o bash |
|---|---|---|---|---|---|
| `. ./nonesuch` | 2 | 0 `R` | 1 | **2** | 0 `R` |
| `readonly a=b; export a=c` | 2 | 0 `R` | 1 | **2** | 0 `R` |
| `: < missing` | 2 | 0 `R` | 1 | **2** | 0 `R` |
| `unset x; echo ${x:?boom}` | 2 | 127 | 127 | **2** | 1 |

The argument is the one the `unset` entry sets out and it is not repeated
here: POSIX.1-2024 XCU 2.8.1 requires the exit and leaves the status
unspecified, so nothing in the standard breaks the tie;
`[spec:nsh:req:compat.smoosh.error-contracts]` and
`[spec:nsh:req:compat.bash.error-boundary]` are both this repository's
rules, and `[spec:nsh:sem:idiom.specified-defects+1]` does not rank two of
those against each other; the tie goes to the contract over the imported
evidence. What is worth writing down is what checking each case *on its
own merits* turned up, because two of the four are not the same argument
twice.

**The fourth row is where the class stops being one accident.** The node
that filed this expected all five to be the shell accidentally reproducing
GNU Bash's POSIX mode, which is what the `unset` case turned out to be.
It holds for the first three: `bash --posix` answers 1 for each. It is
false for the expansion, where both Bash modes answer **127**, so the 1
there was nobody's answer but Smoosh's. That removes a supporting argument
without supplying a competing authority -- Bash is not this dialect's
reference -- so the row still moves, but for a plainer reason than the
others.

**The third row costs no divergence at all; it removes one.** `: < missing`,
`exec < missing`, `exec 3</nonesuch` and `{ exec 8</dev/null; } 8<&-; : <&8`
are now byte-for-byte identical to dash, diagnostic and status alike.
`classify_abandoned_command` had computed the dialect's status, asserted in
a `debug_assert_eq!` that the carried error still held it, and then
returned a literal 1. The one shape in that row still unequal is
`: 2>&9`, where the statuses now agree at 2 and this shell writes
`9: Bad file descriptor` for a duplication dash refuses silently -- an
older difference of dash's own `dupredirect`, unmoved by this and
untouched here.

**The second row removed a contradiction inside this shell.** A plain
`a=c` on a read-only name has always answered 2 here, through
`Diagnostics::dialect_error`; `export a=c` answered a hard-coded 1 at a
different site. The shell gave two numbers for one refusal depending on
whether a declaration utility was written in front of it. One site serves
both `export` and `readonly`, so `readonly a=c` -- a fifth spelling the
node did not name -- moves with it.

**`command` withdraws the fatality, not the number.** `command readonly
x=1` answers 2, which is dash's answer and matches what the same script
does without the `command`. A number that told a script whether `command`
had been written in front of a built-in is information neither reference
offers.

Only the numbers move. Every diagnostic keeps the prefix-less Smoosh
spelling -- `.: ./nonesuch: not found`, `export: a: is read only`,
`x: z` -- in both dialects, so a script does not have to know which
dialect it is running under to read a message. That is the same split the
`unset` entry made.

**Two clauses of the same Smoosh bullet deliberately do not move.**
`source` on a missing file keeps 1: `source` is not a POSIX built-in, dash
has no answer for it, and one oracle is not a collision.
`crates/nsh/tests/smoosh_errors.rs::missing_dot_is_fatal` asserts both
halves side by side, which is what makes the distinction a claim rather
than an oversight. "A failed no-operand `exec` redirection" -- Smoosh's
`exec 9&<-`, which parses as a backgrounded `exec 9` beside a foreground
`<-` -- also keeps 1: dash answers 2 there too, but no rule this
repository wrote has contested it, so it is a plain divergence rather than
a decision and `bash.divergences.redirection-status-without-a-command`
holds it along with the compound-command shapes
(`{ echo hi; } < missing`, `if ... fi < missing`) that have the same
defect at the other frame.

Seven Smoosh cases move to `nonpassing` in
`tests/surveys/smoosh/RESULTS.toml`, taking the recorded total from
180/186 to 173/186: `builtin.dot.nonexistent`,
`builtin.readonly.assign.noninteractive`, `builtin.command.nospecial`,
`builtin.special.redir.error`, `semantics.redir.close`,
`semantics.error.noninteractive` and
`semantics.noninteractive.expansion.exit`. Six of the seven fail on the
recorded exit status alone and every stdout and stderr byte still
matches; the seventh is `builtin.command.nospecial`, whose recorded
stdout is the `?=` line, so the same number reaches it through stdout
instead.

Corrected 2026-09-04, the entry above kept verbatim: one of those two
clauses has since moved. "A failed no-operand `exec` redirection" is the
entry below, which reached it from the other direction -- not as a
collision with a written rule, but as an inconsistency inside this shell
that dash does not have. `source` on a missing file still keeps 1, for
the reason given above and unchanged.

### A redirection failure answers the same number with or without a command word

**Status:** deliberate. `crates/nsh/src/evaluation.rs` (the
`Node::Redirect | Node::Group` arm),
`crates/nsh/src/evaluation/command.rs::classify_abandoned_command`.

This is not a Smoosh-versus-contract collision and it is not decided the
way the two entries above are. It is one shell answering two numbers for
one failure, and the argument for closing it is that neither reference
draws the distinction.

`OpenFailureContext::status` takes the dialect's number for every failed
open and every `noclobber` refusal -- 2 in the POSIX dialect, 1 in Bash
mode. Two frames then threw that number away and returned a literal
`ExitStatus::FAILURE`: `evaluate_tree`'s `Node::Redirect | Node::Group`
arm dropped the error without reading its status, and
`redirection_only_status` replaced the status whenever there was a
redirection error and no command word. Both are gone; the second function
existed for nothing else and was removed with its single caller's call.

Measured 2026-09-04 against `/usr/bin/dash` 0.5.12-12,
`tests/.build/ref/src/dash` of the same version, the pinned Bash 5.3.15
and `bash --posix`, every script run as a file operand with
`echo "rc=$?"` appended. The diagnostic was already byte-identical to
dash's in every row; only the number differed.

| script | dash | bash | bash --posix | nsh | nsh -o bash |
|---|---|---|---|---|---|
| `{ echo hi; } < /nonexistent/zzz` | 2 | 1 | 1 | **2** | 1 |
| `if : ; then echo t; fi < /nonexistent/zzz` | 2 | 1 | 1 | **2** | 1 |
| `u=; $u < /nonexistent/zzz` | 2 | 1 | 1 | **2** | 1 |
| `> /nonesuch-d/x` | 2 | 1 | 1 | **2** | 1 |
| `<-` | 2 | 1 | 1 | **2** | 1 |
| `echo one > f; set -C; > f` | 2 | 1 | 1 | **2** | 1 |
| `true < /no/such/file` | 2 | 1 | 1 | 2 | 1 |

The last row is the one that never moved, and it is the whole argument:
the same failure with a command word written in front of it already
answered 2. dash answers 2 for all seven and Bash answers 1 for all
seven, so the shell was drawing a line neither of them draws.

**What it costs, and why the cost is registered here.**
`tests/surveys/smoosh/shell/builtin.exec.badredir` expects 1 for
`exec 9&<-`, which parses as a backgrounded `exec 9` beside a foreground
`<-` and is therefore one of the command-less shapes above. It now
answers 2, matching dash. That is the eighth Smoosh case to move to
`nonpassing` in `tests/surveys/smoosh/RESULTS.toml`, taking the recorded
total from 173/186 to 172/186; its stdout is empty before and after and
both diagnostics are unchanged, so the recorded exit status is the only
byte that differs. The clause is decided in
`[spec:nsh:req:compat.smoosh.error-contracts]` by a dated paragraph that
keeps the original verbatim, which is this repository's practice for a
correction.

No entry is added to `tests/harness/divergences.sh` for this, because the
change *removes* differences from dash rather than adding one: after it
the port and the reference agree on every row above, diagnostic and
status alike. `crates/nsh/tests/redirection_status_without_a_command.rs`
is the witness for both dialects, and its last case asks the pinned Bash
itself rather than trusting the Bash column recorded above.

Measured over the whole corpus, before and after, on 2026-09-04 with a
debug binary and a `RUNALL_OUT` of its own for each pass and a working
directory of its own for each -- the second pass ran from a `git
worktree` holding this change and nothing else, so no other session's
in-flight files are in the number -- against
`tests/.build/ref/src/dash` 0.5.12-12, 8 jobs, load 42-52:
`TOTAL PASS=61094 FAIL=678 FLAKY=70 XFAIL=1206` became
`TOTAL PASS=61096 FAIL=676 FLAKY=64 XFAIL=1205`.

Three corpora improved by exactly one case each and each of the three is
one of the shapes above: `aud_exec_core` `121/2` to `122/1` on
*exitstatus: NREDIR redirect error*, and `aud_exec_deep` `90/5` to
`91/4` and `aud_exec_more` `60/7` to `61/6`, both on *empty command with
redirect error*. The node predicted two of the three; `aud_exec_deep`
was not named in it.

One corpus moved the other way and it is not a regression.
`aud_parser_fuzz3` went `884/16` to `883/17` on
`echo $(( echo ${w?'a$b'} ... )) ** | printf "<%s>" **`, whose
`parameter_error_diagnostic` excuse stopped matching -- the same case is
the one `XFAIL` that disappeared, which is why both counters moved by one
together. It is a race between two pipeline elements, and the harness
reports both shells producing two distinct outputs over its eleven runs.
Run directly, eight times against each binary, the pre-change and
post-change shells produce byte-identical output for it, so nothing in
this change reaches it. The ten stale register entries the run reports
are the same ten in both passes.

Everything else that was run: the Smoosh survey `full` group 173/186 to
172/186, the one case named above; the Bash closure gate PASS with no
unexpected failure; the Oils `bash-comparison` group `pass=2189
fail=446`, identical to the recorded baseline for a tree without
`exec -a` in it; and `cargo test --workspace`, 963 tests across 87
binaries, all passing.

## Bash-compatibility divergences taken under `[dec:nsh:we-own-the-defects]`

Bash is a reference, not an authority. Where Bash contradicts *itself*,
there is no behaviour to match, and the entry below records which of its
two answers this shell gives everywhere.

### One bracket parser, in every context a pattern appears in

**Status:** deliberate. `crates/nsh/src/pattern.rs::Matcher::bracket`.

A `]` that opens a bracket list is a member of it, not the terminator, so
`[]]` matches `]` and `[^]]` matches everything except `]`. POSIX states
that rule for `[!...]`; for shell patterns it leaves `[^...]` undefined,
so the reading is a choice. This shell makes it once and applies it to
`case`, `[[ ]]`, `${var//pattern/}` and pathname expansion alike.

Bash gives two different answers to the same spelling:

```
$ case a in [^]]) echo MATCH;; *) echo no;; esac
MATCH
$ s=ab^cd; echo "${s//[^]]/z}"
ab^cd            # nsh: zzzzz
```

Its `case` and `[[ ]]` read `[^]]` as "not `]`", which is what this shell
does. Its substitution path instead closes the list at the first `]`,
leaving a pattern that requires a further `]` after any single character
-- and since the negated list is now empty, one that matches nothing at
all, for any subject. Verified against GNU bash 5.2.37: `[^]]` and `[!]]`
never match in `${//}`, while `[]]`, `[^a]` and `[]a]` all behave
normally.

Matching Bash here would mean shipping two bracket parsers and selecting
between them by the syntactic context the pattern was written in. The
Oils corpus notes the same split from the other side -- "This is a
PARSING divergence" -- and OSH also declines to reproduce it.

Costs `var-op-patsub.test.sh:23` in the compatibility survey, registered
as a sanctioned divergence rather than a defect.

## Bash-compatibility divergences taken under `[dec:nsh:minimal-unsafe]`

The shell reaches the host only through `nsh-platform`, and that boundary
publishes the fact a shell needs rather than the syscall Bash happens to
call. Where the two differ in a way a script can see, it is recorded
here.

### `BASH_MONOSECONDS` counts from the shell's own start

**Status:** deliberate. `nsh_platform::facts::monotonic_seconds`.

Bash's value is the system's monotonic clock, which on this host is
seconds since boot: a fresh shell reads about 2.5 million. This shell's
is seconds since the first read in the process, so a fresh shell reads
`0`.

The origin is the only difference, and it is the one thing neither the
name nor Bash's own documentation fixes: the manual says the parameter
expands to "the value of the system's monotonic clock, if one is
available", and that where there is none it expands to the *realtime*
clock instead -- two origins nineteen orders of magnitude apart, from one
documented name. So a script that subtracts two reads gets the same
answer from both shells, and a script that compares a single read against
a constant was never portable between two Bash builds either.

What it buys is the boundary staying as it is.
`nsh_platform::facts::monotonic_seconds` is `Instant::elapsed` against a
`OnceLock` origin and says in its own doc that "a shell reads
differences, never the value, so the origin only has to be stable".
Reading an absolute monotonic clock means `clock_gettime` on POSIX and
`GetTickCount64` on Windows -- two host-specific FFI calls, one of them
`unsafe` -- for a number whose meaning Bash does not fix.
`SECONDS` and `EPOCHSECONDS` already read the two clocks a script can
rely on.

## Bash-compatibility divergences taken under `[dec:nsh:safety-trumps-compatibility]`

These are not dash divergences and have no entry in
`tests/harness/divergences.sh`: the reference is Bash, the evidence is
the oils survey, and the survey has no counterpart to `## BUG bash` for
a case *we* refuse on purpose. Each one is therefore recorded here and
on the plan node that owns the surface, per that decision's deferred
consequence.

### A process-substitution name lives no longer than its syntax node

**Status:** deliberate, with a stated limit.
`crates/nsh/src/evaluation/bash_process_substitution.rs`.

Bash keeps every `<(list)` and `>(list)` name open until the *outermost*
command finishes. A loop body therefore opens one pipe per iteration, and
every unrelated program the shell runs in between inherits the
descriptor. Here the shell's end is an owned close-on-exec `Descriptor`
released by `Drop` when the syntax node whose word produced it finishes
evaluating, so the descriptor cannot outlive the name and no path --
error, interrupt or early return -- can skip the release.

Observable difference: a name produced by one command of a `;` list is
gone by the next command of that same list, where Bash's would still
open. The node that built the name is still the unit, so `for w in
<(list); do ... done` and a redirected group both keep theirs for as long
as they need them.

A second, smaller one: where the system publishes no descriptor-table
directory (`/dev/fd`, `/proc/self/fd`) the substitution is a diagnostic
rather than a temporary FIFO. A carelessly created FIFO is the shape of
CVE-2000-1134, and a plausible pathname that will not open is worse than
a refusal.

The limit worth stating, because the module's ownership rules do not
reach it: close-on-exec is cleared at exactly one site, in
`execute_external_command` after the last fork this process makes -- but
it is cleared for *every* name still live in that process, not only for
the name the command being run was given. A name has to survive
indirection (`for w in <(list); do cat $w; done` reads it from a
different node than the one that built it), and the process cannot tell
which of its live names the image it is about to become will open. So an
external command run from inside a command substitution, or from the body
of a loop redirected from a substitution, inherits the shell's end of a
pipe it has no use for. The descriptor is still owned and still released
on time; what it is not is private to one image.


### The POSIX dialect ends the shell where Bash carries on

**Status:** deliberate, and now the default dialect's alone. Not a
sanctioned divergence: Bash mode takes Bash's boundary.
`crates/nsh/src/error.rs`, `crates/nsh/src/evaluation.rs`,
`crates/nsh/src/redirection.rs`.

POSIX XCU 2.8.1 requires a non-interactive shell to exit when a variable
assignment fails or an expansion is in error. The default dialect does
that: `readonly r=1; r=2` ends the script with status 2, and so do
`${#v:1:3}`, `${(m)x}`, `$((1+))` and `${a[-5]}` on a two-element array.
It is the stronger of the two boundaries and it is the one the
conformance harness is built on, so it does not move.

Bash reports each of them and carries on, and **Bash mode now does the
same**. The failure leaves status 1 and abandons the *input record* it
was raised in -- the unit `parse_and_execute` reads, which is why
`readonly r=1; r=2; echo x` prints nothing while the same three commands
on three lines print `x`. A subshell or a command substitution contains
the recovery; a function call and a loop do not. A subscript that names
no element is the one asymmetric case: it is reported and expands to
nothing, and the command it was written in runs with one fewer field,
exactly as `argv.py "${a[-5]}"` does under Bash.

A **special built-in whose utility fails** is not that shape and does not
abandon the record at all. POSIX makes an error in a special built-in
fatal to a non-interactive shell and the default dialect keeps that;
Bash treats it as an ordinary command failure, takes its status and runs
the next command of the same list, and Bash mode does the same.
`unset -v 'a['; echo after` prints `after` there, and so do `local x=1`,
`export 'a['=1`, `set -o nosuchopt`, `eval 'syntax ((('` and
`. /nonesuch`. Two frames could end a shell over one of these, and Bash
mode withdraws specialness at both: `builtin_error_is_fatal` catches
what the utility returned, and `evaluate_command_in_scope`'s `bail:`
catches a redirection that failed before the utility was entered at all,
which is `exec 3</nonesuch`, `exec 1000000</dev/null` and `: > /nodir/x`.
What
stays fatal in both dialects is what was never the built-in's own
failure: an expansion error crossing the frame on its way out of `eval`
or `.` -- `eval ': ${x:?boom}'` ends both shells -- and an unrecoverable
read of the shell's own input. `break` and `continue` keep it too, and
they are Bash's rule rather than an exception to it: their count is read
through `get_numeric_arg`'s fatal flag, which ends the shell instead of
returning, so `while true; do break oops; done` stops there as well. A
status in place of that refusal would leave the loop that asked to be
left still running.

A **redirection failure answers 1** in Bash mode, which is what Bash
answers for every one of them. Every refusal in `redirection.rs` already
did except the descriptor-dup one, which took dash's `sh_error` 2; that
was invisible from a script while a failed redirection on a special
built-in ended the shell, and is not now. `echo foo >&10` answers 1 in
Bash mode and dash's 2 in the default dialect, and `is_fd_open() { :
>&$1; }` reads the number back.

`set -e` overrides the recovery: Bash's `report_error` ends the shell
where it stands when the option is on, so a script that asked to stop at
the first error still stops at this one, in both dialects.

`set -o posix` leaves the dialect (`[spec:nsh:req:compat.bash.posix-option]`),
so it restores the fatal boundary with no separate switch. That option is
what made this fixable: the earlier entry here said the boundary could
not depend on a dialect flag because the same script would then stop in
different places, and the answer is that there is now a named state in
which the fatal boundary is the contract, and a script that wants it asks
for it in the same three words Bash uses.

**What is still not Bash's.** An assignment *prefix* on a command --
`readonly r=1; r=2 cmd` -- makes Bash report the refusal and then run
`cmd` anyway, with the prefix unapplied and status 0. Reproducing that
would mean running a command whose environment is not the one the script
asked for and calling it success, which is the error boundary weakened
rather than relocated. Here the command does not run, the record is
abandoned, and the status is 1. `(( 1+ ))` and `let 1+` are the same
shape at a smaller scale: Bash treats the arithmetic failure of those two
*commands* as an ordinary built-in failure and stays in the list, while
this abandons the record. Both were previously fatal, so both moved
towards Bash rather than away from it.

The **status** a reported special-built-in failure takes is still this
shell's and not Bash's, outside the redirection layer. Bash answers 1
where the operand was at fault and 2 where its option scan was, and this
shell answers what dash answered: `unset -v 'a['`, `local x=1` and
`export 'a['=1` are 2 here and 1 there. Both shells run the next command
either way, so the number is visible only to a script that reads `$?`,
and under `set -e`, where both stop and only the exit status differs.

Three shapes measured beside those are not the boundary at all and were
left where they are. `shift 99` is a *silent* status 1 in Bash where
this reports `shift: can't shift that many` and answers 2; both shells
run on, so what is left is a question about `shift`. `export -f` is not
implemented here, so `export -f nope` is `Illegal option -f` rather than
Bash's `nope: not a function`; both carry on past it. And a failed
`exec` leaves different descriptors open: `exec 3</dev/null 4</nonesuch`
keeps 3 open here and closes it in Bash, which undoes every redirection
of an `exec` that failed. That last one is the redirection
layer's rule rather than the boundary's, and it was unobservable while
the shell ended at the failure.

The remaining one goes the other way and is left there deliberately.
Under `set -e`, Bash ends the script at every failure in this class
*except* an arithmetic one written inside a word: `echo $((1+))` reports,
answers 1 and carries on, where `echo "${a[0][0]}"` and `${a[-5]}` do
not. That is an inconsistency in which of Bash's reporting functions each
site calls, not a rule, and reproducing it would mean `set -e` skipping
one reported failure. This ends the script for all of them.

### `RANDOM` and `SRANDOM` cannot be seeded

**Status:** deliberate. `crates/nsh/src/variables/special.rs`.

Bash makes `RANDOM` replayable: `RANDOM=n` seeds the generator, so anyone
who knows `n` knows the whole sequence. Here an assignment to `RANDOM`
re-seeds from the host's randomness instead, and `SRANDOM` -- which Bash
already documents as unseedable -- draws from the same source on every
read. A generator that anything security-adjacent might reach must not be
steerable by the data it is generating for.

Observable difference: no sequence from either name is reproducible, so a
test that pins a seed to get a fixed sequence gets a different one each
run. Nothing in the pinned Bash survey depends on it.

### One recursion budget covers parentheses and name re-reads

**Status:** deliberate. `crates/nsh/src/arithmetic.rs`.

Bash evaluates a name's *value* as an expression, which is what makes
`loop='i<=100&&(s+=i,i++,loop)'` count to a hundred, and it bounds that
recursion at 1024 levels. It does not bound the parenthesis nesting the
recursion carries, and the product is real stack: sixty parentheses
inside a self-referring name segfault Bash 5.2.

Both spend the same stack, so both spend one budget here, and the
expression is refused with `expression recursion level exceeded` where
Bash dies. The ceiling is Bash's own 1024, so every expression Bash
evaluates without recursing through parentheses is unaffected.

Observable difference: an expression that nests more than 1024 levels
deep in total is a diagnostic rather than a crash.

### `declare -f` and `$BASH_COMMAND` write a statement as it was written, not as Bash lays it out

**Status:** deliberate. `crates/nsh/src/nodes/emit.rs`,
`crates/nsh/src/variables/special.rs::running_command_text`.

This used to be the other way round, and worse: the body was re-spelled
from its parts, so a printed definition was not always the text Bash
would print and sometimes not even the same program -- `for a in;` came
back as `for a in "$@";`, which iterates over the positional parameters
instead of nothing.

Under `[spec:nsh:req:idiom.printable-ast+2]` a node carries the run of
tokens it was read from, and printing one emits that run. So each word,
each here-document and each here-document delimiter is now the source's
own bytes, and the three cases where the old renderer reached for a
construct the tree did not hold are gone by construction rather than by
being special-cased.

What is left is layout, and it is the opposite kind of difference. Bash
re-indents the grammar inside a statement:

    if x; then
        echo b;
    fi

while a statement here keeps the shape it was written in:

    if x; then echo b; fi

The frame around the body is Bash's and is reproduced exactly -- the
`name () \n{ \n` opening, the four-column indent, one statement to a
line -- because that frame is the renderer's own layout and not something
the source said. Re-indenting the inside would mean deciding where a line
may break, which is a second opinion about the grammar of the kind this
renderer exists not to have.

Nothing re-enters the parser to produce this text, which is the half of
`[dec:nsh:safety-trumps-compatibility]` that matters here: an
introspection call must not be a way to have the shell re-read whatever a
stored definition happened to contain.

`$BASH_COMMAND` is the same answer to the same question, one command
wide. Bash rebuilds the command from its parse tree, so it re-orders and
re-spaces: `>/dev/null echo z` comes back as `echo z > /dev/null`. This
shell hands back the run of tokens the command was read from, less the
separator that closed it, so it comes back as it was written. Measured
against the pinned 5.3.15, every other shape the two shells were asked
agrees exactly -- `echo   one    two` is `echo one two` on both,
`echo "a  b"` is `echo "a b"` on both, a bare `:` is `:`, an assignment
`x=1` is `x=1`, and a call `f q r` is `f q r` -- because the run holds
one token per word and the blanks between them are not in it. Only a
redirection written before the command word moves.

### An associative array iterates in sorted key order

**Status:** deliberate. `crates/nsh/src/variables/value.rs`.

Bash stores an associative array in a hash table and lists it in bucket
order, so `declare -A A=([a]=hello [b]=world [c]=osh [d]=ysh)` expands
`"${A[@]}"` as `ysh osh world hello`. The order is an artefact of the
hash function over the table size, not a property of the script.

The value model here is a `BTreeMap`, so the same array lists as
`hello world osh ysh`: sorted by key, and the same on every run and
every platform. Reproducing Bash's order would mean reproducing Bash's
hash function and its growth policy, which says nothing about what a
script means and would make the output depend on an implementation
detail of another shell.

Every operator that maps over the elements is applied per element and
joined afterwards, as Bash does -- `"${A[*]@Q}"` quotes each element
before joining -- so only the order differs.

Costs `var-op-bash.test.sh:24` and `var-op-bash.test.sh:25`.

### A `{name}` slot is opened by the shell where Bash opens it in the child

**Status:** deliberate for now, and recorded rather than sanctioned: it is
the evaluation model rather than a reading of what a script means.
`crates/nsh/src/redirection.rs`, `crates/nsh/src/evaluation.rs`.

`{name}<word` allocates a descriptor and assigns its number to `name`, and
whichever process applies the redirection is the one that ends up with
both. Bash forks an external command *before* applying its redirections, so
`cat /dev/null {fd}</dev/null` leaves the parent with no slot and `$fd`
unset. This shell applies a simple command's redirections in the parent and
forks afterwards -- the dash arrangement, and the same place a built-in's
redirection is applied -- so the parent keeps the slot and `$fd` is 10.

The two agree everywhere the fork is not in between. A built-in (`echo x
{v}>/dev/null`), a compound command (`{ echo x; } {fd}>/dev/null`), a
function call, a loop and `exec` all set the name in the shell under both,
and a subshell sets it in the child under both. Only an external command
differs, and there it costs a descriptor per execution: a loop running
`cat /dev/null {fd}</dev/null` three times leaves 10, 11 and 12 open here.

Moving it means moving the fork, which changes where *every* redirection is
applied rather than this one form. That is a change to the evaluation model
and wants its own measurement against both references, not a special case
for the one syntax that made it visible.

Pinned as a row in `crates/nsh-cli/tests/bash_named_descriptor.rs` carrying
both answers, so it fails when someone closes it. No survey case sees it.

### `TIMEFORMAT` is not read

**Status:** deliberate, under `[dec:nsh:printf-is-parsed-not-interpreted]`.
`crates/nsh/src/evaluation/timed.rs`.

Bash renders `time`'s report through `TIMEFORMAT`, a variable holding a
layout with `%R`, `%U`, `%S`, `%P` and `%%` in it, read when the report is
written. `TIMEFORMAT="%R"` therefore prints one number where the default
prints three lines. This shell ignores the variable and always writes one
of two fixed reports: Bash's default layout, or `time -p`'s two-decimal
seconds, both rendered with `write!` at the site that knows the types.

The reason is not that a `%` specification cannot be parsed here --
`printf` parses one. It is that the sanction to do so is scoped:
`[dec:nsh:printf-is-parsed-not-interpreted]` grants it to `builtins::printf`
because reading a pattern at runtime *is* that utility's contract, and says
in the same breath that it "travels no further" and that nothing outside
that module may format a value by a pattern chosen at runtime.
`TIMEFORMAT` is precisely such a pattern, and honouring it would mean a
second format interpreter in the crate for a report layout.

Everything else `time` does matches, measured against the pinned Bash
5.3.15 across eighteen shapes: `-p`, a bare `time`, `time` on a pipeline,
`!` and `time` in either order and any number, and `time time` reporting
once.

Costs no survey case; no corpus exercises `TIMEFORMAT`.

## Bash-compatibility divergences taken under `[dec:nsh:bash-compatibility-is-scripts]`

### Four names for surfaces this shell has not got

**Status:** deliberate. `crates/nsh/src/variables/special.rs`, by absence.

Started 2026-09-04 from a whole-set diff of the two shells' start-up
`declare -p`, `env -i PATH=/usr/bin:/bin LC_ALL=C`, fed on standard input:
the reference publishes 47 names, this shell 43, the 43 are a strict subset
of the 47, and the four that are missing are

    BASH_ALIASES  BASH_CMDS  BASH_LOADABLES_PATH  COMP_WORDBREAKS

`[spec:nsh:req:compat.bash.names.only-what-the-reference-has]` allows a
name of this kind exactly two endings -- absent and recorded here, or
genuinely wired to the facility -- and forbids a third, "a value that
describes nothing". All four take the first, and the reason is different
for each pair.

**`BASH_ALIASES` and `BASH_CMDS` are not variables.** They are live views
of the alias table and the command hash table, measured against the pinned
Bash 5.3.15:

```
$ alias zz="echo z"; declare -p BASH_ALIASES
declare -A BASH_ALIASES=([zz]="echo z" )
$ BASH_ALIASES[ll]='ls -l'; alias
alias ll='ls -l'
$ BASH_CMDS[qq]=/bin/echo; hash -t qq
/bin/echo
```

A write defines an alias; a read walks the table; `unset BASH_ALIASES`
destroys the view for the life of the shell and a later `alias` does not
bring it back. This shell has both tables -- `builtins/alias.rs` and the
command hash -- and no machinery for a variable that *is* one: a special
name here computes a scalar through `Callback::Special`, which is a value
and not a table. So the honest choices were absence or a subsystem, and
publishing `declare -A BASH_ALIASES=()` was never among them: it is the
listing the rule's last sentence names, one that agrees with the reference
until a script writes into it and then silently stores an array element
where an alias was meant. `bash.divergences.publish-names.table-views`
holds the wiring if it is ever wanted.

**`COMP_WORDBREAKS` and `BASH_LOADABLES_PATH` name facilities that are not
here, and one of them is not here on purpose.** `COMP_WORDBREAKS` is
Readline's completer word-break set, and
`[dec:nsh:bash-compatibility-is-scripts]` retired the interactive profile
rather than reimplement a library this shell does not link. The subtler
half is that this shell *does* complete: `editor/completion.rs` answers
filename candidates. It still cannot own this name, because its own doc
says nshedit "asks for the candidates that extend a stem and has no
opinion" about how the line was split -- the break characters live in the
editor, so a published `COMP_WORDBREAKS` could be read and assigned and
would change nothing. `BASH_LOADABLES_PATH` is where `enable -f` looks for
a shared object, and `builtins/enable.rs` refuses that option outright with
`enable: -f: loading built-ins from a file is not supported`. A path list
naming six directories a loader will never open is the same false report.

The neighbouring decision was already made the same way and is the
precedent: `progcomp` and `hostcomplete` are recognised by `shopt` but
start *off*, unlike the reference, because "reporting them on would
advertise a facility that is not there" -- and because bash-completion
reads them as a licence to load itself.

What the absence buys is a comparison rather than an enumeration.
`crates/nsh-cli/tests/bash_shell_facts.rs` used to list 42 shared names by
hand; it now takes both shells' whole published sets and asserts that the
reference's, less exactly these four, is ours -- so a name that appears on
either side without being decided is a failure rather than something
nobody wrote down. The exclusion list is these four names and nothing
else.

No survey case sees any of the four, and not because they were excluded:
grepped 2026-09-04, the name appears nowhere in `tests/surveys/oils/`,
`tests/surveys/smoosh/shell/` or `tests/corpus/`. The comparison in
`bash_shell_facts.rs` is the only thing that observes them, which is why
it had to become a whole-set claim to be worth anything.
