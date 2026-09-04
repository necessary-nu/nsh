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

# Some POSIX corrections compose in one generated case: a getopts call can
# both unset OPTARG and print the invoking program in its diagnostic, followed
# by a set-option listing that contains the new hashall option. Decision-style
# entries cannot explain only part of such an output. These normalizers apply
# narrow, independently tested reference-to-port transformations first; the
# final byte-for-byte comparison still has to account for the complete output.
DS_NORMALIZERS=(
	getopts_optarg_unset
	getopts_diagnostic_prefix
	set_hashall_option
	ignoreeof_noninteractive_eof
	fc_listing_format
	ulimit_all_format
	jobs_command_text
	jobs_waited_removal
	case_fallthrough_diagnostic
	fc_substitution_status
	wait_consumed_status
	wait_consumed_jobspec
	getopts_optind_reset
	kill_jobspec
	closed_input_read_error
	closed_output_dup_diagnostic
	missing_command_file_status
	logical_fd_introspection
	ulimit_default_soft_report
	unset_readonly_diagnostic
	dot_missing_file_diagnostic
	parameter_error_diagnostic
	nounset_error_diagnostic
)

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
	local id fn
	DS_DIVERGENCE=
	DS_MATCHED_DIVERGENCES=()
	DS_REF=$1
	DS_PORT=$2
	DS_REF_RC=$3
	DS_PORT_RC=$4
	DS_CASE=$5

	for id in "${DS_NORMALIZERS[@]}"; do
		ds_divergence_enabled "$id" || continue
		"dsnorm_$id"
	done

	if [ "${#DS_MATCHED_DIVERGENCES[@]}" -gt 0 ] &&
		[ "$DS_REF_RC" = "$DS_PORT_RC" ] && [ "$DS_REF" = "$DS_PORT" ]; then
		ds_set_divergence_label
		return 0
	fi

	for id in "${DS_DIVERGENCES[@]}"; do
		fn=dsdiv_$id
		declare -F "$fn" >/dev/null || continue
		if "$fn" "$DS_REF" "$DS_PORT" "$DS_REF_RC" "$DS_PORT_RC" "$DS_CASE"; then
			ds_record_divergence "$id"
			ds_set_divergence_label
			return 0
		fi
	done
	return 1
}

ds_divergence_enabled() {
	local wanted=$1 id
	for id in "${DS_DIVERGENCES[@]}"; do
		[ "$id" = "$wanted" ] && return 0
	done
	return 1
}

ds_record_divergence() {
	local wanted=$1 id
	for id in "${DS_MATCHED_DIVERGENCES[@]}"; do
		[ "$id" = "$wanted" ] && return 0
	done
	DS_MATCHED_DIVERGENCES+=("$wanted")
}

ds_set_divergence_label() {
	local IFS=,
	DS_DIVERGENCE="${DS_MATCHED_DIVERGENCES[*]}"
}

# `getopts` unsets OPTARG when the current option has no argument. dash leaves
# it set to an empty string, so `${OPTARG-U}` observes an empty middle field
# where nsh observes `U`. Restrict the rewrite to that literal observation and
# to complete getopts result records; arbitrary empty fields are untouched.
dsnorm_getopts_optarg_unset() {
	ds_case_matches "$DS_CASE" 'getopts' || return 0
	grep -qF '${OPTARG-U}' "$DS_CASE" 2>/dev/null || return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_REF" |
		sed -E 's/^(([^|]*[[:space:]])?)([[:alnum:]?:])\|\|([0-9]+)$/\1\3|U|\4/')
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence getopts_optarg_unset
}

# POSIX requires the diagnostic to identify the invoking program. The corpus
# invokes file-mode cases as `./script.sh` and command-mode cases as `SH`; no
# other prefix is accepted. Canonicalizing that one known prefix lets this
# compose with OPTARG and option-list changes without ignoring diagnostic text.
dsnorm_getopts_diagnostic_prefix() {
	ds_case_matches "$DS_CASE" 'getopts' || return 0
	local diagnostics normalized
	diagnostics=$(printf '%s\n' "$DS_PORT" |
		grep -E '(Illegal option -.|No arg for -. option)$' || true)
	[ -n "$diagnostics" ] || return 0
	if printf '%s\n' "$diagnostics" |
		grep -Ev '^(SH|\./script\.sh): (Illegal option -.|No arg for -. option)$' |
		grep -q .; then
		return 0
	fi
	normalized=$(printf '%s\n' "$DS_PORT" |
		sed -E 's/^(SH|\.\/script\.sh): (Illegal option -.|No arg for -. option)$/\2/')
	[ "$normalized" != "$DS_PORT" ] || return 0
	DS_PORT=$normalized
	ds_record_divergence getopts_diagnostic_prefix
}

# `set -h` is now accepted, so the option reports gain one exact disabled
# `hashall` record.
dsnorm_set_hashall_option() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{][[:space:]]*)set[[:space:]]+[+-]o([[:space:]]*($|[;&|)`}<>]))' || return 0
	local normalized=$DS_PORT
	normalized=$(printf '%s\n' "$normalized" |
		sed -e '/^hashall         off$/d' -e '/^set +o hashall$/d')
	[ "$normalized" != "$DS_PORT" ] || return 0
	DS_PORT=$normalized
	ds_record_divergence set_hashall_option
}

# dash applies `ignoreeof` to a non-interactive top-level script, prints its
# interactive retry diagnostic fifty times, and only then accepts EOF. POSIX
# limits the option to interactive shells, so the port terminates immediately.
# Remove only dash's complete, exact retry suffix; a different count, message,
# status, or any residual output remains a failure.
dsnorm_ignoreeof_noninteractive_eof() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{][[:space:]]*)set[[:space:]]+(-[A-Za-z]*I[A-Za-z]*|-o[[:space:]]+ignoreeof)([[:space:]]*($|[;&|)`}<>]))' || return 0
	local i suffix=$'\nUse "exit" to leave shell.' long_suffix normalized prompted_suffix=
	for ((i = 1; i < 50; i++)); do
		suffix+=$'\n\nUse "exit" to leave shell.'
	done
	for ((i = 0; i < 50; i++)); do
		prompted_suffix+=$'\nUse "exit" to leave shell.\n$ '
	done
	long_suffix=$'\n'"$suffix"
	if [[ $DS_REF == *"$prompted_suffix" ]]; then
		normalized=${DS_REF%"$prompted_suffix"}
	elif [[ $DS_REF == *"$long_suffix" ]]; then
		normalized=${DS_REF%"$long_suffix"}
	elif [[ $DS_REF == *"$suffix" ]]; then
		normalized=${DS_REF%"$suffix"}
	else
		return 0
	fi
	DS_REF=$normalized
	ds_record_divergence ignoreeof_noninteractive_eof
}

