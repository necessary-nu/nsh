#!/bin/bash
# Which Rust functions does the differential corpus actually reach?
#
#   covrust.sh [CORPUS...]        default: every corpus in tests/corpus
#
# This is the question a restructuring needs answered and the C-side
# `covrun.sh` cannot answer. Two differences, and both matter:
#
#   * It measures the PORT, not the C. What protects a restructuring of
#     the Rust is whether the corpus executes the Rust being
#     restructured; the C's coverage is a different question, and not one
#     anyone is asking.
#
#   * It survives fork+exec, which is what defeated gcov. gcov keeps
#     counters in the process and loses the parent's on exec, so
#     `covrun.sh` reads `main` at 54.17% with no fork and 39.58% with
#     `/bin/true` -- see tests/README.md. LLVM's instrumentation writes
#     one .profraw per process (%p in LLVM_PROFILE_FILE) and
#     `llvm-profdata merge` puts them back together, so a case that forks
#     a hundred children contributes all hundred.
#
# Read the output as a floor, not a verdict: a function the corpus never
# enters is definitely unguarded, while one it enters is guarded only to
# the depth the cases go.
set -u
ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
HERE=$(cd "$(dirname "$0")" && pwd)

LLVM_BIN=$(rustc --print target-libdir)/../bin
PROFDATA=$LLVM_BIN/llvm-profdata
COV=$LLVM_BIN/llvm-cov
for t in "$PROFDATA" "$COV"; do
	[ -x "$t" ] || {
		echo "missing $t -- rustup component add llvm-tools" >&2
		exit 2
	}
done

OUT=$ROOT/tests/.build/cov
rm -rf "$OUT"
mkdir -p "$OUT/raw"

# A separate target dir: the instrumented binary must not become the one
# the differential harness compares against, and must not force a rebuild
# of the normal one on every switch. `--cfg coverage` turns on the
# profile flush before each `_exit`; without it the run produces nothing,
# because dash never returns from main and LLVM writes from an atexit
# handler that `_exit` skips.
echo "building instrumented port..." >&2
RUSTFLAGS="-C instrument-coverage --cfg coverage" \
	CARGO_TARGET_DIR="$ROOT/tests/.build/cov-target" \
	cargo build --manifest-path "$ROOT/Cargo.toml" >&2 || exit 1
PORT=$ROOT/tests/.build/cov-target/debug/dash

# %p is the pid and %m the binary's signature: one profile per process,
# which is the whole reason this works where gcov does not. The directory
# is bound writable into the sandbox by ds_sandboxed; without that the
# case cannot write it and the merge finds nothing.
export DS_COVDIR="$OUT/raw"
export LLVM_PROFILE_FILE="$OUT/raw/dash-%p-%m.profraw"

corpora=("$@")
[ ${#corpora[@]} -eq 0 ] && corpora=("$ROOT"/tests/corpus/*.txt)

for corpus in "${corpora[@]}"; do
	printf '\r%-44s' "$(basename "$corpus")" >&2
	PORT=$PORT FAILOUT=/dev/null FLAKYOUT=/dev/null \
		"$HERE/dsdiff.sh" "$corpus" "${JOBS:-12}" >/dev/null 2>&1
done
echo >&2

# The sandbox binds the case's own directory writable, so profiles written
# inside it land there rather than in $OUT; collect both.
find "$ROOT/tests/.build" "${TMPDIR:-/tmp}" -name 'dash-*.profraw' -newer "$OUT" \
	-exec mv {} "$OUT/raw/" \; 2>/dev/null

n=$(find "$OUT/raw" -name '*.profraw' | wc -l)
echo "merging $n profiles..." >&2
[ "$n" -eq 0 ] && {
	echo "no profiles were written -- the sandbox may be blocking the" >&2
	echo "profile path; check LLVM_PROFILE_FILE points somewhere the" >&2
	echo "case can write." >&2
	exit 3
}
find "$OUT/raw" -name '*.profraw' -print0 |
	xargs -0 "$PROFDATA" merge -sparse -o "$OUT/dash.profdata" || exit 1

"$COV" report "$PORT" -instr-profile="$OUT/dash.profdata" \
	-ignore-filename-regex='(cargo/registry|rustc)' |
	tee "$OUT/report.txt"

echo
echo "functions never entered by the corpus:"
"$COV" show "$PORT" -instr-profile="$OUT/dash.profdata" \
	-show-instantiation-summary -format=text \
	-ignore-filename-regex='(cargo/registry|rustc)' 2>/dev/null |
	awk '/^ *[0-9]+\| *0\| *(pub )?(unsafe )?fn /{print}' | head -60

echo
echo "full report: $OUT/report.txt"
