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
below is registered in `tests/harness/divergences.sh`. Its 210 focused checks
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


### One error boundary for both dialects

**Status:** deliberate, with a stated cost. Registered in
`tests/surveys/oils/BASH_DISPOSITIONS.toml` as `sanctioned-divergence`.

POSIX XCU 2.8.1 requires a non-interactive shell to exit when a variable
assignment fails or an expansion is in error. This shell does that in
both dialects: `readonly r=1; r=2` ends the script with status 2, and so
do `${#v:1:3}`, `${(m)x}` and `${a[-5]}` on a two-element array. Bash
reports each of them, yields nothing for the expansion, and carries on
with status 1.

The cost is real and is the reason this is recorded rather than assumed:
a Bash script that assigns to a read-only name and expects to keep going
stops here instead. It is the one place where "a Bash script means the
same thing" is knowingly not delivered.

It is recorded rather than fixed because the fix is not local. Making the
boundary depend on the dialect means every fatal diagnostic has to carry
"fatal in POSIX, status 1 in Bash", and the same script then stops at a
different place depending on a flag it may never have set -- while the
POSIX-mode guarantee is the stronger of the two and the one the
conformance harness is built on. The third mode this really wants is
`set -o posix`: Bash's own POSIX mode keeps arrays and `[[ ]]` while
tightening exactly this boundary, and it is a state neither `-o bash` nor
`+o bash` can express. That mode is not implemented, and the choice
belongs with it rather than with a gate.

Costs, in the pinned Bash survey: `array-assoc.test.sh:27`,
`assign-extended.test.sh:24`, `:25`, `:35`, `nameref.test.sh:18`,
`array.test.sh:9`, `:10`, `:46`, `:47`, `var-op-slice.test.sh:1`,
`var-ref.test.sh:20`, `zsh-idioms.test.sh:2`.

### `SHELLOPTS` and `BASHOPTS` are not read-only

**Status:** deliberate, and a consequence of the entry above.
`crates/nsh/src/variables/special.rs`.

Bash marks both names read-only. Importing the mark would turn
`SHELLOPTS=x` -- a line Bash tolerates with status 1 -- into an aborted
script under this shell's error boundary. Both listings still answer for
the option table, because the next read recomputes them and discards
whatever was assigned.

Costs `sh-options-bash.test.sh:1` ("SHELLOPTS is readonly").

### `\u` and `\U` above U+10FFFF produce no bytes

**Status:** deliberate. `crates/nsh/src/escape.rs`.

Bash encodes any value the escape names, so `$'\U00110000'` yields
`f4 90 80 80` -- a four-byte sequence that is not UTF-8 for any
character, because no such character exists. This shell produces nothing
for a value at or above `0x11_0000`, in `$'...'`, `printf` and `echo -e`
alike, rather than manufacturing bytes that no decoder will accept.

The refusal is currently silent, which is the wart in it: a script gets a
shorter string with no diagnostic. Diagnosing it would be better, and is
not what Bash does either.

Costs `unicode.test.sh:3` and `unicode.test.sh:5`.

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

### `declare -f` prints a rendering of the definition, not its source

**Status:** a stated limit. `crates/nsh/src/nodes/source.rs`.

Bash keeps each word's original spelling and re-indents the grammar
around it. This tree keeps structure rather than bytes, so a printed body
is re-spelled from its parts: it means what the definition means and
re-reads as the same function, but it is not always the same text Bash
would print. A here-document's delimiter is not retained and is printed
as `EOF`; a brace group holding one command loses its braces, because
`{ list; }` parses to the list itself; `a & b` prints on two lines; and a
parameter takes `${name}` where the byte after it could otherwise be read
as more of the name.

Nothing re-enters the parser to produce this text, which is the half of
`[dec:nsh:safety-trumps-compatibility]` that matters here: an
introspection call must not be a way to have the shell re-read whatever a
stored definition happened to contain.
