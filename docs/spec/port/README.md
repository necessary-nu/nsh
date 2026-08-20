# Dash behavioral provenance

These documents record the behavior inherited from Dash 0.5.13.5. The live
rules are the `[spec:dash:sem:…]` blocks implemented by nsh and exercised by
the differential corpus.

C signatures and type layouts remain beside those rules as historical prose.
They explain where a behavior came from, but they are not `dash:def` rules and
do not require Rust modules, symbols, fields, or control flow to mirror C.

The C tree is deliberately not vendored or indexed as a second implementation.
[`tests/DASH_REFERENCE.env`](../../../tests/DASH_REFERENCE.env) pins the release,
upstream commit, archive digest, and local oracle-patch digests.
[`tests/build-reference.sh`](../../../tests/build-reference.sh) downloads and
verifies those bytes, then builds and executes them only through the repository
containment boundary. The differential and PTY corpora remain the executable
behavioral evidence.

## Retired source-only records

The following extracted `sem` ids did not describe target behavior. Their
source text remains beside the historical C signature, but the ids are no
longer live specification rules:

- `alias.freealias-fn` and `alias.lookupalias-fn` described intrusive hash
  chain mutation. Owned alias entries and `alias.lookupalias-pub-fn` replace
  that topology.
- `bltin.echocmd-fn` was a duplicate declaration. The live behavior is
  `printf.echocmd-fn`.
- `error.exerror-fn`, `error.exraise-fn`, `error.exverror-fn`,
  `error.inton-fn`, and `error.sh-error-fn` described variadic diagnostics,
  `longjmp`, and C interrupt macros. Typed `Error`, `Flow`, and structured
  interrupt deferral replace them.
- `eval.skiploop-fn` described the evaluator's global skip flags. Typed
  evaluator control flow replaces them.
- `exec.getcmdentry-fn`, `jobs.onsigchild-fn`, and `main.etext-fn` were,
  respectively, an uncompiled function, a declaration with no definition,
  and an optional linker profiling symbol. None had runtime behavior in the
  reference configuration.
- `expand.addfname-fn` and `options.freeparam-fn` existed only to manage C
  allocation ownership. Owned Rust collections and `Drop` replace them.
- `expand.memrchr-fn`, `system.mempcpy-fn`, and `system.strchrnul-fn` were
  libc compatibility helpers replaced by slice and iterator operations.
- `system.gl-closedir-fn`, `system.gl-lstat-fn`, `system.gl-opendir-fn`,
  `system.gl-readdir-fn`, `system.gl-stat-fn`, `system.glob64-fn`, and
  `system.globfree64-fn` described unused fallback layout or link stubs. nsh's
  owned pathname-expansion engine replaces the libc/fallback split.
- `var.bltinlookup-fn`, `var.findvar-fn`, `var.var.func-fn`, and
  `var.varnull-fn` described a wrapper, hash-chain link addresses, a function
  pointer member, and double-NUL C-string storage. Owned keyed variables,
  typed callbacks, and optional values replace them.

All remaining `dash:sem` rules describe behavior and carry a Rust
implementation claim. A repository test enforces that relationship so a
future source-only rule cannot silently recreate the old sidecar contract.

This boundary implements `[spec:nsh:req:idiom.port-provenance+1]`.
