#!/bin/bash
# The sanctioned-divergence register, as data the harness can read.
#
# `docs/divergences.md` is the prose: what differs, why, and who decided.
# This file is the executable half. Without it a deliberate divergence
# that any corpus case observes turns FAIL=0 into a permanent FAIL=n
# indistinguishable from a regression -- and that single legible number
# is the whole reason the differential harness is worth running. See
# [dec:nsh:we-own-the-defects].
#
# ## What a register entry is
#
# A shell function that answers one question: *does this specific,
# already-decided divergence explain the difference in front of us?*
#
#   dsdiv_<id> REF_OUT PORT_OUT REF_RC PORT_RC CASE_FILE
#     -> 0  yes, this is that divergence
#     -> 1  no
#
# It is deliberately a function and not a pattern in a config file. A
# divergence is a claim about *behaviour*, and the only honest way to say
# "the outputs differ exactly this way and no other" is with code that
# can inspect both sides. A glob over case names would excuse whatever
# else those cases happened to break.
#
# ## The rule every entry must satisfy
#
# **An entry must not be able to match a regression.** That is the entire
# design constraint, and it is easy to get wrong by writing something too
# permissive. Two habits keep it honest:
#
#   * Compare, do not ignore. `sort`ing both sides and requiring equality
#     says "the same lines in a different order". Dropping the lines
#     entirely would say "anything at all", which is not a divergence,
#     it is a blind spot.
#   * Scope to the feature. An entry about `export -p` ordering has no
#     business excusing a case that never runs `export -p`, so it checks
#     the case text before it excuses anything.
#
# An entry that stops matching is reported as stale rather than left
# lying around: once the port and dash agree again, the excuse should go.

# Every registered divergence, by id. Order is the order they are tried.
DS_DIVERGENCES=()

# Set by ds_sanctioned to the id that matched, for the report.
DS_DIVERGENCE=

# ds_sanctioned REF_OUT PORT_OUT REF_RC PORT_RC CASE_FILE
#
# Returns 0 if some registered divergence explains the difference, with
# DS_DIVERGENCE naming it. Called only after an exact comparison has
# already failed, and before the flakiness classifier, because a
# sanctioned divergence is deterministic and re-running it ten times
# would only cost time.
ds_sanctioned() {
	local id
	DS_DIVERGENCE=
	for id in "${DS_DIVERGENCES[@]}"; do
		if "dsdiv_$id" "$1" "$2" "$3" "$4" "$5"; then
			DS_DIVERGENCE=$id
			return 0
		fi
	done
	return 1
}

# ds_harness_alive PORT REF -- both shells still exist and are runnable.
#
# The other half of making the tally mean something. A case whose port
# side exits 127 with "No such file or directory" naming its own path did
# not behave differently -- it did not run, and counting that as a
# divergence is how a full disk once produced 315 convincing failures in
# a single corpus with nothing wrong with the shell.
#
# `ds_assert_harness_live` in sandboxed.sh already refuses to start
# without both binaries. What it cannot see is a binary that disappears
# *during* a corpus, because it runs once per invocation and a corpus is
# thousands of cases. This is the per-case counterpart, checked only when
# a case has already failed, so it costs two stats on the unhappy path
# and nothing at all on the happy one.
ds_harness_alive() {
	[ -x "$1" ] && [ -x "$2" ]
}

# ---------------------------------------------------------------------
# Helpers for writing entries
# ---------------------------------------------------------------------

# ds_same_lines A B -- true when the two hold the same lines in any order.
#
# Not "ignore the order": the multiset has to be identical, so a line
# whose *content* changed still fails. That is what makes it usable for
# an ordering divergence without opening a hole.
ds_same_lines() {
	[ "$(printf '%s\n' "$1" | LC_ALL=C sort)" = "$(printf '%s\n' "$2" | LC_ALL=C sort)" ]
}

# ds_case_matches CASE_FILE ERE -- true when the case text matches.
#
# The scoping guard. An entry uses this so it can only ever excuse cases
# that exercise the feature it is about.
ds_case_matches() {
	grep -qE "$2" "$1" 2>/dev/null
}

# ---------------------------------------------------------------------
# The register
# ---------------------------------------------------------------------
#
# Empty. The first entry arrives with the `BTreeMap` change that makes
# `env`, `export -p`, `set` and `alias` print in sorted order -- see
# `docs/divergences.md`. It is not written here ahead of the behaviour it
# describes, because an entry that excuses a difference the shell does
# not yet produce is an excuse waiting to be misapplied.
#
# For the shape it will take:
#
#   DS_DIVERGENCES=(env_ordering)
#
#   dsdiv_env_ordering() {
#       [ "$3" = "$4" ] || return 1
#       ds_case_matches "$5" '(^|[;&|( ])(env|export -p|set|alias)([ ;|]|$)' || return 1
#       ds_same_lines "$1" "$2"
#   }

# An extra register, for testing the machinery itself. The mechanism has
# to be exercisable end to end while the real register is empty --
# otherwise the first thing that proves the XFAIL path works is the first
# real divergence, which is precisely the moment to already trust it.
if [ -n "${DS_DIVERGENCES_FILE:-}" ] && [ -r "${DS_DIVERGENCES_FILE}" ]; then
	. "${DS_DIVERGENCES_FILE}"
fi
