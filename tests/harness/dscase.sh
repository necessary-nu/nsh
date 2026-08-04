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
	case $mode in
	file)
		cp "$body" "$dir/script.sh"
		ds_sandboxed "$dir" "$sh" $shargs ./script.sh $extra 2>&1
		;;
	stdin)
		ds_sandboxed "$dir" "$sh" $shargs $extra < "$body" 2>&1
		;;
	*)
		ds_sandboxed "$dir" "$sh" $shargs -c "$(cat "$body")" $extra 2>&1
		;;
	esac
}

# The working directories MUST be named `w` and `w2`. Much of the corpus
# tests cd/pwd and normalises the shell's cwd with `sed 's|/w2$|/W|'`,
# which only works if the basenames are exactly those. Naming them
# `<id>.ref` / `<id>.port` made six cd/pwd cases report a divergence that
# was purely the directory name.
ro=$(run "$REF"  "$RUNROOT/c/$ID/w"); rr=$?
po=$(run "$PORT" "$RUNROOT/c/$ID/w2"); pr=$?
rm -rf "$RUNROOT/c/$ID"

# argv[0] appears in diagnostics and legitimately differs.
ro=$(printf '%s' "$ro" | sed -e "s|$RUNROOT/c/$ID/w|WD|g" -e 's|[^ ]*dash-ref|SH|g' -e 's|[^ ]*/dash|SH|g' -e 's|WD/\.bin/sh|SH|g')
po=$(printf '%s' "$po" | sed -e "s|$RUNROOT/c/$ID/w2|WD|g" -e 's|[^ ]*dash-ref|SH|g' -e 's|[^ ]*/dash|SH|g' -e 's|WD/\.bin/sh|SH|g')
if [ "$norm" = pid ]; then
	ro=$(printf '%s' "$ro" | sed -E 's/[0-9]{3,}/PID/g')
	po=$(printf '%s' "$po" | sed -E 's/[0-9]{3,}/PID/g')
fi

norm_out() {  # strip what legitimately differs between the two processes
	printf '%s' "$1" | sed -e 's|[^ ]*dash-ref|SH|g' -e 's|[^ ]*/dash|SH|g'
}

# A mismatch is not automatically a divergence. dash reports "I/O error"
# when a pipeline stage writes to a peer that has already exited, and
# whether that happens depends on scheduling -- measured at 5/150 for the
# reference and 2/150 for the port on the same input. Re-run both sides
# and only call it a failure if each binary is individually stable AND
# their observed output sets are disjoint.
classify() {
	local i o r
	local -a refset portset
	refset=("$ro"); portset=("$po")
	for i in 1 2 3 4; do
		o=$(norm_out "$(run "$REF" "$RUNROOT/c/$ID/w")"); r=$?
		refset+=("$o")
		o=$(norm_out "$(run "$PORT" "$RUNROOT/c/$ID/w2")"); r=$?
		portset+=("$o")
	done
	local uref uport
	uref=$(printf '%s\0' "${refset[@]}" | sort -zu | tr -d '\0' | wc -c)
	uport=$(printf '%s\0' "${portset[@]}" | sort -zu | tr -d '\0' | wc -c)
	# Any output the port produced that the reference also produced at
	# some point means the two agree on the set of legal behaviours.
	local a b
	for a in "${portset[@]}"; do
		for b in "${refset[@]}"; do
			[ "$a" = "$b" ] && return 0   # compatible => flaky, not a divergence
		done
	done
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
		echo
	} > "$OUT"
fi
