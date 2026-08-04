#!/bin/sh
# Build the C dash that the differential harness compares the Rust port
# against, into tests/.build/ref.
#
# The reference MUST be built from this tree and in the configuration the
# port targets. Getting that wrong is not a small error: a hand-written
# config.h used earlier in this port had 29 defines against the real 68,
# left SMALL undefined *and* WITH_LINENO undefined, and so matched no
# configuration that exists. It produced a false $LINENO divergence and
# hid the fact that the port's libedit support was entirely stubbed.
#
#   --with-libedit  =>  SMALL undefined, and histedit.c compiled.
#                       Without it the whole of histedit.c is #ifndef'd
#                       out: no `fc`, no history, and those paths cannot
#                       fail in testing because neither side has them.
#   WITH_LINENO     =>  on by default (--disable-lineno is opt-out).
#
# Needs: autoconf, automake, libedit-dev.
set -e

ROOT=${DASH_ROOT:-$(cd "$(dirname "$0")/.." && pwd)}
OUT=$ROOT/tests/.build/ref

rm -rf "$OUT"
mkdir -p "$OUT"

# Build in a copy so the repo stays clean and src/ is never written to.
tar -cf - -C "$ROOT" \
    --exclude=target --exclude=crates --exclude=.git --exclude=tests \
    --exclude=plan --exclude=docs --exclude=posix . | tar -xf - -C "$OUT"

cd "$OUT"
./autogen.sh
./configure --with-libedit
make -j"$(nproc 2>/dev/null || echo 4)"

echo
echo "reference built: $OUT/src/dash"
"$OUT/src/dash" -c 'echo ok: $((0b11)) $LINENO' || true
