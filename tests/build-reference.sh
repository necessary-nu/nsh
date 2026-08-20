#!/bin/sh
# Download and build the pinned C dash that the differential harness compares
# the Rust port against. The archive is fetched before entering containment;
# downloaded code is configured, compiled, and executed only in the sandbox.
#
# Getting the configuration wrong is not a small error: a hand-written config.h
# used earlier in this port had 29 defines against the real 68, left SMALL
# undefined *and* WITH_LINENO undefined, and so matched no configuration that
# exists. It produced a false $LINENO divergence and hid the fact that the
# port's libedit support was entirely stubbed.
#
#   --with-libedit  =>  SMALL undefined, and histedit.c compiled.
#                       Without it the whole of histedit.c is #ifndef'd
#                       out: no `fc`, no history, and those paths cannot
#                       fail in testing because neither side has them.
#   WITH_LINENO     =>  on by default (--disable-lineno is opt-out).
#
# Needs: curl, sha256sum, tar, patch, autoconf, automake, libedit-dev.
set -eu

ROOT=${DASH_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
BUILD_ROOT=$ROOT/tests/.build
OUT=$BUILD_ROOT/ref
CACHE=$BUILD_ROOT/downloads
LOCK=$ROOT/tests/DASH_REFERENCE.env
PATCH_ROOT=$ROOT/tests/reference/patches

# This file is repository-owned data, not an environment override.
# shellcheck disable=SC1090
. "$LOCK"

valid_sha256() {
	[ "${#1}" -eq 64 ] && case $1 in *[!0-9a-f]*) false ;; *) true ;; esac
}
valid_sha256 "$DASH_REFERENCE_ARCHIVE_SHA256" &&
	valid_sha256 "$DASH_REFERENCE_PATCH_1_SHA256" &&
	valid_sha256 "$DASH_REFERENCE_PATCH_2_SHA256" || {
	echo "invalid Dash reference SHA-256 in $LOCK" >&2
	exit 1
}

ARCHIVE=$CACHE/$DASH_REFERENCE_ARCHIVE
DOWNLOAD=
WORK=
PREVIOUS=
cleanup() {
	[ -z "$DOWNLOAD" ] || rm -f -- "$DOWNLOAD"
	[ -z "$WORK" ] || rm -rf -- "$WORK"
	[ -z "$PREVIOUS" ] || rm -rf -- "$PREVIOUS"
}
trap cleanup EXIT HUP INT TERM

verify_archive() {
	printf '%s  %s\n' "$DASH_REFERENCE_ARCHIVE_SHA256" "$1" |
		sha256sum -c - >/dev/null 2>&1
}

verify_patch() {
	printf '%s  %s\n' "$1" "$PATCH_ROOT/$2" |
		sha256sum -c - >/dev/null 2>&1
}

verify_patch "$DASH_REFERENCE_PATCH_1_SHA256" "$DASH_REFERENCE_PATCH_1" || {
	echo "Dash reference patch failed SHA-256 verification: $DASH_REFERENCE_PATCH_1" >&2
	exit 1
}
verify_patch "$DASH_REFERENCE_PATCH_2_SHA256" "$DASH_REFERENCE_PATCH_2" || {
	echo "Dash reference patch failed SHA-256 verification: $DASH_REFERENCE_PATCH_2" >&2
	exit 1
}

mkdir -p "$CACHE"
if ! verify_archive "$ARCHIVE"; then
	command -v curl >/dev/null 2>&1 || {
		echo "curl is required to download $DASH_REFERENCE_TAG" >&2
		exit 1
	}
	DOWNLOAD=$ARCHIVE.download.$$
	echo "downloading Dash $DASH_REFERENCE_TAG ($DASH_REFERENCE_COMMIT)"
	curl --fail --location --proto '=https' --tlsv1.2 --retry 3 \
		--output "$DOWNLOAD" "$DASH_REFERENCE_URL"
	verify_archive "$DOWNLOAD" || {
		echo "downloaded Dash archive failed SHA-256 verification" >&2
		exit 1
	}
	mv -f -- "$DOWNLOAD" "$ARCHIVE"
	DOWNLOAD=
fi

# Network access is deliberately unavailable beyond this point. The outer
# stage acquires immutable bytes; the inner stage handles downloaded code.
if [ "${NSH_DASH_REFERENCE_CONTAINED:-}" != 1 ]; then
	trap - EXIT HUP INT TERM
	exec env NSH_DASH_REFERENCE_CONTAINED=1 \
		"$ROOT/scripts/sandboxed" --writable "$BUILD_ROOT" -- "$0"
fi

STAMP=$(printf '%s\n' \
	"url=$DASH_REFERENCE_URL" \
	"tag=$DASH_REFERENCE_TAG" \
	"commit=$DASH_REFERENCE_COMMIT" \
	"archive_sha256=$DASH_REFERENCE_ARCHIVE_SHA256" \
	"patch_1=$DASH_REFERENCE_PATCH_1_SHA256 $DASH_REFERENCE_PATCH_1" \
	"patch_2=$DASH_REFERENCE_PATCH_2_SHA256 $DASH_REFERENCE_PATCH_2" \
	'configure=--with-libedit')
if [ -x "$OUT/src/dash" ] && [ -f "$OUT/.nsh-reference" ] &&
	[ "$(cat "$OUT/.nsh-reference")" = "$STAMP" ]; then
	echo "reference already built: $OUT/src/dash"
	exec "$OUT/src/dash" -c 'test "$((0b11)):$LINENO" = 3:1'
fi

WORK=$OUT.tmp.$$
PREVIOUS=$OUT.previous.$$
rm -rf -- "$WORK" "$PREVIOUS"
mkdir -p "$WORK"

tar -xzf "$ARCHIVE" --strip-components=1 -C "$WORK"
[ -f "$WORK/configure.ac" ] && [ -f "$WORK/src/main.c" ] || {
	echo "Dash archive did not contain $DASH_REFERENCE_TOPDIR" >&2
	exit 1
}
patch --batch --forward -d "$WORK" -p1 <"$PATCH_ROOT/$DASH_REFERENCE_PATCH_1"
patch --batch --forward -d "$WORK" -p1 <"$PATCH_ROOT/$DASH_REFERENCE_PATCH_2"

cd "$WORK"
./autogen.sh
./configure --with-libedit
make -j"$(nproc 2>/dev/null || echo 4)"

PROBE=$(src/dash -c 'printf "%s\n" "$((0b11)):$LINENO"')
[ "$PROBE" = 3:1 ] || {
	echo "Dash reference smoke test failed: $PROBE" >&2
	exit 1
}
printf '%s\n' "$STAMP" >.nsh-reference

if [ -e "$OUT" ]; then
	mv -- "$OUT" "$PREVIOUS"
fi
if ! mv -- "$WORK" "$OUT"; then
	[ ! -e "$PREVIOUS" ] || mv -- "$PREVIOUS" "$OUT"
	exit 1
fi
WORK=
rm -rf -- "$PREVIOUS"
PREVIOUS=

echo
echo "reference built: $OUT/src/dash"
echo "source: $DASH_REFERENCE_TAG ($DASH_REFERENCE_COMMIT)"
