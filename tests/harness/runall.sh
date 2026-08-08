#!/bin/bash
# Run every corpus in tests/corpus through dsdiff.sh and report a
# per-corpus tally, so a whole-tree parity check is one command.
#
#   runall.sh [JOBS]
#
# Per-corpus failure detail lands in tests/.build/fail/<corpus>.out;
# only corpora with failures leave a file behind.
ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
HERE=$(cd "$(dirname "$0")" && pwd)
JOBS=${1:-8}

# For DS_DIVERGENCES, so a registered entry that no corpus triggers can be
# reported at the end of the run.
. "$HERE/divergences.sh"

OUT=$ROOT/tests/.build/fail
rm -rf "$OUT"
mkdir -p "$OUT"

# Pin once for the whole run, not once per corpus.
#
# `dsdiff.sh` hard-links the two shells so a rebuild cannot move the
# binary out from under a running sweep. That protects one corpus. This
# script invokes it 113 times, so a rebuild between corpora silently
# swaps the binary mid-run and the tally covers two different shells --
# an hour of work that reads as one result and is not. Pinning here and
# exporting the paths makes the whole sweep one shell, and dsdiff's own
# pin of an already-pinned path is a second hard link to the same inode.
RUNPIN=$ROOT/tests/.build/runpin.$$
mkdir -p "$RUNPIN"
trap 'rm -rf "$RUNPIN"' EXIT
for pair in "port:${PORT:-$ROOT/target/debug/nsh}" "ref:${REF:-$ROOT/tests/.build/ref/src/dash}"; do
	src=${pair#*:}
	[ -x "$src" ] || { echo "no ${pair%%:*} binary at $src" >&2; exit 2; }
	ln "$src" "$RUNPIN/${pair%%:*}" 2>/dev/null || cp "$src" "$RUNPIN/${pair%%:*}" || exit 2
done
export PORT=$RUNPIN/port REF=$RUNPIN/ref

total_pass=0 total_fail=0 total_flaky=0 total_xfail=0 bad=0
declare -A seen_any=()

for corpus in "$ROOT"/tests/corpus/*.txt; do
	name=$(basename "$corpus" .txt)
	printf '\r%-40s' "$name" >&2
	out=$(FAILOUT=$OUT/$name.out FLAKYOUT=$OUT/$name.flaky \
		XFAILOUT=$OUT/$name.xfail \
		"$HERE/dsdiff.sh" "$corpus" "$JOBS" 2>&1)
	line=$(printf '%s\n' "$out" | grep -m1 '^PASS=')
	while IFS= read -r id; do
		[ -n "$id" ] && seen_any[${id#XFAILID=}]=1
	done < <(printf '%s\n' "$out" | grep '^XFAILID=')
	case $line in
	PASS=*) ;;
	*) echo "!! $name: harness did not report a tally"; bad=$((bad + 1)); continue ;;
	esac
	XFAIL=0
	eval "$line"
	total_pass=$((total_pass + PASS))
	total_fail=$((total_fail + FAIL))
	total_flaky=$((total_flaky + FLAKY))
	total_xfail=$((total_xfail + XFAIL))
	[ "$FAIL" -eq 0 ] && rm -f "$OUT/$name.out"
	[ "$FLAKY" -eq 0 ] && rm -f "$OUT/$name.flaky"
	[ "$XFAIL" -eq 0 ] && rm -f "$OUT/$name.xfail"
	[ "$FAIL" -gt 0 ] && printf '%-32s %s\n' "$name" "$line"
done

echo "==== TOTAL PASS=$total_pass FAIL=$total_fail FLAKY=$total_flaky XFAIL=$total_xfail ===="
# A registered divergence that no corpus in a whole run triggers is an
# excuse for a difference the shell no longer produces, and a stale excuse
# is how a real regression eventually gets waved through. This is the only
# place the question is answerable: per corpus, an entry not matching is
# the normal case.
for id in "${DS_DIVERGENCES[@]:-}"; do
	[ -n "$id" ] || continue
	[ -n "${seen_any[$id]:-}" ] || echo "STALE: register entry '$id' matched nothing in the whole run"
done

[ "$bad" -gt 0 ] && echo "$bad corpora produced no tally — treat this run as invalid"
exit 0
