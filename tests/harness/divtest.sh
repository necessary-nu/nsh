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

# ---- the live register: alias_stdout_format -------------------------
#
# dash quotes the complete definition; the port follows POSIX's
# `name=quoted-value` format. The entry also carries the alias table's sorting
# difference, because the two differences occur in the same listing.
printf "alias AA=1 FF=6 BB=2; alias\n" > "$case_file"
A_BUCKET="'AA=1'
'FF=6'
'BB=2'"
A_SORTED="AA='1'
BB='2'
FF='6'"
check "alias_stdout_format: quote placement and sorting" 0 \
	"$A_BUCKET" "$A_SORTED" 0 0 "$case_file"

printf "alias AA=1; alias AA\n" > "$case_file"
check "alias_stdout_format: one named alias" 0 \
	"'AA=1'" "AA='1'" 0 0 "$case_file"

check "alias_stdout_format: a changed value is not excused" 1 \
	"$A_BUCKET" "AA='1'
BB='2'
FF='9'" 0 0 "$case_file"
check "alias_stdout_format: a missing alias is not excused" 1 \
	"$A_BUCKET" "AA='1'
BB='2'" 0 0 "$case_file"
check "alias_stdout_format: an extra alias is not excused" 1 \
	"$A_BUCKET" "AA='1'
BB='2'
FF='6'
ZZ='9'" 0 0 "$case_file"
check "alias_stdout_format: a duplicate is not excused" 1 \
	"'AA=1'
'BB=2'" "AA='1'
AA='1'" 0 0 "$case_file"
check "alias_stdout_format: a differing exit status is not excused" 1 \
	"$A_BUCKET" "$A_SORTED" 0 1 "$case_file"
check "alias_stdout_format: the old whole-definition quoting is refused" 1 \
	"$A_BUCKET" "'AA=1'
'BB=2'
'FF=6'" 0 0 "$case_file"
check "alias_stdout_format: an unsorted port listing is refused" 1 \
	"$A_BUCKET" "BB='2'
AA='1'
FF='6'" 0 0 "$case_file"
A_WITH_DIAG="'AA=1'"$'\n'"SH: error"
P_WITH_DIAG="SH: error"$'\n'"AA='1'"
check "alias_stdout_format: a reordered diagnostic is not excused" 1 \
	"$A_WITH_DIAG" "$P_WITH_DIAG" 0 0 "$case_file"

# A definition alone prints nothing. Merely mentioning alias must not let a
# newly printed line borrow the formatting exception.
printf "alias AA=1\n" > "$case_file"
check "alias_stdout_format: definition-only command is outside the entry" 1 \
	"'AA=1'" "AA='1'" 0 0 "$case_file"
printf "alias AA=1; command -v AA\n" > "$case_file"
check "alias_stdout_format: command -v displays the definition" 0 \
	"alias 'AA=1'" "alias AA='1'" 0 0 "$case_file"
check "alias_stdout_format: command -v changed value is not excused" 1 \
	"alias 'AA=1'" "alias AA='2'" 0 0 "$case_file"
printf "alias AA=1 BB=2; alias AA BB\n" > "$case_file"
check "alias_stdout_format: multiple name operands display definitions" 0 \
	$'\047AA=1\047\n\047BB=2\047' $'AA=\0471\047\nBB=\0472\047' 0 0 "$case_file"
printf "alias AA=1; alias 2>&1\n" > "$case_file"
check "alias_stdout_format: a redirected bare listing is display" 0 \
	"'AA=1'" "AA='1'" 0 0 "$case_file"
printf "alias ia='echo IA'; alias\n" > "$case_file"
check "alias_stdout_format: an interactive prompt prefix is preserved" 0 \
	"$ 'ia=echo IA'" "$ ia='echo IA'" 0 0 "$case_file"
check "alias_stdout_format: prompted listings still sort by alias name" 0 \
	$'$ \047bb=2\047\n\047aa=1\047' $'$ aa=\0471\047\nbb=\0472\047' 0 0 "$case_file"
check "alias_stdout_format: changed prompted output is not excused" 1 \
	"$ 'ia=echo IA'" "$ ia='echo OTHER'" 0 0 "$case_file"
printf "echo alias\n" > "$case_file"
check "alias_stdout_format: a mention is outside the entry" 1 \
	"'AA=1'" "AA='1'" 0 0 "$case_file"

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

# ---- the live register: sorted_cmdtable -----------------------------
#
# `hash` prints reconstructed paths, with a trailing `*` after `cd` marks
# an entry for rehashing. The map sorts by the command name (the basename),
# not by the full printed path.
printf 'hash\n' > "$case_file"
H_BUCKET=$'/bin/rm\n/bin/cat\n/bin/mkdir\n/bin/cp\n/bin/ls\n/bin/chmod'
H_SORTED=$'/bin/cat\n/bin/chmod\n/bin/cp\n/bin/ls\n/bin/mkdir\n/bin/rm'

check "sorted_cmdtable: the divergence it was written for" 0 \
	"$H_BUCKET" "$H_SORTED" 0 0 "$case_file"

# The key order is basename order even when PATH resolution gives the full
# lines an unrelated order.
check "sorted_cmdtable: sorting is by command name, not printed path" 0 \
	$'/z/mkdir\n/a/cat\n/z/chmod' $'/a/cat\n/z/chmod\n/z/mkdir' 0 0 "$case_file"

# Rehash markers are part of printentry's line shape but not of the key.
check "sorted_cmdtable: rehash markers are accepted and ignored for sorting" 0 \
	$'/bin/rm*\n/bin/cat*\n/bin/ls*' $'/bin/cat*\n/bin/ls*\n/bin/rm*' 0 0 "$case_file"

# Status and diagnostics may surround a hash listing. They are fixed anchors,
# not part of the command-name block, even when their bytes could be a valid
# bare executable name.
check "sorted_cmdtable: surrounding status text is not part of the block" 0 \
	$'rc=0\n/bin/rm\nWD/g1/gp\n/bin/sed' \
	$'rc=0\nWD/g1/gp\n/bin/rm\n/bin/sed' 0 0 "$case_file"

check "sorted_cmdtable: a changed path is not excused" 1 \
	"$H_BUCKET" $'/usr/bin/cat\n/bin/chmod\n/bin/cp\n/bin/ls\n/bin/mkdir\n/bin/rm' 0 0 "$case_file"
check "sorted_cmdtable: a missing command is not excused" 1 \
	"$H_BUCKET" $'/bin/cat\n/bin/chmod\n/bin/cp\n/bin/ls\n/bin/mkdir' 0 0 "$case_file"
check "sorted_cmdtable: an extra command is not excused" 1 \
	"$H_BUCKET" $'/bin/cat\n/bin/chmod\n/bin/cp\n/bin/echo\n/bin/ls\n/bin/mkdir\n/bin/rm' 0 0 "$case_file"
check "sorted_cmdtable: a duplicate command is not excused" 1 \
	$'/bin/cat\n/bin/ls' $'/bin/cat\n/bin/cat' 0 0 "$case_file"
