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
#
# TWO CLASSES OF ARTIFACT, AND ONLY ONE OF THEM FAILS ITS REPLAY. libFuzzer
# files a `crash-`, `leak-` or `oom-` artifact for an input the target could
# not survive, and a replay of one of those exits non-zero. It files a
# `slow-unit-` or a `timeout-` for an input the target survived and took too
# long over -- and a replay of one of *those* exits zero, because nothing
# failed. Deciding liveness by status alone therefore reports every
# performance finding as closed, most eagerly right after a campaign has
# found one, which is when the sweep gets run. Measured on a tree carrying
# the alternation defect `2c40a0e` closed: the sweep that read only the
# status reported all five `matcher` slow units "no longer reproduce" and
# exited 0, while each of them still took between seven and eleven seconds.
# `--prune` would have deleted five live findings. That is the shape
# `[spec:nsh:req:oracle.cannot-measure-is-a-failure]` refuses: a check that
# cannot see a class of result reports the class as absent.
#
# So a cost artifact is judged on what it cost, and the number is one the
# replay already prints. The threshold cannot be a wall clock: the same
# `matcher` artifact read 1.16s on a quiet machine and 25.27s an hour later
# under load, and a verdict that moves by twenty times with the machine is
# not a verdict. It is measured instead against what an ordinary input of
# the same target costs *in the same replay* -- see `measure_artifact` below
# -- so that whatever the machine is doing to one number it is doing to the
# other.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: fuzz/sweep.sh [--containment auto|outer|new] [--prune] [TARGET...]

  --prune   remove the artifacts that no longer reproduce (default: report)

Environment:
  NSH_SWEEP_COST_ALLOWANCE   what a cost artifact may cost, as a multiple of
                             one ordinary input of the same target (default 256)
  NSH_SWEEP_CALIBRATION      corpus inputs sampled to establish that cost
                             (default 256; the cheaper half is kept and at
                             least 16 must remain for a verdict)
  NSH_SWEEP_TIMEOUT          seconds one replay may take (default 60)
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

# What one input is allowed to cost before it is a finding, as a multiple of
# what an ordinary input of the same target costs. The five `matcher`
# artifacts, measured 2026-09-02 against the tree that has the alternation
# defect and against the tree that fixed it -- nine sweeps of the fixed tree
# at loads from 8 to 95, one of them starved on a single contended core, and
# three of the other at loads 12, 20 and 94:
#
#                     with the defect        fixed
#     08eff044f729     2,282x - 3,082x    72x -  83x
#     2abfc17424f2     2,667x - 3,439x     7x -  10x
#     8712dff269ac     2,621x - 3,411x    56x -  69x
#     be14ca8801b3     2,941x - 3,423x    71x -  93x
#     d771ff817c9f     2,694x - 3,294x   480x - 587x
#
# The spread within a column is under a third across a load that moved by a
# factor of twelve, which is the arrangement working: the tick and the
# artifact are measured on the same machine seconds apart, so what the
# machine is doing divides out. Between the columns is a factor of thirty.
#
# 256 sits in that gap at a factor of three above the closed four and a
# factor of nine below the defect, deliberately at the low end rather than
# the middle, because the sweep's two errors do not cost the same: calling a
# closed finding live wastes a triage, and calling a live one closed lets
# `--prune` delete it.
#
# `d771ff817c9f` is over the line on both trees and is meant to be. It is the
# artifact `e6551ef` brought from 11.85s to 1.16s rather than to tenths, and
# `stop-selecting-the-locale-per-character` is the open node that says why:
# it still costs five hundred ordinary inputs where its four siblings cost
# under a hundred. No allowance can call it closed and the four closed with
# room to spare -- the two are only a factor of six apart -- and of the two
# placements that is the one that keeps a finding.
cost_allowance=${NSH_SWEEP_COST_ALLOWANCE:-256}
calibration_sample=${NSH_SWEEP_CALIBRATION:-256}
# Below this many corpus inputs the calibration is noise, and an unmeasured
# artifact is reported undecided rather than guessed at.
calibration_minimum=16
replay_timeout=${NSH_SWEEP_TIMEOUT:-60}
for value in "$cost_allowance" "$calibration_sample" "$replay_timeout"; do
    case $value in
        *[!0-9]*|'') echo "fuzz/sweep.sh: expected a non-negative integer, got: $value" >&2; exit 2 ;;
    esac
done

