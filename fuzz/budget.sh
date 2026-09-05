#!/usr/bin/env bash
# Start a fuzzing budget when the fuzzing starts.
#
#   fuzz/budget.sh REPLAY_ALLOWANCE BUDGET -- COMMAND [ARG...]
#
# libFuzzer runs every input in the seed corpus before it first consults
# `-max_total_time`, and that clock is measured from process start-up, so a
# campaign whose corpus takes longer to replay than its budget mutates
# nothing whatsoever. Measured 2026-09-02 on a warm build: `fuzz/run.sh
# parse 10` reached the fuzzing loop at run #21259 and stopped at run
# #21259 -- the corpus, exactly, and not one mutation past it.
#
# The replay is not waste. It is every stored input run against the build in
# front of it, which is the regression check the corpus exists to be, and a
# campaign that skipped it would be fuzzing against evidence it had not
# looked at. So it is kept in full and given a clock of its own instead.
#
# libFuzzer prints `#N<tab>INITED` at the moment the replay ends and the
# mutation begins. That line is the only honest place a budget can start
# from: it is measured, not estimated, so it stays right however large the
# corpus grows. BUDGET seconds later the campaign is asked to stop, which
# libFuzzer does gracefully -- it prints its final statistics and exits
# zero, so a budgeted campaign still reports like a completed one.
#
# REPLAY_ALLOWANCE bounds the other end: a corpus that cannot be replayed
# inside it stops the run at a named clock with a named knob, rather than
# leaving a campaign that mutates nothing to look like one that mutated and
# found nothing. Either allowance may be 0, meaning no limit.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: fuzz/budget.sh REPLAY_ALLOWANCE BUDGET -- COMMAND [ARG...]

  REPLAY_ALLOWANCE  seconds the seed-corpus replay may take (0: no limit)
  BUDGET            seconds of mutation after the replay ends (0: no limit)
EOF
    exit 2
}

(($# >= 4)) || usage
allowance=$1
budget=$2
shift 2
[[ $1 == -- ]] || usage
shift
(($#)) || usage
for value in "$allowance" "$budget"; do
    case $value in
        *[!0-9]*|'') usage ;;
    esac
done

work=$(mktemp -d "${TMPDIR:-/tmp}/nsh-fuzz-budget.XXXXXX")
cleanup() { rm -rf -- "$work"; }
trap cleanup EXIT

# libFuzzer reports on stderr. Capture it to a file rather than reading it
# down a pipe: a finding's report carries the input that produced it, and
# bytes a shell `read` would mangle are exactly the bytes a fuzzer finds.
# `tail` mirrors the file through unchanged and `--pid` makes it stop, after
# a last read, when the campaign does.
stream="$work/stream"
: >"$stream"

"$@" 2>"$stream" &
child=$!
tail -c +1 -f --pid="$child" -- "$stream" >&2 &
mirror=$!

started=$(date +%s)
inited=0
stopped=

while :; do
    kill -0 "$child" 2>/dev/null || break
    now=$(date +%s)
    if ((inited == 0)) \
       && grep -aqm1 -E '^#[0-9]+[[:space:]]+INITED[[:space:]]' "$stream"; then
        inited=$now
        if ((budget > 0)); then
            printf 'fuzz/budget.sh: the corpus replay ended after %ss; the %ss budget starts here\n' \
                "$((inited - started))" "$budget" >&2
        else
            printf 'fuzz/budget.sh: the corpus replay ended after %ss; the campaign is open-ended\n' \
                "$((inited - started))" >&2
        fi
    fi
    if ((inited == 0)) && ((allowance > 0)) && ((now - started >= allowance)); then
        stopped=replay
        break
    fi
    if ((inited != 0)) && ((budget > 0)) && ((now - inited >= budget)); then
        stopped=budget
        break
    fi
    /bin/sleep 1
done

if [[ -n $stopped ]]; then
    # libFuzzer handles SIGTERM by printing its statistics and exiting at
    # its interrupt status, which is why the campaign is asked rather than
    # killed. The campaign keeps this script's process group, so an
    # interrupt at the terminal still reaches it directly; nothing here
    # changes that.
    kill -TERM "$child" 2>/dev/null || :
    for _ in $(seq 1 30); do
        kill -0 "$child" 2>/dev/null || break
        /bin/sleep 1
    done
    kill -KILL "$child" 2>/dev/null || :
fi

status=0
wait "$child" || status=$?
wait "$mirror" 2>/dev/null || :
finished=$(date +%s)

case $stopped in
    # A campaign whose replay ate the whole allowance mutated for zero
    # seconds. It must not exit the way one that mutated and found nothing
    # exits, because the two are indistinguishable from the status alone
    # and only one of them is evidence.
    # [spec:nsh:req:oracle.cannot-measure-is-a-failure]
    replay)
        printf 'fuzz/budget.sh: the corpus replay did not reach the fuzzing loop within %ss, so nothing was mutated\n' \
            "$allowance" >&2
        printf 'fuzz/budget.sh: raise NSH_FUZZ_REPLAY_ALLOWANCE or reduce the corpus; this run is not evidence of anything\n' >&2
        exit 124
        ;;
    budget)
        printf 'fuzz/budget.sh: replay %ss, then %ss of mutation for a %ss budget\n' \
            "$((inited - started))" "$((finished - inited))" "$budget" >&2
        # The campaign ran its whole budget and was then stopped on purpose,
        # so it succeeded. 72 is what libFuzzer's interrupt handler exits
        # with, and 143 and 137 are a target that does not handle SIGTERM
        # dying by it -- all three are this script's own signal coming back,
        # not a finding. A finding has its own statuses and keeps them: 77
        # for a crash, 70 for a timeout, 71 for the memory limit.
        if ((status == 72 || status == 143 || status == 137)); then
            status=0
        fi
        ;;
    *)
        if ((inited != 0)); then
            printf 'fuzz/budget.sh: replay %ss, then %ss of mutation before the campaign ended by itself\n' \
                "$((inited - started))" "$((finished - inited))" >&2
        else
            printf 'fuzz/budget.sh: the campaign ended after %ss without reaching the fuzzing loop\n' \
                "$((finished - started))" >&2
        fi
        ;;
esac

exit "$status"