check "sorted_cmdtable: a differing exit status is not excused" 1 \
	"$H_BUCKET" "$H_SORTED" 0 1 "$case_file"

# This is the load-bearing assertion: matching the same lines is not enough;
# the port side must actually be in the BTreeMap's command-name order.
check "sorted_cmdtable: an unsorted permutation is not excused" 1 \
	"$H_BUCKET" $'/bin/cat\n/bin/cp\n/bin/chmod\n/bin/ls\n/bin/mkdir\n/bin/rm' 0 0 "$case_file"

# Only printentry-shaped pathname lines may move. A diagnostic or unrelated
# output race remains a failure even in a case that runs hash.
check "sorted_cmdtable: a reordered diagnostic is not excused" 1 \
	$'/bin/cat\nSH: 1: x: not found' $'SH: 1: x: not found\n/bin/cat' 0 0 "$case_file"
check "sorted_cmdtable: a non-path line is not excused" 1 \
	$'/bin/cat\nvalue with spaces' $'value with spaces\n/bin/cat' 0 0 "$case_file"
check "sorted_cmdtable: bare command names are deliberately refused" 1 \
	$'rm\ncat' $'cat\nrm' 0 0 "$case_file"

# Scope is specifically a no-operand hash listing. Refreshes and operand
# lookups do not print the table and cannot acquire this excuse.
printf 'echo hash\n' > "$case_file"
check "sorted_cmdtable: a mention is outside the entry" 1 \
	"$H_BUCKET" "$H_SORTED" 0 0 "$case_file"
printf 'hash cat\n' > "$case_file"
check "sorted_cmdtable: hash with an operand is outside the entry" 1 \
	"$H_BUCKET" "$H_SORTED" 0 0 "$case_file"
printf 'hash -r\n' > "$case_file"
check "sorted_cmdtable: hash -r is outside the entry" 1 \
	"$H_BUCKET" "$H_SORTED" 0 0 "$case_file"

# ---- composable POSIX-over-dash normalizers -------------------------
#
# Generated state cases routinely exercise several corrected behaviours in
# one process.  Each normalizer is therefore tested both alone and in a
# composition, plus against nearby output it must not be able to excuse.

printf '%s\n' 'set -- -a; while getopts ab o; do echo "$o|${OPTARG-U}|$OPTIND"; done' > "$case_file"
check "getopts_optarg_unset: the exact observation is normalized" 0 \
	'a||2' 'a|U|2' 0 0 "$case_file"
[ "$DS_DIVERGENCE" = getopts_optarg_unset ] || no \
	"getopts_optarg_unset: reports its own id (got ${DS_DIVERGENCE:-none})"
check "getopts_optarg_unset: a changed option is not excused" 1 \
	'a||2' 'b|U|2' 0 0 "$case_file"
check "getopts_optarg_unset: a changed OPTIND is not excused" 1 \
	'a||2' 'a|U|3' 0 0 "$case_file"
check "getopts_optarg_unset: an alias prefix composes with the record" 0 \
	'pre a||2' 'pre a|U|2' 0 0 "$case_file"
check "getopts_optarg_unset: an arbitrary empty field is not excused" 1 \
	'value||2' 'value|U|2' 0 0 "$case_file"
printf '%s\n' 'set -- -a; while getopts ab o; do echo "$o|$OPTARG|$OPTIND"; done' > "$case_file"
check "getopts_optarg_unset: literal defaulting observation is required" 1 \
	'a||2' 'a|U|2' 0 0 "$case_file"

printf '%s\n' 'set -- -z; while getopts ab o; do :; done' > "$case_file"
check "getopts_diagnostic_prefix: command-mode program name" 0 \
	'Illegal option -z' 'SH: Illegal option -z' 0 0 "$case_file"
check "getopts_diagnostic_prefix: file-mode program name" 0 \
	'No arg for -a option' './script.sh: No arg for -a option' 0 0 "$case_file"
check "getopts_diagnostic_prefix: an unknown prefix is not excused" 1 \
	'Illegal option -z' 'other: Illegal option -z' 0 0 "$case_file"
check "getopts_diagnostic_prefix: changed diagnostic text is not excused" 1 \
	'Illegal option -z' 'SH: Illegal option -x' 0 0 "$case_file"
check "getopts_diagnostic_prefix: a differing status is not excused" 1 \
	'Illegal option -z' 'SH: Illegal option -z' 2 0 "$case_file"

printf '%s\n' 'set -o' > "$case_file"
check "set_hashall_option: long option record" 0 \
	$'errexit         off\nnoglob          off' \
	$'errexit         off\nhashall         off\nnoglob          off' 0 0 "$case_file"
check "set_hashall_option: changed neighboring state is not excused" 1 \
	$'errexit         off\nnoglob          off' \
	$'errexit         on\nhashall         off\nnoglob          off' 0 0 "$case_file"
check "set_hashall_option: enabled hashall is not excused" 1 \
	$'errexit         off\nnoglob          off' \
	$'errexit         off\nhashall         on\nnoglob          off' 0 0 "$case_file"
printf '%s\n' 'set +o' > "$case_file"
check "set_hashall_option: reusable command record" 0 \
	$'set +o errexit\nset +o noglob' \
	$'set +o errexit\nset +o hashall\nset +o noglob' 0 0 "$case_file"
check "set_hashall_option: an arbitrary digest is not excused" 1 \
	'b5ee36cae31777da8e73f63f693f97fe  -' \
	'00000000000000000000000000000000  -' 0 0 "$case_file"

printf '%s\n' 'set -o; set -- -z; while getopts ab o; do echo "$o|${OPTARG-U}|$OPTIND"; done' > "$case_file"
check "normalizers compose without hiding any residual output" 0 \
	$'errexit         off\nIllegal option -z\n?|z|2' \
	$'errexit         off\nhashall         off\nSH: Illegal option -z\n?|z|2' 0 0 "$case_file"
[ "$DS_DIVERGENCE" = getopts_diagnostic_prefix,set_hashall_option ] || no \
	"normalizers: reports every composed id (got ${DS_DIVERGENCE:-none})"
check "normalizers: a residual changed line is not excused" 1 \
	$'errexit         off\nIllegal option -z\n?|z|2\nend' \
	$'errexit         off\nhashall         off\nSH: Illegal option -z\n?|z|2\nchanged' 0 0 "$case_file"

printf '%s\n' 'set -I' > "$case_file"
IGNORE50=$'\nUse "exit" to leave shell.'
for ((i = 1; i < 50; i++)); do
	IGNORE50+=$'\n\nUse "exit" to leave shell.'
done
IGNORE49=${IGNORE50%$'\n\nUse "exit" to leave shell.'}
check "ignoreeof_noninteractive_eof: exact fifty-retry suffix" 0 \
	"$IGNORE50" '' 0 0 "$case_file"
[ "$DS_DIVERGENCE" = ignoreeof_noninteractive_eof ] || no \
	"ignoreeof_noninteractive_eof: reports its own id (got ${DS_DIVERGENCE:-none})"
