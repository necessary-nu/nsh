#!/usr/bin/env bash
# Replay every stored artifact against the current build and say which still
# reproduce.
#
#   fuzz/sweep.sh                 # every target that has artifacts
#   fuzz/sweep.sh roundtrip       # one target
#   fuzz/sweep.sh --prune         # delete the artifacts that no longer fail
#
# One fix usually kills a whole family: the corpus that took four fixes to
# close held 284 artifacts and 281 of them died with them. Sweeping after a
# fix is what stops the survivors being triaged one at a time, and what keeps
# the artifact directory a list of open findings rather than a history of
# closed ones.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: fuzz/sweep.sh [--containment auto|outer|new] [--prune] [TARGET...]

  --prune   remove the artifacts that no longer reproduce (default: report)
EOF
    exit 2
}

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
prune=false
containment=()
targets=()
while (($#)); do
    case $1 in
        --prune) prune=true; shift ;;
        --containment) (($# >= 2)) || usage; containment=(--containment "$2"); shift 2 ;;
        --containment=*) containment=(--containment "${1#*=}"); shift ;;
        --help|-h) usage ;;
        --) shift; break ;;
        -*) usage ;;
        *) targets+=("$1"); shift ;;
    esac
done
targets+=("$@")

if ((${#targets[@]} == 0)); then
    for directory in "$root"/fuzz/artifacts/*/; do
        [[ -d $directory ]] || continue
        compgen -G "$directory*" >/dev/null || continue
        targets+=("$(basename "$directory")")
    done
fi
((${#targets[@]})) || { echo "fuzz/sweep.sh: no artifacts to sweep" >&2; exit 0; }

# The replay itself is `cargo fuzz build` plus one execution per file, and
# both belong inside whatever boundary a campaign would have run in.
triple=$(rustc -vV | sed -n 's/^host: //p')
live_total=0
dead_total=0

for target in "${targets[@]}"; do
    case $target in
        *[![:alnum:]_-]*|'') echo "fuzz/sweep.sh: invalid target: $target" >&2; exit 2 ;;
    esac
    directory="$root/fuzz/artifacts/$target"
    if ! compgen -G "$directory/*" >/dev/null; then
        printf 'fuzz/sweep.sh: %s has no artifacts\n' "$target" >&2
        continue
    fi

    printf 'fuzz/sweep.sh: building %s\n' "$target" >&2
    "$root/fuzz/run.sh" "${containment[@]}" --build "$target" >&2

    binary="$root/fuzz/target/$triple/release/$target"
    [[ -x $binary ]] || { echo "fuzz/sweep.sh: no binary at $binary" >&2; exit 1; }

    live=()
    dead=()
    for artifact in "$directory"/*; do
        [[ -f $artifact ]] || continue
        # A libFuzzer target given one file runs that input and exits non-zero
        # when it fails. A timeout counts as still failing: a hang is a finding.
        if timeout 60 "$binary" "$artifact" >/dev/null 2>&1; then
            dead+=("$artifact")
        else
            live+=("$artifact")
        fi
    done

    printf '%s: %d live, %d dead\n' "$target" "${#live[@]}" "${#dead[@]}"
    for artifact in "${live[@]}"; do
        printf '  live  %s\n' "$(basename "$artifact")"
    done
    if $prune; then
        for artifact in "${dead[@]}"; do
            rm -f -- "$artifact"
        done
        ((${#dead[@]})) && printf '  pruned %d closed artifact(s)\n' "${#dead[@]}"
    elif ((${#dead[@]})); then
        printf '  %d artifact(s) no longer reproduce; --prune removes them\n' "${#dead[@]}"
    fi

    live_total=$((live_total + ${#live[@]}))
    dead_total=$((dead_total + ${#dead[@]}))
done

printf 'total: %d live, %d dead\n' "$live_total" "$dead_total"
((live_total == 0))
