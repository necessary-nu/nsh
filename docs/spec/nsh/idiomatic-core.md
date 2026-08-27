# Idiomatic Rust core

These rules define the implementation boundary of the `nsh` library after the
Dash translation phase. They constrain representation, ownership, and error
handling without changing the shell language into UTF-8 text or hiding genuine
POSIX process concepts. Shell words remain byte strings, shell descriptor
numbers remain language-level values, and the private platform crate may use
the unsafe POSIX ABI when no sound safe primitive exists.

The POSIX and nsh specifications own observable behavior. The Dash-derived
specification and differential corpus retain implementation provenance and
regression evidence, but do not require preservation of a Dash defect or C
undefined behavior.

## Frontend representation

> [spec:nsh:req:idiom.lexer-tokens]
> Lexer tokens, end-of-input, keyword eligibility, and syntax contexts MUST be
> represented by Rust enums or dedicated value types. The core MUST NOT encode
> them as C integer constants, negative sentinel bytes, keyword-table offsets,
> or arrays biased to permit negative indexing.

> [spec:nsh:def:idiom.word-ir]
> A Parsed Word is an ordered structural sequence of literal bytes, quoting
> boundaries, parameter expansions, command substitutions, arithmetic
> expansions, and other explicitly typed shell-language parts. It MUST preserve
> non-UTF-8 input and exact quote semantics without `CTL*` marker bytes,
> trailing-NUL framing, packed `VS*` flag bytes, or a substitution list whose
> meaning depends on parallel marker order.

> [spec:nsh:req:idiom.structural-ast]
> The syntax tree MUST use enum variants whose payloads contain the fields valid
> for that grammar form. It MUST NOT expose numeric node tags, C-union-shaped
> lowercase payload types, or accessors that panic because a caller selected the
> wrong union arm.

> [spec:nsh:req:idiom.immutable-ast]
> A successfully parsed syntax tree MUST contain every grammar-required child
> and finalized here-document body and MUST thereafter be immutable. Evaluation
> state, expanded filenames, and resolved descriptor operands MUST be stored
> outside the parsed tree rather than through `Cell`, `RefCell`, mutex slots, or
> other delayed mutation of syntax.

> [spec:nsh:req:idiom.canonical-tree]
> A program MUST have exactly one tree. Sources that differ only in how they
> spell the same program MUST parse to the same tree, and the tree MUST NOT
> record which spelling was written: which quote opened a run, whether a byte
> was made inert by a backslash or by the quoting around it, and which of two
> interchangeable delimiters was used are spelling. A distinction that changes
> what the program does is not spelling and MUST be represented. Where no
> spelling of a construct exists, the parser MUST reject it rather than admit a
> tree that cannot be written back.

> [spec:nsh:req:idiom.printable-ast+1]
> Rendering a parsed syntax tree back as shell source and parsing that source
> again MUST produce the same tree, apart from the source positions the render
> relocates. A renderer MUST NOT substitute a different construct for one it
> cannot spell, so every construct the parser accepts MUST be representable in
> the tree together with whatever the renderer needs to spell it again. Spelling
> and layout -- which quote, which escape, which of two interchangeable
> delimiters, indentation, separators, and line breaks -- are not part of the
> tree and are the renderer's to choose.

> [spec:nsh:sem:idiom.typed-expansion]
> Word expansion consumes Parsed Words and produces owned byte-string fields.
> Quoting, splitting, substitution, and globbing MUST be expressed as typed
> transformations rather than in-place surgery over control-byte-encoded
> buffers, while retaining the behavior required by the active shell dialect.

## Platform boundary

> [spec:nsh:req:idiom.platform-errors]
> Every safe `nsh-platform` operation MUST report failure with a typed Rust
> error or `io::Error`. Its public API MUST NOT require callers to interpret raw
> errno values, `0`/`-1` success conventions, or undocumented integer sentinels.

> [spec:nsh:def:idiom.process-identity]
> Process and process-group identities crossing the platform boundary are
> distinct validated types. Core code MUST NOT exchange bare `i32` values for
> PIDs or process groups, and the representation of the POSIX zero and negative
> process-selection forms MUST be explicit in the operation that supports them.

