#!/bin/bash
# Self-tests for scripts/room-to-build, the free-space check cargo carries into
# every compilation.
#
# `scripts/sandboxed` has asked the same question since 2026-09-02 and reaches
# only a command somebody typed it in front of. Both recorded incidents were
# builds spelled without it, and a build that runs out of room does not say so:
# it says `ld terminated with signal 7 [Bus error]` under an LLVM stack trace,
# or it says nothing at all and calls itself an internal compiler error.
#
#   tests/harness/room-to-build-selftest.sh
#
# Like tests/harness/disk-selftest.sh it may be run through the wrapper, and
# for the same reason: it asks nothing of the host that the boundary hides. It
# cannot fill a filesystem to test a refusal -- doing so would be the incident
# rather than a test of it -- so it moves the threshold across the free space,
# which exercises the same comparison from the other side.
#
# Most cases run the script directly with `/bin/echo` standing in for rustc,
# because the question is which invocations it lets past. The last two run
# cargo itself against a throwaway package, because "it reaches a build nobody
# wrapped" is the claim the whole thing exists to make and no amount of direct
# invocation tests it.
set -u

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
CHECK=$ROOT/scripts/room-to-build
MARKER=reached-the-compiler
WORK=$ROOT/target/room-to-build-selftest-$$
trap '/bin/rm -rf -- "$WORK"' EXIT HUP INT TERM
/bin/mkdir -p "$WORK" || exit 1
failures=0

report() { # NAME OK-OR-EMPTY DETAIL
    if [[ -n $2 ]]; then
        printf 'ok   %-52s %s\n' "$1" "$3"
    else
        printf 'FAIL %-52s %s\n' "$1" "$3"
        failures=1
    fi
}

# rustc as cargo spells it: the compiler, then the arguments, with `--out-dir`
# somewhere in the middle rather than first or last.
compile() { # compile OUT-DIR
    "$CHECK" /bin/echo "$MARKER" --crate-name probe --edition=2024 \
        --out-dir "$1" -C opt-level=3
}

# The same product the check uses, and the one `df` prints as "Avail".
free_mib=$(($(/usr/bin/stat -f -c '%a' "$WORK") * $(/usr/bin/stat -f -c '%S' "$WORK") / 1048576))
# One threshold this machine cannot meet and one it cannot fail, derived from
# what is free rather than from constants that would rot.
too_much=$((free_mib + 1024))
plenty=1

output=$(NSH_TEST_DISK_MIN=$plenty compile "$WORK" 2>&1)
status=$?
report "room on the disk compiles" \
    "$([[ $status -eq 0 && $output == *"$MARKER"* ]] && echo y)" \
    "status=$status free=${free_mib}MiB"

output=$(NSH_TEST_DISK_MIN=$too_much compile "$WORK" 2>&1)
status=$?
report "too little is refused before the compiler runs" \
    "$([[ $status -eq 1 && $output != *"$MARKER"* ]] && echo y)" \
    "status=$status"

# A refusal that does not say how much is free and how much was wanted sends
# the reader back to `df` anyway, and one that does not name the directory
# leaves them guessing which of two filesystems it meant.
report "the refusal names the out-dir and both numbers" \
    "$([[ $output == *"$WORK"* && $output == *GiB\ free* && $output == *under*GiB* ]] && echo y)" \
    "$(printf '%s' "$output" | head -1)"

# The line the node was filed for. Somebody looking at an LLVM stack trace has
# no reason to suspect the disk, so the refusal has to name the trace and take
# the suspicion off the compiler.
report "the refusal names the misleading signatures" \
    "$([[ $output == *"signal 7"* && $output == *ICE* && $output == *"No space left"* ]] && echo y)" \
    "all three signatures present"

for mode in ignore warn; do
    output=$(NSH_TEST_DISK=$mode NSH_TEST_DISK_MIN=$too_much compile "$WORK" 2>&1)
    status=$?
    report "NSH_TEST_DISK=$mode compiles and says nothing" \
        "$([[ $status -eq 0 && $output == "$MARKER"* ]] && echo y)" \
        "status=$status"
done

output=$(NSH_TEST_DISK=nonsense compile "$WORK" 2>&1)
status=$?
report "an unknown mode is refused, not assumed" \
    "$([[ $status -eq 2 && $output != *"$MARKER"* ]] && echo y)" \
    "status=$status"

output=$(NSH_TEST_DISK_MIN=nonsense compile "$WORK" 2>&1)
status=$?
report "a non-numeric threshold is refused" \
    "$([[ $status -eq 2 && $output != *"$MARKER"* ]] && echo y)" \
    "status=$status"

# Cargo's startup `rustc -vV` carries no `--out-dir` and writes nothing. Asking
# about the working directory there would refuse a worktree on a full
# filesystem whose build tree has room, before cargo had named a crate.
output=$(NSH_TEST_DISK_MIN=$too_much "$CHECK" /bin/echo "$MARKER" -vV 2>&1)
status=$?
report "an invocation with no output directory is let through" \
    "$([[ $status -eq 0 && $output == *"$MARKER"* ]] && echo y)" \
    "status=$status"

# A path it cannot ask about is not evidence of a full disk, and a build
# stopped by a broken check is worse than the failure this exists to name.
output=$(NSH_TEST_DISK_MIN=$too_much compile "$WORK/no/such/directory" 2>&1)
status=$?
report "a directory it cannot measure is let through" \
    "$([[ $status -eq 0 && $output == *"$MARKER"* ]] && echo y)" \
    "status=$status"

# THE CLAIM. A build nobody typed a wrapper in front of, on a machine under the
# threshold, says so by name. The package is throwaway and trivial, and under
# an impossible threshold it never gets as far as compiling anything -- the
# refusal is what is being measured, not the build.
PACKAGE=$WORK/package
/bin/mkdir -p "$PACKAGE/src"
{
    echo '[package]'
    echo 'name = "room-to-build-selftest"'
    echo 'version = "0.0.0"'
    echo 'edition = "2021"'
    echo
    echo '[workspace]'
} >"$PACKAGE/Cargo.toml"
echo 'fn main() {}' >"$PACKAGE/src/main.rs"

output=$(cd "$PACKAGE" && NSH_TEST_DISK_MIN=$too_much cargo build --offline 2>&1)
status=$?
report "a bare cargo build is refused by name" \
    "$([[ $status -ne 0 && $output == *room-to-build* && $output == *"signal 7"* ]] && echo y)" \
    "status=$status"

output=$(cd "$PACKAGE" && cargo build --offline 2>&1)
status=$?
report "a bare cargo build with room compiles" \
    "$([[ $status -eq 0 && $output != *room-to-build:* ]] && echo y)" \
    "status=$status $(printf '%s' "$output" | tail -1)"

if ((failures)); then
    echo "room-to-build-selftest: FAILED"
    exit 1
fi
echo "room-to-build-selftest: all checks passed"