check "ignoreeof_noninteractive_eof: preserves preceding output" 0 \
	$'READY\n'"$IGNORE50" 'READY' 0 0 "$case_file"
PROMPTED50=
for ((i = 0; i < 50; i++)); do
	PROMPTED50+=$'\nUse "exit" to leave shell.\n$ '
done
printf '%s\n' 'set -i; set -I' > "$case_file"
check "ignoreeof_noninteractive_eof: runtime interactive prompts" 0 \
	'$ '"$PROMPTED50" '$ ' 0 0 "$case_file"
check "ignoreeof_noninteractive_eof: forty-nine retries are not excused" 1 \
	"$IGNORE49" '' 0 0 "$case_file"
check "ignoreeof_noninteractive_eof: changed diagnostic is not excused" 1 \
	"${IGNORE50%?}!" '' 0 0 "$case_file"
check "ignoreeof_noninteractive_eof: a differing status is not excused" 1 \
	"$IGNORE50" '' 0 2 "$case_file"
printf '%s\n' 'echo ignoreeof' > "$case_file"
check "ignoreeof_noninteractive_eof: a mention is outside the entry" 1 \
	"$IGNORE50" '' 0 0 "$case_file"

printf '%s\n' 'fc -l' > "$case_file"
check "fc_listing_format: numbered and continuation records" 0 \
	$'    1 echo one\ncontinued' $'1\techo one\n\tcontinued' 0 0 "$case_file"
check "fc_listing_format: changed command text is not excused" 1 \
	'    1 echo one' $'1\techo two' 0 0 "$case_file"
check "fc_listing_format: a missing continuation is not excused" 1 \
	$'    1 echo one\ncontinued' $'1\techo one' 0 0 "$case_file"
check "fc_listing_format: a differing status is not excused" 1 \
	'    1 echo one' $'1\techo one' 0 1 "$case_file"
printf '%s\n' 'fc -ln' > "$case_file"
check "fc_listing_format: number-suppressed records retain a tab" 0 \
	'echo one' $'\techo one' 0 0 "$case_file"
printf '%s\n' 'echo listing' > "$case_file"
check "fc_listing_format: an unrelated case is outside the entry" 1 \
	'    1 echo one' $'1\techo one' 0 0 "$case_file"

printf '%s\n' 'ulimit -a' > "$case_file"
ULIMIT_REF=$'time(seconds)        unlimited\nfile(blocks)         N\ndata(kbytes)         unlimited\nstack(kbytes)        N\ncoredump(blocks)     N\nmemory(kbytes)       unlimited\nlocked memory(kbytes) N\nprocess              N\nnofiles              N\nvmemory(kbytes)      unlimited\nlocks                unlimited\nrtprio               N'
ULIMIT_PORT=$'CPU time (seconds) (-t) unlimited\nfile size (N-byte units) (-f) N\ndata segment size (N-byte units) (-d) unlimited\nstack size (N-byte units) (-s) N\ncore file size (N-byte units) (-c) N\nresident memory (N-byte units) (-m) unlimited\nlocked memory (N-byte units) (-l) N\nprocesses (-p) N\nopen files (-n) N\naddress space (N-byte units) (-v) unlimited\nfile locks (-w) unlimited\nrealtime priority (-r) N'
check "ulimit_all_format: every resource row is normalized" 0 \
	"$ULIMIT_REF" "$ULIMIT_PORT" 0 0 "$case_file"
check "ulimit_all_format: a changed value is not excused" 1 \
	"$ULIMIT_REF" "${ULIMIT_PORT/open files (-n) N/open files (-n) 9}" 0 0 "$case_file"
check "ulimit_all_format: a wrong resource label is not excused" 1 \
	'nofiles              N' 'file descriptors (-n) N' 0 0 "$case_file"
check "ulimit_all_format: a differing status is not excused" 1 \
	'nofiles              N' 'open files (-n) N' 0 2 "$case_file"
printf '%s\n' 'echo limits' > "$case_file"
check "ulimit_all_format: an unrelated case is outside the entry" 1 \
	'nofiles              N' 'open files (-n) N' 0 0 "$case_file"

JOB_RUNNING='[1] + 123 Running                    '
printf '%s\n' 'sleep 1 & jobs' > "$case_file"
check "jobs_command_text: command text is removed from a status record" 0 \
	"$JOB_RUNNING" "${JOB_RUNNING}sleep 1" 0 0 "$case_file"
check "jobs_command_text: command text absent from the case is refused" 1 \
	"$JOB_RUNNING" "${JOB_RUNNING}other 1" 0 0 "$case_file"
check "jobs_command_text: a changed status prefix is not excused" 1 \
	"$JOB_RUNNING" "[1] + 123 Done                       sleep 1" 0 0 "$case_file"
check "jobs_command_text: a differing status is not excused" 1 \
	"$JOB_RUNNING" "${JOB_RUNNING}sleep 1" 0 1 "$case_file"
JOB_PIPE='[1] + 123 Running                    |'
printf '%s\n' 'sleep 1 | cat & jobs' > "$case_file"
check "jobs_command_text: pipeline component text is removed" 0 \
	"$JOB_PIPE" '[1] + 123 Running                    sleep 1 |' 0 0 "$case_file"
printf '%s\n' 'echo jobs' > "$case_file"
check "jobs_command_text: a mention is outside the entry" 1 \
	"$JOB_RUNNING" "${JOB_RUNNING}sleep 1" 0 0 "$case_file"

JOB_DONE='[1] + Done                       '
printf '%s\n' 'sleep 0 & wait; jobs' > "$case_file"
check "jobs_waited_removal: a waited Done record is removed" 0 \
	$'before\n'"$JOB_DONE"$'\nafter' $'before\nafter' 0 0 "$case_file"
check "jobs_waited_removal: a Running record is not removed" 1 \
	$'before\n'"$JOB_RUNNING"$'\nafter' $'before\nafter' 0 0 "$case_file"
check "jobs_waited_removal: changed surrounding output is not excused" 1 \
	$'before\n'"$JOB_DONE"$'\nafter' $'before\nchanged' 0 0 "$case_file"
check "jobs_waited_removal: a differing status is not excused" 1 \
	"$JOB_DONE" '' 0 1 "$case_file"
printf '%s\n' 'sleep 0 & jobs; wait' > "$case_file"
check "jobs_waited_removal: jobs before wait is outside the entry" 1 \
	"$JOB_DONE" '' 0 0 "$case_file"

printf '%s\n' 'case x in x) : ;& esac' > "$case_file"
CASE_REF='SH: 1: Syntax error: "&" unexpected'
CASE_PORT='SH: 1: Syntax error: ";&" unexpected'
check "case_fallthrough_diagnostic: the complete operator is named" 0 \
	"$CASE_REF" "$CASE_PORT" 2 2 "$case_file"
check "case_fallthrough_diagnostic: changed surrounding text is not excused" 1 \
	"$CASE_REF" 'SH: 2: Syntax error: ";&" unexpected' 2 2 "$case_file"