> [spec:nsh:req:idiom.process-group-zero-state]
> A zero process group reported across a PID-namespace boundary MUST be carried
> as an explicit typed state, not absence or failure. Job control MUST compare
> and round-trip that state without disabling itself before it establishes the
> shell's own positive group.

> [spec:nsh:def:idiom.signal-wait]
> Signals and child wait outcomes crossing the platform boundary are typed
> values. The core MUST NOT decode signal numbers, wait-status words, stopped
> states, or core-dump bits with raw integer tests.

> [spec:nsh:req:idiom.filesystem-account-bytes]
> Filesystem and account lookup APIs MUST accept and return owned byte-preserving
> or operating-system string values. C-string construction, passwd pointers,
> directory-entry pointers, and ABI-specific stat layouts remain private to
> `nsh-platform`.

> [spec:nsh:req:idiom.exec-boundary]
> Program execution MUST accept owned byte-preserving path, argument, and
> environment values through a safe platform API. Exact `execve` pointer arrays
> and terminating null pointers MUST be constructed only inside the private ABI
> implementation and MUST NOT become core data structures.

> [spec:nsh:req:idiom.descriptor-materialization]
> Descriptor duplication and closure needed immediately before process
> execution MUST be represented by an owned, validated transaction whose safe
> API guarantees cleanup. Numeric `dup2` and `close` calls may exist only inside
> that private transaction implementation.

> [spec:nsh:req:idiom.no-raw-fd-core]
> Core code MUST use logical shell-descriptor capabilities and owned I/O values;
> it MUST NOT import, store, return, or inspect `RawFd`, call `AsRawFd`, or close
> an operating-system descriptor by number. Converting owned handles for an ABI
> call is exclusively a platform-boundary responsibility.

## Runtime domain model

> [spec:nsh:req:idiom.status-flow-signal]
> Exit status, evaluator control flow, and signals MUST use their dedicated
> types throughout the core. A plain integer MAY be produced only at an
> external compatibility boundary and MUST NOT be the internal currency for
> success, shell exit, interruption, or evaluator flow.

> [spec:nsh:def:idiom.shell-options]
> Shell options are named typed fields or entries in a typed option set. Their
> long names and invocation letters are declarative metadata; option state MUST
> NOT be a C-macro-shaped character array addressed by integer constants.

> [spec:nsh:def:idiom.variable-expansion-state]
> Variable values, assignment attributes, parameter-expansion modes, and
> expansion intermediate state use structural Rust types. Missing, empty,
> scalar, indexed-array, and associative-array states MUST NOT be conflated
> through delimiter encodings, integer tags, or magic pointer/byte values.

> [spec:nsh:req:idiom.command-dispatch]
> Commands and builtins MUST be dispatched by matching typed variants or typed
> specifications. The core MUST NOT pair an integer command tag with payload
> accessors whose invalid combinations terminate through `unreachable!`.

> [spec:nsh:def:idiom.job-control-model]
> Job identifiers, job states, process membership, process groups, and wait
> outcomes are distinct types with explicit valid transitions. Boolean job
> properties MUST be `bool`, not flag bytes or integer fields.

> [spec:nsh:req:idiom.job-storage]
> The job table MUST represent occupied and vacant entries structurally and
> maintain current-job ordering without C pointer-link emulation, `used` bytes,
> or magic zero values for absence.

> [spec:nsh:def:idiom.trap-dispositions]
> Trap actions, inherited/default/ignored dispositions, and pending-delivery
> state are typed values. They MUST NOT be represented by `c_char` modes or
> unrelated numeric constants sharing one field.

> [spec:nsh:def:idiom.logical-descriptors]
> A shell descriptor number is a language-level identity distinct from an owned
> host handle. APIs MUST make that distinction explicit and MUST prevent an
> arbitrary integer from acquiring ownership or host-close semantics.

> [spec:nsh:req:idiom.operation-modes]
> Evaluation, expansion, redirection, escaping, and display modes MUST use
> enums, option structures, or narrowly scoped bitflags whose valid
> combinations are documented. Unrelated C bit constants MUST NOT share an
> untyped integer parameter.