if ((${#targets[@]} == 0)); then
    for directory in "$root"/fuzz/artifacts/*/; do
        [[ -d $directory ]] || continue
        compgen -G "$directory*" >/dev/null || continue
        targets+=("$(basename "$directory")")
    done
fi
((${#targets[@]})) || { echo "fuzz/sweep.sh: no artifacts to sweep" >&2; exit 0; }

work=$(mktemp -d "${TMPDIR:-/tmp}/nsh-fuzz-sweep.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

# A libFuzzer report carries the bytes that produced it, and bytes a `$(...)`
# would mangle are exactly the bytes a fuzzer finds. Replays write to a file
# and it is read with `grep -a`.
transcript="$work/transcript"

# EPOCHREALTIME is seconds and microseconds around a decimal separator the
# locale chooses, so the separator is dropped rather than assumed. libFuzzer
# prints whole milliseconds per input, which is a quantum at the one and two
# milliseconds an ordinary input of these targets costs -- the same
# calibration set read 205 and 124 in consecutive runs of one binary -- so
# the calibration is timed here instead, and only the artifact's own cost,
# which is the large number, is read off libFuzzer's line.
microseconds() { local LC_ALL=C; printf '%s' "${EPOCHREALTIME//[.,]/}"; }

# Replay FILES in one process. Sets `replay_us` (wall microseconds),
# `replay_status`, and `replay_ms` (libFuzzer's per-input milliseconds, in
# argument order).
replay() {
    local started finished
    started=$(microseconds)
    replay_status=0
    timeout "$replay_timeout" "$binary" "$@" >"$transcript" 2>&1 || replay_status=$?
    finished=$(microseconds)
    replay_us=$((finished - started))
    mapfile -t replay_ms < <(grep -a '^Executed .* in [0-9]* ms$' "$transcript" |
        awk '{ print $(NF - 1) }')
}

# Choose the inputs an artifact's cost is measured against. Sets
# `calibration`, or fails when the corpus cannot say.
#
# The sample is deterministic -- every k-th name in sorted order -- so two
# sweeps of one corpus calibrate against the same inputs. Only the cheaper
# half of it is kept: a corpus accumulates inputs of the same shape as the
# artifacts filed beside it, and on the tree with the `matcher` defect the
# sampled corpus held an input costing 7,082ms, which averaged in would have
# hidden a 9,069ms artifact behind it. The cheap half is the target's own
# floor -- building a shell and running its scripts -- and the machine's
# speed is what is wanted from it, not the code under test.
choose_calibration() {
    local corpus=$1
    local -a all sample ranked
    local n step i

    compgen -G "$corpus/*" >/dev/null || return 1
    mapfile -t all < <(cd "$corpus" && LC_ALL=C ls -A | LC_ALL=C sort)
    n=${#all[@]}
    step=$(((n + calibration_sample - 1) / calibration_sample))
    ((step > 0)) || step=1
    for ((i = 0; i < n; i += step)); do sample+=("$corpus/${all[i]}"); done

    # One pass to find out which of the sample are the ordinary ones. The
    # milliseconds are too coarse to be a clock and quite good enough to be
    # an order.
    replay "${sample[@]}"
    ((replay_status == 0)) || return 1
    ((${#replay_ms[@]} == ${#sample[@]})) || return 1
    mapfile -t ranked < <(
        for i in "${!sample[@]}"; do printf '%s %s\n' "${replay_ms[i]}" "$i"; done |
            LC_ALL=C sort -n -k1,1 -k2,2n | awk '{ print $2 }'
    )
    calibration=()
    for i in "${ranked[@]:0:$((${#ranked[@]} / 2))}"; do calibration+=("${sample[i]}"); done
    ((${#calibration[@]} >= calibration_minimum))
}

# Measure one artifact against the calibration. Sets `cost_us` and `tick_us`;
# returns 1 when the replay did not survive the artifact and 2 when the two
# replays did not come out in an order anything can be divided by.
#
# Two replays, back to back, and the artifact rides inside the second one:
#
#     one   = start-up + one ordinary input
#     many  = start-up + H ordinary inputs + the artifact
#
# so `(many - one - artifact) / (H - 1)` is what an ordinary input costs and
# the process start-up -- ASan's, the loader's, the coverage tables' -- falls
# out of the subtraction instead of being charged to it. Taking the two
# points seconds apart rather than minutes is the point of the arrangement:
# an earlier draft calibrated once per target and divided artifacts measured
# later by it, and on a machine whose load moved from 4 to 12 between the two
# it read an ordinary input at 7,319us and called every artifact closed.
#
# The artifact's own cost is read off libFuzzer's `Executed ... in N ms`
# line, which is whole milliseconds -- a quantum for an ordinary input, and
# three decimal places for an artifact that is a finding.
measure_artifact() {
    local artifact=$1 one_us many_us
    replay "${calibration[0]}"
    ((replay_status == 0)) || return 1
    one_us=$replay_us
    replay "${calibration[@]}" "$artifact"
    # A non-zero status is a crash or the replay timeout; a missing line is
    # an execution that never finished. Either is a finding, and a louder
    # one than being slow.
    ((replay_status == 0)) || return 1
    ((${#replay_ms[@]} == ${#calibration[@]} + 1)) || return 1
    many_us=$replay_us
    cost_us=$((replay_ms[${#calibration[@]}] * 1000))
    ((many_us > one_us + cost_us)) || return 2
    tick_us=$(((many_us - one_us - cost_us) / (${#calibration[@]} - 1)))
    ((tick_us > 0)) || return 2
}

# What class of finding an artifact records. libFuzzer says so in the name it
# files it under, and the two classes need different questions asked of them.
class_of() {
    case ${1##*/} in
        slow-unit-*|timeout-*) printf 'cost' ;;
        *) printf 'status' ;;
    esac
}

