#!/bin/bash
# Self-tests for where the sandbox wrappers spend a command's budget:
# scripts/sandboxed, and tests/harness/sandboxed.sh's ds_sandboxed.
#
# The budget used to be spent from outside: `timeout "$TIMEOUT" sandbox ...`,
# which stops a command by signalling the sandbox process. That is only
# reliable once the sandbox has finished setting up. A signal delivered during
# that window reaps the process this side holds and leaves the process tree
# inside it running, because the teardown that tree depends on had not been
# armed yet -- `86d2ce6` measured 6 leaks in 20 at 0 ms after spawn, 5 in 20
# at 10 ms, and 0 in 15 at 50 ms, in the survey runner, which had the same
# shape and was fixed the same way.
#
# It never leaked at the 900-second default, which is why it lasted: the
# signal cannot land in a setup window nine hundred seconds away. The probe
# below therefore asks for a case-sized budget, which is what `--timeout` and
# `NSH_TEST_TIMEOUT` exist for and where the window reopens.
#
# Like the other two harness self-tests it must not be run through
# scripts/sandboxed: it asks the host whether a descendant of a finished
# command is still running, which is exactly what the boundary hides.
#
#   tests/harness/budget-selftest.sh [RUNS]     # RUNS defaults to 20
set -u

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
WRAPPER=$ROOT/scripts/sandboxed
# Under target/, so the sandbox and the host agree on the path: that is the
# one place a command inside the boundary can write where this script can see
# it afterwards, and seeing it afterwards is the whole measurement.
WORK=$ROOT/target/budget-selftest-$$
# Per budget in the sweep below, so the default is four times this.
RUNS=${1:-20}
failures=0

