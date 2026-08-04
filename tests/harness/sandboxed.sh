#!/bin/bash
# The single place that knows how to run a shell-under-test.
#
# NOTHING in this harness may invoke dash-ref or the port directly. Every
# invocation goes through ds_sandboxed(), because a test case is hostile
# input: on 2026-08-02 a generated case ran `kill -- -1`, which POSIX
# defines as "signal every process this uid may signal". Running as the
# login uid with no namespace, that reached tmux, the login shell, the
# Claude daemon and the harness itself.
#
# `timeout` does NOT contain this. It bounds how long a case runs, not what
# the case can reach. The containment boundary is the PID namespace.
#
# Properties this gives us:
#   pid ns   kill(-1) / kill(-9,-1) can only see processes in the sandbox
#   pid ns   every process dies when the namespace's pid 1 exits, so
#            background jobs from a case cannot leak
#   net ns   no network egress from a test case
#   ro root  a case cannot write outside its own scratch directory
#   nproc    a fork bomb self-caps; RLIMIT_NPROC is checked against the
#            forking process's own limit, so our session still forks fine
#   new session  no controlling terminal to signal or stuff input into

ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
DS_SANDBOX=${DS_SANDBOX:-sandbox}

# NOTE: the timeouts belong INSIDE this function, not at the call site.
# `timeout 10 ds_sandboxed ...` cannot work -- timeout(1) is a binary and
# ds_sandboxed is a shell function, so it exits 127 without running
# anything. Both shells then "fail identically" and every differential
# case passes vacuously. That is exactly what happened on the first
# sandboxed run: a green PASS=6964 where nothing had executed.
#
# The inner timeout runs inside the namespace so it is contained with
# everything else; the outer one guards against the sandbox itself
# wedging during setup.
DS_TIMEOUT=${DS_TIMEOUT:-10}

# `env --default-signal` is not decoration. Signal *dispositions* are
# inherited across fork and exec, so whatever ignores a signal above this
# harness -- an editor, a CI runner, a Node-based agent, all of which
# ignore SIGPIPE -- silently imposes that on both shells under test. Both
# then behave identically-wrongly and the case passes.
#
# That is not hypothetical: it hid a real divergence for the length of
# this port. Rust's runtime sets SIGPIPE to SIG_IGN before main, which
# dash never does, so `... | head -2` printed ~99,930 "I/O error" lines
# from the port and none from dash. Under the harness, the parent's own
# SIG_IGN reached both shells, so both produced the errors and the only
# difference left was how many -- which reads as a scheduling flake.
#
# Start every shell from a known signal state instead of an inherited one.
ds_sandboxed() {  # ds_sandboxed WORKDIR SHELL [ARGS...]
	local dir=$1; shift
	timeout $((DS_TIMEOUT + 5)) \
	"$DS_SANDBOX" --quiet \
		--unshare all \
		--die-with-parent \
		--new-session \
		--bind /:/:ro \
		--dev /dev \
		--proc /proc \
		--bind "$dir:$dir" \
		--chdir "$dir" \
		--setenv TMPDIR "$dir" \
		--setenv PATH "$dir/.bin:/usr/bin:/bin" \
		--limit nproc=64 \
		-- timeout "$DS_TIMEOUT" env --default-signal -- "$@"
}

# Equality between two shells proves nothing if neither ran. Assert that
# each binary independently produces the expected bytes through the real
# code path before trusting a single comparison.
ds_assert_harness_live() {  # ds_assert_harness_live SHELL...
	local probe sh out rc
	probe=$(mktemp -d "${TMPDIR:-/tmp}/dslive.XXXXXX") || return 1
	for sh in "$@"; do
		out=$(ds_sandboxed "$probe" "$sh" -c 'echo __CANARY__' 2>&1); rc=$?
		if [ "$out" != "__CANARY__" ] || [ $rc -ne 0 ]; then
			echo "HARNESS DEAD: '$sh' did not execute through the sandbox." >&2
			echo "  rc=$rc output=[$out]" >&2
			echo "Refusing to report results: identical failures would" >&2
			echo "otherwise be counted as passing cases." >&2
			rm -rf "$probe"
			return 1
		fi
	done
	rm -rf "$probe"
	return 0
}

# Refuse to run at all unless containment is real. There is deliberately no
# fallback path: a harness that silently degrades to "no sandbox" is how the
# original incident happened.
ds_assert_contained() {
	command -v "$DS_SANDBOX" >/dev/null 2>&1 || {
		echo "CONTAINMENT FAILURE: '$DS_SANDBOX' not found in PATH." >&2
		echo "Refusing to run test cases unsandboxed." >&2
		return 1
	}

	local probe sentinel rc
	probe=$(mktemp -d "${TMPDIR:-/tmp}/dsprobe.XXXXXX") || return 1

	# A real process outside the sandbox that the sandbox must not see.
	sleep 30 &
	sentinel=$!

	local seen
	seen=$(ds_sandboxed "$probe" /bin/sh -c '
		test -d /proc/'"$sentinel"' && echo VISIBLE
		ls /proc | grep -c "^[0-9]*$"
	' 2>&1)
	rc=$?

	kill "$sentinel" 2>/dev/null
	wait "$sentinel" 2>/dev/null
	rmdir "$probe" 2>/dev/null

	if [ $rc -ne 0 ]; then
		echo "CONTAINMENT FAILURE: sandbox probe exited $rc: $seen" >&2
		return 1
	fi
	case $seen in
	*VISIBLE*)
		echo "CONTAINMENT FAILURE: host pid $sentinel was visible inside the" >&2
		echo "sandbox -- the PID namespace is not active. Refusing to run." >&2
		return 1
		;;
	esac
	# Only the namespace's own handful of processes should exist.
	local n=${seen##*$'\n'}
	case $n in
	''|*[!0-9]*)
		echo "CONTAINMENT FAILURE: could not read /proc inside sandbox: $seen" >&2
		return 1
		;;
	esac
	if [ "$n" -gt 16 ]; then
		echo "CONTAINMENT FAILURE: $n pids visible inside the sandbox;" >&2
		echo "expected only the sandbox's own. Refusing to run." >&2
		return 1
	fi
	return 0
}