check "case_fallthrough_diagnostic: a differing status is not excused" 1 \
	"$CASE_REF" "$CASE_PORT" 2 0 "$case_file"
printf '%s\n' 'echo case' > "$case_file"
check "case_fallthrough_diagnostic: a case without the operator is outside" 1 \
	"$CASE_REF" "$CASE_PORT" 2 2 "$case_file"

printf '%s\n' 'fc -s true=false 2>&1; echo "rc=$?"' > "$case_file"
check "fc_substitution_status: executed command status is propagated" 0 \
	'rc=0' 'rc=1' 0 0 "$case_file"
check "fc_substitution_status: another status is not excused" 1 \
	'rc=0' 'rc=2' 0 0 "$case_file"
check "fc_substitution_status: changed output is not excused" 1 \
	$'false\nrc=0' $'changed\nrc=1' 0 0 "$case_file"
check "fc_substitution_status: a differing shell status is not excused" 1 \
	'rc=0' 'rc=1' 0 1 "$case_file"
printf '%s\n' 'fc -s false=true' > "$case_file"
check "fc_substitution_status: another substitution is outside the entry" 1 \
	'rc=0' 'rc=1' 0 0 "$case_file"

printf '%s\n' ': & q=$!; wait $q; wait $q; echo second=$?' > "$case_file"
check "wait_consumed_status: repeated variable wait becomes 127" 0 \
	'second=0' 'second=127' 0 0 "$case_file"
check "wait_consumed_status: an unrelated line is preserved" 1 \
	$'unrelated=0\nsecond=0' $'unrelated=127\nsecond=127' 0 0 "$case_file"
check "wait_consumed_status: a different second status is not excused" 1 \
	'second=0' 'second=126' 0 0 "$case_file"
check "wait_consumed_status: a differing shell status is not excused" 1 \
	'second=0' 'second=127' 0 1 "$case_file"
printf '%s\n' ': & q=$!; wait $q; echo second=$?' > "$case_file"
check "wait_consumed_status: a single wait is outside the entry" 1 \
	'second=0' 'second=127' 0 0 "$case_file"
printf '%s\n' ': & wait $! $!; echo $?' > "$case_file"
check "wait_consumed_status: repeated positional PID becomes 127" 0 \
	'0' '127' 0 0 "$case_file"
check "wait_consumed_status: only the final status record may change" 1 \
	$'0\n0' $'127\n127' 0 0 "$case_file"

printf '%s\n' 'sleep 0 & wait; wait %1 2>&1; echo "rc=$?"' > "$case_file"
WAIT_JOB_DIAG='SH: 2: wait: No such job: %1'
check "wait_consumed_jobspec: stale job diagnostic and status" 0 \
	'rc=0' "$WAIT_JOB_DIAG"$'\nrc=2' 0 0 "$case_file"
check "wait_consumed_jobspec: a changed job number is not excused" 1 \
	'rc=0' 'SH: 2: wait: No such job: %2'$'\nrc=2' 0 0 "$case_file"
check "wait_consumed_jobspec: an extra diagnostic is not excused" 1 \
	'rc=0' "$WAIT_JOB_DIAG"$'\nSH: other\nrc=2' 0 0 "$case_file"
check "wait_consumed_jobspec: a differing shell status is not excused" 1 \
	'rc=0' "$WAIT_JOB_DIAG"$'\nrc=2' 0 1 "$case_file"
printf '%s\n' 'sleep 0 & wait %1 2>&1; echo "rc=$?"' > "$case_file"
check "wait_consumed_jobspec: no prior bare wait is outside the entry" 1 \
	'rc=0' "$WAIT_JOB_DIAG"$'\nrc=2' 0 0 "$case_file"

printf '%s\n' 'OPTIND=1; set -- -a; getopts a o; echo "$o"; OPTIND=1; getopts a o; echo "$o"' > "$case_file"
check "getopts_optind_reset: a second scan restarts at option a" 0 \
	$'a\n?' $'a\na' 0 0 "$case_file"
check "getopts_optind_reset: a changed first scan is not excused" 1 \
	$'b\n?' $'a\na' 0 0 "$case_file"
check "getopts_optind_reset: a wrong restarted option is not excused" 1 \
	$'a\n?' $'a\nb' 0 0 "$case_file"
check "getopts_optind_reset: a differing status is not excused" 1 \
	$'a\n?' $'a\na' 0 1 "$case_file"
printf '%s\n' 'set -- -a -b; while getopts ab o; do echo "1:$o"; done; OPTIND=1; while getopts ab o; do echo "2:$o"; done; echo "optind=$OPTIND"' > "$case_file"
check "getopts_optind_reset: a loop reproduces every first-pass option" 0 \
	$'1:a\n1:b\noptind=3' $'1:a\n1:b\n2:a\n2:b\noptind=3' 0 0 "$case_file"
printf '%s\n' 'set -- -b; getopts b o; OPTIND=1; getopts b o' > "$case_file"
check "getopts_optind_reset: a non-a operand is outside the entry" 1 \
	'?' 'b' 0 0 "$case_file"

printf '%s\n' 'sleep 1 & kill %1; wait 2>/dev/null; echo after-kill' > "$case_file"
KILL_DIAG='SH: 2: kill: No such process'
check "kill_jobspec: dash ESRCH diagnostic is removed" 0 \
	"$KILL_DIAG"$'\n\nafter-kill' 'after-kill' 0 0 "$case_file"
check "kill_jobspec: changed diagnostic text is not excused" 1 \
	$'SH: 2: kill: Permission denied\n\nafter-kill' 'after-kill' 0 0 "$case_file"
check "kill_jobspec: additional output is not excused" 1 \
	"$KILL_DIAG"$'\nextra\nafter-kill' 'after-kill' 0 0 "$case_file"
check "kill_jobspec: too many exact diagnostics are not excused" 1 \
	"$KILL_DIAG"$'\n\n'"$KILL_DIAG"$'\n\nafter-kill' 'after-kill' 0 0 "$case_file"
check "kill_jobspec: a differing status is not excused" 1 \
	"$KILL_DIAG"$'\n\nafter-kill' 'after-kill' 0 1 "$case_file"
printf '%s\n' 'kill 999999; echo after-kill' > "$case_file"
check "kill_jobspec: a numeric PID is outside the entry" 1 \
	"$KILL_DIAG"$'\n\nafter-kill' 'after-kill' 0 0 "$case_file"

printf '%s\n' 'exec 0<&-; read x; echo "rc=$?"' > "$case_file"
check "closed_input_read_error: EBADF maps to failed read" 0 \
	'rc=1' $'Bad file descriptor\nrc=128' 0 0 "$case_file"
check "closed_input_read_error: changed diagnostic is not excused" 1 \
	'rc=1' $'Input/output error\nrc=128' 0 0 "$case_file"
check "closed_input_read_error: changed status is not excused" 1 \
	'rc=1' $'Bad file descriptor\nrc=129' 0 0 "$case_file"
check "closed_input_read_error: a differing shell status is not excused" 1 \
	'rc=1' $'Bad file descriptor\nrc=128' 0 1 "$case_file"
