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
usage: fuzz/run.sh [--containment auto|outer|new] [--build|--derive] [--dry-run] TARGET [SECONDS] [libfuzzer flags...]

  auto     use the managed Codex workspace boundary when detected; otherwise
           create the normal host boundary (default)
  outer    use an existing outer containment boundary
  new      create the normal host boundary with scripts/sandboxed
  --build  build the target and stop, so a replay can run the binary itself
  --derive run every archived input against the build and reduce it to the
           campaign corpus a campaign then seeds from; deletes nothing

A campaign builds before its clock starts, under a wall clock of its own:
NSH_FUZZ_BUILD_TIMEOUT seconds, 1800 by default. It then replays its seed
corpus before it mutates anything, under a second clock of its own:
NSH_FUZZ_REPLAY_ALLOWANCE seconds, 900 by default. SECONDS is the mutation
that follows, and buys nothing else.

That seed corpus is fuzz/campaign/TARGET when --derive has produced one, and
the whole of fuzz/corpus/TARGET when it has not -- which is the difference
between seconds of start-up and minutes of it. Nothing is ever deleted from
the archive, and what a campaign finds goes back into it.
EOF
    exit 2
}

containment=${NSH_FUZZ_CONTAINMENT:-auto}
dry_run=false
build_only=false
derive_only=false
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
        --derive)
            derive_only=true
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

if $build_only && $derive_only; then
    echo "fuzz/run.sh: --build and --derive are different things to stop after" >&2
    exit 2
fi

(($#)) || usage
target=$1
shift
seconds=0
if (($#)) && [[ $1 != -* ]]; then
    seconds=$1
    shift
fi

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)

# The name reaches a path and a cargo argument, so the characters are
# checked first; but a well-formed name that is not a target is the error
# that actually happened. `fuzz/run.sh parse 240` typed as
# `fuzz/run.sh 240` used to seed a corpus, create an artifact directory
# and hand `240` to cargo, which is a fuzzing session that measures
# nothing -- the shape `[spec:nsh:req:oracle.cannot-measure-is-a-failure]`
# exists to refuse.
case $target in
    *[![:alnum:]_-]*|'')
        echo "fuzz/run.sh: invalid target: $target" >&2
        exit 2
        ;;
esac
targets=$(cd "$root/fuzz" && cargo +nightly fuzz list) || {
    echo "fuzz/run.sh: cannot list the fuzz targets" >&2
    exit 2
}
if ! printf '%s\n' "$targets" | grep -qxF -- "$target"; then
    echo "fuzz/run.sh: no such target: $target" >&2
    echo "fuzz/run.sh: targets are:" >&2
    printf '%s\n' "$targets" | sed 's/^/  /' >&2
    exit 2
fi
case $seconds in
    *[!0-9]*|'')
        echo "fuzz/run.sh: SECONDS must be a non-negative integer" >&2
        exit 2
        ;;
esac
build_timeout=${NSH_FUZZ_BUILD_TIMEOUT:-1800}
case $build_timeout in
    *[!0-9]*|'')
        echo "fuzz/run.sh: NSH_FUZZ_BUILD_TIMEOUT must be a non-negative integer" >&2
        exit 2
        ;;
esac
replay_allowance=${NSH_FUZZ_REPLAY_ALLOWANCE:-900}
case $replay_allowance in
    *[!0-9]*|'')
        echo "fuzz/run.sh: NSH_FUZZ_REPLAY_ALLOWANCE must be a non-negative integer" >&2
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

# Put one command inside the single boundary this run uses, leaving it alone
# when an outer boundary already holds it. `clock` is the wall clock that
# boundary gets, and `0` there means no limit.
contain() {
    local clock=$1
    shift
    if [[ $containment != new ]]; then
        contained=("$@")
        return
    fi
    contained=(
        "$root/scripts/sandboxed" --timeout "$clock" --writable "$root/fuzz" --
        # libFuzzer's -jobs supervisor writes one worker log per job in its
        # working directory. The repository root is read-only inside this
        # sandbox, while fuzz/ is the wrapper's explicit writable bind.
        /usr/bin/env --chdir="$root/fuzz" "$@"
    )
}

corpus="$root/fuzz/corpus/$target"
campaign_corpus="$root/fuzz/campaign/$target"
artifacts="$root/fuzz/artifacts/$target"
# `cargo fuzz build` puts the target here, and `fuzz/sweep.sh` already
# replays artifacts by executing it. A campaign runs it directly too, so
# that the clock it is on covers fuzzing and nothing else: `cargo fuzz run`
# compiles again before it hands over, and two campaigns started at once
# spend one of the two clocks waiting on the other's Cargo build lock.
triple=$(rustc -vV | sed -n 's/^host: //p')
binary="$root/fuzz/target/$triple/release/$target"

