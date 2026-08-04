#!/usr/bin/env bash
# Convert Open Group POSIX spec pages to clean GFM.
#
#   tools/convert.sh utilities/V3_chap02.html utilities/sh.html
#
# Output lands in build/md/<basename>.md. The intermediate stripped HTML is
# kept alongside it as <basename>.clean.html for debugging.

set -euo pipefail

root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
out="$root/build/md"
mkdir -p "$out"

for page in "$@"; do
  src="$root/$page"
  [ -f "$src" ] || { echo "no such page: $page" >&2; exit 1; }
  name="$(basename "$page" .html)"

  python3 "$root/tools/strip-boilerplate.py" "$src" > "$out/$name.clean.html"
  pandoc -f html -t gfm+raw_html \
    --lua-filter="$root/tools/posix.lua" \
    --wrap=none \
    "$out/$name.clean.html" -o "$out/$name.md"

  printf '%-24s %5s headings  %5s anchors  %4s informative markers\n' \
    "$name.md" \
    "$(grep -c '^#' "$out/$name.md" || true)" \
    "$(grep -c '^<a id=' "$out/$name.md" || true)" \
    "$(grep -c 'INFORMATIVE-' "$out/$name.md" || true)"
done