# POSIX specifies a tab between the fc event number and command, and a leading
# tab when -n suppresses the number. dash instead uses four spaces before the
# number and one afterwards, and leaves continuation lines unindented.
dsnorm_fc_listing_format() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{][[:space:]]*)fc[[:space:]]+-[A-Za-z]*l' || return 0
	[ -n "$DS_PORT" ] || return 0
	local line out prefix numbered=0 changed=0 numberless=0
	local numbered_re=$'^(([$>] )*)([0-9]+)\t(.*)$'
	local numberless_re=$'^(([$>] )*)\t(.*)$'
	local -a lines normalized=()
	ds_case_matches "$DS_CASE" 'fc[[:space:]]+-[A-Za-z]*n[A-Za-z]*' && numberless=1
	mapfile -t lines <<< "$DS_PORT"
	for line in "${lines[@]}"; do
		out=$line
		if [[ $line =~ $numbered_re ]]; then
			prefix=${BASH_REMATCH[1]}
			out="${prefix}    ${BASH_REMATCH[3]} ${BASH_REMATCH[4]}"
			numbered=1
			changed=1
		elif [ "$numberless" -eq 1 ] && [[ $line =~ $numberless_re ]]; then
			out="${BASH_REMATCH[1]}${BASH_REMATCH[3]}"
			numbered=1
			changed=1
		elif [ "$numbered" -eq 1 ] && [[ $line == $'\t'* ]]; then
			out=${line#$'\t'}
			changed=1
		else
			numbered=0
		fi
		normalized+=("$out")
	done
	[ "$changed" -eq 1 ] || return 0
	printf -v DS_PORT '%s\n' "${normalized[@]}"
	DS_PORT=${DS_PORT%$'\n'}
	ds_record_divergence fc_listing_format
}

# POSIX.1-2024 requires every `ulimit -a` row to identify the resource, its
# units, its option, and its value. dash's older labels omit the option and use
# abbreviated units. Map the twelve exact new labels back to dash's layout;
# values remain byte-for-byte compared.
dsnorm_ulimit_all_format() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{][[:space:]]*)ulimit[[:space:]]+-[A-Za-z]*a' || return 0
	[ -n "$DS_PORT" ] || return 0
	local line out name option value old accepted changed=0
	local row_re='^(.+)[[:space:]]\(-([a-z])\)[[:space:]](unlimited|[0-9]+|N)$'
	local -a lines normalized=()
	mapfile -t lines <<< "$DS_PORT"
	for line in "${lines[@]}"; do
		out=$line
		if [[ $line =~ $row_re ]]; then
			name=${BASH_REMATCH[1]}
			option=${BASH_REMATCH[2]}
			value=${BASH_REMATCH[3]}
			accepted=
			case $option in
			t) old='time(seconds)'; accepted='CPU time (seconds)' ;;
			f) old='file(blocks)'; accepted='file size (512-byte units)|file size (N-byte units)' ;;
			d) old='data(kbytes)'; accepted='data segment size (1024-byte units)|data segment size (N-byte units)' ;;
			s) old='stack(kbytes)'; accepted='stack size (1024-byte units)|stack size (N-byte units)' ;;
			c) old='coredump(blocks)'; accepted='core file size (512-byte units)|core file size (N-byte units)' ;;
			m) old='memory(kbytes)'; accepted='resident memory (1024-byte units)|resident memory (N-byte units)' ;;
			l) old='locked memory(kbytes)'; accepted='locked memory (1024-byte units)|locked memory (N-byte units)' ;;
			p) old='process'; accepted='processes' ;;
			n) old='nofiles'; accepted='open files' ;;
			v) old='vmemory(kbytes)'; accepted='address space (1024-byte units)|address space (N-byte units)' ;;
			w) old='locks'; accepted='file locks' ;;
			r) old='rtprio'; accepted='realtime priority' ;;
			esac
			case "|$accepted|" in
			*"|$name|"*) ;;
			*) return 0 ;;
			esac
			printf -v out '%-20s %s' "$old" "$value"
			changed=1
		fi
		normalized+=("$out")
	done
	[ "$changed" -eq 1 ] || return 0
	printf -v DS_PORT '%s\n' "${normalized[@]}"
	DS_PORT=${DS_PORT%$'\n'}
	ds_record_divergence ulimit_all_format
}

ds_command_appears_in_case() {
	local command=$1 compact_command compact_case
	compact_command=$(printf '%s' "$command" | tr -d '[:space:]{}')
	compact_case=$(tr -d '[:space:]{}' < "$DS_CASE")
	[ -n "$compact_command" ] && [[ $compact_case == *"$compact_command"* ]]
}

# dash leaves the POSIX `<command>` field empty in non-monitor `jobs` output.
# The port retains it. Use dash's complete padded status record as the prefix,
# and accept a suffix only when that command text is present in the case.
dsnorm_jobs_command_text() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{])[[:space:]]*jobs([[:space:];&|)`}]|$)' || return 0
	[ -n "$DS_PORT" ] || return 0
	local line ref_line command candidate changed=0 matched
	local -a ref_lines port_lines normalized
	mapfile -t ref_lines <<< "$DS_REF"
	mapfile -t port_lines <<< "$DS_PORT"
	for line in "${port_lines[@]}"; do
		candidate=$line
		matched=0
		for ref_line in "${ref_lines[@]}"; do
			if [[ $ref_line =~ ^\[[0-9]+\][[:space:]][+\ -][[:space:]]([0-9]+[[:space:]]+)?(Running|Done|Done\([0-9]+\)|Stopped.*)[[:space:]]+$ ]] &&
				[[ $line == "$ref_line"?* ]]; then
				command=${line#"$ref_line"}
				ds_command_appears_in_case "$command" || continue
				candidate=$ref_line
				matched=1
				break
			fi
			if [[ $ref_line == *'|' ]]; then
				local without_pipe anchor rest
				without_pipe=${ref_line%|}
				anchor=$(printf '%s' "$without_pipe" | sed -E 's/[[:space:]]+$//')
				if [ -n "$anchor" ] && [[ $line == "$anchor"* ]]; then
					rest=${line#"$anchor"}
					if [[ $rest =~ ^[[:space:]]+(.+)[[:space:]]\|$ ]]; then
						command=${BASH_REMATCH[1]}
						ds_command_appears_in_case "$command" || continue
						candidate=$ref_line
						matched=1
						break
					fi
				fi
			fi
			if [[ $ref_line =~ ^([[:space:]]+)([0-9]+)([[:space:]]+)\|$ ]] &&
				[[ $line =~ ^${BASH_REMATCH[1]}${BASH_REMATCH[2]}[[:space:]]+(.+)[[:space:]]\|$ ]]; then
				command=${BASH_REMATCH[1]}
				ds_command_appears_in_case "$command" || continue
				candidate=$ref_line
				matched=1
				break
			fi
			if [[ $ref_line =~ ^[[:space:]]+[0-9]+[[:space:]]+$ ]] &&
				[[ $line == "$ref_line"?* ]]; then
				command=${line#"$ref_line"}
				ds_command_appears_in_case "$command" || continue
				candidate=$ref_line
				matched=1
				break
			fi
		done
		[ "$matched" -eq 0 ] || changed=1
		normalized+=("$candidate")
	done
	[ "$changed" -eq 1 ] || return 0
	printf -v DS_PORT '%s\n' "${normalized[@]}"
	DS_PORT=${DS_PORT%$'\n'}
	ds_record_divergence jobs_command_text
}

# POSIX requires a successfully waited job to be removed. dash retains it and
# a later `jobs` prints a padded Done record. Remove only those complete Done
# records, and only when the case text has wait before jobs.
dsnorm_jobs_waited_removal() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{])[[:space:]]*wait([[:space:];&|)`}]|$)' || return 0
	ds_case_matches "$DS_CASE" '(^|[;&|(`{])[[:space:]]*jobs([[:space:];&|)`}]|$)' || return 0
	local flat line changed=0
	local -a lines normalized=()
	flat=$(tr '\n' ' ' < "$DS_CASE")
	[[ $flat =~ wait.*jobs ]] || return 0
	mapfile -t lines <<< "$DS_REF"
	for line in "${lines[@]}"; do
		if [[ $line =~ ^\[[0-9]+\][[:space:]][+\ -][[:space:]]([0-9]+[[:space:]]+)?Done(\([0-9]+\))?[[:space:]]*$ ]]; then
			changed=1
			continue
		fi
		normalized+=("$line")
	done
	[ "$changed" -eq 1 ] || return 0
	if [ "${#normalized[@]}" -eq 0 ]; then
		DS_REF=
	else
		printf -v DS_REF '%s\n' "${normalized[@]}"
		DS_REF=${DS_REF%$'\n'}
	fi
	ds_record_divergence jobs_waited_removal
}