# The sandbox gets a wall clock of its own so an unattended run cannot
# outlive its budget. It has to cover the corpus replay as well as the
# budget, because the replay comes first and is not free.
#
# An open-ended session is on no clock at all, which is what "run until
# interrupted" means: no wall clock, and no allowance on the replay either,
# because the allowance exists to keep a *budget* honest and there is no
# budget here.
if [ "$seconds" -gt 0 ]; then
    wall=$((replay_allowance + seconds + 120))
    allowance=$replay_allowance
    backstop=$((replay_allowance + seconds))
else
    wall=0
    allowance=0
    backstop=0
fi

if $build_only; then
    # `fuzz/sweep.sh` replays stored artifacts by executing the target binary
    # once per file, which still needs the build to happen inside the boundary
    # a campaign would have used. The caller is waiting for this build rather
    # than spending a budget on it, so it keeps the caller's clock.
    contain "$wall" cargo +nightly fuzz build "$target" "$@"
    command=("${contained[@]}")
elif $derive_only; then
    # Reducing the archive means running every input in it, so it is the
    # regression check as well as the reduction -- and it belongs inside the
    # boundary a campaign would have used, behind a build that is not on
    # anybody's budget. `fuzz/corpus.sh` carries its own clock
    # (NSH_FUZZ_MERGE_TIMEOUT), so the boundary does not need a second one.
    contain "$build_timeout" cargo +nightly fuzz build "$target"
    build=("${contained[@]}")
    contain 0 "$root/fuzz/corpus.sh" derive \
        "$binary" "$corpus" "$campaign_corpus" "$artifacts"
    command=("${contained[@]}")
