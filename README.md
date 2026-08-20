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

Bootstrap the C reference, then run builds and tests through
`scripts/sandboxed`. The bootstrap downloads the pinned official Dash archive,
verifies its SHA-256 and the two documented oracle patches, and builds it
inside the same containment boundary:

```sh
./tests/build-reference.sh
scripts/sandboxed -- cargo build

scripts/sandboxed -- cargo test --workspace
scripts/sandboxed --writable tests/.build -- tests/harness/runall.sh 12
scripts/sandboxed --writable posix -- \
    python3 posix/harness/run.py --shell target/debug/nsh
```

The wrapper requires the `sandbox` tool and fails closed if containment cannot
be established. Read [tests/README.md](tests/README.md) before adding or
changing differential cases.

The pinned survey suites have native runners:

```sh
scripts/sandboxed -- target/release/nsh-survey run-smoosh
scripts/sandboxed -- target/release/nsh-survey run-oils \
    --group bash-comparison --expect-shell bash
```

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
