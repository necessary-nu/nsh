# Native nshedit integration

`crates/nsh/src/linedit.rs` consumes nshedit's Rust API directly. The
dependency is an exact HTTPS-form Git revision, currently
`cfb16cee5c51144a7a7f1b3574add4dd008a79d4`; there is no floating Git ref
in the build. The repository still requires authentication, so this pin
does not yet make unauthenticated clone-and-build possible.

The old libedit-shaped compatibility layer is gone:

* `LineEditor` owns an `Editor<SystemTerminal>`, `ReadDriver`, duplicated
  terminal descriptors, and the descriptor-lifetime proof joining them.
* `History` owns `HistoryStore` entries and adds only shell policy:
  displayed `fc` numbers, multiline append targets, byte-exact records,
  and the delayed `HISTSIZE` limit.
* Prompt, history, alias, completion, external-editor, terminal-size and
  user-command requests are handled as typed effects. Shell vi bindings
  and POSIX history-pattern matching remain consumer-specific.
* Terminal profiles come from `nshterm`; configured `stty` characters
  come from `nshedit-plat`. Invalid UTF-8 input remains raw bytes rather
  than being replaced.
* Normal teardown and shell signal unwinds restore cooked mode before
  dropping the owned descriptors. Host-side terminal output invalidates
  the committed display image before the next prompt.

The historical C-ABI failures (`el_init` descriptor recovery,
`el_gets` byte counts, variadic history callbacks) are no longer
reachable because those entry points and operation codes are not part of
the integration.

The live acceptance checks are:

    cargo test -p nsh --lib
    python3 tests/harness/ptydiff.py
    python3 posix/harness/run.py --shell target/debug/nsh \
        --reference tests/.build/ref/src/dash