# The replay itself is `cargo fuzz build` plus one execution per file, and
# both belong inside whatever boundary a campaign would have run in.
triple=$(rustc -vV | sed -n 's/^host: //p')
live_total=0
dead_total=0
undecided_total=0

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

    # Choosing the calibration costs a second and is only wanted when this
    # target has an artifact whose finding is a cost.
    calibration=()
    for artifact in "$directory"/*; do
        [[ -f $artifact ]] || continue
        [[ $(class_of "$artifact") == cost ]] || continue
        choose_calibration "$root/fuzz/corpus/$target" || {
            printf 'fuzz/sweep.sh: %s: the corpus cannot say what an ordinary input costs\n' \
                "$target" >&2
            printf 'fuzz/sweep.sh: %s: its cost artifacts are undecided, not closed\n' \
                "$target" >&2
        }
        break
    done

    live=()
    dead=()
    undecided=()
    report=()
    for artifact in "$directory"/*; do
        [[ -f $artifact ]] || continue
        name=${artifact##*/}
        if [[ $(class_of "$artifact") == status ]]; then
            # A libFuzzer target given one file runs that input and exits
            # non-zero when it fails. A timeout counts as still failing: a
            # hang is a finding.
            if timeout "$replay_timeout" "$binary" "$artifact" >/dev/null 2>&1; then
                dead+=("$artifact")
            else
                live+=("$artifact")
                report+=("  live      $name")
            fi
            continue
        fi

        if ((${#calibration[@]} == 0)); then
            undecided+=("$artifact")
            report+=("  undecided $name, with nothing to measure it against")
            continue
        fi

        cost_us=0
        tick_us=0
        measured=0
        measure_artifact "$artifact" || measured=$?
        if ((measured == 1)); then
            live+=("$artifact")
            report+=("  live      $name, whose replay did not survive it")
            continue
        elif ((measured != 0)); then
            undecided+=("$artifact")
            report+=("  undecided $name, whose two replays could not be divided")
            continue
        fi
        multiple=$((cost_us / tick_us))
        if ((multiple > cost_allowance)); then
            live+=("$artifact")
            report+=("  live      $name, ${multiple}x an ordinary input against an allowance of ${cost_allowance}x")
        else
            dead+=("$artifact")
            report+=("  dead      $name, ${multiple}x an ordinary input against an allowance of ${cost_allowance}x")
        fi
    done

    printf '%s: %d live, %d dead' "$target" "${#live[@]}" "${#dead[@]}"
    if ((${#undecided[@]})); then printf ', %d undecided' "${#undecided[@]}"; fi
    printf '\n'
    for line in "${report[@]}"; do printf '%s\n' "$line"; done
    if $prune; then
        for artifact in "${dead[@]}"; do
            rm -f -- "$artifact"
        done
        if ((${#dead[@]})); then printf '  pruned %d closed artifact(s)\n' "${#dead[@]}"; fi
    elif ((${#dead[@]})); then
        printf '  %d artifact(s) no longer reproduce; --prune removes them\n' "${#dead[@]}"
    fi

    live_total=$((live_total + ${#live[@]}))
    dead_total=$((dead_total + ${#dead[@]}))
    undecided_total=$((undecided_total + ${#undecided[@]}))
done

printf 'total: %d live, %d dead' "$live_total" "$dead_total"
if ((undecided_total)); then printf ', %d undecided' "$undecided_total"; fi
printf '\n'
# An artifact nothing could measure is not an artifact that reproduced, and
# it is not one that stopped either. Reporting the sweep clean over it would
# be the same mistake in a second place.
((live_total == 0 && undecided_total == 0))