printf '%s\n' "sh -c 'read x; echo \"rc=\$? x=[\$x]\"' 0<&-" > "$case_file"
check "closed_input_read_error: nested read status keeps its suffix" 0 \
	'rc=1 x=[]' $'sh: 1: read: read error: Bad file descriptor\nrc=128 x=[]' 0 0 "$case_file"
printf '%s\n' 'read x; echo "rc=$?"' > "$case_file"
check "closed_input_read_error: open input is outside the entry" 1 \
	'rc=1' $'Bad file descriptor\nrc=128' 0 0 "$case_file"

printf '%s\n' 'sh -c : 1>&- 2>&1' > "$case_file"
CLOSED_DUP_DIAG='SH: 1: 1: Bad file descriptor'
check "closed_output_dup_diagnostic: exact EBADF diagnostic is removed" 0 \
	'' "$CLOSED_DUP_DIAG" 2 2 "$case_file"
check "closed_output_dup_diagnostic: changed descriptor is not excused" 1 \
	'' 'SH: 1: 2: Bad file descriptor' 2 2 "$case_file"
check "closed_output_dup_diagnostic: a differing status is not excused" 1 \
	'' "$CLOSED_DUP_DIAG" 2 1 "$case_file"
printf '%s\n' 'sh -c : 2>&1 1>&-' > "$case_file"
check "closed_output_dup_diagnostic: another order is outside the entry" 1 \
	'' "$CLOSED_DUP_DIAG" 2 2 "$case_file"

printf '%s\n' '"$0" - -c '\''echo dash'\'' 2>&1; echo "rc=$?"' > "$case_file"
check "missing_command_file_status: missing script is status 127" 0 \
	$'cannot open -c\nrc=2' $'cannot open -c\nrc=127' 0 0 "$case_file"
check "missing_command_file_status: changed diagnostic is not excused" 1 \
	$'cannot open -c\nrc=2' $'different\nrc=127' 0 0 "$case_file"
check "missing_command_file_status: another status is not excused" 1 \
	'rc=2' 'rc=126' 0 0 "$case_file"
check "missing_command_file_status: a differing outer status is not excused" 1 \
	'rc=2' 'rc=127' 0 1 "$case_file"
printf '%s\n' 'echo "rc=$?"' > "$case_file"
check "missing_command_file_status: an ordinary status is outside the entry" 1 \
	'rc=2' 'rc=127' 0 0 "$case_file"

printf '%s\n' 'if [ -f /dev/stdin ]; then echo REGFILE; else echo OTHER; fi <<EOF' > "$case_file"
check "logical_fd_introspection: regular here-doc backing is hidden" 0 \
	'REGFILE' 'OTHER' 0 0 "$case_file"
check "logical_fd_introspection: changed surrounding output is not excused" 1 \
	$'REGFILE\ndata' $'OTHER\nchanged' 0 0 "$case_file"
check "logical_fd_introspection: a differing status is not excused" 1 \
	'REGFILE' 'OTHER' 0 1 "$case_file"
printf '%s\n' 'if [ -p /dev/stdin ]; then echo PIPE; else echo OTHER; fi <<EOF' > "$case_file"
check "logical_fd_introspection: pipe here-doc backing is hidden" 0 \
	'PIPE' 'OTHER' 0 0 "$case_file"
printf '%s\n' 'test -f /dev/stdin' > "$case_file"
check "logical_fd_introspection: no here-doc is outside the entry" 1 \
	'REGFILE' 'OTHER' 0 0 "$case_file"

printf '%s\n' '( ulimit -S -n 10; echo rc=$?; ulimit -n; ulimit -Hn )' > "$case_file"
check "ulimit_default_soft_report: one soft-limit line is retained" 0 \
	$'rc=0\n20' $'rc=0\n10\n20' 0 0 "$case_file"
check "ulimit_default_soft_report: a wrong soft value is not excused" 1 \
	$'rc=0\n20' $'rc=0\n11\n20' 0 0 "$case_file"
check "ulimit_default_soft_report: two added lines are not excused" 1 \
	$'rc=0\n20' $'rc=0\n10\n10\n20' 0 0 "$case_file"
check "ulimit_default_soft_report: changed surrounding output is not excused" 1 \
	$'rc=0\n20' $'rc=1\n10\n20' 0 0 "$case_file"
check "ulimit_default_soft_report: a differing status is not excused" 1 \
	$'rc=0\n20' $'rc=0\n10\n20' 0 1 "$case_file"
printf '%s\n' 'ulimit -n' > "$case_file"
check "ulimit_default_soft_report: a query without a set is outside" 1 \
	'20' $'10\n20' 0 0 "$case_file"

# ---- decision-style POSIX corrections -------------------------------

printf '%s\n' 'fc -s' > "$case_file"
FC_RECURSION='fc: called recursively too many times'
check "fc_recursion_error_status: exact utility error status" 0 \
	"$FC_RECURSION" "$FC_RECURSION" 0 2 "$case_file"
check "fc_recursion_error_status: changed diagnostic is not excused" 1 \
	"$FC_RECURSION" 'fc: another failure' 0 2 "$case_file"
check "fc_recursion_error_status: another status pair is not excused" 1 \
	"$FC_RECURSION" "$FC_RECURSION" 0 1 "$case_file"
printf '%s\n' 'fc -l' > "$case_file"
check "fc_recursion_error_status: another fc operation is outside" 1 \
	"$FC_RECURSION" "$FC_RECURSION" 0 2 "$case_file"

printf '%s\n' '( ulimit -S -n 1; echo rc=$?; ulimit -Sn; ulimit -Hn )' > "$case_file"
check "logical_fd_low_nofile_survival: soft limit one probe" 0 \
	'rc=0' $'rc=0\n1\n1024' 2 0 "$case_file"
check "logical_fd_low_nofile_survival: changed query output is not excused" 1 \
	'rc=0' $'rc=0\n2\n1024' 2 0 "$case_file"
check "logical_fd_low_nofile_survival: another status pair is not excused" 1 \
	'rc=0' $'rc=0\n1\n1024' 1 0 "$case_file"
printf '%s\n' '( ulimit -HS -n 0; echo rc=$?; ulimit -Sn; ulimit -Hn )' > "$case_file"
check "logical_fd_low_nofile_survival: hard and soft zero probe" 0 \
	'rc=0' $'rc=0\n0\n0' 2 0 "$case_file"
printf '%s\n' '( ulimit -n 0; echo rc=$?; ulimit -Sn; ulimit -Hn )' > "$case_file"
check "logical_fd_low_nofile_survival: default zero probe" 0 \
	'rc=0' $'rc=0\n0\n0' 2 0 "$case_file"
printf '%s\n' '( ulimit -n 2; echo rc=$? )' > "$case_file"
check "logical_fd_low_nofile_survival: another limit is outside" 1 \
	'rc=0' $'rc=0\n2\n2' 2 0 "$case_file"

