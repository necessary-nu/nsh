#!/usr/bin/env bash
# Keep two corpora in step: the archive that must not lose an input, and the
# campaign corpus that a campaign can afford to start from.
#
#   fuzz/corpus.sh derive BINARY ARCHIVE CAMPAIGN ARTIFACTS
#   fuzz/corpus.sh seed   ARCHIVE CAMPAIGN
#   fuzz/corpus.sh return ARCHIVE CAMPAIGN
#
# `fuzz/run.sh --derive TARGET` and a campaign are what call these; the paths
# are arguments rather than derived here so that each half can be exercised
# against directories that are not the repository's.
#
# WHY TWO CORPORA. `fuzz/corpus/TARGET` is the archive: every input any
# campaign has kept, seeded from the Smoosh and Oils suites, and it is the
# regression set. It grows without bound, and libFuzzer replays the whole
# seed corpus before it mutates anything -- so `fuzz/run.sh parse 10` was a
# four-minute command, and every campaign made the next one longer.
#
# `cargo fuzz cmin` is not the answer, for the reason the archive exists: it
# minimises in place and discards inputs, and those inputs are real scripts
# that reach constructs a generator takes a very long time to stumble into. A
# minimised archive keeps the coverage and loses the provenance.
#
# `fuzz/campaign/TARGET` is derived beside it and discards nothing. libFuzzer's
# `-merge=1` writes a set that reaches the same features, bounded by how many
# distinct feature sets there are rather than by how many inputs have
# accumulated -- and it produces it by executing every archived input against
# the build in front of it, so the expensive pass yields the regression
# evidence and the reduced set at once.
#
# THE MERGE IS CRASH-RESISTANT BY DESIGN: it skips an input that kills the
# target, files the artifact and carries on, so it writes findings and exits
# zero. Anything treating it as a check has to read the artifact directory
# rather than the status, which is `[spec:nsh:req:oracle.cannot-measure-is-a-failure]`
# and is why `derive` counts that directory before and after.
set -euo pipefail

usage() {
    cat >&2 <<'EOF'
usage: fuzz/corpus.sh derive BINARY ARCHIVE CAMPAIGN ARTIFACTS
       fuzz/corpus.sh seed   ARCHIVE CAMPAIGN
       fuzz/corpus.sh return ARCHIVE CAMPAIGN

  derive  reduce ARCHIVE to CAMPAIGN by running every archived input, filing
          what it finds under ARTIFACTS; deletes nothing from ARCHIVE
  seed    print the corpus directories a campaign should seed from, and
          report what build the archive was last run against
  return  copy what a campaign found back into ARCHIVE

Environment:
  NSH_FUZZ_MERGE_TIMEOUT   seconds a derivation may take (default 3600)
EOF
    exit 2
}

# The names in both directories are the SHA-1 of the contents libFuzzer put
# there, so a name that is absent is an input that is absent, and comparing
# listings compares corpora.
listing() { [[ -d $1 ]] || return 0; (cd "$1" && LC_ALL=C ls -A | LC_ALL=C sort); }

# Same filesystem, and a corpus input is never edited, so a link is the copy.
adopt() {
    ln -f -- "$1" "$2" 2>/dev/null || cp -- "$1" "$2"
}

# WHAT THE LISTING CANNOT SAY, AND WHY A SECOND FILE SAYS IT.
#
# `$campaign.archived` answers "which inputs was this derived from", which
# is what keeps a stale derivation costing start-up rather than coverage:
# `seed` hands back everything that has arrived since. It cannot answer "how
# stale", and neither can the line every campaign prints --
#
#     seeding from 2659 campaign inputs standing for 21305 archived
#
# -- which reads exactly the same whether the archive was last run against
# this build or against the tree of a fortnight ago. Measured 2026-09-04 at
# d27cf47: `fuzz/campaign/parse.archived` was written on 2026-09-02 against
# 04582ce, 73 commits back, 51 of them in `crates/`, and nothing anywhere
# said so. The archive is the regression set, and a regression set nothing
# has run is not one.
#
# Reporting it is a fact and is required; what age, if any, should refuse a
# campaign is a separate argument and nothing here settles it.
# `[spec:nsh:req:oracle.recording-carries-its-age]`

