#!/bin/bash
# Self-tests for the free-space check in scripts/sandboxed.
#
# The check exists because a full filesystem does not report itself. On
# 2026-09-02 it reported eight simultaneous `ld terminated with signal 7
# [Bus error]` failures under an LLVM stack trace inviting an upstream bug
# report, and in the same hour a `rustc` ICE carrying no message at all. Only
# the third signature of the three, `cargo`'s ENOSPC, contained the word
# "space", and an hour went into the first before anybody ran `df`.
#
#   tests/harness/disk-selftest.sh
#
# Unlike tests/harness/abandoned-selftest.sh and tests/harness/budget-selftest.sh
# this one may be run through the wrapper: it asks nothing of the host that the
# boundary hides. It cannot fill a filesystem to test a refusal -- doing so
# would be the incident rather than a test of it -- so it moves the threshold
# across the free space instead, which exercises the same comparison from the
# other side.
set -u

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
WRAPPER=$ROOT/scripts/sandboxed
# Every case here is about the check in front of the command, so the command
# behind the `--` is only ever a marker that the run got past it.
MARKER=reached-the-command
failures=0

report() { # NAME OK-OR-EMPTY DETAIL
    if [[ -n $2 ]]; then
        printf 'ok   %-48s %s\n' "$1" "$3"
    else
        printf 'FAIL %-48s %s\n' "$1" "$3"
        failures=1
    fi
}

# The same product the check uses, and the one `df` prints as "Avail".
free_mib=$(($(/usr/bin/stat -f -c '%a' "$ROOT") * $(/usr/bin/stat -f -c '%S' "$ROOT") / 1048576))
# One threshold this machine cannot meet and one it cannot fail, derived from
# what is actually free rather than from constants that would rot.
too_much=$((free_mib + 1024))
plenty=1

output=$(NSH_TEST_DISK_MIN=$plenty "$WRAPPER" -- /bin/echo "$MARKER" 2>&1)
status=$?
report "room on the disk is not refused" \
    "$([[ $status -eq 0 && $output == "$MARKER" ]] && echo y)" \
    "status=$status free=${free_mib}MiB"

output=$(NSH_TEST_DISK_MIN=$too_much "$WRAPPER" -- /bin/echo "$MARKER" 2>&1)
status=$?
report "too little is refused before the command" \
    "$([[ $status -eq 1 && $output != *"$MARKER"* ]] && echo y)" \
    "status=$status"

# The numbers are the whole point: a refusal that does not say how much is
# free and how much was wanted sends the reader back to `df` anyway.
report "the refusal names the path and both numbers" \
    "$([[ $output == *"$ROOT/target"* && $output == *GiB\ free* && $output == *under*GiB* ]] && echo y)" \
    "$(printf '%s' "$output" | head -1)"

# This line is what the node was filed for. An agent looking at an LLVM stack
# trace has no reason to suspect the disk, so the refusal has to name the
# trace and take the suspicion off the compiler.
report "the refusal names the misleading signature" \
    "$([[ $output == *"signal 7"* && $output == *ICE* ]] && echo y)" \
    "signature lines present"

# A second filesystem among the writable binds is a second question. The
# worktree-with-its-own-CARGO_TARGET_DIR practice this was filed for puts the
# build output somewhere other than $ROOT/target, and a check that only ever
# looked at $ROOT/target would pass a machine whose build had nowhere to go.
# The refusal happens before the sandbox is built, so binding /tmp writable
# here never reaches a containment argument.
if [[ $(/usr/bin/stat -c '%d' /tmp) != $(/usr/bin/stat -c '%d' "$ROOT/target") ]]; then
    output=$(NSH_TEST_DISK_MIN=$((too_much * 8)) "$WRAPPER" --writable /tmp -- /bin/echo "$MARKER" 2>&1)
    status=$?
    named=$(printf '%s\n' "$output" | grep -c ' is on ')
    report "each writable filesystem is asked separately" \
        "$([[ $status -eq 1 && $named -eq 2 ]] && echo y)" \
        "status=$status filesystems-named=$named"
else
    report "each writable filesystem is asked separately" y \
        "skipped: /tmp and $ROOT/target are one filesystem here"
fi

# Two paths on one filesystem are one question. Asking it twice would print
# the same refusal twice and read as two problems.
output=$(NSH_TEST_DISK_MIN=$too_much "$WRAPPER" --writable "$ROOT/target" -- /bin/echo "$MARKER" 2>&1)
named=$(printf '%s\n' "$output" | grep -c ' is on ')
report "one filesystem is asked once" \
    "$([[ $named -eq 1 ]] && echo y)" \
    "filesystems-named=$named"

output=$(NSH_TEST_DISK=warn NSH_TEST_DISK_MIN=$too_much "$WRAPPER" -- /bin/echo "$MARKER" 2>&1)
status=$?
report "NSH_TEST_DISK=warn says so and runs" \
    "$([[ $status -eq 0 && $output == *"$MARKER"* && $output == *"signal 7"* ]] && echo y)" \
    "status=$status"

output=$(NSH_TEST_DISK=ignore NSH_TEST_DISK_MIN=$too_much "$WRAPPER" -- /bin/echo "$MARKER" 2>&1)
status=$?
report "NSH_TEST_DISK=ignore runs and says nothing" \
    "$([[ $status -eq 0 && $output == "$MARKER" ]] && echo y)" \
    "status=$status output=[$output]"

output=$(NSH_TEST_DISK=nonsense "$WRAPPER" -- /bin/echo "$MARKER" 2>&1)
status=$?
report "an unknown mode is refused, not assumed" \
    "$([[ $status -eq 2 && $output != *"$MARKER"* ]] && echo y)" \
    "status=$status"

output=$(NSH_TEST_DISK_MIN=nonsense "$WRAPPER" -- /bin/echo "$MARKER" 2>&1)
status=$?
report "a non-numeric threshold is refused" \
    "$([[ $status -eq 2 && $output != *"$MARKER"* ]] && echo y)" \
    "status=$status"

# Cargo calls the runner once per test binary, and it calls it after every
# link has already happened -- so the question this check asks has been
# answered by then, and asking it again could refuse in the middle of a suite
# for a filesystem that filled while the suite ran. The abandoned scan is
# skipped in runner mode for the same reason.
output=$(NSH_TEST_DISK_MIN=$too_much "$WRAPPER" --cargo-runner -- /bin/echo "$MARKER" 2>&1)
status=$?
report "the cargo runner is never refused for space" \
    "$([[ $status -eq 0 && $output == "$MARKER" ]] && echo y)" \
    "status=$status output=[$output]"

if ((failures)); then
    echo "disk-selftest: FAILED"
    exit 1
fi
echo "disk-selftest: all checks passed"
