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

# A campaign seeds from whatever `fuzz/corpus.sh seed` says to seed from --
# the campaign corpus once something has derived one, the whole archive until
# then. Asserting they agree rather than asserting which is which is what
# makes this independent of whether this checkout has derived anything.
seeded_from=$("$root/fuzz/corpus.sh" seed "$root/fuzz/corpus/parse" \
    "$root/fuzz/campaign/parse" 2>/dev/null | head -1)
expect_contains "$budgeted" "release/parse $seeded_from "

# Deriving is a third thing to stop after, and it is the archive that goes in
# and the campaign corpus that comes out.
derived=$(env -u CODEX_SESSION_ID -u CODEX_PERMISSION_PROFILE \
    "$runner" --dry-run --derive parse 2>&1)
expect_contains "$derived" "$root/fuzz/corpus.sh derive"
expect_contains "$derived" "release/parse $root/fuzz/corpus/parse $root/fuzz/campaign/parse"
[[ $derived != *budget.sh* ]] || fail 'a derivation went through the fuzzing budget'

if "$runner" --build --derive --dry-run parse >/dev/null 2>&1; then
    fail '--build and --derive were accepted together'
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

# ---------------------------------------------------------------------------
# fuzz/sweep.sh: which artifacts it can see.
#
# libFuzzer files a `slow-unit-` for an input the target survived and took too
# long over, so its replay exits zero and a sweep that reads only the status
# reports it closed -- and `--prune` then deletes it. What follows stands a
# script in for a fuzz target and a fabricated tree in for the repository, so
# the verdicts can be checked without a fuzz build and without touching the
# real corpus or the real artifacts. Each input file holds the work it
# costs; the stand-in spends them and reports what they took the way libFuzzer
# does.

sweep_tree=$work/tree
mkdir -p "$sweep_tree/fuzz/artifacts/example" "$sweep_tree/fuzz/corpus/example"
cp "$root/fuzz/sweep.sh" "$sweep_tree/fuzz/sweep.sh"
# sweep.sh builds the target inside the containment a campaign would use. There
# is nothing to build here.
printf '#!/usr/bin/env bash\nexit 0\n' >"$sweep_tree/fuzz/run.sh"
chmod +x "$sweep_tree/fuzz/run.sh"