## Control flow and lifetimes

> [spec:nsh:req:idiom.parser-control-flow]
> Parser and expander control flow MUST use Rust loops, matches, returns, and
> typed states. It MUST NOT reproduce C labels with labelled blocks plus a
> mutable integer program counter.

> [spec:nsh:req:idiom.evaluator-control-flow]
> Evaluator and builtin control transfer MUST use `Result`, `Flow`, and
> structured Rust control flow. Integer program counters, translated `goto`
> dispatch, and global skip codes MUST NOT be used to emulate local control.

> [spec:nsh:req:idiom.jobs-startup-control-flow]
> Job formatting, `read`, and shell startup MUST be decomposed into structured
> operations. A genuine multi-step protocol MAY use a typed state enum, but
> MUST NOT encode C labels as integer states or preserve unreachable C
> fallthrough behavior.

> [spec:nsh:req:idiom.resource-scopes]
> Temporary redirections, input frames, local-variable frames, and analogous
> resources MUST be owned by structured scope operations that restore or
> consume state on every specified return path. Correctness MUST NOT depend on
> manually matching distant `push`/`pop` or `unwind` calls.

> [spec:nsh:sem:idiom.interrupt-deferral]
> Interrupt deferral is an explicit scoped state transition. Nested scopes MUST
> restore the previous depth, and a pending interrupt MUST be delivered at the
> documented polling boundary; replacing `INTOFF`-style pairs MUST NOT silently
> change observable trap, cleanup, or fork-child behavior.

> [spec:nsh:req:idiom.owned-lifecycle]
> Construction, reset, fork-child reset, and exit cleanup MUST be methods of the
> state they affect with explicit ordering. The core MUST NOT reproduce the C
> `mkinit` generator, fragment registries, or `INIT`/`RESET` macro lifecycle.

> [spec:nsh:req:idiom.narrow-shell-context]
> A function MUST receive only the subsystem state and capabilities it needs,
> except at orchestration boundaries that genuinely coordinate the whole
> shell. `&mut Shell` MUST NOT serve as a universal replacement for C globals,
> and sibling subsystems MUST NOT freely mutate each other's private state.

## Components and source organization

> [spec:nsh:req:idiom.builtin-registry]
> Builtin metadata MUST use byte-preserving Rust names, typed attributes, and a
> total handler representation. The registry MUST NOT use C strings, integer
> flags, nullable function pointers, a manual `NUMBUILTINS`, or C-generator
> table conventions.

> [spec:nsh:req:idiom.output-results]
> Internal output operations MUST implement or compose `Write` and return
> `io::Result` or a typed shell error. A separate error flag and `0`/`-1`
> convention MUST NOT substitute for returning the failure to its owner.

> [spec:nsh:req:idiom.no-artificial-limits]
> The implementation MUST NOT truncate diagnostics, command renderings, signal
> descriptions, or other dynamically representable values merely because the
> C implementation used a fixed local buffer. Fixed-size storage MAY remain
> only where an ABI or encoding maximum makes the bound semantic and tested.

> [spec:nsh:req:idiom.no-mystring]
> There MUST NOT be a generic `mystring` compatibility module. Byte parsing,
> validation, prefix handling, and shell quoting belong to their domain modules
> and MUST expose descriptive Rust names and typed errors.

> [spec:nsh:req:idiom.no-port-fossils]
> Production code MUST NOT retain constant preprocessor configurations,
> impossible `cfg` branches, declaration-only raw-pointer stubs, unused C type
> aliases, or no-op branch-prediction helpers solely because the source port
> contained them.

> [spec:nsh:req:idiom.shell-entrypoint]
> `Shell` and its builder are the core runtime entrypoint. Invocation parsing,
> process exit policy, and presentation belong to the CLI crate; a public
> `shellmain`-style C entrypoint MUST NOT bypass the library surface.

## Closure

> [spec:nsh:req:idiom.no-c-strings-core]
> `crates/nsh` MUST NOT use `CStr`, `CString`, C string literals, or artificial
> trailing NUL bytes as its vocabulary. Shell data uses `BStr`/`BString` or
> operating-system string types, and the private platform ABI owns any required
> terminator conversion.

