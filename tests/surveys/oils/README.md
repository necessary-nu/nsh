# Oils shell-spec survey

This directory is the offline, reviewable input boundary for the Oils
shell-spec survey. Oils is a differential compatibility survey; it is not the
normative POSIX oracle. The normative rules and their harness remain under
`posix/`.

`SOURCE.toml` identifies the exact upstream commit and tree. Ordinary builds
and test runs MUST consume files committed below this directory and MUST NOT
fetch, clone, or otherwise depend on the network. A changed commit, tree,
license, file selection, or case count is a reviewed source update rather than
an incidental test result.

## Updating the pin

1. Fetch the proposed commit into a disposable checkout of the repository in
   `SOURCE.toml` and detach at that commit. Never update from a moving branch.
2. Verify that `git rev-parse HEAD` and `git rev-parse HEAD^{tree}` equal the
   proposed lock values.
3. Review `LICENSE.txt`, `spec/README.md`, `test/sh_spec.py`,
   `test/spec-runner.sh`, and `test/spec-compat.sh` for license, format,
   selection, and execution changes.
4. Run the repository importer against that local checkout. The importer must
   regenerate the corpus, fixtures, hashes, and manifests without network
   access and must reject a checkout whose commit or tree differs from the
   lock.
5. Review the complete generated diff. Count changes are expected to fail the
   verification command until the values in `SOURCE.toml` and the generated
   manifests are consciously updated together.
6. Run the survey runner's parser tests, negative controls, and every imported
   manifest before committing the new pin.

The initial pin is Oils commit
`15de8fd779569e6e3a9f5fcbfc00e7df0ebe0380`, whose metadata reports 222 total
spec files with 3,964 cases and 134 active OSH files with 2,755 cases.

## Generated survey manifest

`MANIFEST.toml` is generated from the imported file metadata and
`FILES.sha256`; do not edit it by hand. Regenerate and verify it with:

```text
cargo run -p nsh-survey -- generate-oils-manifests
cargo run -p nsh-survey -- verify-oils
```

The manifest exposes five stable selections:

- `full`: all 134 active OSH files and 2,755 cases;
- `posix-candidate`: the 76 files and 1,614 cases whose `compare_shells`
  metadata contains Dash;
- `bash-comparison`: the 130 files and 2,735 cases whose metadata contains a
  Bash token;
- `bash-extension`: the 54 Bash-selected files and 1,121 cases not selected
  for Dash; and
- `bash-named-diagnostic`: the seven `*-bash` files and 112 cases.

The Dash-selected set is a differential POSIX candidate survey, not a claim
that each case is normative POSIX. Normative conformance remains under
`posix/`. Each manifest entry records its complete source hash and qualified
assertion count; the native runner reads the exact per-case shell qualifiers
from those hashed source files.
