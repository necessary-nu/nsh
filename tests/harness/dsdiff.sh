#!/bin/bash
# Differential harness: the Rust port vs a C dash built from this same tree.
#
#   dsdiff.sh CORPUS [JOBS]
#
# Corpus formats, auto-detected:
#   * multi-line — cases separated by lines that are exactly `%%%`
#   * one-per-line — every non-empty, non-`#` line is its own case
#
# Per-case directives, on the leading lines of a case:
#   #!mode=c        run as `sh -c BODY`                      (default)
#   #!mode=file     write BODY to ./script.sh, run `sh ./script.sh`
#   #!mode=stdin    pipe BODY into `sh` on stdin
#   #!args a b c    extra argv words after the script / -c body
#   #!norm=pid      also normalise runs of 3+ digits (for $$, job pids)
#   #!name some text   label for the failure report
#
# Every case runs in its own scratch directory and every invocation of
# this script uses its own root, so concurrent runs never collide — that
# is what broke the previous shared-directory harness.
#
# Every shell-under-test runs inside a PID namespace (see sandboxed.sh).
# This script aborts rather than running a single case if that namespace
# cannot be established.
ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
set -u

. "$(cd "$(dirname "$0")" && pwd)/sandboxed.sh"
. "$(cd "$(dirname "$0")" && pwd)/divergences.sh"

PORT=${PORT:-$ROOT/target/debug/nsh}
REF=${REF:-$ROOT/tests/.build/ref/src/dash}
CORPUS=${1:?usage: dsdiff.sh CORPUS [JOBS]}
JOBS=${2:-8}

[ -x "$PORT" ] || { echo "no port binary at $PORT" >&2; exit 2; }
[ -x "$REF" ]  || { echo "no reference binary at $REF" >&2; exit 2; }

HERE=$(cd "$(dirname "$0")" && pwd)

# Containment first. No fallback: a harness that quietly degrades to
# "no sandbox" is exactly how the 2026-08-02 incident happened.
ds_assert_contained || exit 3

# ...and liveness. PASS counts mean nothing unless both binaries are
# actually executing through this exact code path.
ds_assert_harness_live "$REF" "$PORT" || exit 4

RUNROOT=$(mktemp -d "${TMPDIR:-/tmp}/dsdiff.XXXXXXXX")
trap 'rm -rf "$RUNROOT"' EXIT
mkdir -p "$RUNROOT/cases" "$RUNROOT/out"

# Second layer: drop signal-delivery cases before they ever run.
LINTED=$RUNROOT/corpus.linted
"$HERE/corpus-lint.sh" "$CORPUS" > "$LINTED" 2> "$RUNROOT/lint.log"
tail -1 "$RUNROOT/lint.log"
grep '^DROP' "$RUNROOT/lint.log" | head -20
CORPUS=$LINTED

# Split the corpus into one file per case.
if grep -qx '%%%' "$CORPUS"; then
	awk -v dir="$RUNROOT/cases" '
		BEGIN { n = 0; f = sprintf("%s/%06d", dir, n) }
		/^%%%$/ { n++; f = sprintf("%s/%06d", dir, n); next }
		{ print > f }
	' "$CORPUS"
else
	awk -v dir="$RUNROOT/cases" '
		/^[[:space:]]*$/ { next }
		/^[[:space:]]*#/ { next }
		{ printf "%s\n", $0 > sprintf("%s/%06d", dir, ++n) }
	' "$CORPUS"
fi

# Drop cases that ended up empty (trailing separators, blank blocks).
for f in "$RUNROOT"/cases/*; do
	[ -s "$f" ] || rm -f "$f"
done

find "$RUNROOT/cases" -type f -print0 |
	PORT=$PORT REF=$REF RUNROOT=$RUNROOT xargs -0 -P "$JOBS" -n 1 "$HERE/dscase.sh"

# The sidecars are reports, not verdicts: the verdict for a case is
# always its bare `out/<id>` file. Every new sidecar suffix has to be
# excluded here or it is counted as a case in its own right -- an
# `.xfail` report read as a failure is how the register would have
# quietly inverted its own meaning.
pass=$(find "$RUNROOT/out" -maxdepth 1 -type f ! -name '*.flaky' ! -name '*.xfail' ! -name '*.dead' -exec grep -lx PASS {} + 2>/dev/null | wc -l)
fail=$(find "$RUNROOT/out" -maxdepth 1 -type f ! -name '*.flaky' ! -name '*.xfail' ! -name '*.dead' -exec grep -Lx PASS {} + 2>/dev/null | wc -l)

# Reports go under tests/.build (gitignored), not the caller's cwd —
# this script now lives in the repo, and a test run must not dirty it.
mkdir -p "$ROOT/tests/.build"
: > "${FAILOUT:=$ROOT/tests/.build/failures.out}"
for f in $(find "$RUNROOT/out" -maxdepth 1 -type f ! -name '*.flaky' ! -name '*.xfail' ! -name '*.dead' -exec grep -Lx PASS {} + 2>/dev/null | sort); do
	cat "$f" >> "$FAILOUT"
done

: > "${FLAKYOUT:=$ROOT/tests/.build/flaky.out}"
nflaky=0
for f in "$RUNROOT"/out/*.flaky; do
	[ -e "$f" ] || continue
	cat "$f" >> "$FLAKYOUT"
	nflaky=$((nflaky + 1))
done

# A binary that vanished mid-corpus makes every case after it look like a
# divergence. Refuse to report a tally rather than report a wrong one:
# runall.sh counts a missing tally as a corpus that invalidates the run,
# which is the honest outcome.
ndead=$(find "$RUNROOT/out" -maxdepth 1 -name '*.dead' 2>/dev/null | wc -l)
if [ "$ndead" -gt 0 ]; then
	echo "HARNESS DEAD: $ndead case(s) ran without a shell to run." >&2
	cat "$RUNROOT"/out/*.dead >&2
	echo "Refusing to report a tally for this corpus: the run is void." >&2
	exit 2
fi

: > "${XFAILOUT:=$ROOT/tests/.build/xfail.out}"
nxfail=0
declare -A seen_div=()
for f in "$RUNROOT"/out/*.xfail; do
	[ -e "$f" ] || continue
	cat "$f" >> "$XFAILOUT"
	nxfail=$((nxfail + 1))
	id=$(sed -n '1s/^### XFAIL(\([^)]*\)).*/\1/p' "$f")
	[ -n "$id" ] && seen_div[$id]=1
done

echo "PASS=$pass FAIL=$fail FLAKY=$nflaky XFAIL=$nxfail"
[ "$nflaky" -gt 0 ] && echo "(flaky cases counted as passing; detail in $FLAKYOUT)"
[ "$nxfail" -gt 0 ] && echo "(sanctioned divergences, counted as passing; detail in $XFAILOUT)"

# An entry nothing matches is an excuse for a difference the shell no
# longer produces. Say so: a stale register is how a real regression
# eventually gets waved through.
for id in "${DS_DIVERGENCES[@]:-}"; do
	[ -n "$id" ] || continue
	[ -n "${seen_div[$id]:-}" ] || echo "(register entry '$id' matched nothing in this corpus)"
done

[ "$fail" -gt 0 ] && { echo "--- failures (also in $FAILOUT) ---"; head -c 200000 "$FAILOUT"; }
exit 0
