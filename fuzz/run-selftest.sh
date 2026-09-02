#!/usr/bin/env bash
# Exercise containment selection and the campaign's clocks without building
# or executing a fuzz target.
#
# The clock checks stand in a fuzzer for a few seconds of `sleep`, because
# what they are checking is when the budget starts, and that answer must not
# depend on how long the corpus took -- which is the whole of the bug they
# exist against.
set -euo pipefail

root=$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd -P)
runner="$root/fuzz/run.sh"

fail() {
    echo "fuzz/run-selftest.sh: $*" >&2
    exit 1
}

expect_contains() {
    local output=$1 needle=$2
    [[ $output == *"$needle"* ]] || fail "expected $needle in: $output"
}

host=$(env -u CODEX_SESSION_ID -u CODEX_PERMISSION_PROFILE \
    "$runner" --dry-run parse 1 2>&1)
expect_contains "$host" 'containment=new'
expect_contains "$host" "$root/scripts/sandboxed"
expect_contains "$host" "--chdir=$root/fuzz"

managed=$(env CODEX_SESSION_ID=test CODEX_PERMISSION_PROFILE=:workspace \
    "$runner" --dry-run parse 1 2>&1)
expect_contains "$managed" 'containment=outer'
[[ $managed != *scripts/sandboxed* ]] || fail 'managed mode nested scripts/sandboxed'

forced=$(env CODEX_SESSION_ID=test CODEX_PERMISSION_PROFILE=:workspace \
    "$runner" --containment new --dry-run parse 1 2>&1)
expect_contains "$forced" 'containment=new'
expect_contains "$forced" "$root/scripts/sandboxed"

omitted_seconds=$(env -u CODEX_SESSION_ID -u CODEX_PERMISSION_PROFILE \
    "$runner" --dry-run parse -jobs=4 2>&1)
expect_contains "$omitted_seconds" '-max_total_time=0'
expect_contains "$omitted_seconds" '-jobs=4'

if "$runner" --containment invalid --dry-run parse 1 >/dev/null 2>&1; then
    fail 'invalid containment mode was accepted'
fi

# A campaign runs the built binary under fuzz/budget.sh rather than under
# `cargo fuzz run`, so that the clock it is on covers fuzzing and nothing
# else -- no second Cargo build, and no waiting on another campaign's build
# lock.
budgeted=$(env -u CODEX_SESSION_ID -u CODEX_PERMISSION_PROFILE \
    NSH_FUZZ_REPLAY_ALLOWANCE=300 "$runner" --dry-run parse 30 2>&1)
expect_contains "$budgeted" "$root/fuzz/budget.sh 300 30 --"
expect_contains "$budgeted" "release/parse"
expect_contains "$budgeted" '-max_total_time=330'
[[ $budgeted != *'fuzz run'* ]] || fail 'the campaign still goes through cargo fuzz run'
# 300 replay + 30 budget + the 120s the boundary has always added.
expect_contains "$budgeted" '--timeout 450'

if env NSH_FUZZ_REPLAY_ALLOWANCE=x "$runner" --dry-run parse 1 >/dev/null 2>&1; then
    fail 'a non-numeric NSH_FUZZ_REPLAY_ALLOWANCE was accepted'
fi

# The clocks themselves, against a stand-in that replays for four seconds
# and then mutates for ever. The budget must start at the fuzzer's INITED
# line: a three-second budget behind a four-second replay ends at about
# seven seconds, not at about three.
budget=$root/fuzz/budget.sh
work=$(mktemp -d "${TMPDIR:-/tmp}/nsh-fuzz-selftest.XXXXXX")
trap 'rm -rf -- "$work"' EXIT

cat >"$work/replays" <<'EOF'
#!/usr/bin/env bash
sleep 4
printf '#123\tINITED cov: 1 ft: 1 corp: 1/1b exec/s: 1 rss: 1Mb\n' >&2
while :; do sleep 1; done
EOF
cat >"$work/never-inits" <<'EOF'
#!/usr/bin/env bash
sleep 60
EOF
# libFuzzer answers SIGTERM by printing its statistics and exiting 72, which
# is a campaign that ran its whole budget rather than one that failed.
cat >"$work/answers-term" <<'EOF'
#!/usr/bin/env bash
trap 'exit 72' TERM
printf '#9\tINITED cov: 1 ft: 1 corp: 1/1b exec/s: 1 rss: 1Mb\n' >&2
while :; do sleep 1 & wait $!; done
EOF
chmod +x "$work/replays" "$work/never-inits" "$work/answers-term"

started=$(date +%s)
"$budget" 60 3 -- "$work/replays" >/dev/null 2>&1 || fail 'a budgeted campaign reported failure'
elapsed=$(($(date +%s) - started))
((elapsed >= 6)) || fail "the budget started before the replay ended (${elapsed}s)"
((elapsed <= 15)) || fail "the budget did not stop the campaign (${elapsed}s)"

"$budget" 60 2 -- "$work/answers-term" >/dev/null 2>&1 \
    || fail "libFuzzer's own interrupt status was reported as a failure"

started=$(date +%s)
status=0
"$budget" 3 5 -- "$work/never-inits" >/dev/null 2>&1 || status=$?
elapsed=$(($(date +%s) - started))
((status == 124)) || fail "a campaign that never fuzzed reported $status, not 124"
((elapsed <= 15)) || fail "the replay allowance did not stop the campaign (${elapsed}s)"

if "$budget" 1 1 >/dev/null 2>&1; then
    fail 'fuzz/budget.sh accepted a call with no command'
fi
