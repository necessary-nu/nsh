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

for corpus in "$ROOT"/tests/corpus/*.txt; do
	name=$(basename "$corpus" .txt)
	printf '\r%-40s' "$name" >&2
	line=$(FAILOUT=$OUT/$name.out FLAKYOUT=$OUT/$name.flaky \
		XFAILOUT=$OUT/$name.xfail \
		"$HERE/dsdiff.sh" "$corpus" "$JOBS" 2>&1 | grep -m1 '^PASS=')
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
[ "$bad" -gt 0 ] && echo "$bad corpora produced no tally — treat this run as invalid"
exit 0
