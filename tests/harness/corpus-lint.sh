#!/bin/bash
# Corpus linter. Writes the accepted cases to stdout and a report of what
# it dropped to stderr.
#
#   corpus-lint.sh [--report-only] [FILE]      (reads stdin if no FILE)
#
# Handles both corpus formats: `%%%`-separated multi-line cases, and
# one-case-per-line. The format is detected the same way dsdiff.sh detects
# it, so the linter and the runner always agree on case boundaries.
#
# The PID namespace in sandboxed.sh is the containment boundary; this is
# the second layer. It exists because signal-delivery cases exercise
# neither dash's parser nor its builtins in any way this port cares about,
# so the cheapest correct thing is to not generate them at all.
#
# Dropped:
#   killall / pkill                   -- never in scope
#   any `kill`                        -- broadcast (`kill -1`), process-group
#                                        (`kill -- -PGID`, `kill -0`) and
#                                        bare `kill` are indistinguishable
#                                        from safe forms often enough that
#                                        the whole verb goes
#
# Kept:
#   a case carrying an explicit `#!allow-kill` directive. Only hand-curated
#   cases may use it, and it must target the shell's own `$$` or a `%job`.
#   These stay because they are real coverage of trap dispatch, and the PID
#   namespace bounds them regardless.
ROOT=${DASH_ROOT:-$(cd "$(dirname "${BASH_SOURCE[0]:-$0}")/../.." && pwd)}
set -u

report_only=
case ${1:-} in --report-only) report_only=1; shift ;; esac

src=${1:-}
tmp=
if [ -z "$src" ]; then
	tmp=$(mktemp "${TMPDIR:-/tmp}/dslint.XXXXXX"); trap 'rm -f "$tmp"' EXIT
	cat > "$tmp"; src=$tmp
fi

if grep -qx '%%%' "$src"; then mode=multi; else mode=line; fi

awk -v report_only="$report_only" -v mode="$mode" '
	function judge(blob) {
		if (blob ~ /(^|[^[:alnum:]_])(killall|pkill)([^[:alnum:]_]|$)/)
			return "killall/pkill"
		if (blob !~ /(^|[^[:alnum:]_])kill([^[:alnum:]_]|$)/)
			return ""
		if (allow) return ""
		return "kill"
	}
	function flush_case(   i, verdict, blob) {
		if (n == 0) { allow = 0; return }
		blob = ""
		for (i = 0; i < n; i++) blob = blob " " buf[i]
		verdict = judge(blob)
		if (verdict == "") {
			if (!report_only) {
				if (emitted++) print "%%%"
				if (allow) print "#!allow-kill"
				for (i = 0; i < n; i++) print buf[i]
			}
			kept++
		} else {
			dropped++
			printf "DROP [%s]: %s\n", verdict, substr(blob, 2, 160) > "/dev/stderr"
		}
		n = 0; allow = 0
	}

	mode == "multi" {
		if ($0 == "%%%") { flush_case(); next }
		if ($0 ~ /^#!allow-kill[[:space:]]*$/) { allow = 1; next }
		buf[n++] = $0
		next
	}
	# one-case-per-line: blanks and comments pass through untouched
	{
		if ($0 ~ /^[[:space:]]*$/ || $0 ~ /^[[:space:]]*#/) next
		n = 0; allow = 0; buf[n++] = $0
		verdict = judge(" " $0)
		if (verdict == "") { if (!report_only) print $0; kept++ }
		else { dropped++; printf "DROP [%s]: %s\n", verdict, $0 > "/dev/stderr" }
		n = 0
	}
	END {
		if (mode == "multi") flush_case()
		printf "corpus-lint: kept=%d dropped=%d\n", kept, dropped > "/dev/stderr"
	}
' "$src"
