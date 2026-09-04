# Necessary Shell

`nsh` is a POSIX shell written in Rust. The repository provides both:

- `nsh`, an embeddable shell library; and
- `nsh-cli`, the `nsh` command-line shell.

The implementation began as a port of dash 0.5.13.5. POSIX behavior remains
the default, while an opt-in GNU Bash 5.3 compatibility mode is under
development.

Development and testing currently target Linux. Building requires Rust 1.85 or
newer.

## Status

The POSIX implementation is checked against the POSIX.1-2024 rule corpus, a C
dash reference build, and the Smoosh and Oils survey suites. Deliberate
differences from dash are listed in [docs/divergences.md](docs/divergences.md).

Bash mode is experimental and incomplete. Its target behavior is defined in
the [Bash compatibility profile](docs/spec/nsh/bash-compatibility.md); it should
not yet be treated as a drop-in replacement for Bash.

## Build and run

```sh
cargo build --release

./target/release/nsh -c 'printf "hello\n"'
./target/release/nsh script.sh
./target/release/nsh
```

To install the command-line shell from this checkout:

```sh
cargo install --path crates/nsh-cli
```

### Bash mode

Bash mode is disabled by default. It can be selected on invocation, enabled
from a running shell, or inferred from an exact `bash` or `-bash` invocation
name:

```sh
nsh -o bash script.bash
ln -s "$(command -v nsh)" ./bash
./bash script.bash
```

Within a running shell, use `set -o bash` to enable Bash mode and `set +o bash`
to return to POSIX mode for subsequently parsed input.

## Library

`Shell` owns its variables, options, parser state, jobs, and logical file
descriptors. Shell data is represented as byte strings, so arguments and
variable values do not have to be UTF-8.

```rust
use bstr::BStr;
use nsh::{Error, Shell};

fn run_script() -> Result<(), Error> {
    let mut shell = Shell::builder()
        .arg0(BStr::new(b"myapp"))
        .inherit_env()
        .build()?;

    shell.run(b"greeting=hello")?;
    shell.run(b"printf '%s from nsh\\n' \"$greeting\"")?;
    Ok(())
}
```

The builder does not inherit the process environment unless
`inherit_env()` is requested. It also uses a non-privileged host by default,
so operations such as replacing the process or taking control of the terminal
must be granted explicitly. See the runnable
[embedding example](crates/nsh/examples/embed.rs) for captured streams, word
expansion, positional arguments, and a custom `Host` implementation:

```sh
cargo run -p nsh --example embed
```

Embedding has three process-level constraints:

- The shell may reap any child of the process. Do not concurrently manage
  other child processes from the same host process.
- Shell jobs use `fork`; the usual restrictions on forking a multithreaded
  process apply.
- Dropping a `Shell` neither waits for nor kills background jobs.

The public API is documented by the crate and checked with
`#![deny(missing_docs)]`.

## Testing

> [!WARNING]
> Test cases are hostile input. Never run them outside the repository's
> containment wrapper. Some cases deliberately exercise process-wide signals
> and job-control behavior.

Bootstrap the C reference and the generated locales, then run builds and tests
through `scripts/sandboxed`. The reference bootstrap downloads the pinned
official Dash archive, verifies its SHA-256 and the two documented oracle
patches, and builds it inside the same containment boundary:

```sh
./tests/build-reference.sh
export LOCPATH=$(./tests/build-locales.sh)
scripts/sandboxed -- cargo build

scripts/sandboxed -- cargo test --workspace
scripts/sandboxed --writable tests/.build -- tests/harness/runall.sh 12
scripts/sandboxed --writable posix -- \
    python3 posix/harness/run.py --shell target/debug/nsh
```

The wrapper requires the `sandbox` tool and fails closed if containment cannot
be established. Read [tests/README.md](tests/README.md) before adding or
changing differential cases.

Cargo carries that boundary itself: `.cargo/config.toml` names
`scripts/sandboxed --cargo-runner` as the target runner, so every binary
`cargo test`, `cargo run` and `cargo bench` execute goes into a PID namespace
whether or not the wrapper was typed in front. There is no spelling of
`cargo test` that runs a shell case with the session's own process table, and a
sandbox that cannot be established is a failed command rather than an
unsandboxed one. Keep the wrapper on the outside anyway — it is what asks
whether the machine is carrying an abandoned case, a question about the whole
command rather than about each of its sixty-two test binaries, and it is what
contains the commands cargo has nothing to do with. Two boundaries nest without
complaint; the run above is unchanged by the runner except that it costs about
40 ms per test binary.
`tests/harness/containment-selftest.sh` is the runner's self-test and, like
`tests/harness/abandoned-selftest.sh`, must not be run through the wrapper: it
asks the host whether a descendant survived, which is what the boundary hides.

`LOCPATH` is an export rather than a path the tests look up themselves. A
locale is opened by name, and glibc resolves every name under its own locale
directory, so a generated one is reachable only through that variable in the
process environment — which a test cannot set without the ambient mutation
`[dec:nsh:no-ambient-state]` refuses. Tests that need the generated
`en_US.ISO-8859-1` therefore fail, with that command, rather than skipping:
they compare a single-byte charmap against UTF-8, so without it there is
nothing for them to measure and passing would mean nothing.