printf '%s\n' "trap 'echo caught' PIPE; trap -p PIPE" > "$case_file"
TRAP_P_REF='SH: 1: trap: Illegal option -p'
TRAP_P_PORT="trap -- 'echo caught' PIPE"
check "trap_p_option: exact option diagnostic and listing" 0 \
	"$TRAP_P_REF" "$TRAP_P_PORT" 2 0 "$case_file"
check "trap_p_option: a changed diagnostic is not excused" 1 \
	'SH: 1: trap: Illegal option -x' "$TRAP_P_PORT" 2 0 "$case_file"
check "trap_p_option: a changed listing is not excused" 1 \
	"$TRAP_P_REF" "trap -- 'echo other' PIPE" 2 0 "$case_file"
check "trap_p_option: another status pair is not excused" 1 \
	"$TRAP_P_REF" "$TRAP_P_PORT" 1 0 "$case_file"
printf '%s\n' "trap 'echo caught' PIPE; trap PIPE" > "$case_file"
check "trap_p_option: no -p operand is outside the entry" 1 \
	"$TRAP_P_REF" "$TRAP_P_PORT" 2 0 "$case_file"

printf '%s\n' 'export LC_ALL=en_US.UTF-8; case héllo in h?llo) echo m;; *) echo n;; esac' > "$case_file"
check "utf8_pattern_characters: exact character wildcard witness" 0 \
	n m 0 0 "$case_file"
[ "$DS_DIVERGENCE" = utf8_pattern_characters ] || no \
	"utf8_pattern_characters: reports its own id (got ${DS_DIVERGENCE:-none})"
check "utf8_pattern_characters: changed result is not excused" 1 \
	n x 0 0 "$case_file"
check "utf8_pattern_characters: differing status is not excused" 1 \
	n m 0 1 "$case_file"
printf '%s\n' 'export LC_ALL=en_US.UTF-8; echo n' > "$case_file"
check "utf8_pattern_characters: unrelated UTF-8 case is outside" 1 \
	n m 0 0 "$case_file"

printf '%s\n' "IFS='é'; echo \${\$:+é} 'a\$b' a?c" > "$case_file"
check "c_locale_multibyte_ifs: exact byte-splitting witness" 0 \
	$'é a$b a?c' $'  a$b a?c' 0 0 "$case_file"
[ "$DS_DIVERGENCE" = c_locale_multibyte_ifs ] || no \
	"c_locale_multibyte_ifs: reports its own id (got ${DS_DIVERGENCE:-none})"
check "c_locale_multibyte_ifs: changed spacing is not excused" 1 \
	$'é a$b a?c' $' a$b a?c' 0 0 "$case_file"
check "c_locale_multibyte_ifs: differing status is not excused" 1 \
	$'é a$b a?c' $'  a$b a?c' 0 1 "$case_file"
printf '%s\n' "IFS=':'; echo \${\$:+é} 'a\$b' a?c" > "$case_file"
check "c_locale_multibyte_ifs: another IFS is outside" 1 \
	$'é a$b a?c' $'  a$b a?c' 0 0 "$case_file"

printf '%s\n' 'while false; do :; done; echo ${v=a\*b}${x1:=\*}' > "$case_file"
check "parameter_operand_quote_preservation: exact escaped-glob witness" 0 \
	'ab.txt' 'a*b*' 0 0 "$case_file"
[ "$DS_DIVERGENCE" = parameter_operand_quote_preservation ] || no \
	"parameter_operand_quote_preservation: reports its own id (got ${DS_DIVERGENCE:-none})"
check "parameter_operand_quote_preservation: changed literal is not excused" 1 \
	'ab.txt' 'a*b?' 0 0 "$case_file"
check "parameter_operand_quote_preservation: differing status is not excused" 1 \
	'ab.txt' 'a*b*' 0 1 "$case_file"
printf '%s\n' 'echo ${v=a*b}' > "$case_file"
check "parameter_operand_quote_preservation: unescaped operand is outside" 1 \
	'ab.txt' 'a*b*' 0 0 "$case_file"
printf '%s\n' 'printf "<%s>" aéb ${v:="$(echo é '\''a%b'\'')"}; echo $((v &= ~1))' > "$case_file"
check "parameter_operand_quote_preservation: surrounding diagnostic is exact" 0 \
	$'<aéb><é><a%b>SH: 1: Illegal number: é a%b\n0' \
	$'<aéb><é a%b>SH: 1: Illegal number: é a%b\n0' 0 0 "$case_file"
check "parameter_operand_quote_preservation: changed diagnostic is not excused" 1 \
	$'<aéb><é><a%b>SH: 1: Illegal number: é a%b\n0' \
	$'<aéb><é a%b>SH: 1: Illegal number: other\n0' 0 0 "$case_file"

printf '%s\n' 'while false; do :; done; echo $* ""$IFS"`echo $((IFS))`" [' > "$case_file"
check "empty_quote_field_anchors: exact adjacent-substitution witness" 0 \
	' 0 [' '0 [' 0 0 "$case_file"
[ "$DS_DIVERGENCE" = empty_quote_field_anchors ] || no \
	"empty_quote_field_anchors: reports its own id (got ${DS_DIVERGENCE:-none})"
check "empty_quote_field_anchors: changed field is not excused" 1 \
	' 0 [' '1 [' 0 0 "$case_file"
check "empty_quote_field_anchors: differing status is not excused" 1 \
	' 0 [' '0 [' 0 1 "$case_file"
printf '%s\n' 'while false; do :; done; echo $* $IFS `echo $((IFS))` [' > "$case_file"
check "empty_quote_field_anchors: no empty quote anchor is outside" 1 \
	' 0 [' '0 [' 0 0 "$case_file"

printf '%s\n' "trap 'echo X' TERM; (trap); echo ---; trap" > "$case_file"
TRAP_LINE="trap -- 'echo X' TERM"
check "trap_subshell_listing: inherited listing is added" 0 \
	$'---\n'"$TRAP_LINE" "$TRAP_LINE"$'\n---\n'"$TRAP_LINE" 0 0 "$case_file"
[ "$DS_DIVERGENCE" = trap_subshell_listing ] || no \
	"trap_subshell_listing: reports its own id (got ${DS_DIVERGENCE:-none})"
check "trap_subshell_listing: changed inherited listing is not excused" 1 \
	$'---\n'"$TRAP_LINE" $'trap -- '\''echo Y'\'' TERM\n---\n'"$TRAP_LINE" 0 0 "$case_file"
check "trap_subshell_listing: arbitrary added output is not excused" 1 \
	$'---\n'"$TRAP_LINE" $'extra\n---\n'"$TRAP_LINE" 0 0 "$case_file"
check "trap_subshell_listing: too many inherited listings are not excused" 1 \
	$'---\n'"$TRAP_LINE" "$TRAP_LINE"$'\n'"$TRAP_LINE"$'\n---\n'"$TRAP_LINE" 0 0 "$case_file"
check "trap_subshell_listing: a differing status is not excused" 1 \
	$'---\n'"$TRAP_LINE" "$TRAP_LINE"$'\n---\n'"$TRAP_LINE" 0 1 "$case_file"