else
    # What a campaign seeds from, and the reason `parse 10` stopped being a
    # four-minute command: the archive when nothing has derived a campaign
    # corpus from it, and that campaign corpus plus whatever has reached the
    # archive since when something has.
    mapfile -t seed_dirs < <("$root/fuzz/corpus.sh" seed "$corpus" "$campaign_corpus")
    ((${#seed_dirs[@]})) || {
        echo "fuzz/run.sh: nothing to seed the campaign from" >&2
        exit 1
    }
    # `cargo fuzz run` compiles first and only then hands the binary its
    # -max_total_time, so a build inside the campaign's wall clock is paid
    # for out of the fuzzing budget. Measured 2026-09-02: a cold-cache
    # `differential 60` spent 54 of its 180 seconds compiling and was killed
    # part-way through the campaign at exit 124 -- which is exactly what the
    # fuzzer stopping at its own timeout looks like, so a run that barely
    # ran reported as a short campaign that found nothing. The build goes
    # first, under a clock of its own, and the campaign below then runs the
    # built binary rather than `cargo fuzz run`, so no part of a second
    # compile -- or of waiting on another campaign's Cargo build lock, which
    # cost a `differential 20` all 140 seconds of its wall clock on
    # 2026-09-02 -- can land inside the clock the budget is on.
    contain "$build_timeout" cargo +nightly fuzz build "$target"
    build=("${contained[@]}")
    # libFuzzer runs the whole seed corpus before it first consults
    # `-max_total_time`, and that clock is measured from process start-up,
    # so on a corpus larger than the budget the budget buys no mutation at
    # all. `fuzz/budget.sh` starts the budget where the mutation starts,
    # which it reads off the fuzzer's own `INITED` line. `-max_total_time`
    # stays as the fail-safe under it, for a supervisor that has died.
    contain "$wall" "$root/fuzz/budget.sh" "$allowance" "$seconds" -- \
        "$binary" "${seed_dirs[@]}" \
        -artifact_prefix="$artifacts/" \
        -max_total_time="$backstop" \
        -print_final_stats=1 \
        -rss_limit_mb=4096 \
        -max_len=65536 \
        "$@"
    command=("${contained[@]}")
fi

printf 'fuzz/run.sh: containment=%s\n' "$containment" >&2
if $dry_run; then
    if ! $build_only; then
        printf 'fuzz/run.sh: build:' >&2
        printf ' %q' "${build[@]}" >&2
        printf '\n' >&2
    fi
    printf 'fuzz/run.sh: command:' >&2
    printf ' %q' "${command[@]}" >&2
    printf '\n' >&2
    exit 0
fi

if $build_only; then
    exec "${command[@]}"
fi

printf 'fuzz/run.sh: building %s before the clock starts\n' "$target" >&2
build_started=$(date +%s)
build_status=0
"${build[@]}" || build_status=$?
build_elapsed=$(($(date +%s) - build_started))
if ((build_status != 0)); then
    if [[ $containment == new ]] && ((build_status == 124)) \
       && ((build_elapsed + 5 >= build_timeout)); then
        printf 'fuzz/run.sh: the build hit its own %ss wall clock; raise NSH_FUZZ_BUILD_TIMEOUT\n' \
            "$build_timeout" >&2
    fi
    printf 'fuzz/run.sh: the build failed after %ss, so nothing was fuzzed\n' "$build_elapsed" >&2
    exit "$build_status"
fi

mkdir -p "$corpus" "$artifacts"

# The build reported success, so the binary the campaign is about to run
# exists. Saying so here is what stops a wrong triple reaching libFuzzer's
# argument parser, where the missing path would be read as an input file.
[[ -x $binary ]] || {
    printf 'fuzz/run.sh: the build left no target binary at %s\n' "$binary" >&2
    exit 1
}

# Seed from the shell text the repository already has. Real scripts are
# far better starting points than random bytes: they reach constructs a
# generator would take a very long time to stumble into, and the corpora
# below are already vendored and licence-cleared.
#
# Named by the SHA-1 of the contents, which is how libFuzzer names everything
# it writes. That is what makes "a name that is not in the archive is an
# input that is not in the archive" true, and `fuzz/corpus.sh` compares
# listings on exactly that. Under the old `seed-N` names a derivation renamed
# each surviving seed to its hash and the copy-back then filed it as a fresh
# find -- harmless, self-limiting, and untrue.
if [ -z "$(ls -A "$corpus" 2>/dev/null)" ]; then
    n=0
    for source in "$root"/tests/surveys/smoosh/shell/*.test \
                  "$root"/tests/surveys/oils/spec/*.test.sh; do
        [ -f "$source" ] || continue
        name=$(sha1sum <"$source" 2>/dev/null | cut -d' ' -f1) || name=
        [ -n "$name" ] || name=seed-$n
        cp "$source" "$corpus/$name" 2>/dev/null || true
        n=$((n + 1))
    done
    echo "seeded $n corpus entries into fuzz/corpus/$target"
fi

campaign_started=$(date +%s)
status=0
"${command[@]}" || status=$?
campaign_elapsed=$(($(date +%s) - campaign_started))

if $derive_only; then
    printf 'fuzz/run.sh: build %ss before the clock; the derivation ran every archived input in %ss\n' \
        "$build_elapsed" "$campaign_elapsed" >&2
    exit "$status"
fi

# libFuzzer writes what it finds into the first corpus directory it was
# given, and that is the campaign corpus whenever there is one -- so the
# archive has not seen those inputs, and the archive is the one that must
# never lose an input. This runs whatever the campaign's status was: a
# campaign that ended on a finding still found the units it found on the way.
"$root/fuzz/corpus.sh" return "$corpus" "$campaign_corpus" || :

if [ "$seconds" -gt 0 ]; then budget="a ${seconds}s budget"; else budget="an open-ended budget"; fi
# The replay and the mutation are both inside this number, and only the
# second of the two is what the budget bought. `fuzz/budget.sh` has already
# printed which was which, off the clock it started at the fuzzer's own
# `INITED` line.
printf 'fuzz/run.sh: build %ss before the clock; replay and campaign %ss for %s\n' \
    "$build_elapsed" "$campaign_elapsed" "$budget" >&2

# A run stopped by the boundary's wall clock exits 124, and so does a
# fuzzer stopped by its own timeout -- so the truncated run used to be
# indistinguishable from a completed short campaign, and reported "no
# artifact" for a campaign that had barely started. That is
# `[spec:nsh:req:oracle.cannot-measure-is-a-failure]` read off the clock,
# so say which clock ended the run. `timeout` is identified by both its
# status and the elapsed time, because 124 is also a status a target could
# choose for itself.
if [[ $containment == new ]] && ((wall > 0)) && ((status == 124)) \
   && ((campaign_elapsed + 5 >= wall)); then
    printf 'fuzz/run.sh: the %ss containment wall clock stopped the campaign, not the %ss fuzzing budget\n' \
        "$wall" "$seconds" >&2
    printf 'fuzz/run.sh: this run measured less than it was asked to, so finding no artifact is not evidence there is none\n' >&2
fi

exit "$status"