# The port tokenizes the POSIX.1-2024 case fall-through operator as one token,
# so its syntax diagnostic names `;&`; dash stops at the semicolon or ampersand.
dsnorm_case_fallthrough_diagnostic() {
	grep -qF ';&' "$DS_CASE" 2>/dev/null || return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_REF" |
		sed -E 's/Syntax error: "(&&|&|;)" unexpected/Syntax error: ";\&" unexpected/')
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence case_fallthrough_diagnostic
}

# `fc -s` returns the status of the command it executes. dash reports zero for
# the corpus's `true=false` substitution even though the resulting `false`
# returns one; the port propagates one.
dsnorm_fc_substitution_status() {
	grep -qF 'fc -s true=false' "$DS_CASE" 2>/dev/null || return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_REF" | sed 's/^rc=0$/rc=1/')
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence fc_substitution_status
}

# A successfully waited PID is no longer known. POSIX consequently requires a
# repeated wait for it to return 127; dash retains the completed job and
# returns zero again.
dsnorm_wait_consumed_status() {
	local mode= operand line changed=0 last i
	local -a lines normalized=()
	if grep -qE 'wait[[:space:]]+\$![[:space:]]+\$!' "$DS_CASE" 2>/dev/null; then
		mode=positional
	else
		operand=$(grep -oE 'wait[[:space:]]+\$[A-Za-z_][A-Za-z0-9_]*' "$DS_CASE" 2>/dev/null |
			sed 's/.*\$//' | sort | uniq -d | head -1)
		[ -n "$operand" ] && mode=variable
	fi
	[ -n "$mode" ] || return 0
	mapfile -t lines <<< "$DS_REF"
	last=$((${#lines[@]} - 1))
	for ((i = 0; i < ${#lines[@]}; i++)); do
		line=${lines[i]}
		if [ "$mode" = positional ] && [ "$i" -eq "$last" ] && [ "$line" = 0 ]; then
			line=127
			changed=$((changed + 1))
		elif [ "$mode" = variable ] && [ "$line" = second=0 ]; then
			line=second=127
			changed=$((changed + 1))
		fi
		normalized+=("$line")
	done
	[ "$changed" -eq 1 ] || return 0
	printf -v DS_REF '%s\n' "${normalized[@]}"
	DS_REF=${DS_REF%$'\n'}
	ds_record_divergence wait_consumed_status
}

# The same removal rule applies to a job ID: after a successful bare wait,
# `%1` no longer names a job. dash returns the stale status again; the port
# diagnoses the unknown job and returns its utility-error status.
dsnorm_wait_consumed_jobspec() {
	grep -qE 'wait[[:space:]]+%[0-9]+' "$DS_CASE" 2>/dev/null || return 0
	local flat line jobspec diagnostics=0 statuses=0
	local -a lines normalized=()
	flat=$(tr '\n' ' ' < "$DS_CASE")
	[[ $flat =~ (^|[; ])wait([; ]).*wait[[:space:]]+%[0-9]+ ]] || return 0
	jobspec=$(grep -oE 'wait[[:space:]]+%[0-9]+' "$DS_CASE" 2>/dev/null |
		tail -1 | sed 's/.*[[:space:]]//')
	[ -n "$jobspec" ] || return 0
	mapfile -t lines <<< "$DS_PORT"
	for line in "${lines[@]}"; do
		[[ $line =~ ^SH:[[:space:]][0-9]+:[[:space:]]wait:[[:space:]]No[[:space:]]such[[:space:]]job:[[:space:]]${jobspec}$ ]] &&
			diagnostics=$((diagnostics + 1))
		[ "$line" = 'rc=2' ] && statuses=$((statuses + 1))
	done
	[ "$diagnostics" -eq 1 ] && [ "$statuses" -eq 1 ] || return 0
	for line in "${lines[@]}"; do
		if [[ $line =~ ^SH:[[:space:]][0-9]+:[[:space:]]wait:[[:space:]]No[[:space:]]such[[:space:]]job:[[:space:]]${jobspec}$ ]]; then
			continue
		fi
		if [ "$line" = 'rc=2' ]; then
			line='rc=0'
		fi
		normalized+=("$line")
	done
	printf -v DS_PORT '%s\n' "${normalized[@]}"
	DS_PORT=${DS_PORT%$'\n'}
	ds_record_divergence wait_consumed_jobspec
}

# Assigning OPTIND=1 restarts getopts at the first operand. dash keeps its
# hidden scan cursor and continues (or immediately finishes); the port's cursor
# is the specified OPTIND variable. The corpus deliberately starts with -a, so
# every accepted restart must reproduce option a and OPTIND 2.
dsnorm_getopts_optind_reset() {
	grep -qF 'OPTIND=1' "$DS_CASE" 2>/dev/null || return 0
	grep -qE '(^|[;&[:space:]])getopts([;&[:space:]]|$)' "$DS_CASE" 2>/dev/null || return 0
	grep -qE 'set --[[:space:]]+-a([[:space:];&|)]|$)' "$DS_CASE" 2>/dev/null || return 0
	local line candidate changed=0
	local -a lines first_pass normalized
	mapfile -t lines <<< "$DS_REF"
	for line in "${lines[@]}"; do
		if [[ $line == 1:* ]]; then
			first_pass+=("$line")
		fi
		if [[ $line == optind=* ]] && [ "${#first_pass[@]}" -gt 0 ]; then
			local prior
			for prior in "${first_pass[@]}"; do
				normalized+=("2:${prior#1:}")
			done
			changed=1
		fi
		candidate=$line
		case $candidate in
		after:*) candidate='after:a 2' ;;
		again=*) candidate='again=a ind=2' ;;
		'o=?') candidate='o=a' ;;
		'?') candidate='a' ;;
		esac
		if [[ $candidate =~ ^[b-z][[:space:]][3-9][0-9]*$ ]]; then
			candidate='a 2'
		fi
		[ "$candidate" = "$line" ] || changed=1
		normalized+=("$candidate")
	done
	[ "$changed" -eq 1 ] || return 0
	printf -v candidate '%s\n' "${normalized[@]}"
	candidate=${candidate%$'\n'}
	[ "$candidate" = "$DS_PORT" ] || return 0
	DS_REF=$candidate
	ds_record_divergence getopts_optind_reset
}

# POSIX kill accepts a job-control job ID. dash passes `%1` to kill(2) as if it
# were a PID and diagnoses ESRCH; the port resolves the job and signals it.
dsnorm_kill_jobspec() {
	grep -qE 'kill([^\n]|\n)*%[0-9]+' "$DS_CASE" 2>/dev/null || return 0
	local line skip_blank=0 changed=0 expected diagnostics=0
	local -a lines normalized=()
	expected=$(grep -oE '(^|[;&])[[:space:]]*kill[^;&]*%[0-9]+' "$DS_CASE" 2>/dev/null | wc -l)
	[ "$expected" -gt 0 ] || return 0
	mapfile -t lines <<< "$DS_REF"
	for line in "${lines[@]}"; do
		[[ $line =~ ^SH:[[:space:]][0-9]+:[[:space:]]kill:[[:space:]]No[[:space:]]such[[:space:]]process$ ]] &&
			diagnostics=$((diagnostics + 1))
	done
	[ "$diagnostics" -eq "$expected" ] || return 0
	for line in "${lines[@]}"; do
		if [[ $line =~ ^SH:[[:space:]][0-9]+:[[:space:]]kill:[[:space:]]No[[:space:]]such[[:space:]]process$ ]]; then
			changed=1
			skip_blank=1
			continue
		fi
		if [ "$skip_blank" -eq 1 ] && [ -z "$line" ]; then
			skip_blank=0
			continue
		fi
		skip_blank=0
		normalized+=("$line")
	done
	[ "$changed" -eq 1 ] || return 0
	if [ "${#normalized[@]}" -eq 0 ]; then
		DS_REF=
	else
		printf -v DS_REF '%s\n' "${normalized[@]}"
		DS_REF=${DS_REF%$'\n'}
	fi
	ds_record_divergence kill_jobspec
}

# The safe logical descriptor layer reports a read from closed input as an I/O
# error (status 128 plus a diagnostic); dash's stdio path reports ordinary EOF
# (status 1). Both are failures, but the former preserves the actual EBADF.
dsnorm_closed_input_read_error() {
	ds_case_matches "$DS_CASE" '0<&-' || return 0
	grep -qE 'read[[:space:]]' "$DS_CASE" 2>/dev/null || return 0
	local line changed=0
	local -a lines normalized
	mapfile -t lines <<< "$DS_PORT"
	for line in "${lines[@]}"; do
		if [[ $line =~ ^(SH|sh):[[:space:]][0-9]+:[[:space:]]read:[[:space:]]read[[:space:]]error:[[:space:]]Bad[[:space:]]file[[:space:]]descriptor$ ]] ||
			[ "$line" = 'Bad file descriptor' ]; then
			changed=1
			continue
		fi
		if [[ $line == 'rc=128' || $line == 'rc=128 '* ]]; then
			line="rc=1${line#rc=128}"
			changed=1
		fi
		normalized+=("$line")
	done
	[ "$changed" -eq 1 ] || return 0
	printf -v DS_PORT '%s\n' "${normalized[@]}"
	DS_PORT=${DS_PORT%$'\n'}
	ds_record_divergence closed_input_read_error
}

# Redirections are applied left-to-right. After `1>&-`, `2>&1` names a closed
# source; the port diagnoses that exact EBADF while dash exits two silently.
dsnorm_closed_output_dup_diagnostic() {
	grep -qF '1>&- 2>&1' "$DS_CASE" 2>/dev/null || return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_PORT" |
		sed -E '/^(SH|sh): [0-9]+: 1: Bad file descriptor$/d')
	[ "$normalized" != "$DS_PORT" ] || return 0
	DS_PORT=$normalized
	ds_record_divergence closed_output_dup_diagnostic
}

# POSIX assigns 127 when the command file named to sh cannot be found. dash
# uses its generic special-builtin error status 2 for the same diagnostic.
dsnorm_missing_command_file_status() {
	grep -qF '"$0" - -c' "$DS_CASE" 2>/dev/null || return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_REF" | sed 's/^rc=2$/rc=127/')
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence missing_command_file_status
}

# Builtins observe the shell's logical descriptor table, not temporary host fd
# projection. Linux's /dev/stdin therefore cannot reveal whether a here-doc is
# backed by dash's pipe or temporary file; content and redirection semantics are
# unchanged. These implementation-introspection probes report OTHER.
dsnorm_logical_fd_introspection() {
	grep -qF '/dev/stdin' "$DS_CASE" 2>/dev/null || return 0
	grep -qF '<<' "$DS_CASE" 2>/dev/null || return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_REF" | sed -e 's/^REGFILE$/OTHER/' -e 's/^PIPE$/OTHER/')
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence logical_fd_introspection
}

# With no -H/-S and no operand, POSIX defaults a ulimit query to the soft
# limit. dash suppresses that query in these sequences; remove exactly one port
# line equal to the numeric limit just set and require everything else equal.
dsnorm_ulimit_default_soft_report() {
	grep -qE 'ulimit[[:space:]]+(-S[[:space:]]+)?-n[[:space:]]+[0-9]+' "$DS_CASE" 2>/dev/null || return 0
	grep -qE 'ulimit[[:space:]]+-n([[:space:];&|)]|$)' "$DS_CASE" 2>/dev/null || return 0
	local value i candidate
	local -a lines copy
	value=$(grep -oE 'ulimit[[:space:]]+(-S[[:space:]]+)?-n[[:space:]]+[0-9]+' "$DS_CASE" |
		sed -n '1s/.*[[:space:]]\([0-9][0-9]*\)$/\1/p')
	[ -n "$value" ] || return 0
	mapfile -t lines <<< "$DS_PORT"
	[ "${#lines[@]}" -eq "$(($(printf '%s\n' "$DS_REF" | wc -l) + 1))" ] || return 0
	for ((i = 0; i < ${#lines[@]}; i++)); do
		[ "${lines[i]}" = "$value" ] || continue
		copy=("${lines[@]:0:i}" "${lines[@]:i+1}")
		printf -v candidate '%s\n' "${copy[@]}"
		candidate=${candidate%$'\n'}
		if [ "$candidate" = "$DS_REF" ]; then
			DS_PORT=$candidate
			ds_record_divergence ulimit_default_soft_report
			return 0
		fi
	done
}

# `unset` refusing a read-only name. Both shells end a non-interactive shell
# with status 2, so the whole of the difference is the diagnostic: dash
# writes it through its `$0: line: ` spine, and the port keeps the
# prefix-less `unset: NAME is read-only` that
# `[spec:nsh:req:compat.smoosh.error-contracts]` fixes. Rewrite only that one
# complete line, only in a case that both makes a name read-only and unsets
# one, and carry the name across so a diagnostic about a different variable
# is not excused. The statuses and the rest of the output stay compared byte
# for byte, which is what holds the fatality here: a port that ran on to the
# next command would differ in a line this cannot reach.
dsnorm_unset_readonly_diagnostic() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{][[:space:]]*)readonly[[:space:]]' || return 0
	ds_case_matches "$DS_CASE" '(^|[;&|(`{][[:space:]]*)unset[[:space:]]' || return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_REF" |
		sed -E 's#^(([$>] )*)(SH|\./script\.sh): [0-9]+: unset: ([A-Za-z_][A-Za-z0-9_]*): is read only$#\1unset: \4 is read-only#')
	# Two corpus cases route the diagnostic through `sed 's|^[^:]*: ||'`,
	# which strips one colon field from each side: dash keeps its line number
	# and the port loses its `unset: `. Rewrite that shape only for a case
	# that contains that exact filter, so an unfiltered diagnostic missing
	# its command name is still a difference.
	if ds_case_contains "$DS_CASE" "sed 's|^[^:]*: ||'"; then
		normalized=$(printf '%s\n' "$normalized" |
			sed -E 's#^[0-9]+: unset: ([A-Za-z_][A-Za-z0-9_]*): is read only$#\1 is read-only#')
	fi
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence unset_readonly_diagnostic
}

# `.` refusing a file it cannot find. Both shells end a non-interactive
# shell with status 2 since `bash.divergences.error-boundary-status-collisions`,
# so the whole of the difference is the diagnostic: dash writes it through its
# `$0: line: ` spine and names the failed `open`, where the port keeps the
# prefix-less `.: NAME: not found` that
# `[spec:nsh:req:compat.smoosh.error-contracts]` fixes. Rewrite only that one
# complete line, only in a case that runs a `.` at a command position, and
# carry the operand across so a diagnostic about a different file is not
# excused. The statuses and the rest of the output stay compared byte for
# byte, which is what holds the fatality here.
dsnorm_dot_missing_file_diagnostic() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{][[:space:]]*)\.[[:space:]]' || return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_REF" |
		sed -E 's#^(([$>] )*)(SH|\./script\.sh): [0-9]+: \.: cannot open (.+): No such file$#\1.: \4: not found#')
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence dot_missing_file_diagnostic
}