# The checkout a path given to this script is in, if it is in one.
#
# The commands here take directories rather than a repository root, so that
# each half can be exercised against directories that are not this
# repository's -- and provenance is a fact about a repository. Asking git
# about the path it was handed keeps both true: a real run resolves the
# checkout the fuzz binary was built in, and a fabricated tree resolves
# nothing and is told so rather than being given a wrong answer.
checkout_of() {
    local path=$1
    [[ -d $path ]] || path=$(dirname -- "$path")
    git --no-optional-locks -C "$path" rev-parse --show-toplevel 2>/dev/null
}

# Which build ran the archive: the short commit, marked when the tree it was
# built from carried uncommitted work.
#
# The commit is the better half of the pair the record keeps. Wall-clock age
# says nothing when nothing has changed; one commit to crates/nsh/src/pattern.rs
# says a great deal.
build_identity() {
    local checkout commit
    checkout=$(checkout_of "$1") || checkout=
    commit=
    [[ -z $checkout ]] \
        || commit=$(git --no-optional-locks -C "$checkout" rev-parse --short HEAD 2>/dev/null) \
        || commit=
    [[ -n $commit ]] || { printf 'unknown\n'; return 0; }
    if [[ -n $(git --no-optional-locks -C "$checkout" status --porcelain 2>/dev/null) ]]; then
        printf '%s+dirty\n' "$commit"
    else
        printf '%s\n' "$commit"
    fi
}

# One `key=value` per line, beside the listing and never inside it: `seed`
# compares `.archived` against the archive with `comm`, so it has to stay a
# bare sorted listing of names.
stamp() {
    local campaign=$1 binary=$2 archived=$3 derived=$4
    {
        printf 'build=%s\n' "$(build_identity "$binary")"
        printf 'at=%s\n' "$(date -u +%Y-%m-%dT%H:%M:%SZ)"
        printf 'epoch=%s\n' "$(date +%s)"
        printf 'archived=%s\n' "$archived"
        printf 'derived=%s\n' "$derived"
    } >"$campaign.provenance"
}

whole_number() { case ${1:-} in ''|*[!0-9]*) return 1 ;; esac; }

# How far this checkout has moved since the recorded commit, when it is a
# commit this checkout has. A rebase, a fresh clone or a `+dirty` build that
# was never committed all land in the second branch, which says what it
# cannot say rather than nothing.
distance_from() {
    local campaign=$1 commit=${2%+dirty} checkout
    checkout=$(checkout_of "$campaign") || checkout=
    if [[ -z $checkout ]] || [[ $commit == unknown ]] \
       || ! git --no-optional-locks -C "$checkout" rev-parse --verify --quiet "$commit^{commit}" >/dev/null 2>&1; then
        printf 'fuzz/corpus.sh: %s is not a commit this checkout has, so how far the tree has moved since cannot be said\n' \
            "$commit" >&2
        return 0
    fi
    printf 'fuzz/corpus.sh: %s commit(s) since, %s of them in crates/\n' \
        "$(git --no-optional-locks -C "$checkout" rev-list --count "$commit..HEAD" 2>/dev/null)" \
        "$(git --no-optional-locks -C "$checkout" rev-list --count "$commit..HEAD" -- crates 2>/dev/null)" >&2
}

