#!/usr/bin/env bash
# Build and run one fuzz target under process-tree containment.
#
# Containment is not optional even though the parse target runs under
# `noexec`. The whole point of a fuzzer is to reach states nobody
# predicted, and "noexec cannot execute anything" is one of the things it
# is looking for a counterexample to. Everything here therefore goes
# through scripts/sandboxed, which is the same rule the rest of the test
# tree follows.
#
#   fuzz/run.sh parse              # run until interrupted
#   fuzz/run.sh parse 300          # run for 300 seconds
#   fuzz/run.sh parse 300 -jobs=4  # extra libFuzzer flags after the time
set -euo pipefail

target=${1:?usage: fuzz/run.sh TARGET [SECONDS] [libfuzzer flags...]}
seconds=${2:-0}
shift $(( $# > 1 ? 2 : 1 ))

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
corpus="$root/fuzz/corpus/$target"
mkdir -p "$corpus" "$root/fuzz/artifacts/$target"

# Seed from the shell text the repository already has. Real scripts are
# far better starting points than random bytes: they reach constructs a
# generator would take a very long time to stumble into, and the corpora
# below are already vendored and licence-cleared.
if [ -z "$(ls -A "$corpus" 2>/dev/null)" ]; then
    n=0
    for source in "$root"/tests/surveys/smoosh/shell/*.test \
                  "$root"/tests/surveys/oils/spec/*.test.sh; do
        [ -f "$source" ] || continue
        cp "$source" "$corpus/seed-$n" 2>/dev/null || true
        n=$((n + 1))
    done
    echo "seeded $n corpus entries into fuzz/corpus/$target"
fi

# The sandbox gets a wall clock of its own so an unattended run cannot
# outlive its budget; `0` there means "no limit", which is what an
# open-ended session wants.
if [ "$seconds" -gt 0 ]; then wall=$(( seconds + 120 )); else wall=0; fi

"$root/scripts/sandboxed" --timeout "$wall" \
    --writable "$root/fuzz" \
    -- cargo +nightly fuzz run "$target" "$corpus" -- \
        -max_total_time="$seconds" \
        -rss_limit_mb=4096 \
        -max_len=65536 \
        "$@"