cleanup() {
    pkill -KILL -f "$WORK" 2>/dev/null || :
    rm -rf "$WORK"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$WORK"

report() { # NAME OK-OR-EMPTY DETAIL
    if [[ -n $2 ]]; then
        printf 'ok   %-44s %s\n' "$1" "$3"
    else
        printf 'FAIL %-44s %s\n' "$1" "$3"
        failures=1
    fi
}

# Zero means no budget -- GNU `timeout`'s own spelling, and what
# `fuzz/run.sh` passes for "run until interrupted". It has to survive the
# backstop: adding five seconds to zero would quietly turn an open-ended
# campaign into a five-second one, so this asks for longer than the backstop
# and expects to get it.
started=$SECONDS
"$WRAPPER" --timeout 0 -- /bin/sh -c 'sleep 7' >/dev/null 2>&1
status=$?
elapsed=$((SECONDS - started))
report "a zero budget still means no budget" \
    "$([[ $status -eq 0 && $elapsed -ge 7 ]] && echo y)" \
    "status=$status elapsed=${elapsed}s"

# ...and it must not collapse the abandoned-process threshold, which defaults
# to this wrapper's budget. A threshold of zero calls every orphan on the
# machine abandoned however young it is, which is the mistake the 2026-09-02
# erratum was written against -- and `--timeout 0` reached it, so an
# open-ended fuzzing campaign refused to start while any orphaned repository
# binary sat at ppid 1. The decoy below is that shape, one second old.
#
# It briefly puts one orphan on the machine, so a concurrent scripts/sandboxed
# run that has lowered NSH_TEST_ABANDONED_AFTER may refuse for the second or
# so that it lives, exactly as tests/harness/abandoned-selftest.sh does.
decoy=$WORK/nsh
cp /bin/sleep "$decoy"
setsid --fork "$decoy" 60 &
sleep 1
orphan=$(pgrep -f "^$decoy 60$" | head -1)
if [[ -z ${orphan:-} ]]; then
    echo "budget-selftest: could not orphan the decoy" >&2
    exit 1
fi
NSH_TEST_TIMEOUT=0 "$WRAPPER" -- /bin/true >/dev/null 2>&1
status=$?
report "no budget does not age out every orphan" \
    "$([[ $status -eq 0 ]] && echo y)" "status=$status pid=$orphan"
kill -KILL "$orphan" 2>/dev/null || :
orphan=

# A case-sized budget has to be expressible, or a focused run cannot ask for
# one without also giving the sandbox that long to start.
"$WRAPPER" --timeout 0.5 -- /bin/true >/dev/null 2>&1
status=$?
report "a fractional budget is accepted" \
    "$([[ $status -eq 0 ]] && echo y)" "status=$status"

# The budget still ends a command that overstays it, and still reports 124.
started=$SECONDS
"$WRAPPER" --timeout 1 -- /bin/sh -c 'sleep 30' >/dev/null 2>&1
status=$?
elapsed=$((SECONDS - started))
report "an overstaying command is stopped" \
    "$([[ $status -eq 124 && $elapsed -lt 15 ]] && echo y)" \
    "status=$status elapsed=${elapsed}s"

# ...including one that refuses the signal. The inner budget follows its TERM
# with a KILL a second later, so this cannot outlast the outer backstop.
started=$SECONDS
"$WRAPPER" --timeout 1 -- /bin/sh -c 'trap "" TERM; while :; do :; done' >/dev/null 2>&1
status=$?
elapsed=$((SECONDS - started))
report "a command that ignores TERM is stopped" \
    "$([[ $status -ne 0 && $elapsed -lt 15 ]] && echo y)" \
    "status=$status elapsed=${elapsed}s"

# The differential harness is the third wrapper with this shape, and its
# inner budget had no KILL behind it: a case that ignored TERM outlived the
# inner deadline and was stopped by the outer one, five seconds later, by a
# signal aimed at the sandbox from outside the boundary. Measured at load
# 21.1 with a 2-second budget, 5 runs each: 7.00s and status 124 without the
# KILL, 3.01s and status 137 with it. Five seconds is the mark between them.
#
# All three of status, floor and ceiling are asserted. 137 is the inner
# `timeout`'s KILL, which is the mechanism being claimed; the floor is what
# says the case ran its budget rather than failing to start, and an earlier
# spelling without it reported `ok status=1 elapsed=0s` for a sandbox that
# never came up.
#
# `sandboxed.sh` sets ROOT itself, from DASH_ROOT when that is exported, so
# it is put back afterwards rather than trusted to agree.
budget_selftest_root=$ROOT
. "$ROOT/tests/harness/sandboxed.sh"
ROOT=$budget_selftest_root
differential=$WORK/differential
mkdir -p "$differential"
started=$SECONDS
DS_TIMEOUT=2 ds_sandboxed "$differential" /bin/sh -c \
    'trap "" TERM; while :; do sleep 0.2; done' >/dev/null 2>&1
status=$?
elapsed=$((SECONDS - started))
report "the differential wrapper stops a TERM-ignoring case" \
    "$([[ $status -eq 137 && $elapsed -ge 2 && $elapsed -lt 5 ]] && echo y)" \
    "status=$status elapsed=${elapsed}s"

# A command that finishes inside its budget still reports its own status,
# which is what says the two timeouts are a backstop and not a second answer.
"$WRAPPER" --timeout 30 -- /bin/sh -c 'exit 7' >/dev/null 2>&1
status=$?
report "a command inside its budget keeps its status" \
    "$([[ $status -eq 7 ]] && echo y)" "status=$status"

# THE LEAK. Each run backgrounds a descendant that writes a marker one second
# from now, then overstays a budget small enough to fire while the sandbox is
# still coming up. If the budget is spent from outside, the signal reaps the
# sandbox process and leaves its tree running: the marker appears, and the
# descendants are still on the host afterwards. If it is spent inside, the
# namespace goes and takes them with it.
#
# The budget is swept rather than fixed, because the window is a property of
# the machine and the load rather than a number anybody chose. Measured
# against the previous shape at load 21 on this machine:
#
#   1 ms    0 of 20 leaked   -- the signal beats the command into existence
#   2 ms    0 of 20
#   5 ms   17 of 20 leaked, 27 descendants still running afterwards
#  10 ms   14 of 20 leaked, 12 still running
#  20 ms    1 of 20 leaked,  2 still running
#  30 ms    0 of 20          -- setup is done, the teardown is armed
#  50 ms    0 of 20
#
# A single point would therefore have measured nothing on a faster machine or
# a quieter one, and 50 ms -- the first budget tried here -- measured exactly
# that: it is past the window, and the old shape passed it.
leaked=0
survived=0
for budget in 0.002 0.005 0.010 0.020; do
    for ((run = 0; run < RUNS; run++)); do
        "$WRAPPER" --timeout "$budget" -- /bin/sh -c \
            "( sleep 1; printf leaked >'$WORK/leaked-$budget-$run' ) </dev/null >/dev/null 2>&1 &
             sleep 3" >/dev/null 2>&1
    done
    # Long enough for every descendant that outlived its run to have written.
    sleep 2
    leaked=$((leaked + $(find "$WORK" -name "leaked-$budget-*" | wc -l)))
    survived=$((survived + $(pgrep -f "$WORK" | wc -l)))
    pkill -KILL -f "$WORK" 2>/dev/null || :
done
report "a budget spent inside leaks no descendant" \
    "$([[ $leaked -eq 0 && $survived -eq 0 ]] && echo y)" \
    "leaked=$leaked survived=$survived of $((RUNS * 4)) runs at load $(cut -d' ' -f1 /proc/loadavg)"

if ((failures)); then
    echo "budget-selftest: FAILED"
    exit 1
fi
echo "budget-selftest: all checks passed"
