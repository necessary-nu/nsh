#!/bin/bash
# Self-tests for the abandoned-process check in scripts/sandboxed.
#
# The check exists because four `nsh` processes spun at 98% CPU for
# forty-seven hours on 2026-08-31 after the worktree, the binary and the
# case files they came from had all been deleted. They were in the host's
# PID namespace at ppid 1, so no boundary and no timeout could reach them,
# and nothing on this machine noticed. The check is what notices.
#
# It cannot be run through scripts/sandboxed: the whole point is that it
# looks at the host's process table, which is exactly what the boundary
# hides. It runs unsandboxed, deliberately, and it starts no shell and
# reads no case -- the only processes it creates are copies of /bin/sleep.
#
#   tests/harness/abandoned-selftest.sh
#
# It briefly puts one abandoned process on the machine, so a concurrent
# scripts/sandboxed may refuse for the second or so that it lives. That is
# the check working.
set -u

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
WRAPPER=$ROOT/scripts/sandboxed
WORK=$ROOT/target/abandoned-selftest-$$
# The check names the binaries this repository builds, so the decoy has to
# carry one of those names; the second copy carries a name it does not.
DECOY=$WORK/nsh
STRANGER=$WORK/someone-elses-test-binary
failures=0

cleanup() {
    [[ -z ${orphan:-} ]] || kill -KILL "$orphan" 2>/dev/null || :
    [[ -z ${stranger:-} ]] || kill -KILL "$stranger" 2>/dev/null || :
    rm -rf "$WORK"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$WORK"
cp /bin/sleep "$DECOY"
cp /bin/sleep "$STRANGER"

report() { # NAME OK-OR-EMPTY DETAIL
    if [[ -n $2 ]]; then
        printf 'ok   %-44s %s\n' "$1" "$3"
    else
        printf 'FAIL %-44s %s\n' "$1" "$3"
        failures=1
    fi
}

# A machine with nothing abandoned on it runs commands as it always did.
output=$("$WRAPPER" -- /bin/echo clean 2>&1)
status=$?
report "a clean machine is not refused" \
    "$([[ $status -eq 0 && $output == clean ]] && echo y)" \
    "status=$status output=[$output]"

# A contained run is not abandoned, however long it lasts: it is in its own
# PID namespace, which is the property the check tests. Without that test a
# concurrent test run would look exactly like the incident.
"$WRAPPER" --timeout 20 -- "$DECOY" 8 &
contained=$!
sleep 2
output=$("$WRAPPER" -- /bin/echo concurrent 2>&1)
status=$?
report "a contained run is not called abandoned" \
    "$([[ $status -eq 0 && $output == concurrent ]] && echo y)" \
    "status=$status output=[$output]"
kill -KILL "$contained" 2>/dev/null || :
wait "$contained" 2>/dev/null || :

# This machine carries checkouts of other projects, and their orphaned test
# binaries also live under a `target/` directory. Refusing to run because of
# one of those would be a wrapper that cries wolf about work it has no claim
# over, so the name is part of the fingerprint and this proves it.
setsid --fork "$STRANGER" 300 &
sleep 0.3
stranger=$(pgrep -f "^$STRANGER 300$" | head -1)
if [[ -z ${stranger:-} ]]; then
    echo "abandoned-selftest: could not orphan the stranger" >&2
    exit 1
fi
output=$("$WRAPPER" -- /bin/echo stranger 2>&1)
status=$?
report "another project's orphan is left alone" \
    "$([[ $status -eq 0 && $output == stranger ]] && echo y)" \
    "status=$status output=[$output]"
kill -KILL "$stranger" 2>/dev/null || :
stranger=

# The incident's own shape: a process built into target/, reparented to
# init, in this process's PID namespace. `setsid --fork` forks and exits, so
# the copy is at ppid 1 before it has run a millisecond.
setsid --fork "$DECOY" 300 &
sleep 0.3
orphan=$(pgrep -f "^$DECOY 300$" | head -1)
if [[ -z ${orphan:-} ]]; then
    echo "abandoned-selftest: could not orphan the decoy" >&2
    exit 1
fi

output=$("$WRAPPER" -- /bin/echo unreachable 2>&1)
status=$?
report "an abandoned process is refused" \
    "$([[ $status -eq 1 && $output == *"$orphan"* && $output != *unreachable* ]] && echo y)" \
    "status=$status"

output=$(NSH_TEST_ABANDONED=ignore "$WRAPPER" -- /bin/echo ignored 2>&1)
status=$?
report "NSH_TEST_ABANDONED=ignore runs anyway" \
    "$([[ $status -eq 0 && $output == ignored ]] && echo y)" \
    "status=$status output=[$output]"
report "ignore leaves the process alive" \
    "$(kill -0 "$orphan" 2>/dev/null && echo y)" \
    "pid=$orphan"

output=$(NSH_TEST_ABANDONED=kill "$WRAPPER" -- /bin/echo killed 2>&1)
status=$?
report "NSH_TEST_ABANDONED=kill clears and runs" \
    "$([[ $status -eq 0 && $output == *killed* ]] && echo y)" \
    "status=$status output=[$output]"
sleep 0.3
report "kill leaves nothing behind" \
    "$(kill -0 "$orphan" 2>/dev/null || echo y)" \
    "pid=$orphan"
orphan=

output=$(NSH_TEST_ABANDONED=nonsense "$WRAPPER" -- /bin/echo unreachable 2>&1)
status=$?
report "an unknown mode is refused, not assumed" \
    "$([[ $status -eq 2 && $output != *unreachable* ]] && echo y)" \
    "status=$status"

if ((failures)); then
    echo "abandoned-selftest: FAILED"
    exit 1
fi
echo "abandoned-selftest: all checks passed"
