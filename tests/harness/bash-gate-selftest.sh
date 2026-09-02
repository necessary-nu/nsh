#!/bin/bash
# Negative self-tests for the Bash closure gate.
#
# `nsh-survey gate-bash` decides whether a non-passing case is expected, so
# the gate is only worth its exit status if it refuses the ways a register
# can go wrong. Each mutation below is applied to a *copy* of the survey
# root and must be refused, for the stated reason. A gate that accepts one
# of them is a gate that would wave a regression through.
#
# Run it through the containment wrapper, like every other executable here:
#
#   scripts/sandboxed --timeout 2400 -- tests/harness/bash-gate-selftest.sh
#
# The copy lives under target/, which the wrapper already makes writable;
# naming it with --writable would bind it as a mount point and the copy
# could not be replaced.
#
# It needs target/release/nsh-survey and a shell installed as
# target/bash-mode/bash -- the gate refuses any other basename, which is
# itself one of the cases below.
set -u

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
COPY=$ROOT/target/gate-selftest
SURVEY=$ROOT/target/release/nsh-survey
SHELL_UNDER_TEST=$ROOT/target/bash-mode/bash
REGISTER=$COPY/BASH_DISPOSITIONS.toml
failures=0

for required in "$SURVEY" "$SHELL_UNDER_TEST"; do
    [[ -x $required ]] || {
        echo "bash-gate-selftest: missing $required" >&2
        exit 1
    }
done

rm -rf "$COPY"
cp -r "$ROOT/tests/surveys/oils" "$COPY"
PRISTINE=$(cat "$REGISTER")
restore() { printf '%s' "$PRISTINE" >"$REGISTER"; }

refuses() { # NAME EXPECTED-SUBSTRING
    local name=$1 wanted=$2 output status
    output=$("$SURVEY" gate-bash --shell "$SHELL_UNDER_TEST" "$COPY" 2>&1)
    status=$?
    if ((status == 0)); then
        printf 'FAIL %-36s the gate accepted the mutation\n' "$name"
        failures=1
    elif ! printf '%s' "$output" | grep -qF "$wanted"; then
        printf 'FAIL %-36s refused, but not for the stated reason\n' "$name"
        printf '%s\n' "$output" | tail -4
        failures=1
    else
        printf 'ok   %-36s\n' "$name"
    fi
    restore
}

# A dispositioned case loses its entry: it becomes an unexpected failure.
# The case named here is a divergence taken on purpose rather than a gap,
# so it stays non-passing and the mutation stays meaningful as the
# not-implemented backlog shrinks.
#
# The mutation checks that it mutated, because it stopped being one and
# nothing noticed: it named `unicode.test.sh:3` until 2026-09-02, that
# entry had been removed from the register when the case started passing,
# the substitution matched nothing, and an unmutated register passing the
# gate read as "the gate accepted the mutation".
if ! python3 - "$REGISTER" <<'PY'
import re, sys
path = sys.argv[1]
text = open(path).read()
text, count = re.subn(
    r'\[\[case\]\]\nid = "var-op-patsub\.test\.sh:23"\ndisposition = [^\n]*\nreason = [^\n]*\n\n?',
    '', text, count=1)
if count != 1:
    sys.exit('the dropped-entry mutation matched nothing; name a case the register still has')
open(path, 'w').write(text)
PY
then
    printf 'FAIL %-36s the mutation could not be applied\n' "dropped entry"
    failures=1
    restore
else
    refuses "dropped entry" "var-op-patsub.test.sh:23 is an unexpected"
fi

# An entry survives the fix it was written to excuse.
printf '\n[[case]]\nid = "append.test.sh:0"\ndisposition = "not-implemented"\nreason = "stale"\n' >>"$REGISTER"
refuses "stale entry on a passing case" "still registered"

# An entry names a case that does not exist.
printf '\n[[case]]\nid = "no-such.test.sh:0"\ndisposition = "not-implemented"\nreason = "ghost"\n' >>"$REGISTER"
refuses "entry for an unknown case" "is not a case in the group"

# An entry claims a case the reference calibration already excludes, which
# would let one failure be counted as expected twice over.
printf '\n[[case]]\nid = "append.test.sh:7"\ndisposition = "not-implemented"\nreason = "double"\n' >>"$REGISTER"
refuses "entry outside the eligible set" "already excludes it"

# An entry with no reason is an exclusion with no argument behind it.
printf '\n[[case]]\nid = "append.test.sh:0"\ndisposition = "not-implemented"\nreason = ""\n' >>"$REGISTER"
refuses "entry with no reason" "has no reason"

# A whole file may only leave the contract by the scope decision.
printf '\n[[scope]]\nspec = "arith.test.sh"\ndisposition = "not-implemented"\nreason = "whole file"\n' >>"$REGISTER"
refuses "scope entry with a case category" "can only be out-of-contract"

# The register drifts away from the corpus it was written against.
python3 - "$REGISTER" <<'PY'
import sys
path = sys.argv[1]
text = open(path).read().replace(
    'oils_commit = "15de8fd779569e6e3a9f5fcbfc00e7df0ebe0380"',
    'oils_commit = "0000000000000000000000000000000000000000"')
open(path, 'w').write(text)
PY
refuses "register pinned to another corpus" "the corpus is"

# argv[0] selects the dialect, so any other basename measures the profile
# with the profile turned off.
output=$("$SURVEY" gate-bash --shell "$ROOT/target/release/nsh" "$COPY" 2>&1)
if (($? == 0)) || ! printf '%s' "$output" | grep -qF "must be named exactly"; then
    printf 'FAIL %-36s a shell not named bash was accepted\n' "wrong basename"
    failures=1
else
    printf 'ok   %-36s\n' "wrong basename"
fi

if ((failures == 0)); then
    echo "bash-gate-selftest: every mutation was refused"
else
    echo "bash-gate-selftest: FAILURES"
fi
exit "$failures"