The pinned survey suites have native runners:

```sh
scripts/sandboxed -- target/release/nsh-survey run-smoosh
scripts/sandboxed -- target/release/nsh-survey run-oils \
    --group bash-comparison --expect-shell bash
```

One case runs the same way. The runner selects it by the id the baseline and
the gate use, so there is never a reason to start a shell on a case by hand:

```sh
scripts/sandboxed -- target/release/nsh-survey run-oils \
    --group bash-comparison --expect-shell bash \
    --case process-sub.test.sh:2 \
    --shell target/release/nsh --verbose
```

A case started by hand has none of this: no PID namespace, no read-only root
and, above all, no budget. On 2026-08-31 four such shells spun at 98% CPU for
forty-seven hours after the worktree, the binary and the case files they came
from had all been deleted. `scripts/sandboxed` now refuses to run while one
of them is on the machine, because nothing else can see them — inside the
boundary the process table is the sandbox's own. Orphaned is not abandoned:
a process qualifies only once it has outlived the wrapper's own budget
(`NSH_TEST_ABANDONED_AFTER`, defaulting to `--timeout` rounded up to a whole
second), so a probe somebody is watching is left alone.
`NSH_TEST_ABANDONED=kill` clears them and continues; `=ignore` skips the
check. `tests/harness/abandoned-selftest.sh` is its self-test.

The wrapper also asks whether the machine has room, because **a full
filesystem does not report itself as one**. It reports, for every link running
at that moment:

```text
collect2: fatal error: ld terminated with signal 7 [Bus error], core dumped
PLEASE submit a bug report to https://github.com/llvm/llvm-project/issues/
 #4 llvm::StringTableBuilder::write(unsigned char*) const
 #7 lld::elf::MergeNoTailSection::writeTo(unsigned char*)
```

SIGBUS under `writeTo` is the linker writing into an mmap'd output file it can
no longer extend, and the stack trace and the invitation to file an upstream
bug are both beside the point. The same cause also appears as a `rustc` ICE
carrying no message at all, and — only sometimes — as `cargo`'s own
`No space left on device`. On 2026-09-02 all three came out of one afternoon,
and an hour went into the first of them before anybody ran `df`. **Run `df`
before believing that the compiler or the linker crashed.**

The default threshold is 2 GiB free on each filesystem the run may write to —
`target/` always, plus anything `--writable` named, asked once per filesystem
rather than once per path. It is not a claim that 2 GiB is enough to build:
measured here, a from-clean `cargo test --workspace` costs 2.79 GiB, and up to
1.50 GiB of linker output is in flight at once (32 parallel jobs against a
largest link output of 48.1 MiB). It is the mark below which a failure stops
naming its own cause — the recorded ENOSPC happened at 1.6 GiB free and the
bus error above at 670 MiB. Above the mark a build may still not fit, and
`cargo` will say so in those words. `NSH_TEST_DISK=warn` prints the verdict
and runs anyway, `=ignore` skips the check, and `NSH_TEST_DISK_MIN` sets the
threshold in mebibytes. `tests/harness/disk-selftest.sh` is its self-test, and
unlike the other two it may be run through the wrapper.

**Cargo carries that question too, and it has to, because the wrapper only
answers it when somebody types the wrapper.** The measuring commands in this
file and elsewhere are spelled bare — `cargo build --release -p nsh-cli` — and
the runner entry cannot stand in, since cargo calls a runner to *run* a binary,
which is after every link has already happened. So `.cargo/config.toml` also
names `scripts/room-to-build` as `build.rustc-wrapper`, and every crate cargo
compiles is asked whether its output directory has room first. It reads
`NSH_TEST_DISK` and `NSH_TEST_DISK_MIN` from the same place the wrapper does,
so the two can never disagree; `warn` builds here rather than warning, because
the check runs once per crate and a warning would arrive forty-three times for
a build that was going to succeed. An invocation with no `--out-dir` — cargo's
startup `rustc -vV` — and a directory it cannot `statfs` are both let through:
neither is evidence of a full disk, and a build stopped by a broken check is
worse than the failure this exists to name.

It costs nothing to add and nothing to remove: cargo does not put the wrapper
in a unit's fingerprint, so switching it on recompiled nothing. What it costs
per build is 2.29 ms per rustc, and rustc runs once per crate rather than once
per file — measured at load 21-28, 43 invocations and 98 ms against a 20.9 s
from-clean `cargo build --release -p nsh-cli`, and 150 invocations and 344 ms
against a 33.7 s from-clean `cargo test --workspace --no-run` whose output is
2.9 GiB. `tests/harness/room-to-build-selftest.sh` is its self-test; its last
two cases run a bare `cargo build` against a throwaway package, because
"it reaches a build nobody wrapped" is the claim and nothing else tests it.

