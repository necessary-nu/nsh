#!/bin/bash
# Self-tests for the containment `.cargo/config.toml` gives every cargo-run
# binary in this workspace.
#
# `enforce-survey-test-containment` named "workspace-test" among the commands
# that must run inside a fail-closed PID namespace, and `cargo test --workspace`
# enforced nothing until 2026-09-02: it ran shell cases with the session's own
# uid, PID namespace and controlling terminal. The runner entry in
# `.cargo/config.toml` is what fixed that, and this is what notices when the
# entry stops working -- cargo ignoring it, a sandbox that comes up without the
# namespace it promised, or somebody deleting it.
#
# Like tests/harness/abandoned-selftest.sh, it cannot be run through
# scripts/sandboxed: the leak check asks the *host* whether a descendant
# survived, which is exactly what the boundary hides. It runs unsandboxed,
# deliberately, and the only shell it starts is one cargo starts for it.
#
#   tests/harness/containment-selftest.sh
#
# It builds, so the first run costs a debug build of nsh-cli and one test
# binary.
set -u

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
# The probe lives under target/ because that is the one path the boundary binds
# writable at its own name, so the host and the sandbox agree on where it is.
WORK=$ROOT/target/containment-selftest-$$
SLEEPER=$WORK/sleeper
# Long enough that a survivor is unmistakably a survivor rather than a race,
# short enough that a failed run costs a minute rather than an afternoon.
LINGER=45
failures=0

cleanup() {
    pkill -KILL -f "^$SLEEPER $LINGER\$" 2>/dev/null || :
    rm -rf "$WORK"
}
trap cleanup EXIT HUP INT TERM

mkdir -p "$WORK"
cp /bin/sleep "$SLEEPER"

report() { # NAME OK-OR-EMPTY DETAIL
    if [[ -n $2 ]]; then
        printf 'ok   %-46s %s\n' "$1" "$3"
    else
        printf 'FAIL %-46s %s\n' "$1" "$3"
        failures=1
    fi
}

cd "$ROOT" || exit 1

# The workspace's own witness passes when cargo runs it. This is the check
# that fails on a tree without the runner entry, and it is the one a developer
# sees first, because it is an ordinary `cargo test` failure.
output=$(cargo test -q -p nsh --test containment_boundary 2>&1)
status=$?
report "cargo test runs a test binary contained" \
    "$([[ $status -eq 0 ]] && echo y)" \
    "status=$status"

# ...and it is a real question rather than something that passes anywhere. The
# same binary run by hand out of target/ is outside every boundary, which is
# the one case no repository can prevent and this one must at least notice.
# Only the cheapest case is selected, because the others start processes.
binary=$(cargo test -q -p nsh --test containment_boundary --no-run --message-format=json 2>/dev/null |
    sed -n 's/.*"executable":"\([^"]*containment_boundary[^"]*\)".*/\1/p' | tail -1)
if [[ -z $binary ]]; then
    echo "containment-selftest: could not find the built test binary" >&2
    exit 1
fi
output=$("$binary" --exact the_host_process_table_is_not_visible_to_a_test 2>&1)
status=$?
report "the same binary run by hand is not contained" \
    "$([[ $status -ne 0 && $output == *"session's own PID namespace"* ]] && echo y)" \
    "status=$status"

# The leak `86d2ce6` was about, asked of the workspace's other cargo entry
# point. A shell that backgrounds a descendant and exits leaves it reparented
# to init; inside the boundary that init is the sandbox's, and it dies with
# the namespace instead of outliving the command by forty-seven hours.
cargo run -q -p nsh-cli -- -c "$SLEEPER $LINGER & echo started" >/dev/null 2>&1
survivors=$(pgrep -f "^$SLEEPER $LINGER\$" 2>/dev/null | wc -l)
report "a shell cargo runs cannot leak a descendant" \
    "$([[ $survivors -eq 0 ]] && echo y)" \
    "survivors=$survivors"
pkill -KILL -f "^$SLEEPER $LINGER\$" 2>/dev/null || :

# Fail-closed, the property the other two callers have: a boundary that cannot
# be established is a failed command, never an unsandboxed one.
output=$(NSH_TEST_SANDBOX=$WORK/not-a-sandbox cargo test -q -p nsh --test containment_boundary 2>&1)
status=$?
report "a missing sandbox fails the run, not opens it" \
    "$([[ $status -ne 0 && $output == *"refusing to run tests unsandboxed"* ]] && echo y)" \
    "status=$status"

# Cargo picks the directory a binary starts in -- the caller's for `cargo run`,
# the package's for a test binary -- and the runner stands in the middle of
# that. The command-level wrapper starts everything at the repository root,
# which would silently move the ground under every relative path a cargo-run
# program uses, so it is asked from somewhere that is not the root.
where=$(cd "$ROOT/crates/nsh-cli" && cargo run -q -p nsh-cli -- -c 'pwd' 2>/dev/null)
report "the runner keeps the directory cargo chose" \
    "$([[ $where == "$ROOT/crates/nsh-cli" ]] && echo y)" \
    "pwd=[$where]"

if ((failures)); then
    echo "containment-selftest: FAILED"
    exit 1
fi
echo "containment-selftest: all checks passed"
