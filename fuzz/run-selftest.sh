#!/usr/bin/env bash
# Exercise containment selection without building or executing a fuzz target.
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