printf '%s\n' "trap 'echo X' TERM; echo ---; trap" > "$case_file"
check "trap_subshell_listing: no subshell listing is outside the entry" 1 \
	$'---\n'"$TRAP_LINE" "$TRAP_LINE"$'\n---\n'"$TRAP_LINE" 0 0 "$case_file"

printf '%s\n' 'trap '\''( trap "echo inner" EXIT; exit 2 ); echo $?'\'' EXIT' > "$case_file"
check "exit_trap_final_status: exact nested action" 0 \
	$'inner\n2' $'inner\n0' 0 0 "$case_file"
[ "$DS_DIVERGENCE" = exit_trap_final_status ] || no \
	"exit_trap_final_status: reports its own id (got ${DS_DIVERGENCE:-none})"
check "exit_trap_final_status: changed output is not excused" 1 \
	$'inner\n2' $'inner\n1' 0 0 "$case_file"
check "exit_trap_final_status: differing status is not excused" 1 \
	$'inner\n2' $'inner\n0' 0 1 "$case_file"
printf '%s\n' 'trap '\''echo inner; exit 2'\'' EXIT' > "$case_file"
check "exit_trap_final_status: another action is outside" 1 \
	$'inner\n2' $'inner\n0' 0 0 "$case_file"

printf '%s\n' 'readonly R=1; unset R; echo "rc=$?"; echo "$R"' > "$case_file"
UNSET_RO_REF='SH: 1: unset: R: is read only'
UNSET_RO_PORT='unset: R is read-only'
check "unset_readonly_diagnostic: only the diagnostic spelling differs" 0 \
	"$UNSET_RO_REF" "$UNSET_RO_PORT" 2 2 "$case_file"
[ "$DS_DIVERGENCE" = unset_readonly_diagnostic ] || no \
	"unset_readonly_diagnostic: reports its own id (got ${DS_DIVERGENCE:-none})"
check "unset_readonly_diagnostic: a diagnostic about another name is not excused" 1 \
	"$UNSET_RO_REF" 'unset: Q is read-only' 2 2 "$case_file"
check "unset_readonly_diagnostic: the old status 1 is not excused" 1 \
	"$UNSET_RO_REF" "$UNSET_RO_PORT" 2 1 "$case_file"
check "unset_readonly_diagnostic: a shell that carried on is not excused" 1 \
	"$UNSET_RO_REF" "$UNSET_RO_PORT"$'\nrc=2\n1' 2 2 "$case_file"
check "unset_readonly_diagnostic: a dropped diagnostic is not excused" 1 \
	"$UNSET_RO_REF" '' 2 2 "$case_file"
check "unset_readonly_diagnostic: a refusal dash never made is not excused" 1 \
	'' "$UNSET_RO_PORT" 2 2 "$case_file"
printf '%s\n' 'readonly R=1; unset R' > "$case_file"
check "unset_readonly_diagnostic: the file-mode program name is normalised too" 0 \
	'./script.sh: 1: unset: R: is read only' "$UNSET_RO_PORT" 2 2 "$case_file"
check "unset_readonly_diagnostic: another program name is not excused" 1 \
	'other.sh: 1: unset: R: is read only' "$UNSET_RO_PORT" 2 2 "$case_file"
printf '%s\n' 'readonly r=1; unset r 2>&1 | sed '"'"'s|^[^:]*: ||'"'"'; echo rc=$?' > "$case_file"
check "unset_readonly_diagnostic: the filtered spelling both cases use" 0 \
	$'1: unset: r: is read only\nrc=0' $'r is read-only\nrc=0' 0 0 "$case_file"
check "unset_readonly_diagnostic: a filtered diagnostic about another name is not excused" 1 \
	$'1: unset: r: is read only\nrc=0' $'q is read-only\nrc=0' 0 0 "$case_file"
check "unset_readonly_diagnostic: a changed filtered status record is not excused" 1 \
	$'1: unset: r: is read only\nrc=0' $'r is read-only\nrc=1' 0 0 "$case_file"
printf '%s\n' 'readonly r=1; unset r 2>&1; echo rc=$?' > "$case_file"
check "unset_readonly_diagnostic: an unfiltered case does not get the filtered rewrite" 1 \
	$'1: unset: r: is read only\nrc=0' $'r is read-only\nrc=0' 0 0 "$case_file"
printf '%s\n' 'unset R; echo "rc=$?"' > "$case_file"
check "unset_readonly_diagnostic: a case with no readonly is outside the entry" 1 \
	"$UNSET_RO_REF" "$UNSET_RO_PORT" 2 2 "$case_file"
printf '%s\n' 'readonly R=1; echo "rc=$?"' > "$case_file"
check "unset_readonly_diagnostic: a case that never unsets is outside the entry" 1 \
	"$UNSET_RO_REF" "$UNSET_RO_PORT" 2 2 "$case_file"
printf '%s\n' 'readonly R=1; R=2; echo "rc=$?"' > "$case_file"
check "unset_readonly_diagnostic: the assignment refusal is not excused" 1 \
	'SH: 1: R: is read only' 'R: is read-only' 2 2 "$case_file"

printf '%s\n' '. ./nosuchfile.sh 2>&1; echo "rc=$?"' > "$case_file"
DOT_REF='SH: 1: .: cannot open ./nosuchfile.sh: No such file'
DOT_PORT='.: ./nosuchfile.sh: not found'
check "dot_missing_file_diagnostic: only the diagnostic spelling differs" 0 \
	"$DOT_REF" "$DOT_PORT" 2 2 "$case_file"
[ "$DS_DIVERGENCE" = dot_missing_file_diagnostic ] || no \
	"dot_missing_file_diagnostic: reports its own id (got ${DS_DIVERGENCE:-none})"
check "dot_missing_file_diagnostic: a diagnostic about another file is not excused" 1 \
	"$DOT_REF" '.: ./other.sh: not found' 2 2 "$case_file"
check "dot_missing_file_diagnostic: the old status 1 is not excused" 1 \
	"$DOT_REF" "$DOT_PORT" 2 1 "$case_file"
check "dot_missing_file_diagnostic: a shell that carried on is not excused" 1 \
	"$DOT_REF" "$DOT_PORT"$'\nrc=2' 2 2 "$case_file"
check "dot_missing_file_diagnostic: a dropped diagnostic is not excused" 1 \
	"$DOT_REF" '' 2 2 "$case_file"
check "dot_missing_file_diagnostic: a different open failure is not excused" 1 \
	'SH: 1: .: cannot open ./nosuchfile.sh: Permission denied' "$DOT_PORT" 2 2 "$case_file"
printf '%s\n' 'echo hi' > "$case_file"
check "dot_missing_file_diagnostic: a case that runs no dot is outside the entry" 1 \
	"$DOT_REF" "$DOT_PORT" 2 2 "$case_file"

printf '%s\n' ': ${x?boom}' 'echo NOTREACHED' > "$case_file"
PARAM_REF='./script.sh: 1: x: boom'
check "parameter_error_diagnostic: only dash's spine differs" 0 \
	"$PARAM_REF" 'x: boom' 2 2 "$case_file"
