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
# permissive. Three habits keep it honest:
#
#   * Compare, do not ignore. `sort`ing both sides and requiring equality
#     says "the same lines in a different order". Dropping the lines
#     entirely would say "anything at all", which is not a divergence,
#     it is a blind spot.
#   * Scope to the feature. An entry about `env` ordering has no business
#     excusing a case that never runs `env`, so it checks the case text
#     before it excuses anything -- and it names only the commands that
#     actually diverge, not every command in the neighbourhood.
#   * Say which side is right. "The same lines in a different order"
#     excuses every order, including one that is neither shell's. An
#     ordering entry also asserts the order, and which lines were allowed
#     to move.
#
# An entry that stops matching is reported as stale rather than left
# lying around: once the port and dash agree again, the excuse should go.

# Every registered divergence, by id. Order is the order they are tried.
# Set by the register at the bottom of this file.
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
#
# `dsdiff.sh` now hard-links both shells into a pin directory for the
# duration of a run, which prevents the common causes rather than
# detecting them -- a concurrent rebuild or an `rm -rf target` no longer
# reaches the inodes the run is using. This stays because prevention and
# detection fail differently: the pin cannot survive its own directory
# being removed, and a check that costs two stats on a path already
# heading for a failure report is not worth optimising away.
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

# ds_moved_lines_match A B ERE -- true when every line that sits at a
# different position in the two outputs matches ERE, on both sides.
#
# `ds_same_lines` says "the same lines, some order". This says *which*
# lines were allowed to move. An ordering divergence knows the shape of
# the lines it reorders, so an entry can refuse a permutation of anything
# else -- a diagnostic that raced stdout, a byte dump, an echo -- instead
# of excusing every reordering that happens to preserve the multiset.
ds_moved_lines_match() {
	local -a x y
	local i

	mapfile -t x <<< "$1"
	mapfile -t y <<< "$2"
	[ "${#x[@]}" = "${#y[@]}" ] || return 1

	for ((i = 0; i < ${#x[@]}; i++)); do
		[ "${x[i]}" = "${y[i]}" ] && continue
		[[ ${x[i]} =~ $3 ]] || return 1
		[[ ${y[i]} =~ $3 ]] || return 1
	done
	return 0
}

# ds_blocks_sorted OUT ERE -- true when every maximal run of consecutive
# ERE-matching lines in OUT is in non-decreasing C-collation order.
#
# The other half of a *sorting* divergence, and the half that is easy to
# leave out. "The same lines in a different order" would excuse any order
# at all, including a future one that is neither the reference's nor
# sorted. This is the entry saying which side is the sorted one.
#
# Runs rather than the whole output, so a case that prints an environment,
# something else, and another environment is still judged block by block.
ds_blocks_sorted() {
	local -a y
	local i block=

	mapfile -t y <<< "$1"
	for ((i = 0; i <= ${#y[@]}; i++)); do
		if [ "$i" -lt "${#y[@]}" ] && [[ ${y[i]} =~ $2 ]]; then
			block+=${y[i]}$'\n'
			continue
		fi
		if [ -n "$block" ]; then
			printf '%s' "$block" | LC_ALL=C sort -C || return 1
			block=
		fi
	done
	return 0
}

# ---------------------------------------------------------------------
# The register
# ---------------------------------------------------------------------

DS_DIVERGENCES=(sorted_tables sorted_cmdtable)

# A line the sorted tables produce: an environment entry, `NAME=value`, or
# an alias listing, which is the same thing inside the single quotes
# `single_quote` puts round it. Names are what `endofname` accepts, because
# that is the only way a name reaches either table.
DS_ORDERED_LINE="^'?[A-Za-z_][A-Za-z0-9_]*="

# `env`, `printenv` and `alias` print in name order; dash prints in the
# order its 39 hash buckets happen to chain. See `docs/divergences.md` for
# why the port sorts.
#
# Five conditions, and each one is a regression class the entry must not
# reach:
#
#   * the exit status matches. Reordering output changes nothing else.
#   * the case runs `env`, `printenv` or `alias`. `export -p` and `set`
#     are deliberately *not* in that list even though they print
#     variables: dash already `qsort`s them in `showvars`, so both shells
#     print those sorted and always did. A permutation there is a
#     regression, and an entry naming them could excuse it.
#   * the two outputs hold the same lines, so a changed value, a dropped
#     line, an extra line or a duplicate still fails.
#   * only assignment-shaped lines moved, so a reordering of anything else
#     -- a diagnostic racing stdout, an `od` dump -- is not this.
#   * the port's blocks of those lines are sorted. Without this the entry
#     would excuse any environment order at all.
#
# Known limit, and the reason it is acceptable: the sortedness test reads
# each maximal run of assignment-shaped lines as one block, so a case that
# printed two environments back to back with nothing between them would be
# refused and reported as a failure. Nothing in `tests/corpus` does, and
# for an entry whose job is to not excuse too much, a loud refusal is the
# right way to be wrong. `tests/harness/divtest.sh` pins it so it cannot
# drift silently.
dsdiv_sorted_tables() {
	[ "$3" = "$4" ] || return 1
	ds_case_matches "$5" '(^|[;&|(`{ ])(env|printenv|alias)([ ;&|)`}]|$)' || return 1
	ds_same_lines "$1" "$2" || return 1
	ds_moved_lines_match "$1" "$2" "$DS_ORDERED_LINE" || return 1
	ds_blocks_sorted "$2" "$DS_ORDERED_LINE"
}

# A line `printentry` produces: the reconstructed command path, optionally
# followed by `*` when `cd` marked the cache entry for rehashing. This is
# deliberately narrower than every pathname the shell language can express.
# It covers the corpus's hash listings; an exotic path is refused rather than
# letting this entry mistake arbitrary output for a command-table line.
DS_HASH_LINE='^/?([A-Za-z0-9_.,+%=-]+/)*[A-Za-z0-9_.,+%=-]+\*?$'

# ds_hash_blocks_sorted OUT -- true when each run of printentry-shaped lines
# is ordered by command name. The map is keyed by the name, while printentry
# exposes the full resolved path, so sorting whole lines would be the wrong
# assertion whenever commands came from different PATH elements.
ds_hash_blocks_sorted() {
	local -a y
	local i line block=

	mapfile -t y <<< "$1"
	for ((i = 0; i <= ${#y[@]}; i++)); do
		if [ "$i" -lt "${#y[@]}" ] && [[ ${y[i]} =~ $DS_HASH_LINE ]]; then
			line=${y[i]%\*}
			block+=${line##*/}$'\n'
			continue
		fi
		if [ -n "$block" ]; then
			printf '%s' "$block" | LC_ALL=C sort -C || return 1
			block=
		fi
	done
	return 0
}

# `hash` with no operands prints cached external commands in command-name
# order; dash prints the same entries in its 31-bucket chain order.
#
# The entry requires the same status and line multiset, scopes itself to a
# no-operand hash command, permits only printentry-shaped lines to move, and
# independently proves that the port ordered each such block by command name.
# It intentionally refuses `hash name`, `hash -r`, and exotic path bytes.
dsdiv_sorted_cmdtable() {
	[ "$3" = "$4" ] || return 1
	ds_case_matches "$5" '(^|[;&|(`{][[:space:]]*)hash([[:space:]]*($|[;&|)`}<>]))' || return 1
	ds_same_lines "$1" "$2" || return 1
	ds_moved_lines_match "$1" "$2" "$DS_HASH_LINE" || return 1
	ds_hash_blocks_sorted "$2"
}

# An extra register, for testing the machinery itself. It was written
# because the mechanism had to be exercisable end to end while the real
# register was still empty -- otherwise the first thing to prove the XFAIL
# path worked would have been the first real divergence, which is
# precisely the moment to already trust it. It stays useful for the same
# reason in reverse: a hypothetical entry can be tried against a corpus
# without being registered.
if [ -n "${DS_DIVERGENCES_FILE:-}" ] && [ -r "${DS_DIVERGENCES_FILE}" ]; then
	. "${DS_DIVERGENCES_FILE}"
fi
