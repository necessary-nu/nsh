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
