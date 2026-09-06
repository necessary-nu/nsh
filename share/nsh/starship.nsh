# Render this shell's prompt with starship (https://starship.rs).
#
# Source this from your startup file:
#
#     . /usr/share/nsh/starship.nsh
#
# WHY THIS IS NOT `eval "$(starship init bash)"`. That script is 140 lines
# and nearly all of it recovers state a shell cannot hand over: it lays a
# `DEBUG` trap or an arithmetic side effect in `PS0` to time a command, runs
# `jobs` twice to work around a Bash defect in order to count jobs, and keeps
# a flag so a pipeline is not timed once per stage. This shell measures the
# duration around the command it reports and lends the count to the hook, so
# none of that is needed here. See `[spec:nsh:req:interactive.prompt-state]`.
#
# It also tells starship we are Bash, which makes it wrap every non-printing
# run in `\[ \]` for readline's benefit. This shell reads escape sequences
# itself -- `[spec:nsh:req:interactive.prompt-display-width]` -- so it asks
# for the plain form and there is nothing to unwrap.

STARSHIP_SHELL=nsh
export STARSHIP_SHELL

# `$?` is the status of the command the user just ran: the shell saves it
# before the hook and restores it after, so reading it here is safe and
# assigning to `PS1` below cannot leak the substitution's own status into the
# next command. `NSH_JOBS` and `NSH_DURATION_MS` are lent to the hook for its
# duration and taken back, so they are not part of the shell's variable set.
#
# `COLUMNS` is absent until a terminal has reported a width. It is passed
# through empty rather than defaulted, because starship then picks its own
# answer -- which is the point of the shell not inventing one.
# The line continuations are load-bearing: a newline inside `$( )` ends a
# command there as it does anywhere else, so an unescaped break would run
# each argument as a command of its own and leave `starship prompt` to
# render with no arguments at all -- which produces a plausible prompt
# carrying none of this shell's state.
PROMPT_COMMAND='PS1=$(starship prompt \
    --status="$?" \
    --jobs="${NSH_JOBS-0}" \
    --cmd-duration="${NSH_DURATION_MS-0}" \
    --terminal-width="${COLUMNS-}")'

PS2=$(starship prompt --continuation)
