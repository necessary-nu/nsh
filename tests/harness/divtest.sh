#!/bin/bash
# Does the sanctioned-divergence register actually refuse a regression?
#
#   tests/harness/divtest.sh
#
# The register exists to stop a deliberate divergence from spending
# FAIL=0. Its whole risk is the opposite failure: an entry written too
# permissively, which then swallows a real regression and leaves the
# harness reporting green on a broken shell. That is worse than having no
# register at all, because it is silent.
#
# So the register is tested the way it will fail, not the way it will
# work. Every case below is a difference an entry must NOT excuse.
set -u
HERE=$(cd "$(dirname "$0")" && pwd)
. "$HERE/divergences.sh"

pass=0 fail=0
ok() { pass=$((pass + 1)); }
no() {
	fail=$((fail + 1))
	echo "FAIL: $1" >&2
}
check() { # check DESC EXPECT(0|1) ARGS...
	local desc=$1 expect=$2
	shift 2
	if ds_sanctioned "$@"; then local got=0; else local got=1; fi
	if [ "$got" = "$expect" ]; then ok; else
		no "$desc (expected $expect, got $got${DS_DIVERGENCE:+, matched $DS_DIVERGENCE})"
	fi
}

case_file=$(mktemp) || exit 1
trap 'rm -f "$case_file"' EXIT

# ---- an empty register excuses nothing ------------------------------
printf 'export A=1 B=2; env\n' > "$case_file"
check "empty register excuses nothing" 1 "a" "b" 0 0 "$case_file"

# ---- a representative entry, exercised against its own failure modes -
#
# This is the shape the `env` ordering entry will take when the BTreeMap
# change lands. Registering it here rather than in divergences.sh keeps
# the live register honest -- it stays empty until the behaviour exists
# -- while still testing the thing that will go in it.
DS_DIVERGENCES=(sample_ordering)
dsdiv_sample_ordering() {
	[ "$3" = "$4" ] || return 1
	ds_case_matches "$5" '(^|[;&|( ])(env|export -p|set|alias)([ ;|]|$)' || return 1
	ds_same_lines "$1" "$2"
}

A=$'AA=1\nFF=6\nBB=2'
SORTED=$'AA=1\nBB=2\nFF=6'

check "the divergence it was written for" 0 "$A" "$SORTED" 0 0 "$case_file"

# A line that changed content is a regression, not a reordering.
check "a changed value is not excused" 1 "$A" $'AA=9\nBB=2\nFF=6' 0 0 "$case_file"

# A line that vanished is a regression even though the rest reordered.
check "a dropped line is not excused" 1 "$A" $'AA=1\nBB=2' 0 0 "$case_file"

# A line that appeared is a regression.
check "an extra line is not excused" 1 "$A" $'AA=1\nBB=2\nFF=6\nZZ=9' 0 0 "$case_file"

# A duplicate is a regression: the multiset differs even though the set
# does not, which is exactly what `sort -u` would have missed.
check "a duplicated line is not excused" 1 $'A\nB' $'A\nB\nB' 0 0 "$case_file"

# Same output, different exit status.
check "a differing exit status is not excused" 1 "$A" "$SORTED" 0 1 "$case_file"

# ---- scoping: the entry may not reach outside its feature -----------
printf 'echo one; echo two\n' > "$case_file"
check "a case that never runs the builtin is not excused" 1 \
	$'one\ntwo' $'two\none' 0 0 "$case_file"

# A case whose text merely mentions the word must not qualify either.
printf 'echo "the environment"\n' > "$case_file"
check "a mention in a string is not excused" 1 \
	$'one\ntwo' $'two\none' 0 0 "$case_file"

# ---- the dead-harness guard --------------------------------------
#
# A shell that stopped existing is not a shell that behaved differently.
missing=$(mktemp -u)
if ds_harness_alive "$missing" /bin/sh; then no "a missing port is not alive"; else ok; fi
if ds_harness_alive /bin/sh "$missing"; then no "a missing reference is not alive"; else ok; fi
if ds_harness_alive /bin/sh /bin/sh; then ok; else no "two real shells are alive"; fi
# A file that exists but cannot be executed is just as dead.
notexec=$(mktemp); printf '#!/bin/sh\n' > "$notexec"; chmod -x "$notexec"
if ds_harness_alive "$notexec" /bin/sh; then no "a non-executable port is not alive"; else ok; fi
rm -f "$notexec"

echo "DIVERGENCE REGISTER: PASS=$pass FAIL=$fail"
[ "$fail" -eq 0 ]
