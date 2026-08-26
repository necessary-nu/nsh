#!/usr/bin/env bash
# Build and run one fuzz target under exactly one containment layer.
#
#   fuzz/run.sh parse              # run until interrupted
#   fuzz/run.sh parse 300          # run for 300 seconds
#   fuzz/run.sh parse 300 -jobs=4  # extra libFuzzer flags after the time
#
# `auto` uses the managed Codex workspace boundary when it is present and
# otherwise creates the normal host boundary with scripts/sandboxed. The
# explicit modes exist for other managed environments and for diagnostics.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: fuzz/run.sh [--containment auto|outer|new] [--build] [--dry-run] TARGET [SECONDS] [libfuzzer flags...]

  auto    use the managed Codex workspace boundary when detected; otherwise
          create the normal host boundary (default)
  outer   use an existing outer containment boundary
  new     create the normal host boundary with scripts/sandboxed
  --build build the target and stop, so a replay can run the binary itself
EOF
    exit 2
}

containment=${NSH_FUZZ_CONTAINMENT:-auto}
dry_run=false
build_only=false
while (($#)); do
    case $1 in
        --containment)
            (($# >= 2)) || usage
            containment=$2
            shift 2
            ;;
        --containment=*)
            containment=${1#*=}
            shift
            ;;
        --dry-run)
            dry_run=true
            shift
            ;;
        --build)
            build_only=true
            shift
            ;;
        --help|-h)
            usage
            ;;
        --)
            shift
            break
            ;;
        -*)
            usage
            ;;
        *)
            break
            ;;
    esac
done

(($#)) || usage
target=$1
shift
seconds=0
if (($#)) && [[ $1 != -* ]]; then
    seconds=$1
    shift
fi

case $target in
    *[![:alnum:]_-]*|'')
        echo "fuzz/run.sh: invalid target: $target" >&2
        exit 2
        ;;
esac
case $seconds in
    *[!0-9]*|'')
        echo "fuzz/run.sh: SECONDS must be a non-negative integer" >&2
        exit 2
        ;;
esac

case $containment in
    auto)
        # A managed Codex workspace provides a writable project mount inside
        # its own process boundary. Re-wrapping it would create an unnecessary
        # nested sandbox, so use that outer boundary directly.
        if [[ -n ${CODEX_SESSION_ID:-} ]]; then
            case ${CODEX_PERMISSION_PROFILE:-} in
                :workspace|workspace|workspace-write) containment=outer ;;
                *) containment=new ;;
            esac
        else
            containment=new
        fi
        ;;
    outer|new)
        ;;
    *)
        echo "fuzz/run.sh: containment must be auto, outer, or new" >&2
        exit 2
        ;;
esac

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
corpus="$root/fuzz/corpus/$target"
if $build_only; then
    # `fuzz/sweep.sh` replays stored artifacts by executing the target binary
    # once per file, which still needs the build to happen inside the boundary
    # a campaign would have used.
    command=(cargo +nightly fuzz build "$target" "$@")
else
    command=(
        cargo +nightly fuzz run "$target" "$corpus" --
        -max_total_time="$seconds"
        -rss_limit_mb=4096
        -max_len=65536
        "$@"
    )
fi

if [[ $containment == new ]]; then
    # The sandbox gets a wall clock of its own so an unattended run cannot
    # outlive its budget; `0` there means "no limit", which is what an
    # open-ended session wants.
    if [ "$seconds" -gt 0 ]; then wall=$(( seconds + 120 )); else wall=0; fi
    command=(
        "$root/scripts/sandboxed" --timeout "$wall" --writable "$root/fuzz" --
        # cargo-fuzz's -jobs supervisor writes worker logs in its working
        # directory. The repository root is read-only inside this sandbox,
        # while fuzz/ is the wrapper's explicit writable bind.
        /usr/bin/env --chdir="$root/fuzz" "${command[@]}"
    )
fi

printf 'fuzz/run.sh: containment=%s\n' "$containment" >&2
if $dry_run; then
    printf 'fuzz/run.sh: command:' >&2
    printf ' %q' "${command[@]}" >&2
    printf '\n' >&2
    exit 0
fi

if $build_only; then
    exec "${command[@]}"
fi

mkdir -p "$corpus" "$root/fuzz/artifacts/$target"

# Seed from the shell text the repository already has. Real scripts are
# far better starting points than random bytes: they reach constructs a
# generator would take a very long time to stumble into, and the corpora
# below are already vendored and licence-cleared.
if [ -z "$(ls -A "$corpus" 2>/dev/null)" ]; then
    n=0
    for source in "$root"/tests/surveys/smoosh/shell/*.test \
                  "$root"/tests/surveys/oils/spec/*.test.sh; do
        [ -f "$source" ] || continue
        cp "$source" "$corpus/seed-$n" 2>/dev/null || true
        n=$((n + 1))
    done
    echo "seeded $n corpus entries into fuzz/corpus/$target"
fi

exec "${command[@]}"