> [spec:nsh:req:idiom.no-abi-scalars-core]
> `crates/nsh` MUST NOT use `core::ffi` or `std::ffi` C scalar aliases. Domain
> values use enums, booleans, newtypes, or explicitly sized numeric types; a
> bulk substitution from `c_int` to `i32` does not satisfy this rule.

> [spec:nsh:req:idiom.module-boundaries]
> Modules MUST be organized around Rust subsystems and ownership boundaries,
> not one-for-one C translation units. Compatibility filenames such as
> `arith_yacc`, `shellmain`, and `pmatch` MUST be retired once their behavior is
> owned by typed arithmetic, runtime, and pattern modules.

> [spec:nsh:req:idiom.rust-naming]
> Types, functions, constants, modules, and local variables MUST follow Rust
> naming conventions and use descriptive words. Lowercase type names,
> run-together C function names, faux-macro uppercase functions, `*cmd`
> dispatch suffixes, and unexplained C abbreviations MUST NOT remain merely for
> source correspondence.

> [spec:nsh:sem:idiom.specified-defects+1]
> Observable behavior follows POSIX, an explicit nsh compatibility rule, or a
> documented sanctioned divergence in that precedence order. Dash behavior is
> regression evidence, not authority to preserve a defect, undefined behavior,
> impossible fallthrough, null dereference, or uninitialized value.
>
> Invalid internal states MUST be unrepresentable or produce an explicit error;
> they MUST NOT be interpreted through a C union arm or release-build
> fallthrough. Where POSIX leaves signed shell-arithmetic overflow unspecified,
> nsh defines two's-complement `i64` wrapping and masks shift counts modulo 64,
> so the result is deterministic and independent of host-language undefined
> behavior. Numeric operands whose valid range is specified by the
> implementation MUST be range-checked rather than wrapped.

> [spec:nsh:req:idiom.port-provenance+1]
> Source-port provenance MUST remain discoverable without constraining the Rust
> implementation to C symbol topology. Historical C signatures MAY remain as
> non-normative prose, but the live Dash-derived specification MUST contain only
> behavioral `dash:sem` rules, and Rust MUST NOT claim `dash:def` correspondence.
> The repository MUST NOT index a missing C implementation, retain a source-port
> manifest, or keep sidecar claims for removed paths. Behavioral rules and
> differential cases MUST be retained or replaced before their structural
> anchors are removed.
>
> The exact Dash oracle MUST be pinned by release, commit, archive digest, and
> local patch digests. Its source MUST be downloaded and verified rather than
> copied from a repository source tree; downloaded code MUST be built and run
> only inside the repository containment boundary.

> [spec:nsh:req:idiom.no-ignored-results]
> Every fallible operation MUST be propagated, handled according to an explicit
> shell rule, or deliberately discarded with a local explanation. Blanket
> `let _ =` result loss, sticky error side channels, and unchecked numeric
> sentinel returns MUST NOT hide failures.

> [spec:nsh:req:idiom.strict-lints]
> The core crate MUST deny unsafe code and enforce standard Rust naming,
> dead-code, unused-variable, and selected Clippy correctness lints. Crate-wide
> allowances for `dead_code`, nonstandard naming, unused variables, or
> `clippy::all` MUST NOT be used to accommodate translated C structure.

> [spec:nsh:req:idiom.regression-gates]
> Repository checks MUST fail when forbidden core C scalar aliases, C strings,
> control-byte word encodings, raw descriptors, translated integer program
> counters, blanket lint suppressions, or reintroduced unsafe/libc use appear
> outside the documented private platform allowlist.

> [spec:nsh:req:idiom.conformance-closure]
> The cleanup is complete only when workspace checks and tests pass and the
> contained differential, POSIX, Smoosh, and applicable Oils survey manifests
> have no unexpected regressions, timeouts, harness errors, or unrecorded
> divergences. Verification MUST execute through the repository's containment
> boundary and MUST NOT signal or terminate the invoking terminal session.
