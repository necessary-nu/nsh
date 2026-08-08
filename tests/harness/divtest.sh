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
#
# The live register is not empty any more, so empty it for this one check
# and put it back before the section that tests what is in it.
REAL_DIVERGENCES=("${DS_DIVERGENCES[@]}")
DS_DIVERGENCES=()
printf 'export A=1 B=2; env\n' > "$case_file"
check "an empty register excuses nothing" 1 "a" "b" 0 0 "$case_file"

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

# ---- the live register: sorted_tables -------------------------------
#
# The entry that excuses `env`, `printenv` and `alias` printing in name
# order. Same discipline as above: one check that it matches the thing it
# was written for, and then every way it could be too permissive.
DS_DIVERGENCES=("${REAL_DIVERGENCES[@]}")

printf "export AA=1 BB=2 FF=6; env\n" > "$case_file"
BUCKET=$'AA=1\nFF=6\nBB=2'
SORTED=$'AA=1\nBB=2\nFF=6'

check "sorted_tables: the divergence it was written for" 0 \
	"$BUCKET" "$SORTED" 0 0 "$case_file"

# A value that changed is a regression however the lines are arranged.
check "sorted_tables: a changed variable value is not excused" 1 \
	"$BUCKET" $'AA=9\nBB=2\nFF=6' 0 0 "$case_file"

# A variable that vanished is a regression even though the rest sorted.
check "sorted_tables: a missing variable is not excused" 1 \
	"$BUCKET" $'AA=1\nBB=2' 0 0 "$case_file"

# One that appeared is a regression too.
check "sorted_tables: an extra variable is not excused" 1 \
	"$BUCKET" $'AA=1\nBB=2\nFF=6\nZZ=9' 0 0 "$case_file"

# And one that appeared twice: the set is unchanged, the multiset is not.
check "sorted_tables: a duplicated variable is not excused" 1 \
	$'AA=1\nBB=2' $'AA=1\nAA=1' 0 0 "$case_file"

# The same environment, a different exit status.
check "sorted_tables: a differing exit status is not excused" 1 \
	"$BUCKET" "$SORTED" 0 1 "$case_file"

# The permutation has to be the *sorted* one. This is the check that stops
# the entry from excusing any environment order the port might ever
# produce, which is what "the same lines in a different order" alone would
# have done.
check "sorted_tables: an unsorted permutation is not excused" 1 \
	"$BUCKET" $'BB=2\nAA=1\nFF=6' 0 0 "$case_file"

# Lines that are not environment entries may not move, so a diagnostic
# arriving on the other side of the output is not this divergence -- it is
# either a flush-order regression or scheduling, and the classifier that
# already exists gets to say which.
check "sorted_tables: a reordered diagnostic is not excused" 1 \
	$'AA=1\nSH: 1: x: not found' $'SH: 1: x: not found\nAA=1' 0 0 "$case_file"

# ---- scoping: only the commands that actually diverge ----------------
printf 'echo one; echo two\n' > "$case_file"
check "sorted_tables: a case that runs none of the commands is not excused" 1 \
	$'BB=2\nAA=1' $'AA=1\nBB=2' 0 0 "$case_file"

# `set` and `export -p` print variables, and dash already sorts them in
# `showvars`. Naming them in the entry would let it excuse a permutation
# that could only be a regression, so it does not.
printf 'AA=1 BB=2; set\n' > "$case_file"
check "sorted_tables: a bare set is outside the entry" 1 \
	$'BB=2\nAA=1' $'AA=1\nBB=2' 0 0 "$case_file"
printf 'export AA=1 BB=2; export -p\n' > "$case_file"
check "sorted_tables: export -p is outside the entry" 1 \
	$'BB=2\nAA=1' $'AA=1\nBB=2' 0 0 "$case_file"

# A case that only mentions the word does not run the command.
printf 'echo "the environment"\n' > "$case_file"
check "sorted_tables: a mention in a string is not excused" 1 \
	$'BB=2\nAA=1' $'AA=1\nBB=2' 0 0 "$case_file"

# ---- alias listings, the other half of the entry --------------------
#
# `printalias` runs the whole `name=value` through `single_quote`, so an
# alias listing is an environment entry inside a pair of quotes.
printf "alias AA=1 FF=6 BB=2; alias\n" > "$case_file"
A_BUCKET="'AA=1'
'FF=6'
'BB=2'"
A_SORTED="'AA=1'
'BB=2'
'FF=6'"
check "sorted_tables: alias listings sort too" 0 \
	"$A_BUCKET" "$A_SORTED" 0 0 "$case_file"
check "sorted_tables: a changed alias value is not excused" 1 \
	"$A_BUCKET" "'AA=1'
'BB=2'
'FF=9'" 0 0 "$case_file"

# ---- the entry's documented limit -----------------------------------
#
# Two environments printed back to back with nothing between them read as
# one block, so the entry refuses them and the case is reported as a
# failure rather than an XFAIL. Nothing in tests/corpus does this. Pinned
# here so that loosening the entry has to be a decision rather than a
# side effect -- if this ever starts matching, both this test and the
# comment in divergences.sh need rewriting together.
printf 'env; env\n' > "$case_file"
check "sorted_tables: two adjacent environments are refused (known limit)" 1 \
	$'AA=1\nFF=6\nBB=2\nAA=1\nFF=6\nBB=2' \
	$'AA=1\nBB=2\nFF=6\nAA=1\nBB=2\nFF=6' 0 0 "$case_file"

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