sweep_triple=$(rustc -vV | sed -n 's/^host: //p')
mkdir -p "$sweep_tree/fuzz/target/$sweep_triple/release"
cat >"$sweep_tree/fuzz/target/$sweep_triple/release/example" <<'EOF'
#!/usr/bin/env bash
# Each input holds the loop iterations it costs. Work rather than `sleep`,
# because the sweep's whole claim is that load divides out: a stand-in whose
# cost is a fork and a timer would be the one thing on the machine that does
# not slow down when the machine does, and the check would fail at load 95
# for a reason that is the stand-in's and not the sweep's.
for file in "$@"; do
    read -r cost <"$file" || cost=0
    case $cost in
        crash) printf 'ERROR: libFuzzer: deadly signal\n' >&2; exit 77 ;;
    esac
    printf 'Running: %s\n' "$file"
    started=${EPOCHREALTIME//[.,]/}
    for ((spin = 0; spin < cost; spin++)); do :; done
    finished=${EPOCHREALTIME//[.,]/}
    printf 'Executed %s in %d ms\n' "$file" "$(((finished - started) / 1000))"
done
EOF
chmod +x "$sweep_tree/fuzz/target/$sweep_triple/release/example"

for i in $(seq 0 63); do
    printf '5000\n' >"$sweep_tree/fuzz/corpus/example/seed-$i"
done

# The allowance is pinned low so the stand-in's "slow" input can be eighty
# ordinary ones rather than three hundred, which is the difference between a
# check that costs a second and one that costs a minute. What the default is
# and why is a measured table in sweep.sh; that it is still the default is
# checked once, below.
sweep_run() {
    env NSH_SWEEP_CALIBRATION=64 NSH_SWEEP_COST_ALLOWANCE=32 \
        "$sweep_tree/fuzz/sweep.sh" --containment outer example 2>/dev/null
}

# A slow unit that still costs eighty ordinary inputs is live and one that
# costs eight of them is not. Neither replay fails, so the exit status says
# nothing about either, which is the whole point.
printf '400000\n' >"$sweep_tree/fuzz/artifacts/example/slow-unit-still-slow"
printf '40000\n' >"$sweep_tree/fuzz/artifacts/example/slow-unit-now-fast"
sweep=$(sweep_run) || :
expect_contains "$sweep" 'live      slow-unit-still-slow'
[[ $sweep != *'live      slow-unit-now-fast'* ]] \
    || fail "a slow unit that no longer costs anything was reported live: $sweep"
expect_contains "$sweep" 'total: 1 live, 1 dead'
if sweep_run >/dev/null; then
    fail 'a sweep with a live slow unit reported success'
fi

# The default allowance is the measured one, and it is 256. A run without the
# knob says so on every line it decides.
sweep=$(env NSH_SWEEP_CALIBRATION=64 "$sweep_tree/fuzz/sweep.sh" \
    --containment outer example 2>/dev/null) || :
expect_contains "$sweep" 'against an allowance of 256x'

# The status classes keep the status test: a crash artifact is live because
# its replay fails, whatever it cost.
printf 'crash\n' >"$sweep_tree/fuzz/artifacts/example/crash-still-crashing"
printf '5000\n' >"$sweep_tree/fuzz/artifacts/example/crash-fixed"
sweep=$(sweep_run) || :
expect_contains "$sweep" 'live      crash-still-crashing'
expect_contains "$sweep" 'total: 2 live, 2 dead'
rm -f "$sweep_tree/fuzz/artifacts/example/crash-still-crashing" \
      "$sweep_tree/fuzz/artifacts/example/crash-fixed"

# `--prune` removes what died and keeps what did not. The point of measuring
# the cost at all is that this line used to delete both.
sweep=$(env NSH_SWEEP_CALIBRATION=64 NSH_SWEEP_COST_ALLOWANCE=32 \
    "$sweep_tree/fuzz/sweep.sh" --containment outer --prune example 2>/dev/null) || :
expect_contains "$sweep" 'pruned 1 closed artifact'
[[ -f $sweep_tree/fuzz/artifacts/example/slow-unit-still-slow ]] \
    || fail '--prune deleted a slow unit that still reproduces'
[[ ! -f $sweep_tree/fuzz/artifacts/example/slow-unit-now-fast ]] \
    || fail '--prune kept a slow unit that no longer reproduces'

# A corpus that cannot say what an ordinary input costs leaves the artifact
# undecided rather than closed, and the sweep says so in its status.
# `[spec:nsh:req:oracle.cannot-measure-is-a-failure]`.
mv "$sweep_tree/fuzz/corpus/example" "$sweep_tree/fuzz/corpus/example-kept"
printf '40000\n' >"$sweep_tree/fuzz/artifacts/example/slow-unit-now-fast"
sweep=$(sweep_run) || :
expect_contains "$sweep" 'undecided slow-unit-now-fast'
expect_contains "$sweep" 'total: 0 live, 0 dead, 2 undecided'
if sweep_run >/dev/null; then
    fail 'a sweep that could measure nothing reported success'
fi
mv "$sweep_tree/fuzz/corpus/example-kept" "$sweep_tree/fuzz/corpus/example"

# ---------------------------------------------------------------------------
# fuzz/corpus.sh: two corpora, and keeping them in step.
#
# The archive is every input any campaign has kept and must never lose one;
# the campaign corpus is what a campaign can afford to replay before it
# mutates anything. A stand-in stands in for `-merge=1` here: what is being
# checked is the bookkeeping around it -- that nothing leaves the archive,
# that what a campaign finds gets back into it, that an input which reached
# the archive after the derivation is still seeded, and that a merge which
# files an artifact and exits zero is not read as a clean run.

corpus_sh=$root/fuzz/corpus.sh
archive=$work/archive
campaign=$work/campaign/example
artifacts=$work/artifacts
mkdir -p "$archive" "$artifacts" "$work/campaign"
for i in $(seq 0 19); do printf 'input %d\n' "$i" >"$archive/input-$i"; done

cat >"$work/merges" <<'EOF'
#!/usr/bin/env bash
# Stand in for `TARGET -merge=1 OUT IN`. libFuzzer keeps the inputs that add
# coverage; this keeps every other one, which is the same shape. An input
# that says `crash` is skipped and filed as an artifact, and the merge still
# exits zero -- which is the behaviour that makes the status unreadable.
set -euo pipefail
prefix=; out=; in=
for argument in "$@"; do
    case $argument in
        -artifact_prefix=*) prefix=${argument#*=} ;;
        -*) ;;
        *) if [[ -z $out ]]; then out=$argument; else in=$argument; fi ;;
    esac