[ "$DS_DIVERGENCE" = parameter_error_diagnostic ] || no \
	"parameter_error_diagnostic: reports its own id (got ${DS_DIVERGENCE:-none})"
check "parameter_error_diagnostic: another word is not excused" 1 \
	"$PARAM_REF" 'x: bang' 2 2 "$case_file"
check "parameter_error_diagnostic: another name is not excused" 1 \
	'./script.sh: 1: y: boom' 'y: boom' 2 2 "$case_file"
check "parameter_error_diagnostic: the old status 1 is not excused" 1 \
	"$PARAM_REF" 'x: boom' 2 1 "$case_file"
check "parameter_error_diagnostic: a shell that carried on is not excused" 1 \
	"$PARAM_REF" $'x: boom\nNOTREACHED' 2 2 "$case_file"
printf '%s\n' 'printf "%s\n" "cat <<EOF" "${x:?boom}" EOF > t.sh; . ./t.sh 2>&1; echo rc=$?' > "$case_file"
check "parameter_error_diagnostic: the sourced script name is a second field" 0 \
	$'SH: 1: ./t.sh: x: boom\nrc=2' $'x: boom\nrc=2' 0 0 "$case_file"
printf '%s\n' 'echo hi' > "$case_file"
check "parameter_error_diagnostic: a case with no ? expansion is outside the entry" 1 \
	'SH: 1: x: boom' 'x: boom' 2 2 "$case_file"

printf '%s\n' 'set -u' 'echo $x' > "$case_file"
NOUNSET_REF='SH: 2: x: parameter not set'
check "nounset_error_diagnostic: only dash's spine differs" 0 \
	"$NOUNSET_REF" 'x: parameter not set' 2 2 "$case_file"
[ "$DS_DIVERGENCE" = nounset_error_diagnostic ] || no \
	"nounset_error_diagnostic: reports its own id (got ${DS_DIVERGENCE:-none})"
check "nounset_error_diagnostic: another name is not excused" 1 \
	"$NOUNSET_REF" 'y: parameter not set' 2 2 "$case_file"
check "nounset_error_diagnostic: another message is not excused" 1 \
	'SH: 2: x: something else' 'x: something else' 2 2 "$case_file"
check "nounset_error_diagnostic: the old status 1 is not excused" 1 \
	"$NOUNSET_REF" 'x: parameter not set' 2 1 "$case_file"
check "nounset_error_diagnostic: a dropped diagnostic is not excused" 1 \
	"$NOUNSET_REF" '' 2 2 "$case_file"
printf '%s\n' 'echo $x' > "$case_file"
check "nounset_error_diagnostic: a case that never enables nounset is outside the entry" 1 \
	"$NOUNSET_REF" 'x: parameter not set' 2 2 "$case_file"

printf '%s\n' 'PS4='"'"'[$(echo sub)] '"'"'; set -x; echo hi' > "$case_file"
RE_SYNTAX='SH: 1: Syntax error: end of file unexpected (expecting ")")'
check "re_entered_prompt_substitution: the substitution dash loses to a stale token" 0 \
	"$RE_SYNTAX"$'\n[$(echo sub)] echo hi\nhi' $'[sub] echo hi\nhi' 0 0 "$case_file"
[ "$DS_DIVERGENCE" = re_entered_prompt_substitution ] || no \
	"re_entered_prompt_substitution: reports its own id (got ${DS_DIVERGENCE:-none})"
check "re_entered_prompt_substitution: a different expansion is not excused" 1 \
	"$RE_SYNTAX"$'\n[$(echo sub)] echo hi\nhi' $'[other] echo hi\nhi' 0 0 "$case_file"
check "re_entered_prompt_substitution: a changed reference diagnostic is not excused" 1 \
	$'SH: 1: Syntax error: something else\n[$(echo sub)] echo hi\nhi' \
	$'[sub] echo hi\nhi' 0 0 "$case_file"
check "re_entered_prompt_substitution: a lost traced command is not excused" 1 \
	"$RE_SYNTAX"$'\n[$(echo sub)] echo hi\nhi' $'[sub] echo hi' 0 0 "$case_file"
check "re_entered_prompt_substitution: a differing exit status is not excused" 1 \
	"$RE_SYNTAX"$'\n[$(echo sub)] echo hi\nhi' $'[sub] echo hi\nhi' 0 2 "$case_file"
printf '%s\n' 'PS4='"'"'[`echo bq`] '"'"'; set -x; echo hi' > "$case_file"
check "re_entered_prompt_substitution: the backquote dash silently drops" 0 \
	$'[] echo hi\nhi' $'[bq] echo hi\nhi' 0 0 "$case_file"
check "re_entered_prompt_substitution: a backquote expanding to something else is refused" 1 \
	$'[] echo hi\nhi' $'[zz] echo hi\nhi' 0 0 "$case_file"
printf '%s\n' 'PS4='"'"'$(exit 3)x '"'"'; set -x; echo hi; echo rc=$?' > "$case_file"
check "re_entered_prompt_substitution: dash recovering on its second prompt" 0 \
	"$RE_SYNTAX"$'\n$(exit 3)x echo hi\nhi\nx echo rc=0\nrc=0' \
	$'x echo hi\nhi\nx echo rc=0\nrc=0' 0 0 "$case_file"
check "re_entered_prompt_substitution: a changed status record is not excused" 1 \
	"$RE_SYNTAX"$'\n$(exit 3)x echo hi\nhi\nx echo rc=0\nrc=0' \
	$'x echo hi\nhi\nx echo rc=3\nrc=3' 0 0 "$case_file"
printf '%s\n' $'PS4=\'$(echo PS) \'\nset -x\necho hi\nset +x' > "$case_file"
check "re_entered_prompt_substitution: only the last prompt of a file-shaped case" 0 \
	$'PS echo hi\nhi\n'"$RE_SYNTAX"$'\n$(echo PS) set +x' \
	$'PS echo hi\nhi\nPS set +x' 0 0 "$case_file"
check "re_entered_prompt_substitution: a first prompt that also differed is refused" 1 \
	$'XX echo hi\nhi\n'"$RE_SYNTAX"$'\n$(echo PS) set +x' \
	$'PS echo hi\nhi\nPS set +x' 0 0 "$case_file"
printf '%s\n' 'PS4='"'"'[$(echo other)] '"'"'; set -x; echo hi' > "$case_file"
check "re_entered_prompt_substitution: another prompt value is outside the entry" 1 \
	"$RE_SYNTAX"$'\n[$(echo other)] echo hi\nhi' $'[other] echo hi\nhi' 0 0 "$case_file"
printf '%s\n' 'echo hi' > "$case_file"
check "re_entered_prompt_substitution: a case with no prompt is outside the entry" 1 \
	"$RE_SYNTAX"$'\n[$(echo sub)] echo hi\nhi' $'[sub] echo hi\nhi' 0 0 "$case_file"

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