# The age of the derivation, said beside the count it qualifies.
report_provenance() {
    local campaign=$1 record="$campaign.provenance"
    if [[ ! -f $record ]]; then
        printf 'fuzz/corpus.sh: this campaign corpus carries no provenance, so what build the archive was last run against cannot be said; fuzz/run.sh --derive records it\n' >&2
        return 0
    fi
    local key value build=unknown at=unknown epoch= derived= age='age unknown'
    while IFS='=' read -r key value; do
        case $key in
            build) build=$value ;;
            at) at=$value ;;
            epoch) epoch=$value ;;
            derived) derived=$value ;;
        esac
    done <"$record"
    if whole_number "$epoch"; then
        age="$((($(date +%s) - epoch) / 86400))d ago"
    fi
    printf 'fuzz/corpus.sh: the archive was last run against %s at %s, %s\n' "$build" "$at" "$age" >&2
    distance_from "$campaign" "$build"

    # The other half of the same bookkeeping: a campaign writes its finds
    # into the campaign corpus, so the set the derivation bounded grows
    # between derivations and the start-up it bounds grows with it.
    local now
    now=$(listing "$campaign" | wc -l)
    if whole_number "$derived" && ((now > derived)); then
        printf 'fuzz/corpus.sh: the campaign corpus was %s inputs at the derivation and is %s now\n' \
            "$derived" "$now" >&2
    fi
}

derive() {
    local binary=$1 archive=$2 campaign=$3 artifacts=$4
    local timeout_secs=${NSH_FUZZ_MERGE_TIMEOUT:-3600}
    case $timeout_secs in
        *[!0-9]*|'') echo "fuzz/corpus.sh: NSH_FUZZ_MERGE_TIMEOUT must be a non-negative integer" >&2; exit 2 ;;
    esac
    [[ -x $binary ]] || { echo "fuzz/corpus.sh: no target binary at $binary" >&2; exit 1; }
    compgen -G "$archive/*" >/dev/null || {
        echo "fuzz/corpus.sh: $archive has nothing to derive from" >&2
        exit 1
    }
    mkdir -p "$artifacts" "$(dirname -- "$campaign")"

    local before after archived derived started elapsed status fresh
    before=$(listing "$artifacts" | wc -l)
    # The merge writes into a directory beside the one it will replace, so a
    # merge that dies part-way leaves the campaign corpus that was there
    # rather than half of a new one.
    fresh="$campaign.new"
    rm -rf -- "$fresh"
    mkdir -p "$fresh"

    archived=$(listing "$archive" | wc -l)
    printf 'fuzz/corpus.sh: merging %s archived inputs; every one of them runs\n' "$archived" >&2
    started=$(date +%s)
    status=0
    timeout "$timeout_secs" "$binary" -merge=1 \
        -artifact_prefix="$artifacts/" \
        -rss_limit_mb=4096 \
        -max_len=65536 \
        "$fresh" "$archive" >&2 || status=$?
    elapsed=$(($(date +%s) - started))

    if ((status != 0)); then
        rm -rf -- "$fresh"
        if ((status == 124)); then
            printf 'fuzz/corpus.sh: the merge hit its own %ss clock; raise NSH_FUZZ_MERGE_TIMEOUT\n' \
                "$timeout_secs" >&2
        fi
        printf 'fuzz/corpus.sh: the merge failed after %ss, so the campaign corpus is unchanged\n' \
            "$elapsed" >&2
        exit "$status"
    fi

    derived=$(listing "$fresh" | wc -l)
    if ((derived == 0)); then
        rm -rf -- "$fresh"
        printf 'fuzz/corpus.sh: the merge kept nothing, which cannot be right; the campaign corpus is unchanged\n' >&2
        exit 1
    fi

    # Installed before the artifact count is read, so a derivation that turned
    # up a crash still leaves a usable campaign corpus -- and one that no
    # longer opens every campaign by running the input that crashes. The
    # archive keeps it; the artifact directory records it; `fuzz/sweep.sh` is
    # what re-asks about it.
    rm -rf -- "$campaign"
    mv -- "$fresh" "$campaign"
    # What the derived set was derived from. `seed` compares the archive
    # against this to find what has arrived since, so a derivation going stale
    # costs a longer start-up and never costs coverage.
    listing "$archive" >"$campaign.archived"
    stamp "$campaign" "$binary" "$archived" "$derived"

    printf 'fuzz/corpus.sh: %s archived inputs -> %s campaign inputs in %ss, against %s\n' \
        "$archived" "$derived" "$elapsed" "$(sed -n 's/^build=//p' "$campaign.provenance")"

    after=$(listing "$artifacts" | wc -l)
    if ((after > before)); then
        printf 'fuzz/corpus.sh: the merge filed %d new artifact(s) and exited zero, which is why this is counted rather than read off the status\n' \
            "$((after - before))" >&2
        exit 1
    fi
}

