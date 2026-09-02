#!/bin/bash
# Self-tests for scripts/disk-headroom, which removes directories.
#
# Every case here is about something it must NOT remove. The script exists
# because sessions on this machine were hand-writing `rm -rf` against eight
# `target/` directories under time pressure, and the failure mode of that is
# deleting a colleague's work rather than running out of disk. A tool that
# does it wrongly is worse than the problem, so the safety rules are pinned
# here rather than trusted.
#
#   tests/harness/disk-headroom-selftest.sh
#
# It creates throwaway git worktrees under target/ and removes them again. It
# starts no shell, reads no case, and may be run through scripts/sandboxed.
set -u

ROOT=$(cd "$(dirname "${BASH_SOURCE[0]}")/../.." && pwd -P)
TOOL=$ROOT/scripts/disk-headroom
WORK=$ROOT/target/disk-headroom-selftest-$$
CLEAN=$WORK/clean
DIRTY=$WORK/dirty
BUSY=$WORK/busy
MENTIONED=$WORK/mentioned
STRANGER=$WORK/stranger
failures=0
decoy=
mentioner=

cleanup() {
    for p in $decoy $mentioner; do
        kill -KILL "$p" 2>/dev/null || :
        wait "$p" 2>/dev/null || :
    done
    for w in "$CLEAN" "$DIRTY" "$BUSY" "$MENTIONED"; do
        [[ ! -d $w ]] || /usr/bin/git -C "$ROOT" worktree remove --force "$w" 2>/dev/null || :
    done
    rm -rf "$WORK"
    /usr/bin/git -C "$ROOT" worktree prune 2>/dev/null || :
}
trap cleanup EXIT HUP INT TERM

report() { # NAME OK-OR-EMPTY DETAIL
    if [[ -n $2 ]]; then
        printf 'ok   %-50s %s\n' "$1" "$3"
    else
        printf 'FAIL %-50s %s\n' "$1" "$3"
        failures=1
    fi
}

mkdir -p "$WORK"
for w in "$CLEAN" "$DIRTY" "$BUSY" "$MENTIONED"; do
    /usr/bin/git -C "$ROOT" worktree add --detach "$w" HEAD >/dev/null 2>&1 || {
        echo "disk-headroom-selftest: could not create worktree $w" >&2
        exit 1
    }
    # Build output big enough to be worth reporting, and old enough that the
    # recency guard is not what decides the case.
    mkdir -p "$w/target/debug/deps"
    /bin/dd if=/dev/zero of="$w/target/debug/deps/blob" bs=1M count=8 status=none
    touch -d '4 hours ago' "$w/target/debug/deps" "$w/target/debug" "$w/target"
done
# A checkout this repository does not know about. Other projects live on this
# machine; their build output is not ours to reclaim.
mkdir -p "$STRANGER/target/debug"
touch -d '4 hours ago' "$STRANGER/target/debug" "$STRANGER/target"

# Uncommitted work is the one thing near a target/ that no build regenerates.
echo 'uncommitted' >>"$DIRTY/README.md"

# A live build is a build tool that names the directory. `exec -a rustc` gives
# the process the argv[0] a real rustc has, and the trailing `; true` keeps
# bash from exec-ing sleep in place, so it holds an argv naming the path too --
# the shape of a real `rustc --out-dir .../target/debug/deps`.
(exec -a rustc /bin/bash -c 'sleep 120; true' "$BUSY/target/debug/deps") &
decoy=$!

# NAMING THE PATH IS NOT USING IT. This is the process above with the one
# difference that matters: an ordinary shell rather than a build tool. It is
# the shape of the agent that ran this script -- a session whose own command
# line quotes the directory it is asking about -- and treating it as a live
# build made the shared checkout's cache unreclaimable forever.
(exec -a bash /bin/bash -c 'sleep 120; true' "$MENTIONED/target/debug/deps") &
mentioner=$!
sleep 0.5

output=$("$TOOL" 2>&1)
status=$?

report "the report succeeds" \
    "$([[ $status -eq 0 ]] && echo y)" "status=$status"

report "a clean idle worktree's output is offered" \
    "$([[ $output == *"$CLEAN/target"* ]] && echo y)" \
    "$(printf '%s\n' "$output" | grep -c "$CLEAN/target") mention(s)"

report "a dirty worktree is never offered" \
    "$([[ $output == *"$DIRTY/target -- "*uncommitted* ]] && echo y)" \
    "kept for uncommitted changes"

report "a tree a live process names is never offered" \
    "$([[ $output == *"$BUSY/target -- "* ]] && echo y)" \
    "kept while pid $decoy names it"

report "a build tool naming it is what counts, not a mention" \
    "$([[ $output == *"  $MENTIONED/target"* ]] && echo y)" \
    "offered though pid $mentioner names it"

report "a checkout git does not know is never mentioned" \
    "$([[ $output != *"$STRANGER"* ]] && echo y)" \
    "not in the report at all"

report "the shared checkout is never offered whole" \
    "$([[ $output == *"keeping"*"$ROOT/target"* && $output != *"  $ROOT/target"$'\n'* ]] && echo y)" \
    "reported, not counted"

report "the cost of removing it is stated" \
    "$([[ $output == *"full rebuild"* ]] && echo y)" \
    "the rebuild it would cost is named"

# The recency guard, isolated from every other rule: the same clean worktree,
# asked with a window wide enough to cover the files just written into it.
output_recent=$("$TOOL" --recent 100000 2>&1)
report "a recently written tree is never offered" \
    "$([[ $output_recent == *"$CLEAN/target -- written to within"* ]] && echo y)" \
    "held by the window alone"

report "a malformed window is refused" \
    "$("$TOOL" --recent later >/dev/null 2>&1 || [[ $? -eq 2 ]] && echo y)" \
    "exit 2"

# Only now, and only what was offered.
before=$(/usr/bin/du -sb "$CLEAN/target" 2>/dev/null | /usr/bin/awk '{print $1}')
# The shared checkout's incremental cache is real and belongs to whoever is
# working here. Whether it happens to be idle while this runs is not this
# test's business, so it is pinned out of play: a self-test must not cost a
# bystander their incrementality to prove a point about worktrees.
NSH_DISK_CACHE_RECENT_MINUTES=999999 "$TOOL" --reclaim >/dev/null 2>&1
report "--reclaim removes what it offered" \
    "$([[ ! -d $CLEAN/target ]] && echo y)" \
    "was $before bytes"
report "--reclaim leaves the dirty worktree alone" \
    "$([[ -f $DIRTY/target/debug/deps/blob ]] && echo y)" \
    "$DIRTY/target intact"
report "--reclaim leaves the busy worktree alone" \
    "$([[ -f $BUSY/target/debug/deps/blob ]] && echo y)" \
    "$BUSY/target intact"
report "--reclaim removes the merely-mentioned one" \
    "$([[ ! -d $MENTIONED/target ]] && echo y)" \
    "a mention did not save it"
report "--reclaim leaves the stranger alone" \
    "$([[ -d $STRANGER/target ]] && echo y)" \
    "$STRANGER/target intact"
report "--reclaim leaves the shared checkout's build" \
    "$([[ -d $ROOT/target ]] && echo y)" \
    "$ROOT/target intact"

if ((failures)); then
    echo "disk-headroom-selftest: FAILED"
    exit 1
fi
echo "disk-headroom-selftest: all checks passed"
