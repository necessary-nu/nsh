# GNU Bash compatibility profile

This profile defines an opt-in GNU Bash language dialect for `nsh`. It does
not replace the POSIX.1-2024 contract or the documented `nsh` extensions, and
it does not make Bash behavior the default. The compatibility target is the
[GNU Bash 5.3 Reference Manual](https://www.gnu.org/software/bash/manual/bash.html)
plus a reproducibly built 5.3 reference executable and a pinned survey
manifest. The exact executable and manifest are evidence artifacts governed by
the reference-profile rule below; a developer's ambient `bash` is not an
oracle.

The implementation is clean-room: the manual, black-box reference behavior,
and independently maintained surveys define observations. GNU Bash
implementation source is not an implementation input.

## Dialect and reference profile

> [spec:nsh:def:compat.bash.mode]
> Bash Compatibility Mode is a per-`Shell` dialect selected by the no-letter
> `bash` shell option. The option is disabled by default. It affects only
> behavior that this profile assigns to Bash mode; POSIX behavior and
> independently documented `nsh` extensions remain the contract when it is
> disabled.

> [spec:nsh:req:compat.bash.posix-option]
> The `posix` shell option MUST be the `bash` dialect option read inversely:
> `set -o posix` MUST disable Bash Compatibility Mode and `set +o posix` MUST
> enable it, subject to the same parse boundary as `[spec:nsh:req:compat.bash.parse-boundary]`.
> It MUST NOT reproduce GNU Bash's own POSIX mode, which corrects a fixed list
> of behaviours where Bash's default contradicts the standard while retaining
> Bash's extensions. This shell's default is the standard, so the dialect is
> the departure and a request for POSIX ends it. A script that sets `posix`
> after detecting `$BASH_VERSION` therefore gets a conforming shell rather
> than a Bash that conforms selectively.
>
> Source: `[dec:nsh:we-own-the-defects]`

> [spec:nsh:req:compat.bash.reference-profile]
> The repository MUST carry a machine-readable Bash Reference Profile that
> identifies the GNU Bash 5.3 release and patch level, source archive digest,
> build configuration, execution environment, Oils corpus revision, and exact
> eligible case manifest used for differential verification. Eligibility MUST
> be calibrated by running that reference executable: a case the reference
> passes is eligible, while every reference failure, unsupported result, known
> upstream defect, timeout, harness error, or version-inapplicable expectation
> MUST have an explicit disposition. Calibration MUST NOT rewrite imported
> expectations to make either shell pass.

## Mode selection and lifetime

> [spec:nsh:req:compat.bash.selection]
> `set -o bash` and the invocation option `-o bash` MUST enable Bash
> Compatibility Mode; `set +o bash` and invocation `+o bash` MUST disable it.
> The command-line frontend MUST enable it before parsing input when the raw
> invocation basename is exactly `bash`, including a login-shell basename
> `-bash`. A later explicit `+o bash` MUST override that inference. A command
> operand used as `$0`, including the `bash` in `nsh -c script bash`, MUST NOT
> trigger inference. Setting an embedder's `$0` value MUST likewise remain
> independent of dialect selection; an embedder selects the dialect through
> the shell-option builder API.

> [spec:nsh:req:compat.bash.parse-boundary]
> A Bash-mode option change MUST affect the next shell input unit parsed after
> the change and MUST NOT reinterpret tokens or an abstract syntax tree that
> has already been produced. `eval`, dot, and `source` MUST parse newly supplied
> text in the mode current at that parser entry. A function or compound command
> accepted in Bash mode MUST retain an executable owned syntax tree after a
> later mode change.

> [spec:nsh:req:compat.bash.state-isolation]
> Dialect state MUST have one source of truth in each `Shell`'s options. A
> subshell or explicitly cloned execution environment MUST inherit a snapshot
> and subsequent option changes MUST remain local to that environment. Two
> concurrently driven `Shell` values MUST be able to use different dialects
> without shared mutable process state, thread-local dialect state, or ambient
> environment variables.

> [spec:nsh:req:compat.bash.default-isolation]
> With the `bash` option disabled, syntax, expansion, built-in dispatch,
> variable semantics, diagnostics, and exit status MUST retain the established
> POSIX and documented `nsh` behavior. A feature introduced solely for this
> profile MUST be gated by the current dialect. Enabling and then disabling
> Bash mode MUST restore that baseline for subsequently parsed input without
> requiring a new process or `Shell`.

## Parser and runtime foundations

> [spec:nsh:req:compat.bash.parser-ast]
> The owned `nsh` parser and syntax tree MUST represent Bash-only grammar as
> explicit, dialect-gated nodes in the existing parser architecture. This
> includes `[[ ... ]]`, arithmetic commands and arithmetic `for`, Bash function
> forms, array assignment and subscripting syntax, and process substitution.
> `[[` and `((` MUST NOT be modeled as ordinary command-name built-ins, and the
> implementation MUST NOT introduce a second forked parser.

> [spec:nsh:req:compat.bash.select-time-grammar]
> Bash mode MUST accept the `select` and `time` reserved words as grammar
> rather than as command names. `select` takes `for`'s syntax exactly, and
> what it adds -- the numbered menu, `PS3`, reading a reply, and `REPLY` --
> belongs to the evaluator rather than to the parse. `time` prefixes a
> pipeline, MUST be read before the pipeline's own negation because it times
> that too, and MUST accept having no pipeline at all. Neither word may be
> reported as a keyword by `type` in a dialect that does not parse it, and
> the POSIX dialect MUST go on refusing both -- `[spec:nsh:req:compat.bash.default-isolation]`
> covers that, and this rule adds only that the refusal is the grammar's.

> [spec:nsh:req:compat.bash.value-model]
> Bash-mode variables MUST structurally distinguish scalar, sparse indexed
> array, and associative array values while retaining byte-preserving names,
> keys, and elements wherever the operating-system boundary permits them.
> Arrays MUST NOT be encoded into delimiter-separated scalar strings. Existing
> POSIX scalar reads and writes MUST remain source-compatible and MUST have the
> same observable behavior while Bash mode is disabled.

> [spec:nsh:req:compat.bash.options-builtins-dispatch]
> Shell-option lookup, built-in lookup, command hashing, and any related cache
> MUST account for the current dialect. `set -o`, `set +o`, and `shopt` MUST
> truthfully report restorable option state, including the `bash` option, and
> changing dialect or a dispatch-affecting option MUST invalidate observations
> that would otherwise retain the wrong command class or semantics.

## Bash language surface

> [spec:nsh:req:compat.bash.arrays-declarations]
> Bash mode MUST implement indexed and associative array creation, assignment,
> compound assignment, sparse subscripts, element and whole-array expansion,
> length and key queries, append, unset, attributes, and scope behavior through
> the declaration-family built-ins. Quoting, assignment context, arithmetic
> indices, and exit statuses MUST match the Bash Reference Profile.

> [spec:nsh:req:compat.bash.conditionals-arithmetic]
> Bash mode MUST implement `[[ ... ]]` conditional expressions, pattern and
> regular-expression operands, conditional short-circuiting, arithmetic
> commands, and arithmetic `for` loops with the quoting, precedence, side
> effects, diagnostics, and truth-status conventions of the Bash Reference
> Profile.

> [spec:nsh:req:compat.bash.expansion-globbing]
> Bash mode MUST implement brace expansion, Bash tilde contexts, Bash parameter
> transformations, substring and pattern substitutions, indirect expansion,
> extended glob patterns, `globstar`, and the glob-related shell options in the
> expansion order and quoting contexts required by the Bash Reference Profile.
> Enabling these features MUST NOT alter the expansion pipeline used while Bash
> mode is disabled.

> [spec:nsh:req:compat.bash.process-substitution]
> Bash mode MUST implement input and output process substitution and produce a
> pathname that remains usable for the lifetime required by the consuming
> command. Producer and consumer endpoints MUST be represented by owned or
> borrowed descriptor objects with explicit lifetimes. Core and public library
> code MUST NOT store or accept `RawFd`, call `dup2` or `close` manually, or
> rely on integer-descriptor ownership conventions.

> [spec:nsh:req:compat.bash.functions-scoping]
> Bash mode MUST implement Bash function declaration forms, positional-parameter
> frames, dynamic local-variable visibility, `local`/`declare` attributes,
> namerefs, recursion, and return behavior. Function syntax accepted in one
> parse unit MUST remain represented by its owned syntax tree rather than by
> reparsing retained source text at each call.

> [spec:nsh:req:compat.bash.traps-introspection]
> Bash mode MUST implement the Bash `DEBUG`, `RETURN`, and `ERR` trap conditions,
> inheritance options, call-stack observations, and function/source tracing
> semantics required by the Bash Reference Profile. Existing POSIX signal and
> EXIT trap behavior MUST remain unchanged when those Bash facilities are not
> enabled.

> [spec:nsh:req:compat.bash.builtins-special-variables]
> Bash mode MUST provide the Bash built-ins, declaration flags, invocation
> flags, and special variables exercised by the Bash Reference Profile.
> Version, option, call-stack, random, timing, matching, directory-stack, and
> process identity variables MUST report values derived from the current
> `Shell` and its children rather than fabricated constants or host-global
> state. `SHELLOPTS` and `BASHOPTS` MUST agree with the effective option sets.

### The names the shell publishes

Bash's variable set is part of its script surface. A script reads `BASH_SOURCE`
to find out where it is, `FUNCNAME` to find out who called it, and
`${PS1+set}` to find out whether anyone is watching. Answering those wrongly
is not a formatting difference.

`[spec:nsh:req:compat.bash.builtins-special-variables]` requires the values
this shell publishes to be derived from the shell rather than fabricated. It
says nothing about *which names exist*, and five separate divergences were
found in that gap on one afternoon -- names absent where the reference has
them, names present where it has none, and names whose meaning needs a
facility this shell does not provide.

> [spec:nsh:req:compat.bash.names.call-stack]
> In Bash mode `FUNCNAME`, `BASH_SOURCE`, `BASH_LINENO`, `BASH_ARGC` and
> `BASH_ARGV` MUST exist wherever the reference has them, including at rest
> with no function running, where the reference publishes them empty rather
> than absent. A name the reference declares without assigning MUST be
> declared without a value here too, because the difference is observable
> in the listing a script walks: `declare -a FUNCNAME` against
> `declare -a BASH_SOURCE=()`. It is not observable through `set -u`, which
> diagnoses both -- an empty indexed array has no element zero either, so
> `$FUNCNAME` and `$BASH_SOURCE` are equally unbound and `${FUNCNAME[@]}`
> and `${BASH_SOURCE[@]}` are equally silent. Each MUST be an indexed array
> value per `[spec:nsh:req:compat.bash.value-model]`, not a scalar spelled
> to resemble one, and MUST hold the frames of the call actually in
> progress. For `BASH_ARGC` and `BASH_ARGV` those frames are installed by a
> read rather than computed on one: the first read taken with nothing on
> the call stack pushes the shell's own positional parameters and they then
> stand, and a read taken with a frame on the stack pushes nothing.

> [spec:nsh:req:compat.bash.names.environment-facts]
> `TERM` and `SHELL` MUST carry the reference's defaults when the invoking
> environment supplies neither, and MUST keep an inherited value untouched when
> it does. A default MUST be established where the dialect is applied and MUST
> NOT be added to shared start-up, which would give the POSIX dialect a name
> dash has not got.

> [spec:nsh:req:compat.bash.names.ordinary-state]
> A name whose Bash meaning is state this shell already keeps MUST be published
> from that state rather than as a placeholder. This covers at least `OLDPWD`,
> `OPTERR`, `HISTCMD`, `_`, `BASH_COMMAND`, `BASH_ARGV0`, `BASH_MONOSECONDS`
> and `BASH_EXECUTION_STRING`. Publishing one of these as an empty name is
> specifically refused: it would make a listing agree while making the shell
> answer a script wrongly, since `${BASH_COMMAND}` reading empty is a different
> claim from `BASH_COMMAND` being unset.

> [spec:nsh:req:compat.bash.names.only-what-the-reference-has]
> Bash mode MUST NOT publish a name the reference does not have in the same
> configuration; a non-interactive shell MUST NOT publish `PS1` or `PS2`. A
> name whose Bash meaning requires a facility this shell does not provide --
> programmable completion, loadable built-ins, or a variable that is a live
> view of an internal table -- MUST be absent and recorded as a sanctioned
> divergence, or else be genuinely wired to that facility. It MUST NOT hold a
> value that describes nothing. A script reads these names to discover what it
> is running under, so a plausible answer is worse than no answer, and an
> associative array published as empty where the reference's is a live table is
> a listing that lies to whatever writes into it.

> [spec:nsh:req:compat.bash.error-boundary]
> Bash mode MUST take Bash's boundary for a failed variable assignment and a
> failed expansion instead of the POSIX one. A refused assignment to a
> read-only name, a bad substitution, a subscript that will not evaluate and
> any other arithmetic failure MUST be reported, MUST leave exit status 1, and
> MUST abandon the input record they were raised in rather than the shell,
> which resumes at the next record. A subscript that names no element MUST be
> reported and MUST expand to nothing, leaving the command it was written in
> to run; only an assignment or an `unset` through that subscript refuses. A
> special built-in's refusal of a read-only name MUST become that command's
> status rather than ending the shell. A subshell or a command substitution
> MUST contain the recovery, so the enclosing shell sees only a status. With
> `errexit` enabled the failure MUST remain fatal, because a script that asked
> to stop at the first error must not be carried past a reported one.
>
> The default dialect MUST retain the fatal boundary unchanged: status 2 and a
> non-interactive shell that exits, as POSIX.1-2024 XCU 2.8.1 requires and as
> the conformance harness observes. `set -o posix` leaves Bash mode per
> `[spec:nsh:req:compat.bash.posix-option]` and therefore restores it.
>
> Corrected 2026-09-04, original kept verbatim above: "as the conformance
> harness observes" is not true. Nothing in `posix/harness` covers `unset` on
> a read-only name, and nothing there asserts the number 2 for this class at
> all: the three adversarial cases that reach the boundary --
> `adv-param-error-if-unset-exits`, `adv-redir-error-special-builtin-exits`
> and `adv-exec-dot-not-found-exits` -- assert `status="nonzero"`, so this
> shell's 1 and dash's 2 both pass. The only check that observed the `unset`
> case at all asserted status 1, which is GNU Bash's POSIX mode rather than
> this dialect's boundary, and is what the shell answered until
> `bash.divergences.unset-readonly-contract`. The requirement itself stands --
> XCU 2.8.1 is its ground, not a harness result -- and what now observes it
> for `unset` is `crates/nsh/tests/smoosh_errors.rs` plus the `readonly unset`
> case of `tests/corpus/aud_state_var.txt`, whose remaining diagnostic
> difference from dash is the `unset_readonly_diagnostic` entry in
> `docs/divergences.md`. Four other failures of the same class still answer 1
> where dash answers 2, measured 2026-09-04;
> `bash.divergences.error-boundary-status-collisions` holds them.
>
> Source: `[dec:nsh:we-own-the-defects]`

## Interactive profile

Interactive behaviour is deliberately outside this profile. Bash mode is a
contract about scripts and syntax: what a Bash script means when this shell
runs it. Prompt rendering, history controls, `bind` and programmable
completion are the interactive shell's user interface, and reproducing them
would mean reproducing GNU Readline -- a separate project this shell does not
use, since it edits with `nshedit`. A rule requiring them was carried here
until 2026-08-23 and is retired; see `[dec:nsh:bash-compatibility-is-scripts]`.

Baseline up/down history navigation without vi or emacs mode remains an `nsh`
property in its own right, documented outside this profile, and is unaffected.

## Implementation and closure gates

> [spec:nsh:req:compat.bash.safe-core]
> Bash compatibility work in the core and public library MUST remain safe Rust.
> It MUST NOT introduce `unsafe` blocks or functions, direct `libc` bindings,
> `RawFd` storage or parameters, or manual `dup2`/`close` calls. Platform
> operations that cannot be expressed by `std` MUST stay behind safe,
> ownership-preserving platform APIs. The implementation MUST NOT copy code from
> GNU Bash.

> [spec:nsh:req:compat.bash.survey-closure]
> The final Bash-mode gate MUST pass every case in the pinned eligible manifest
> against the exact Bash Reference Profile with zero unexpected failures,
> timeouts, or harness errors. Every non-eligible case MUST retain its explicit
> disposition. The same revision MUST also pass the default-mode POSIX.1-2024
> harness and the complete pinned Smoosh profile, and MUST verify that default
> mode did not acquire Bash-only behavior. Every executable build, test, or
> survey used as closure evidence MUST run through the repository's process-tree
> containment wrapper.

### What "unexpected" means

"Zero unexpected failures" is a claim about a register, not about a score.
Three checked-in files decide it, and `nsh-survey gate-bash` is the executable
that reads all three:

  * `tests/surveys/oils/MANIFEST.toml` fixes which cases the group contains.
  * `tests/surveys/oils/BASH_REFERENCE_CASES.json` fixes which of them the
    pinned Bash 5.3 build itself passes -- the eligible manifest -- and carries
    a disposition for every case it does not.
  * `tests/surveys/oils/BASH_DISPOSITIONS.toml` carries one entry, with a
    category and a reason, for every eligible case this shell does not pass,
    and names the whole files that `[dec:nsh:bash-compatibility-is-scripts]`
    puts outside the contract.

The categories exist to keep two different things apart. `sanctioned-divergence`
means Bash's behaviour is refused on purpose, with the reason at the point of
divergence and, where the difference is observable outside the survey, an entry
in `docs/divergences.md`. `not-implemented` means nobody has built it, and is an
honest backlog entry. `defect` means a bug here with a known reproduction. A
gate that let the second read as the first would be worse than no gate, so the
register keeps them as separate categories and the gate reports their counts
separately.

The gate is symmetric: a registered case that starts passing fails it, exactly
as an unregistered case that stops passing does. A stale excuse is how a real
regression eventually gets waved through. The gate also refuses to run a shell
whose basename is not exactly `bash`, because `argv[0]` selects the dialect and
any other name would measure the profile with the profile turned off.

### The control run

The three files above are a recording, made on a quiet machine. A case whose
expectation depends on a race is a case where that recording and the machine
the gate is running on can disagree, and then the gate's verdict is a property
of the load rather than of the shell.

So when -- and only when -- the gate has found something to report, it runs the
group again with the pinned Bash itself, three times, on the machine it is
running on. A case where that live reference does not reproduce its own
recorded result is *undecided this run*: it is named in the report, it is not
counted as a pass, and it is not counted against this shell either.

This is not a retry. A retry would hide a real regression that only appears
under contention, which is the objection that matters; this asks a different
question -- whether the case is still measuring the shell at all -- and answers
it with the only thing that can tell the difference, which is the reference
under the same conditions. A regression the reference does not share still
fails the gate, at any load.

**Correction, 2026-09-02.** "The group again, three times" was both too much
work and too few samples, and the gate still failed about one run in nine on
`process-sub.test.sh:1` because of it. The control now re-runs only the spec
files the verdict actually turned on, and spends a budget of one group run on
them: fifteen runs for a single nine-case file, fewer as the dispute widens,
one when it spans the group. How much that buys was measured rather than
reasoned: at load 84 to 87 the pinned Bash lost `process-sub.test.sh:1` in 45
of 300 runs, and cutting that record into consecutive blocks says what each
size of control would have concluded -- three runs saw the race in 37 of 100
blocks, fifteen in 19 of 20. Nothing else about the control changed -- it still
asks only the reference, and a case this shell loses that the reference wins
every time still fails the gate.

What that bought is less than the arithmetic predicts, and the gate says so
rather than claiming otherwise. Over 48 runs at load 78 to 84 it failed 3
times, always on `process-sub.test.sh:1`, against the 1 run in 9 recorded on
`750758b`; the control fired 5 times and excused 2. The three it did not excuse
each saw the reference lose the case 0 times in 15, which at the 10% per-run
rate measured at that load is a 1-in-1000 coincidence -- so the control's
samples are probably not independent of the moment the gate's own run lost the
case. A larger count is not therefore the fix, and would make the control
likelier to excuse a barely-flaky case where this shell has a real regression.

Each undecided case is now reported with the count behind it -- *the pinned
Bash lost it in 6 of 15 control runs* -- because "the reference lost this six
times in fifteen" and "the reference lost it once in fifteen" are different
claims and only the first says the case cannot measure a shell.

`process-sub.test.sh:1` is the case that made it necessary. `seq 3 > >(tac)`
writes from a process the shell does not wait for, so the sandbox can tear the
process substitution down before it writes. Measured 2026-09-01 at load 65 and
run on its own, this shell lost that race in 15 runs of 20 and the pinned Bash
lost it in 4 of 20. Neither number is a property of either shell, and the gate
used to report the first as a compatibility failure. That this shell loses it
nearly four times as often as Bash is a separate finding and a real one; the
control keeps the gate honest about the machine, not about that.

**Correction, 2026-09-02.** The last sentence is wrong: there is no such
finding. Those two numbers were taken one shell after the other, and a rate
measured under load means nothing unless both shells meet the same load. Run
interleaved, 100 harness runs each at load 87: the pinned Bash lost the case 11
times, this shell 9, and the pre-`750758b` build 10. Those three shells were
built for the measurement and kept where nothing else writes, which matters on
a shared checkout: `target/bash-mode/bash` was rebuilt by another agent
mid-session, so three earlier runs cannot vouch for which binary they scored.
They agree in direction anyway -- at load 71, Bash 18 and this shell 4 in 100;
at load 81, Bash 29, this shell 17 and pre-`750758b` 12 in 100; under a
fork-heavy load at 88, Bash 11 and this shell 6 in 60.

This shell does not lose the race more often than the reference does, on either
tree, under a spinning load or a forking one. The surviving gate failure was
the control missing the reference's own loss, which the correction above fixes.

What the gate cannot be asked for is for the case to stop appearing. Both
shells lose it, and the only way this one could stop would be to wait for a
`>(list)` child at exit, which Bash does not do -- measured, `seq 3 > >(sleep
1; tac)` returns in 4 ms under both -- and which would hang the shell on
`exec 3> >(sleep 5); exec 3>&-`. A run that names the case as undecided is the
control working, not the gate failing.

### The same control, for the failing-case baseline

`nsh-survey run-oils --group bash-comparison --baseline PATH` is the other
comparison this profile is checked with: `tests/surveys/oils/BASH_COMPARISON_FAILURES.toml`
lists every case in that group this shell is expected to fail, and the run
exits non-zero and names every id on either side of a difference. It had the
gate's load-dependence and none of the gate's answer, in both directions.
Measured 2026-09-02 over 100 harness runs each: `process-sub.test.sh:2` is in
that list as an expected failure and this shell passed it 41 times at load 87,
while `process-sub.test.sh:1` is not in it and this shell failed it about 9
times in 100. So a loaded run reported a difference one way or the other and a
quiet one did not.

Both checks now call one control, with the same budget and the same rule: when
a comparison turns on a case, the pinned Bash is asked whether it still
reproduces its own recorded result on the disputed spec files, and a case it
cannot is undecided this run -- named with the count behind it, and left out of
the verdict. It is asked of the reference alone in both, so a case this shell
loses that the reference wins every time is still a difference at any load.

**The generator needed it more than the comparison did.** `--update-baseline`
takes the run's failing set as the new list, so a lucky run silently *deletes*
a known-flaky entry rather than merely reporting one. `process-sub.test.sh:2`
went that way on 2026-09-01 and had to be measured again and put back by hand,
and it was noticed only because somebody remembered it should be there. A
refresh now keeps the answer already recorded for every case the run could not
decide, in both directions: a case the run newly failed is not enshrined as an
expected failure either.

**What the control cannot do, measured.** Over ten consecutive comparisons at
load 6 to 14 the verdict was *matched* ten times, with the control excusing
`process-sub.test.sh:2` on the one run that flipped it. Over ten more at load
64 to 97 it was *matched* six times and reported a difference four times -- and
every one of those four was the same case, `sh-options.test.sh:23` ("noclobber
on `&>> >>`"), with nothing else on the list. The control did its half on those
same runs: it excused `process-sub.test.sh:1` at 2 of 15, `:2` at 4, 7, 8 and
10 of 15, `background.test.sh:7` at 5 of 15 and `:12` at 3 and 7 of 15.

`sh-options.test.sh:23` is not one the control may excuse. It is in the list as
an expected failure, it passed four of those ten loaded runs, and the pinned
Bash reproduced its own recorded result in all fifteen control runs every time
it was asked. A case only *this* shell is unstable on is a defect of this
shell, and a control that excused it would be the retry this design exists not
to be. Nor is load the variable it looks like: at load 94 the case fails 6 of 6
runs on its own, 4 of 6 when its whole spec file runs, and 6 of 10 in the whole
group -- so it is stable alone and unstable behind other cases, at one load. It
is filed as `stop-the-noclobber-append-case-flaking` rather than absorbed into
the machine's column.

That shape looked like a lead on the gate's three misses at 0 of 15 -- a
one-file control run being a different environment from the group run whose
verdict it checks -- and it was tested rather than assumed. Interleaved at load
89 to 112, the pinned Bash lost `process-sub.test.sh:1` in 2 of 9 whole-group
runs and 26 of 135 one-file runs, 22% against 19%, which nine group samples
cannot separate from equal. So the guess in the constant's own doc comment is
not supported by this evidence, the three misses stay unexplained, and the
count behind every undecided case stays printed so that the next person can see
the thing rather than infer it.