# A `${name?word}` expansion refusing an unset parameter. Same shape and same
# reason as the entry above: the statuses agree at 2 and only dash's spine
# differs, with a sourced script's name in it as a second field when the
# failure happened inside one.
#
# The names are read out of the case rather than matched as a pattern, so the
# rewrite reaches exactly the parameters the script wrote a `?` expansion for.
# A diagnostic about any other name -- or any other diagnostic that lost its
# prefix -- is still a difference, which is what stops this from becoming a
# blanket excuse for a missing spine.
dsnorm_parameter_error_diagnostic() {
	local name normalized=$DS_REF
	local -a names
	mapfile -t names < <(grep -oE '\$\{[A-Za-z_][A-Za-z0-9_]*(\[[^]]*\])?:?\?' "$DS_CASE" 2>/dev/null |
		sed -E 's/^\$\{//; s/(\[[^]]*\])?:?\?$//' | sort -u)
	[ "${#names[@]}" -gt 0 ] || return 0
	for name in "${names[@]}"; do
		case $name in
		[A-Za-z_]*) ;;
		*) continue ;;
		esac
		normalized=$(printf '%s\n' "$normalized" |
			sed -E "s#^(([\$>] )*)(SH|\./script\.sh): [0-9]+: (\./[^ :]+: )?($name): #\1$name: #")
	done
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence parameter_error_diagnostic
}

