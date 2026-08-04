#!/bin/bash
# One differential case. Invoked by dsdiff.sh via xargs; $1 is the case file.
# Writes $RUNROOT/out/<id> containing either the single word PASS or a
# human-readable failure report.
#
# Both shells run inside a PID namespace via ds_sandboxed -- see
# sandboxed.sh for why that is not optional.
ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
set -u
. "$(dirname "$0")/sandboxed.sh"

CASE=$1
ID=$(basename "$CASE")
OUT=$RUNROOT/out/$ID

mode=c
shargs=
name=
norm=
extra=

# Peel the directive lines off the front of the case.
body=$CASE.body
: > "$body"
directives_done=
while IFS= read -r line; do
	if [ -z "$directives_done" ]; then
		case $line in
		'#!mode='*) mode=${line#'#!mode='}; continue ;;
		'#!args '*) extra=${line#'#!args '}; continue ;;
		'#!shargs '*) shargs=${line#'#!shargs '}; continue ;;
		'#!norm='*) norm=${line#'#!norm='}; continue ;;
		'#!name '*) name=${line#'#!name '}; continue ;;
		'#!allow-kill') continue ;;
		esac
		directives_done=1
	fi
	printf '%s\n' "$line" >> "$body"
done < "$CASE"
[ -n "$name" ] || name=$(head -c 300 "$body" | tr '\n' '~')

run() {  # run SHELL WORKDIR
	local sh=$1 dir=$2
	rm -rf "$dir"; mkdir -p "$dir/.bin"
	# A few legacy cases in the corpus expect these to exist.
	(cd "$dir" && touch ab.txt bb.txt cb.txt)
	# Put the shell UNDER TEST on PATH as `sh`. Without this a case that
	# writes `sh -c ...` or `| sh` runs the SYSTEM shell -- which is
	# dash 0.5.12 here -- so both sides execute the same third binary and
	# the case passes while testing nothing.
	ln -sf "$sh" "$dir/.bin/sh"
	# ...and invoke it through that same path, so argv[0] is byte-identical
	# for the two shells. dash prints argv[0] in every diagnostic, and a
	# case that renders its own stderr -- `2>&1 | od -c` is all over the
	# printf corpora -- lays those bytes out one per column, where no
	# amount of path-normalising sed can reach them. Comparing
	# `.../ref/src/dash` against `.../target/debug/dash` reported 3002
	# divergences in aud_bltin_printffuzz alone, every one of them the
	# path and not the behaviour.
	local exe=$dir/.bin/sh
	case $mode in
	file)
		cp "$body" "$dir/script.sh"
		ds_sandboxed "$dir" "$exe" $shargs ./script.sh $extra 2>&1
		;;
	stdin)
		ds_sandboxed "$dir" "$exe" $shargs $extra < "$body" 2>&1
		;;
	*)
		ds_sandboxed "$dir" "$exe" $shargs -c "$(cat "$body")" $extra 2>&1
		;;
	esac
}

# The working directory MUST be named `w`. Much of the corpus tests
# cd/pwd and normalises the shell's cwd with `sed 's|/w$|/W|'`, which only
# works if the basename is exactly that. Naming them `<id>.ref` /
# `<id>.port` made six cd/pwd cases report a divergence that was purely
# the directory name.
#
# The two shells run sequentially in the *same* directory rather than in
# `w` and `w2`. Everything a case can observe about where it is -- $PWD,
# $0, the argv[0] in a diagnostic, a path echoed by `ls` -- is then
# identical for the two runs by construction, instead of being identical
# only in whatever the sed below happens to catch.
ro=$(run "$REF"  "$RUNROOT/c/$ID/w"); rr=$?
po=$(run "$PORT" "$RUNROOT/c/$ID/w"); pr=$?
rm -rf "$RUNROOT/c/$ID"

# The run root differs between concurrent harness invocations, so it is
# still normalised away; the shell paths no longer can differ.
#
# There is exactly one normaliser, and that matters: when the initial
# comparison and the re-run comparison in classify() used different seds,
# a re-run could never equal the original, so classify() answered "not
# flaky" for every case whose output mentions a path -- which is every
# case that produces a diagnostic.
norm_out() {
	local s
	s=$(printf '%s' "$1" | sed -e "s|$RUNROOT/c/$ID/w|WD|g" -e 's|WD/\.bin/sh|SH|g')
	[ "$norm" = pid ] && s=$(printf '%s' "$s" | sed -E 's/[0-9]{3,}/PID/g')
	printf '%s' "$s"
}

ro=$(norm_out "$ro")
po=$(norm_out "$po")

# A mismatch is not automatically a divergence. Where two processes write
# to one fd -- both stages of a pipeline reporting "not found", a `&` job
# racing the shell's own exit, dash's "I/O error" when a stage writes to a
# peer that has already gone -- the byte order is decided by the
# scheduler, not by the shell. Measured directly on `42>:|echo esac`:
# the reference produced one ordering 53/60 and the other 7/60, the port
# 53/60 and 4/60 plus three runs where the two writes interleaved inside a
# line. Same behaviour, different coin.
#
# So re-run both sides and call it a divergence only if their observed
# output sets stay disjoint. Ten rounds, not four: at a ~12% minority
# branch, four rounds leaves a real chance of never sampling the variant
# the other side happened to produce, and that is what put fifteen of
# these in the failure report on a tree with no divergence in it.
# Sampling stops as soon as the sets overlap, so a genuine failure pays
# for at most ten extra runs and only once.
CLASSIFY_ROUNDS=${CLASSIFY_ROUNDS:-10}
classify_detail=
classify() {
	local i a b
	local -a refset portset
	refset=("$ro"); portset=("$po")
	for ((i = 0; i < CLASSIFY_ROUNDS; i++)); do
		# Any output the port produced that the reference also produced
		# at some point means the two agree on the set of legal
		# behaviours.
		for a in "${portset[@]}"; do
			for b in "${refset[@]}"; do
				[ "$a" = "$b" ] && return 0
			done
		done
		refset+=("$(norm_out "$(run "$REF" "$RUNROOT/c/$ID/w")")")
		portset+=("$(norm_out "$(run "$PORT" "$RUNROOT/c/$ID/w")")")
	done
	# Give up, but say how varied each side was: a reference that
	# disagreed with itself here means the case cannot separate the two
	# shells and belongs in the corpus's list of things to rewrite, not
	# in a bug report.
	local uref uport
	uref=$(printf '%s\0' "${refset[@]}" | sort -zu | grep -zc '' )
	uport=$(printf '%s\0' "${portset[@]}" | sort -zu | grep -zc '')
	classify_detail="  over $((CLASSIFY_ROUNDS + 1)) runs each: ref produced $uref distinct outputs, port $uport"
	return 1
}

if [ "$ro" = "$po" ] && [ "$rr" = "$pr" ]; then
	echo PASS > "$OUT"
elif classify; then
	{
		echo "### FLAKY $name"
		echo "--- case ---"
		cat "$body"
		echo "  the port produced an output the reference also produces;"
		echo "  the difference is scheduling, not behaviour."
		echo "  ref  rc=$rr [$ro]"
		echo "  port rc=$pr [$po]"
		echo
	} > "$OUT.flaky"
	echo PASS > "$OUT"
else
	{
		echo "### $name"
		echo "--- case ---"
		cat "$body"
		echo "  ref  rc=$rr [$ro]"
		echo "  port rc=$pr [$po]"
		[ -n "$classify_detail" ] && echo "$classify_detail"
		echo
	} > "$OUT"
fi
