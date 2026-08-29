#!/bin/sh
# Build the locales the test suite requires and print the directory glibc has
# to be told about. They are generated with `localedef --no-archive` below
# tests/.build rather than installed, so nothing outside the repository
# changes; the reference bootstrap next door works the same way.
#
#   export LOCPATH=$(tests/build-locales.sh)
#
# The path has to reach the tests through the environment, and that is a
# property of glibc rather than a choice. `newlocale` resolves every name
# under its own locale directory, including an absolute one -- measured,
# `newlocale("/abs/dir")` opens `/usr/lib/locale//abs/dir/LC_CTYPE` -- so
# there is no per-call search path a test could pass instead, and a test
# cannot export LOCPATH for itself without the ambient mutation
# [dec:nsh:no-ambient-state] refuses. So the fixture is a precondition of
# the run, and the tests that need it fail when it is absent.
#
# The UTF-8 locale is generated as well, and is not redundant: setting
# LOCPATH makes glibc skip the system locale archive entirely, so a host
# whose UTF-8 locales live only in that archive loses them for as long as
# LOCPATH is set. Generating one keeps this directory self-sufficient.
# `en_US.utf8` is the spelling that answers to both `en_US.utf8` and
# `en_US.UTF-8`, because glibc retries a failed name in its normalized form.
set -eu

ROOT=${DASH_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
OUT=$ROOT/tests/.build/locales

build() {  # build DIRECTORY CHARMAP
	[ ! -f "$OUT/$1/LC_CTYPE" ] || return 0
	command -v localedef >/dev/null 2>&1 || {
		echo "build-locales.sh: localedef is required to build $1" >&2
		exit 2
	}
	mkdir -p "$OUT"
	localedef --quiet --no-archive -i en_US -f "$2" "$OUT/$1" >&2
}

build en_US.ISO-8859-1 ISO-8859-1
build en_US.utf8 UTF-8

printf '%s\n' "$OUT"
