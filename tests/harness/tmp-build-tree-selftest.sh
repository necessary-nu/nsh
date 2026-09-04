#!/bin/bash
# Self-tests for the check in scripts/sandboxed that refuses a build tree on
# /tmp.
#
# The boundary replaces /tmp with an empty tmpfs and binds back only the one
# directory holding the program cargo handed it. A whole CARGO_TARGET_DIR under
# /tmp therefore delivers `deps/<test binary>` and hides its sibling
# `<profile>/nsh`, so every test that starts CARGO_BIN_EXE_nsh fails with "No
# such file or directory" for a binary that runs from a prompt. It reads as a
# broken workspace and cost a session a detour on 2026-09-04.
#
#   tests/harness/tmp-build-tree-selftest.sh
#
# Like tests/harness/disk-selftest.sh this may be run through the wrapper: it
# asks nothing of the host that the boundary hides. Nothing here builds; the
# programs are shell scripts standing where cargo's would be, because the
# question is which paths exist inside the boundary and a real test binary
# would answer it no differently for several minutes more.
set -u

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
WRAPPER=$ROOT/scripts/sandboxed
MARKER=reached-the-command
WORK=$(/usr/bin/mktemp -d /tmp/tmp-build-tree-selftest-XXXXXX) || exit 1
# The one case that must keep working lives outside /tmp on purpose: a build
# tree the boundary already reaches is never the thing this refuses.
KEPT=$ROOT/target/tmp-build-tree-selftest-$$
trap '/bin/rm -rf -- "$WORK" "$KEPT"' EXIT HUP INT TERM
failures=0

report() { # NAME OK-OR-EMPTY DETAIL
    if [[ -n $2 ]]; then
        printf 'ok   %-52s %s\n' "$1" "$3"
    else
        printf 'FAIL %-52s %s\n' "$1" "$3"
        failures=1
    fi
}

# A cargo build tree as cargo lays one out: CACHEDIR.TAG at the root, the test
# binary under <profile>/deps, and the binary it will want to start beside that
# directory rather than in it. The second is the whole defect.
build_tree() { # build_tree ROOT
    /bin/mkdir -p "$1/debug/deps" || return 1
    printf 'Signature: 8a477f597d28d172789f06886806bc55\n' >"$1/CACHEDIR.TAG" || return 1
    printf '#!/bin/sh\necho %s\n' "$MARKER" >"$1/debug/nsh" || return 1
    printf '#!/bin/sh\nexec "$(dirname "$0")/../nsh"\n' >"$1/debug/deps/case-1" || return 1
    /bin/chmod +x "$1/debug/nsh" "$1/debug/deps/case-1" || return 1
}

build_tree "$WORK/target" || {
    echo "tmp-build-tree-selftest: could not lay out a build tree in $WORK" >&2
    exit 1
}
build_tree "$KEPT/target" || {
    echo "tmp-build-tree-selftest: could not lay out a build tree in $KEPT" >&2
    exit 1
}

output=$("$WRAPPER" --cargo-runner -- "$WORK/target/debug/deps/case-1" 2>&1)
status=$?
report "a build tree on /tmp is refused before the command" \
    "$([[ $status -eq 1 && $output != *"$MARKER"* ]] && echo y)" \
    "status=$status"

# The refusal is the whole deliverable: the reader is looking at ENOENT for a
# file they can see, so it has to name the tree, name the symptom, and say
# where to put the tree instead.
report "the refusal names the program and its build tree" \
    "$([[ $output == *"$WORK/target/debug/deps/case-1"* && $output == *"$WORK/target  <- "* ]] && echo y)" \
    "$(printf '%s' "$output" | head -1)"

report "the refusal names the symptom and the remedy" \
    "$([[ $output == *"os error 2"* && $output == *CARGO_TARGET_DIR* ]] && echo y)" \
    "symptom and remedy lines present"

# The rustdoc carve-out this check must not swallow. A merged doctest binary
# sits in a bare /tmp directory with no CACHEDIR.TAG above it, and it has to
# keep running.
/bin/mkdir -p "$WORK/rustdoctest-lookalike"
printf '#!/bin/sh\necho %s\n' "$MARKER" >"$WORK/rustdoctest-lookalike/rust_out"
/bin/chmod +x "$WORK/rustdoctest-lookalike/rust_out"
output=$("$WRAPPER" --cargo-runner -- "$WORK/rustdoctest-lookalike/rust_out" 2>&1)
status=$?
report "a program on /tmp outside a build tree still runs" \
    "$([[ $status -eq 0 && $output == "$MARKER" ]] && echo y)" \
    "status=$status output=[$output]"

# A build tree the boundary already reaches is not the thing being refused, and
# the sibling binary the defect is about is reachable there.
output=$("$WRAPPER" --cargo-runner -- "$KEPT/target/debug/deps/case-1" 2>&1)
status=$?
report "a build tree under target/ reaches its own binaries" \
    "$([[ $status -eq 0 && $output == "$MARKER" ]] && echo y)" \
    "status=$status output=[$output]"

# The check is about a program cargo chose, so a command typed at the wrapper
# is not its business -- and a wrapper that refused one would refuse this
# self-test's own working directory.
#
# The other two preconditions are switched off for this one case because they
# are questions about the machine rather than about the argument, they have
# self-tests of their own, and a machine short of space or carrying an orphan
# would otherwise fail this case for something it is not measuring. Runner mode
# skips both already, which is why only this case says so.
output=$(NSH_TEST_DISK=ignore NSH_TEST_ABANDONED=ignore "$WRAPPER" -- /bin/echo "$MARKER" 2>&1)
status=$?
report "a command typed at the wrapper is not asked" \
    "$([[ $status -eq 0 && $output == "$MARKER" ]] && echo y)" \
    "status=$status output=[$output]"

if ((failures)); then
    echo "tmp-build-tree-selftest: FAILED"
    exit 1
fi
echo "tmp-build-tree-selftest: all checks passed"