# Reading an unset parameter under `set -u`. The third member of the same
# class, and the one the generated corpora reach most often. Scoped to a case
# that actually enables `nounset`, and to the one message that option
# produces: a line whose remainder is anything but `parameter not set` keeps
# its prefix and stays a difference, so this cannot excuse a spine dropped
# from an unrelated diagnostic.
dsnorm_nounset_error_diagnostic() {
	ds_case_matches "$DS_CASE" '(^|[;&|(`{][[:space:]]*)set[[:space:]]+([-+][A-Za-z]*u|-o[[:space:]]+nounset)' ||
		return 0
	local normalized
	normalized=$(printf '%s\n' "$DS_REF" |
		sed -E 's#^(([$>] )*)(SH|\./script\.sh): [0-9]+: (\./[^ :]+: )?([A-Za-z_][A-Za-z0-9_]*): parameter not set$#\1\5: parameter not set#')
	[ "$normalized" != "$DS_REF" ] || return 0
	DS_REF=$normalized
	ds_record_divergence nounset_error_diagnostic
}

# A command substitution in a prompt is expanded, where dash replays the outer
# parse's pushed-back token into the re-entered parse. `parser::expand_string`
# starts a fresh parse of the prompt's text on top of whatever the caller was
# reading; with a string-fed shell the outer parse has already pushed `Eof`
# back, so dash's `$( )` reaches end of file at once and its backquote form
# expands to nothing without saying so. `[spec:posix:req:param.ps4]` leaves
# substitution in `PS4` unspecified, so *declining* to expand would conform --
# but a parse diagnostic for text that parses is a stale token rather than a
# choice, which is why the port is the right side here.
#
# Four complete result pairs, one per generated witness. Every byte of both
# sides is pinned, including where in the script dash's outer parse happens to
# have reached: in the `xtrace nested PS4` case its first traced command
# expands correctly and only the last one does not.
dsdiv_re_entered_prompt_substitution() {
	[ "$3" = 0 ] && [ "$4" = 0 ] || return 1
	local syntax='SH: 1: Syntax error: end of file unexpected (expecting ")")'

	ds_case_contains "$5" "PS4='[\$(echo sub)] '" &&
		ds_exact_pair "$1" "$2" \
		"$syntax"$'\n[$(echo sub)] echo hi\nhi' \
		$'[sub] echo hi\nhi' && return 0
	ds_case_contains "$5" "PS4='[\`echo bq\`] '" &&
		ds_exact_pair "$1" "$2" $'[] echo hi\nhi' $'[bq] echo hi\nhi' && return 0
	ds_case_contains "$5" "PS4='\$(exit 3)x '" &&
		ds_exact_pair "$1" "$2" \
		"$syntax"$'\n$(exit 3)x echo hi\nhi\nx echo rc=0\nrc=0' \
		$'x echo hi\nhi\nx echo rc=0\nrc=0' && return 0
	ds_case_contains "$5" "PS4='\$(echo PS) '" &&
		ds_exact_pair "$1" "$2" \
		$'PS echo hi\nhi\n'"$syntax"$'\n$(echo PS) set +x' \
		$'PS echo hi\nhi\nPS set +x' && return 0
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

# ds_case_contains CASE_FILE TEXT -- fixed-string form of the scoping guard.
ds_case_contains() {
	grep -qF -- "$2" "$1" 2>/dev/null
}

# ds_exact_pair REF PORT EXPECTED_REF EXPECTED_PORT -- accept only one fully
# specified result pair. This is intentionally stronger than normalizing a
# fragment: a changed prefix, suffix, diagnostic, field count, or byte fails.
ds_exact_pair() {
	[ "$1" = "$3" ] && [ "$2" = "$4" ]
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

DS_DIVERGENCES=(
	alias_stdout_format
	sorted_tables
	sorted_cmdtable
	trap_subshell_listing
	exit_trap_final_status
	fc_recursion_error_status
	logical_fd_low_nofile_survival
	trap_p_option
	utf8_pattern_characters
	c_locale_multibyte_ifs
	parameter_operand_quote_preservation
	empty_quote_field_anchors
	re_entered_prompt_substitution
	"${DS_NORMALIZERS[@]}"
)

# dash quotes an alias's complete `name=value` definition. POSIX requires the
# name and equals sign outside the quoted value, so the port moves only the
# opening quote: `'name=value'` becomes `name='value'`. The rest of the line is
# byte-identical even when the value itself contains quotes.
DS_ALIAS_PORT_LINE="^(([$>] )*)(alias[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*='"

# A definition-only command must not qualify: it prints nothing, and newly
# printing a definition would be a regression. Match only a bare `alias` or a
# name operand without `=` at the end of a simple command.
ds_case_displays_alias() {
	local start='(^|[;&|(`{][[:space:]]*)'
	local redirection='[0-9]*[<>][^[:space:];|)]*'
	local end='[[:space:]]*($|[;&|)`}])'
	grep -qE "${start}alias([[:space:]]+${redirection})*${end}" "$1" 2>/dev/null && return 0
	grep -qE "${start}alias([[:space:]]+[A-Za-z_][A-Za-z0-9_]*)+([[:space:]]+${redirection})*${end}" "$1" 2>/dev/null && return 0
	grep -qE '(^|[;&|(`{][[:space:]]*)alias[[:space:]]+[A-Za-z_][A-Za-z0-9_]*=' "$1" 2>/dev/null || return 1
	grep -qE '(^|[;&|(`{][[:space:]]*)(command[[:space:]]+-v|type)[[:space:]]+[A-Za-z_][A-Za-z0-9_]*' "$1" 2>/dev/null
}

# Rewrite only dash-shaped alias listing lines into the port's POSIX shape.
# Return failure unless at least one line changed, so an arbitrary permutation
# in a case that merely runs `alias` cannot borrow this entry.
ds_alias_reference_to_port() {
	local input=$1 line changed=1
	while IFS= read -r line || [ -n "$line" ]; do
		if [[ $line =~ ^(([$\>] )*)alias[[:space:]]\'([A-Za-z_][A-Za-z0-9_]*)=(.*)$ ]]; then
			printf "%salias %s='%s\n" "${BASH_REMATCH[1]}" \
				"${BASH_REMATCH[3]}" "${BASH_REMATCH[4]}"
			changed=0
		elif [[ $line =~ ^(([$\>] )*)\'([A-Za-z_][A-Za-z0-9_]*)=(.*)$ ]]; then
			printf "%s%s='%s\n" "${BASH_REMATCH[1]}" \
				"${BASH_REMATCH[3]}" "${BASH_REMATCH[4]}"
			changed=0
		else
			printf '%s\n' "$line"
		fi
	done <<< "$input"
	return "$changed"
}

# Prompt prefixes are merged into the first listing line in interactive
# captures. Assert ordering by the alias key after removing those prefixes,
# rather than accidentally sorting by `$` or excluding the first alias.
ds_alias_blocks_sorted() {
	local -a lines
	local i line block=
	mapfile -t lines <<< "$1"
	for ((i = 0; i <= ${#lines[@]}; i++)); do
		if [ "$i" -lt "${#lines[@]}" ] && [[ ${lines[i]} =~ $DS_ALIAS_PORT_LINE ]]; then
			line=${lines[i]}
			while [[ $line =~ ^[$\>][[:space:]] ]]; do line=${line:2}; done
			line=${line#alias }
			block+=${line%%=*}$'\n'
			continue
		fi
		if [ -n "$block" ]; then
			printf '%s' "$block" | LC_ALL=C sort -C || return 1
			block=
		fi
	done
	return 0
}

ds_alias_prompt_prefixes_match() {
	local -a left right
	local i left_prefix right_prefix
	mapfile -t left <<< "$1"
	mapfile -t right <<< "$2"
	[ "${#left[@]}" = "${#right[@]}" ] || return 1
	for ((i = 0; i < ${#left[@]}; i++)); do
		left_prefix=NON_ALIAS
		right_prefix=NON_ALIAS
		[[ ${left[i]} =~ $DS_ALIAS_PORT_LINE ]] && left_prefix=${BASH_REMATCH[1]}
		[[ ${right[i]} =~ $DS_ALIAS_PORT_LINE ]] && right_prefix=${BASH_REMATCH[1]}
		[ "$left_prefix" = "$right_prefix" ] || return 1
	done
}

ds_alias_strip_prompt_prefixes() {
	local line prefix
	while IFS= read -r line || [ -n "$line" ]; do
		if [[ $line =~ $DS_ALIAS_PORT_LINE ]]; then
			prefix=${BASH_REMATCH[1]}
			line=${line#"$prefix"}
		fi
		printf '%s\n' "$line"
	done <<< "$1"
}

# `alias` output differs from dash in quote placement, and a multi-entry
# listing also differs in order. This entry proves both differences together:
# after the one exact quote move the line multisets must match, only
# alias-shaped lines may move, and the port's alias blocks must be sorted.
dsdiv_alias_stdout_format() {
	local normalized unprompted_reference unprompted_port
	[ "$3" = "$4" ] || return 1
	ds_case_displays_alias "$5" || return 1
	normalized=$(ds_alias_reference_to_port "$1") || return 1
	ds_alias_prompt_prefixes_match "$normalized" "$2" || return 1
	unprompted_reference=$(ds_alias_strip_prompt_prefixes "$normalized")
	unprompted_port=$(ds_alias_strip_prompt_prefixes "$2")
	ds_same_lines "$unprompted_reference" "$unprompted_port" || return 1
	ds_moved_lines_match "$unprompted_reference" "$unprompted_port" \
		"^(alias[[:space:]]+)?[A-Za-z_][A-Za-z0-9_]*='" || return 1
	ds_alias_blocks_sorted "$2"
}

# A line the variable table produces. Names are what `endofname` accepts,
# because that is the only way a name reaches the table.
DS_ORDERED_LINE="^[A-Za-z_][A-Za-z0-9_]*="

# `env` and `printenv` print in name order; dash prints in the order its 39
# hash buckets happen to chain. Alias ordering is checked by
# `alias_stdout_format`, because its quote-placement difference composes with
# the ordering difference. See `docs/divergences.md` for why the port sorts.
#
# Five conditions, and each one is a regression class the entry must not
# reach:
#
#   * the exit status matches. Reordering output changes nothing else.
#   * the case runs `env` or `printenv`. `export -p` and `set`
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
	ds_case_matches "$5" '(^|[;&|(`{ ])(env|printenv)([ ;&|)`}]|$)' || return 1
	ds_same_lines "$1" "$2" || return 1
	ds_moved_lines_match "$1" "$2" "$DS_ORDERED_LINE" || return 1
	ds_blocks_sorted "$2" "$DS_ORDERED_LINE"
}

# A line `printentry` produces: the reconstructed command path, optionally
# followed by `*` when `cd` marked the cache entry for rehashing. Requiring a
# directory separator is deliberately narrower than every pathname the shell
# can resolve: it keeps status text such as `rc=0` and arbitrary bare words out
# of the sortable block. The corpus's hash listings use paths; a bare or exotic
# path is refused rather than mistaken for command-table output.
DS_HASH_LINE='^/?([A-Za-z0-9_.,+%=-]+/)+[A-Za-z0-9_.,+%=-]+\*?$'

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

# POSIX 2024 requires a no-operand `trap` executed in a subshell, before that
# subshell changes a trap, to report the commands inherited on entry even
# though the live dispositions were reset. dash reports nothing there. The
# port output must be the reference output with only additional, byte-identical
# copies of listing lines already proved by the reference's outer listing.
# The number of added lines is bounded by the lexical number of subshell trap
# listings times the reference listing size.
dsdiv_trap_subshell_listing() {
	[ "$3" = "$4" ] || return 1
	local listings inherited_limit removed=0 ri=0 line
	local -a ref_lines port_lines
	listings=$({
		grep -oE '\([[:space:]]*trap([[:space:]]+--)?([[:space:]]*;|[[:space:]]*\))' "$5" 2>/dev/null
		grep -oE '(^|[;&])[[:space:]]*trap([[:space:]]+--)?[[:space:]]*\|' "$5" 2>/dev/null
	} | wc -l)
	[ "$listings" -gt 0 ] || return 1
	inherited_limit=$(printf '%s\n' "$1" | grep -c '^trap -- .* [A-Za-z0-9][A-Za-z0-9]*$')
	[ "$inherited_limit" -gt 0 ] || return 1
	inherited_limit=$((inherited_limit * listings))
	mapfile -t ref_lines <<< "$1"
	mapfile -t port_lines <<< "$2"
	for line in "${port_lines[@]}"; do
		if [ "$ri" -lt "${#ref_lines[@]}" ] && [ "$line" = "${ref_lines[ri]}" ]; then
			ri=$((ri + 1))
			continue
		fi
		[[ $line =~ ^trap\ --\ .*\ [A-Za-z0-9][A-Za-z0-9]*$ ]] || return 1
		printf '%s\n' "$1" | grep -qxF -- "$line" || return 1
		removed=$((removed + 1))
		[ "$removed" -le "$inherited_limit" ] || return 1
	done
	[ "$removed" -gt 0 ] && [ "$ri" = "${#ref_lines[@]}" ]
}

# The adopted Smoosh EXIT-action rule makes a normally completed action's
# final command status the subshell status. dash instead restores the status
# originally supplied to `exit`. This exact differential witness has a nested
# child EXIT action whose successful `echo inner` changes 2 to 0; no other
# bytes or status pair are excused.
dsdiv_exit_trap_final_status() {
	[ "$3" = 0 ] && [ "$4" = 0 ] || return 1
	[ "$1" = $'inner\n2' ] && [ "$2" = $'inner\n0' ] || return 1
	grep -Fqx 'trap '\''( trap "echo inner" EXIT; exit 2 ); echo $?'\'' EXIT' "$5" 2>/dev/null
}

# The recursive fc guard is a utility error. The port returns 2; dash prints
# the same diagnostic and then reports success from the interactive shell.
dsdiv_fc_recursion_error_status() {
	[ "$1" = "$2" ] || return 1
	[ "$3" = 0 ] && [ "$4" = 2 ] || return 1
	grep -qE 'fc[[:space:]]+-s([[:space:]]|$)' "$5" 2>/dev/null || return 1
	[[ $1 == *'fc: called recursively too many times'* ]]
}

# The logical descriptor table can lower the shell-visible nofile limit below
# the number of host descriptors used to implement the shell and still query
# it. dash consumes host fds directly and aborts the subshell. This entry is
# deliberately limited to the three generated probes at limits zero and one.
dsdiv_logical_fd_low_nofile_survival() {
	[ "$3" = 2 ] && [ "$4" = 0 ] || return 1
	[ "$1" = 'rc=0' ] || return 1
	if grep -qE 'ulimit[[:space:]]+-S[[:space:]]+-n[[:space:]]+1' "$5" 2>/dev/null; then
		[[ $2 =~ ^rc=0$'\n'1$'\n'(unlimited|[0-9]+)$ ]]
		return
	fi
	if grep -qE 'ulimit[[:space:]]+-HS[[:space:]]+-n[[:space:]]+0' "$5" 2>/dev/null; then
		[[ $2 =~ ^rc=0$'\n'0$'\n'(unlimited|[0-9]+)$ ]]
		return
	fi
	if grep -qE 'ulimit[[:space:]]+-n[[:space:]]+0' "$5" 2>/dev/null; then
		[ "$2" = $'rc=0\n0\n0' ]
		return
	fi
	return 1
}

# POSIX.1-2024 specifies trap -p; dash rejects the option. Pin the one
# differential probe's complete diagnostic and listing so neither arbitrary
# trap output nor another option error can borrow the exception.
dsdiv_trap_p_option() {
	grep -qE 'trap[[:space:]]+-p[[:space:]]+PIPE' "$5" 2>/dev/null || return 1
	[ "$3" = 2 ] && [ "$4" = 0 ] || return 1
	[[ $1 =~ ^SH:[[:space:]][0-9]+:[[:space:]]trap:[[:space:]]Illegal[[:space:]]option[[:space:]]-p$ ]] || return 1
	[ "$2" = "trap -- 'echo caught' PIPE" ]
}

# dash's matcher walks the UTF-8 bytes as if each byte were a character in
# several literal, `?`, bracket, and parameter-trim paths. The typed matcher
# walks decoded characters. Every accepted pair below is the complete output
# of one pinned multibyte corpus witness; the feature substring prevents an
# unrelated case with the same short output from borrowing the entry.
dsdiv_utf8_pattern_characters() {
	[ "$3" = 0 ] && [ "$4" = 0 ] || return 1
	ds_case_contains "$5" 'LC_ALL=en_US.UTF-8' || return 1

	ds_case_contains "$5" '${v#h}' &&
		ds_exact_pair "$1" "$2" 'éllo héllo héll hél' 'éllo llo héll hél' && return 0
	ds_case_contains "$5" 'case héllo in h?llo)' &&
		ds_exact_pair "$1" "$2" n m && return 0
	ds_case_contains "$5" 'case é in [é])' &&
		ds_exact_pair "$1" "$2" n m && return 0
	ds_case_contains "$5" 'case é in [[:alpha:]])' &&
		ds_exact_pair "$1" "$2" n m && return 0
	ds_case_contains "$5" '${v#é}' &&
		ds_exact_pair "$1" "$2" 'éé' 'é' && return 0
	ds_case_contains "$5" '${v##*é}' &&
		ds_exact_pair "$1" "$2" 'aéa aéa' 'a a' && return 0
	ds_case_contains "$5" '${v#*é}' &&
		ds_exact_pair "$1" "$2" 'aéaéa aéaéa aéaéa aéaéa' 'aéa a aéa a' && return 0
	ds_case_contains "$5" 'v=é; case $v in é)' &&
		ds_exact_pair "$1" "$2" '' m && return 0
	ds_case_contains "$5" 'v=é; case "$v" in "é")' &&
		ds_exact_pair "$1" "$2" '' m && return 0
	ds_case_contains "$5" $'v=\'é*\'; case \'éx\' in $v)' &&
		ds_exact_pair "$1" "$2" n m && return 0
	ds_case_contains "$5" '${v%é?}' &&
		ds_exact_pair "$1" "$2" 'aébéc aébéc' 'aébéc aéb' && return 0
	ds_case_contains "$5" '${v#[é]}' &&
		ds_exact_pair "$1" "$2" 'éa éa' 'a a' && return 0
	ds_case_contains "$5" '${v#[!é]}' &&
		ds_exact_pair "$1" "$2" a 'éa' && return 0
	return 1
}

# In the C locale the two bytes of UTF-8 `é` are two non-whitespace IFS
# separators. dash incorrectly treats the byte sequence as one indivisible
# character. These are exact generated-corpus observations, including all
# surrounding output, so only the POSIX byte-wise split is sanctioned.
dsdiv_c_locale_multibyte_ifs() {
	[ "$3" = 0 ] && [ "$4" = 0 ] || return 1
	ds_case_contains "$5" "IFS='é'" || return 1

	ds_case_contains "$5" "echo 'a*b' \$IFS\$_a" &&
		ds_exact_pair "$1" "$2" 'a*b é abc' 'a*b   abc' && return 0
	ds_case_contains "$5" 'echo "aéb" aaaa}b' &&
		ds_exact_pair "$1" "$2" $'SH: 1: Illegal number: é\né' $'SH: 1: Illegal number: é\n ' && return 0
	ds_case_contains "$5" 'echo a\*b ${HOME?$*}' &&
		ds_exact_pair "$1" "$2" "a*b $HOME 01 aéb" "a*b $HOME 01 a  b" && return 0
	ds_case_contains "$5" 'echo $9 ${#IFS}' &&
		ds_exact_pair "$1" "$2" $'2\nab.txt bb.txt cb.txt b 1 é' $'2\nab.txt bb.txt cb.txt b 1  ' && return 0
	ds_case_contains "$5" 'echo  $((~v ? 31 : _a))' &&
		ds_exact_pair "$1" "$2" $'31\né' $'31\n ' && return 0
	ds_case_contains "$5" "echo '/a/b' '/a/b' \$IFS" &&
		ds_exact_pair "$1" "$2" '<a*b></a/b></a/b><é><a b>' '<a*b></a/b></a/b><a b>' && return 0
	ds_case_contains "$5" '${IFS%%$?}' &&
		ds_exact_pair "$1" "$2" "1 a$HOME/z a=b a*b é" "1 a$HOME/z a=b a*b  " && return 0
	ds_case_contains "$5" 'echo "$({ echo ${IFS}; })"' &&
		ds_exact_pair "$1" "$2" 'é' ' ' && return 0
	ds_case_contains "$5" '${w:=$(IFS=' &&
		ds_exact_pair "$1" "$2" 'é aa a*b a*b n 15' '   aa a*b a*b n 15' && return 0
	ds_case_contains "$5" '${$:+é}' &&
		ds_exact_pair "$1" "$2" $'é a$b a?c' $'  a$b a?c' && return 0
	ds_case_contains "$5" 'echo $(set -- b; echo "${#HOME}${#$}"' &&
		ds_exact_pair "$1" "$2" "${#HOME}1 éx:y" "${#HOME}1   x:y" && return 0
	ds_case_contains "$5" 'printf "<%s>"  ${IFS} $@' &&
		ds_exact_pair "$1" "$2" $'<é><\n<a?b>>' $'<><><\n<a?b>>' && return 0
	return 1
}

# A quoted or escaped `word` operand of `${parameter op word}` contributes its
# quote mask to the resulting field. dash discards that mask by assigning and
# then reparsing encoded bytes, which can split an intended field, glob an
# escaped metacharacter, or discard an explicitly quoted empty value.
dsdiv_parameter_operand_quote_preservation() {
	[ "$3" = 0 ] && [ "$4" = 0 ] || return 1

	ds_case_contains "$5" '${v=a\*b}' &&
		ds_exact_pair "$1" "$2" 'ab.txt' 'a*b*' && return 0
	ds_case_contains "$5" '${v:="$(echo é' &&
		ds_exact_pair "$1" "$2" \
		$'<aéb><é><a%b>SH: 1: Illegal number: é a%b\n0' \
		$'<aéb><é a%b>SH: 1: Illegal number: é a%b\n0' && return 0
	ds_case_contains "$5" '${v=\*}' &&
		ds_exact_pair "$1" "$2" \
		$'SH: 1: arithmetic expression: expecting EOF: "08 | 2"\nab.txt bb.txt cb.txt' \
		$'SH: 1: arithmetic expression: expecting EOF: "08 | 2"\n*' && return 0
	ds_case_contains "$5" '${v="${u}"}' &&
		ds_exact_pair "$1" "$2" 'a[b' ' a[b' && return 0
	ds_case_contains "$5" '${_a:=""$w""}' &&
		ds_exact_pair "$1" "$2" '[abc] 0' ' [abc] 0' && return 0
	ds_case_contains "$5" '${w="${IFS%$_a}"}' && {
		local quoted_ifs=$' \t\n '"$HOME"
		ds_exact_pair "$1" "$2" "$HOME" "$quoted_ifs"
	} && return 0
	return 1
}

# Empty quote fragments are field anchors, not extra fields and not licence to
# suppress splitting of a neighbouring unquoted substitution. dash gets both
# directions wrong in the three generated witnesses below.
dsdiv_empty_quote_field_anchors() {
	[ "$3" = 0 ] && [ "$4" = 0 ] || return 1

	ds_case_contains "$5" '""$IFS""' &&
		ds_exact_pair "$1" "$2" '<N><><><a[b>' '<N><><a[b>' && return 0
	ds_case_contains "$5" '""`echo \ $#`""' &&
		ds_exact_pair "$1" "$2" $' 0 a[b\n$ ' $'0 a[b\n$ ' && return 0
	ds_case_contains "$5" '""$IFS"`echo $((IFS))`"' &&
		ds_exact_pair "$1" "$2" ' 0 [' '0 [' && return 0
	return 1
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
