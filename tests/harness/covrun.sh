#!/bin/bash
# Run a corpus against the instrumented C reference to measure which of
# dash's own functions the corpus actually reaches.
#
#   covrun.sh CORPUS [JOBS]
#
# This answers the question the differential pass cannot: a green
# "PASS=6964" says the port agreed with the C on 6964 inputs, but says
# nothing about how much of dash those inputs touched. gcov on the C
# side gives a number neither the port nor the spec can influence.
#
# Only the reference runs here -- there is nothing to diff, we are just
# collecting counters.
ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
set -u

HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/sandboxed.sh"

COVDIR=${COVDIR:-$ROOT/tests/.build/cov}
COV=$COVDIR/src/dash
CORPUS=${1:?usage: covrun.sh CORPUS [JOBS]}
JOBS=${2:-4}

[ -x "$COV" ] || { echo "no instrumented binary at $COV" >&2; exit 2; }
ds_assert_contained || exit 3

# Liveness must be probed through the *coverage* sandbox: the plain one
# does not bind COVDIR writable, so libgcov cannot write its .gcda files
# and prints "Cannot open" all over the probe's output.
cov_probe() {
	local probe out rc
	probe=$(mktemp -d "${TMPDIR:-/tmp}/dscovlive.XXXXXX") || return 1
	out=$(timeout 20 "$DS_SANDBOX" --quiet \
		--unshare all --die-with-parent --new-session \
		--bind /:/:ro --dev /dev --proc /proc \
		--bind "$probe:$probe" --bind "$COVDIR:$COVDIR" \
		--chdir "$probe" --limit nproc=64 \
		-- timeout 10 "$COV" -c 'echo __CANARY__' 2>&1)
	rc=$?
	rm -rf "$probe"
	if [ "$out" != "__CANARY__" ] || [ $rc -ne 0 ]; then
		echo "HARNESS DEAD: instrumented binary did not run cleanly." >&2
		echo "  rc=$rc output=[$out]" >&2
		return 1
	fi
	return 0
}
cov_probe || exit 4

# The .gcda counters live next to the objects, so the coverage directory
# must be writable inside the sandbox even though the root is read-only.
export DS_COVDIR=$COVDIR

RUNROOT=$(mktemp -d "${TMPDIR:-/tmp}/dscov.XXXXXXXX")
trap 'rm -rf "$RUNROOT"' EXIT
mkdir -p "$RUNROOT/cases"

LINTED=$RUNROOT/corpus.linted
"$HERE/corpus-lint.sh" "$CORPUS" > "$LINTED" 2> "$RUNROOT/lint.log"
tail -1 "$RUNROOT/lint.log"

if grep -qx '%%%' "$LINTED"; then
	awk -v dir="$RUNROOT/cases" '
		BEGIN { n = 0; f = sprintf("%s/%06d", dir, n) }
		/^%%%$/ { n++; f = sprintf("%s/%06d", dir, n); next }
		{ print > f }
	' "$LINTED"
else
	awk -v dir="$RUNROOT/cases" '
		/^[[:space:]]*$/ { next }
		/^[[:space:]]*#/ { next }
		{ printf "%s\n", $0 > sprintf("%s/%06d", dir, ++n) }
	' "$LINTED"
fi
for f in "$RUNROOT"/cases/*; do [ -s "$f" ] || rm -f "$f"; done

echo "reset counters"
find "$COVDIR" -name '*.gcda' -delete

n=$(ls "$RUNROOT/cases" | wc -l)
echo "running $n cases against the instrumented reference (jobs=$JOBS)"

find "$RUNROOT/cases" -type f -print0 |
	COV=$COV RUNROOT=$RUNROOT COVDIR=$COVDIR \
	xargs -0 -P "$JOBS" -n 1 "$HERE/covcase.sh"

echo "done; gcda files: $(find "$COVDIR" -name '*.gcda' | wc -l)"
