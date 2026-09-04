#!/bin/bash
# Establish the differential baseline across the locale models that can make
# dash take observably different ctype, collation, and multibyte paths.
#
#   locale-sweep.sh [JOBS]
#
# The locales are generated below tests/.build rather than installed into the
# host archive, by tests/build-locales.sh, which the Rust suite also depends
# on.  Both shells receive them through LOCPATH inside the ordinary test
# sandbox. Set LOCALE_SWEEP_RECORD=1 to preserve and report an existing
# differential baseline; the default is the strict regression gate.
set -euo pipefail

ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
JOBS=${1:-8}
LOG_ROOT=$ROOT/tests/.build/locale-sweep
SINGLE_BYTE=en_US.ISO-8859-1

# A differential result names the source tree only if the default port binary
# was built from that tree.  Custom PORT values remain the caller's explicit
# provenance choice.
if [ -z "${PORT:-}" ]; then
	cargo build --quiet --manifest-path "$ROOT/Cargo.toml" -p nsh-cli --bin nsh
fi

mkdir -p "$LOG_ROOT"
LOCALE_ROOT=$("$ROOT/tests/build-locales.sh")
export LOCPATH=$LOCALE_ROOT

# Prove the third axis is semantically different from C before spending a
# full sweep on it.  In ISO-8859-1, byte E9 is alphabetic and therefore a
# valid shell-name byte.  Equality alone would pass if both shells silently
# fell back to C, so require the value as well.
# [spec:nsh:req:oracle.cannot-measure-is-a-failure]
. "$ROOT/tests/harness/sandboxed.sh"
probe=$(mktemp -d "${TMPDIR:-/tmp}/nsh-locale-probe.XXXXXXXX")
trap 'rmdir "$probe"' EXIT
script=$'\351=ok; printf \'<%s>\\n\' "$\351"'
reference=${REF:-$ROOT/tests/.build/ref/src/dash}
port=${PORT:-$ROOT/target/debug/nsh}
reference_rc=0
result=$(DS_LOCALE=$SINGLE_BYTE ds_sandboxed "$probe" "$reference" -c "$script") || reference_rc=$?
if [ "$reference_rc" -ne 0 ] || [ "$result" != '<ok>' ]; then
	echo "single-byte locale fixture failed for $reference: [$result]" >&2
	exit 1
fi
port_rc=0
result=$(DS_LOCALE=$SINGLE_BYTE ds_sandboxed "$probe" "$port" -c "$script" 2>&1) || port_rc=$?
if [ "$port_rc" -ne 0 ] || [ "$result" != '<ok>' ]; then
	if [ "${LOCALE_SWEEP_RECORD:-0}" != 1 ]; then
		echo "single-byte locale probe failed for $port: [$result]" >&2
		exit 1
	fi
	echo "recorded single-byte probe difference for $port: [$result]" >&2
fi
rmdir "$probe"
trap - EXIT

# `en_US.utf8` is one of the generated locales rather than the host's own.
# LOCPATH is set from here down and glibc consults no locale archive while it
# is, so an archive-only `en_US.utf8` would fail to load and leave both shells
# in C -- a UTF-8 axis measuring nothing, which is what it did until the
# fixture directory started carrying its own.
# [spec:nsh:req:oracle.cannot-measure-is-a-failure]
for locale_name in C en_US.utf8 "$SINGLE_BYTE"; do
	echo "==== locale baseline: $locale_name ===="
	label=${locale_name//[^A-Za-z0-9_.-]/_}
	log=$LOG_ROOT/$label.log
	DS_LOCALE=$locale_name RUNALL_OUT=$LOG_ROOT/$label.fail \
		"$ROOT/tests/harness/runall.sh" "$JOBS" 2>&1 |
		tee "$log"
	summary=$(grep '^==== TOTAL ' "$log" | tail -1)
	if [ -z "$summary" ]; then
		echo "locale baseline failed: no final tally" >&2
		exit 1
	fi
	case $summary in
	*' FAIL=0 '*) ;;
	*)
		if [ "${LOCALE_SWEEP_RECORD:-0}" != 1 ]; then
			echo "locale baseline failed: ${summary:-no final tally}" >&2
			exit 1
		fi
		echo "recorded existing differences: $summary" >&2
		;;
	esac
	if grep -qE '^(STALE:|[0-9]+ corpora produced no tally)' "$log"; then
		echo "locale baseline was invalid; see $log" >&2
		exit 1
	fi
done