done
kept=0
for file in "$in"/*; do
    [[ -f $file ]] || continue
    if [[ $(cat "$file") == crash ]]; then
        cp -- "$file" "$prefix/crash-${file##*/}"
        continue
    fi
    if ((kept % 2 == 0)); then cp -- "$file" "$out/${file##*/}"; fi
    kept=$((kept + 1))
done
exit 0
EOF
chmod +x "$work/merges"

"$corpus_sh" derive "$work/merges" "$archive" "$campaign" "$artifacts" >/dev/null 2>&1 \
    || fail 'a clean derivation reported failure'
(($(ls -A "$archive" | wc -l) == 20)) || fail 'the derivation lost an archived input'
(($(ls -A "$campaign" | wc -l) == 10)) || fail 'the derivation did not reduce the archive'
(($(wc -l <"$campaign.archived") == 20)) \
    || fail 'the derivation did not record what it was derived from'

# With a campaign corpus in place a campaign seeds from it alone.
seeds=$("$corpus_sh" seed "$archive" "$campaign" 2>/dev/null)
[[ $seeds == "$campaign" ]] || fail "seeded from $seeds rather than the campaign corpus"

# An input that reaches the archive out of band is seeded as well, so a
# derivation going stale costs start-up and never costs coverage.
printf 'arrived later\n' >"$archive/input-late"
seeds=$("$corpus_sh" seed "$archive" "$campaign" 2>/dev/null)
[[ $seeds == "$campaign"$'\n'"$campaign.delta" ]] \
    || fail "an archived arrival was not seeded: $seeds"
[[ -f $campaign.delta/input-late ]] || fail 'the arrival is not in the delta'
(($(ls -A "$campaign.delta" | wc -l) == 1)) || fail 'the delta is not just the arrival'

# What a campaign finds is written into the campaign corpus, and the archive
# is the one that must not lose it.
printf 'a new find\n' >"$campaign/found-by-the-campaign"
"$corpus_sh" return "$archive" "$campaign" 2>/dev/null
[[ -f $archive/found-by-the-campaign ]] || fail 'a campaign find never reached the archive'
# ... and the record of what the campaign corpus stands for has to absorb it,
# or the next campaign seeds its own last findings a second time.
seeds=$("$corpus_sh" seed "$archive" "$campaign" 2>/dev/null)
[[ $seeds == "$campaign"$'\n'"$campaign.delta" ]] || fail "seeded from $seeds after a return"
(($(ls -A "$campaign.delta" | wc -l) == 1)) \
    || fail 'a returned find was seeded back as an arrival'

# With no campaign corpus at all, the archive is the seed -- the behaviour
# every campaign had before there were two corpora.
rm -rf "$campaign" "$campaign.archived" "$campaign.delta"
seeds=$("$corpus_sh" seed "$archive" "$campaign" 2>/dev/null)
[[ $seeds == "$archive" ]] || fail "without a campaign corpus it seeded from $seeds"

# A merge is crash-resistant: it files the artifact, skips the input and
# exits zero. Reading the status alone would call that a clean run, so the
# artifact directory is what is read. `[spec:nsh:req:oracle.cannot-measure-is-a-failure]`.
printf 'crash\n' >"$archive/input-that-crashes"
if "$corpus_sh" derive "$work/merges" "$archive" "$campaign" "$artifacts" >/dev/null 2>&1; then
    fail 'a derivation that filed an artifact reported success'
fi
[[ -f $artifacts/crash-input-that-crashes ]] || fail 'the merge filed no artifact'
[[ -f $archive/input-that-crashes ]] || fail 'the derivation removed the crashing input from the archive'
[[ ! -f $campaign/input-that-crashes ]] || fail 'the campaign corpus kept the crashing input'
compgen -G "$campaign/*" >/dev/null || fail 'a derivation that found something left no campaign corpus'

if "$corpus_sh" derive "$work/merges" "$work/nothing" "$campaign" "$artifacts" >/dev/null 2>&1; then
    fail 'a derivation from an archive that does not exist reported success'
fi
if "$corpus_sh" seed "$archive" >/dev/null 2>&1; then
    fail 'fuzz/corpus.sh accepted seed with one argument'
fi