seed() {
    local archive=$1 campaign=$2
    local delta="$campaign.delta"
    rm -rf -- "$delta"
    if ! compgen -G "$campaign/*" >/dev/null; then
        printf '%s\n' "$archive"
        printf 'fuzz/corpus.sh: no campaign corpus, so the whole archive is the seed and the replay is the whole of it\n' >&2
        printf 'fuzz/corpus.sh: fuzz/run.sh --derive reduces it, and runs every archived input while doing so\n' >&2
        return 0
    fi
    printf '%s\n' "$campaign"
    # Said every campaign, because it is the trade being made every campaign:
    # the archive is no longer replayed in front of the mutation, and the
    # command that does replay it is right there.
    printf 'fuzz/corpus.sh: seeding from %s campaign inputs standing for %s archived; fuzz/run.sh --derive runs the archive against the build again\n' \
        "$(listing "$campaign" | wc -l)" "$(listing "$archive" | wc -l)" >&2
    report_provenance "$campaign"

    # Anything that reached the archive after the derivation. Normally nothing:
    # a campaign writes its finds into the campaign corpus and `return` puts
    # them in the archive, which records both. It is re-seeding a fresh
    # archive, or a hand-added input, that lands here.
    local -a arrived=()
    if [[ -f $campaign.archived ]]; then
        mapfile -t arrived < <(comm -13 "$campaign.archived" <(listing "$archive"))
    else
        mapfile -t arrived < <(listing "$archive")
    fi
    ((${#arrived[@]})) || return 0

    mkdir -p "$delta"
    local name
    for name in "${arrived[@]}"; do
        adopt "$archive/$name" "$delta/$name"
    done
    printf '%s\n' "$delta"
    printf 'fuzz/corpus.sh: %d archived input(s) arrived after the derivation and are seeded as well\n' \
        "${#arrived[@]}" >&2
}

do_return() {
    local archive=$1 campaign=$2
    compgen -G "$campaign/*" >/dev/null || return 0
    mkdir -p "$archive"
    local file name returned=0
    local -a names=()
    for file in "$campaign"/*; do
        [[ -f $file ]] || continue
        name=${file##*/}
        if [[ -e $archive/$name ]]; then continue; fi
        adopt "$file" "$archive/$name"
        names+=("$name")
        returned=$((returned + 1))
    done
    ((returned)) || return 0
    # The campaign corpus now accounts for these archived names too, so the
    # record of what it was derived from has to say so -- otherwise `seed`
    # would hand them back as arrivals and every campaign would replay its own
    # last findings twice.
    if [[ -f $campaign.archived ]]; then
        { cat "$campaign.archived"; printf '%s\n' "${names[@]}"; } |
            LC_ALL=C sort -u >"$campaign.archived.new"
        mv -- "$campaign.archived.new" "$campaign.archived"
    fi
    printf 'fuzz/corpus.sh: %d new input(s) went back into the archive\n' "$returned" >&2
}

(($#)) || usage
command=$1
shift
case $command in
    derive) (($# == 4)) || usage; derive "$1" "$2" "$3" "$4" ;;
    seed) (($# == 2)) || usage; seed "$1" "$2" ;;
    return) (($# == 2)) || usage; do_return "$1" "$2" ;;
    *) usage ;;
esac
