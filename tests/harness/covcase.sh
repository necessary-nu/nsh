#!/bin/bash
# One coverage case: run the instrumented reference, discard the output,
# keep the counters. Invoked by covrun.sh via xargs.
ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
set -u
. "$(dirname "$0")/sandboxed.sh"

CASE=$1
ID=$(basename "$CASE")
dir=$RUNROOT/w/$ID
mkdir -p "$dir"

mode=c
shargs=
extra=
body=$CASE.body
: > "$body"
directives_done=
while IFS= read -r line; do
	if [ -z "$directives_done" ]; then
		case $line in
		'#!mode='*) mode=${line#'#!mode='}; continue ;;
		'#!args '*) extra=${line#'#!args '}; continue ;;
		'#!shargs '*) shargs=${line#'#!shargs '}; continue ;;
		'#!norm='*|'#!name '*|'#!allow-kill') continue ;;
		esac
		directives_done=1
	fi
	printf '%s\n' "$line" >> "$body"
done < "$CASE"

(cd "$dir" && touch ab.txt bb.txt cb.txt)

# Same containment as the differential harness, plus a writable bind for
# the coverage directory so libgcov can merge its .gcda files.
cov_sandboxed() {
	timeout $(( ${DS_TIMEOUT:-10} + 5 )) \
	"$DS_SANDBOX" --quiet \
		--unshare all --die-with-parent --new-session \
		--bind /:/:ro --dev /dev --proc /proc \
		--bind "$dir:$dir" \
		--bind "$COVDIR:$COVDIR" \
		--chdir "$dir" --setenv TMPDIR "$dir" \
		--limit nproc=64 \
		-- timeout "${DS_TIMEOUT:-10}" "$@"
}

case $mode in
file)
	cp "$body" "$dir/script.sh"
	cov_sandboxed "$COV" $shargs ./script.sh $extra >/dev/null 2>&1
	;;
stdin)
	cov_sandboxed "$COV" $shargs $extra < "$body" >/dev/null 2>&1
	;;
*)
	cov_sandboxed "$COV" $shargs -c "$(cat "$body")" $extra >/dev/null 2>&1
	;;
esac

rm -rf "$dir"