A refusal leaves you with a number and nowhere to go, so `scripts/disk-headroom`
answers the other half: where the space went, and which of it is safe to take
back.

```sh
scripts/disk-headroom              # report
scripts/disk-headroom --reclaim    # remove what it has shown to be safe
```

It looks only at checkouts `git worktree list` names — other projects live on
this machine and their build output is not ours — and it offers a `target/`
only when the worktree is clean and nothing is building into it. "Building into
it" is two tests: a **build tool** whose command line names the directory
(`rustc`, `cargo`, `ld`, `cc` and friends — naming the path is not using it,
or the session that typed the question would keep hiding the answer), and a
recent write inside it. The window is as long as a wrong answer is expensive:
an hour for a worktree's whole `target/`, whose loss is a full rebuild, and ten
minutes for an incremental cache, whose loss is only some incrementality —
`NSH_DISK_RECENT_MINUTES` and `NSH_DISK_CACHE_RECENT_MINUTES` set them. The
shared checkout's `target/` is reported but never offered whole: only its
caches are pure cache, and the rest is a rebuild somebody else would pay for.

Budget for this. The practice that makes measurements here trustworthy — a
detached worktree with its own `CARGO_TARGET_DIR`, because the shared checkout
is transiently unbuildable from another session's in-flight files — is also
what fills the disk, at up to 2.79 GiB a time with six to eight worktrees live
at once. Remove a worktree when its node is done, and note that a checkout
under `/tmp` is spending RAM rather than disk, which `disk-headroom` says out
loud. `tests/harness/disk-headroom-selftest.sh` is its self-test; every case in
it is about something the tool must refuse to delete.

Two ways that practice used to come back red for reasons that were not the
change are now the harness's problem rather than the reader's. **The pinned
Bash is a build artefact, so a fresh worktree has none of its own.** The
differential tests and the survey gate ask git which checkout the repository
shares — `git rev-parse --git-common-dir` — and use the reference built there,
after the worktree's own `target/` and after `NSH_FUZZ_BASH`, which still names
one outright and is still the way to point a run at a particular build. When no
checkout has one the run fails and names every place it looked, because "could
not measure" is a result and never a pass. **And `CARGO_TARGET_DIR` may not be
under `/tmp`**: the boundary replaces `/tmp` with an empty tmpfs and binds back
only the directory holding the program cargo handed it, so
`<profile>/deps/<test binary>` runs while the `CARGO_BIN_EXE_nsh` beside it
does not exist — `No such file or directory (os error 2)` for a binary that is
present, executable and runs from a prompt. `scripts/sandboxed` now refuses
that layout by name instead; `tests/harness/tmp-build-tree-selftest.sh` is its
self-test.

The budget itself is spent inside the boundary: `timeout` stands in front of
the command *within* the namespace, with a wider one outside as a backstop for
a sandbox that never got as far as running anything. Stopping a command by
signalling its sandbox from outside only works once the sandbox has finished
setting up, and `--timeout`/`NSH_TEST_TIMEOUT` are exactly how a focused run
asks for a budget short enough to land in that window — measured against the
old shape at load 21, a five-millisecond budget left a descendant running 17
times in 20. `--timeout` therefore takes a fractional number of seconds, so a
focused run can ask for a case-sized budget without also giving the sandbox
that long to start. Zero keeps GNU `timeout`'s own meaning — no limit, which
is what `fuzz/run.sh` passes to run until interrupted — and no longer drags
the abandoned-process threshold down with it.
`tests/harness/budget-selftest.sh` is the self-test for that.

The abandoned-process and budget self-tests may not be run through the
wrapper: both ask the host what survived a finished command, which is what the
boundary hides. The free-space, build-tree and room-to-build ones ask nothing
the boundary hides, and run either way.

## Repository layout

```text
crates/nsh/           shell library
crates/nsh-cli/       command-line frontend
crates/nsh-platform/  safe wrappers around syscalls, locale, and terminal APIs
crates/nsh-survey/    survey import and execution tools
posix/                POSIX.1-2024 rule corpus and conformance harness
tests/                differential tests, reference lock, and survey suites
docs/                 API design, specifications, and divergence records
plan/                 nplan work breakdown and decision records
scripts/sandboxed     test containment wrapper
scripts/disk-headroom where the disk went and what is safe to reclaim
```

The C oracle is fetched at its pinned upstream tag into ignored test build
state and receives two small, hash-locked compatibility patches; the full C
tree is not vendored in the repository. Rust-specific behavior and API
contracts live under `docs/spec/nsh/`; current project state is kept under
`plan/`.

## License

Licensed under the [BSD 3-Clause License](LICENSE).

`nsh` began as a Rust port of
[dash](https://git.kernel.org/pub/scm/utils/dash/dash.git/). The inherited dash
work is by Herbert Xu and Christos Zoulas and derives from software Kenneth
Almquist contributed to the University of California, Berkeley. The complete
copyright and attribution notices are in [LICENSE](LICENSE).

Vendored POSIX text and survey corpora retain their respective licenses.
