# Smoosh compatibility profile

This profile records the choices needed to make the pinned Smoosh shell survey
an exact, repeatable compatibility target. It is deliberately narrower than a
claim that every Smoosh expectation is required by POSIX. Where POSIX already
specifies the behavior, the POSIX rule remains the normative source. The rules
below freeze an otherwise unspecified choice, an adopted extension, an exact
diagnostic convention, or a property of the native survey runner.

The pinned corpus is commit
`cc67dbe6a4953e51431997eac025b5e3f46c3d2d`. A dash rule in the inventory is an
implementation anchor, not authority to keep a conflicting observable result.
For the cases explicitly covered here, the POSIX rule or this compatibility
profile takes precedence over a conflicting dash-port behavior. Internal dash
contracts that do not determine the named observation remain in force.

## Native runner fidelity

> [spec:nsh:req:compat.smoosh.ifs-launch]
> The native Smoosh runner MUST invoke the selected shell through a wrapper
> pathname that is not split by any `IFS` value assigned by the pinned
> `sh.set.ifs.test`, and MUST apply the same selected shell and flags to the
> top-level script and every nested `$TEST_SHELL` invocation. It MUST
> process-replace the wrapper, present the compatibility invocation name
> `smoosh` to the selected shell, and preserve the runner's containment,
> isolated working directory, and environment boundaries. The test MUST reach
> all three nested invocations, each of which observes the initial IFS value
> <space><tab><newline>; the aggregate stdout is
> `" \t\n \t\n \t\n"` and the status is zero.

## Control-flow boundaries

> [spec:nsh:req:compat.smoosh.control-boundaries]
> A `break` read from a dot script MUST NOT select a caller loop that does not
> lexically enclose that command, so the caller continues every iteration. A
> `break 2` executed in a subshell MUST NOT cross the subshell execution
> environment into a loop in its parent. Consequently,
> `builtin.dot.break.test` writes `"a\nb\nc\n"`,
> `semantics.subshell.break.test` writes `"a\nb\n"`, and both return zero.

## Trap status

> [spec:nsh:req:compat.smoosh.trap-status]
> After a signal trap action finishes normally, the interrupted command's
> status MUST be restored even if commands in the action fail or cause another
> trapped signal. An EXIT trap MUST see the status that caused exit, execute as
> ordinary shell input in the exiting environment, and, unless it explicitly
> exits, its final command status MUST become the status of that shell or
> subshell. An `exit` inside the EXIT action MUST use the action's then-current
> status and MUST NOT re-enter the EXIT action. A `return` that terminates a
> function implemented as a subshell MUST still run that subshell's EXIT action.
> These rules produce the exact Smoosh results: `trap.chained` and
> `trap.exitcode` return 0; `trap.subshell.false.exit` returns 1 with empty
> stdout; `trap.subshell.loud` writes `"WEIRD\n"` and returns 0;
> `trap.subshell.loud2` writes `"HUH\nWEIRD\n"` and returns 0;
> `trap.subshell.true.ec1` has empty stdout and returns 0; and `return.trap`
> writes `"FOO\n"` and returns 0.

## Error status and diagnostics

> [spec:nsh:req:compat.smoosh.error-contracts]
> The Smoosh compatibility profile fixes the following results in addition to
> the mapped POSIX requirements:
>
> - `command readonly x=bar` demotes `readonly` from special-builtin fatality,
>   returns 1, and writes `"readonly: x: is read only\n"`; the enclosing shell
>   continues and the complete case writes `"?=1\n"` and returns 0.
> - A missing dot file terminates the non-interactive shell with status 1 and
>   writes `".: ./nonesuch: not found\n"`. A failed no-operand `exec`
>   redirection, assignment to a readonly variable through `export`, and a
>   redirection error on a directly invoked special built-in likewise terminate
>   the non-interactive shell with status 1.
> - A `times` stdout write failure returns 2. Under the survey invocation name
>   it writes `"smoosh: times: I/O error\n"`; the complete pipeline case writes
>   `"?=2\n"` to its saved descriptor and returns 0.
> - `unset` succeeds for absent and ordinary variables, but attempting to unset
>   readonly `x` writes `"unset: x is read-only\n"` and returns 1. The complete
>   case stdout is `"unset\nfoo\nunset\n"`.
> - An unset-parameter `?` expansion writes the supplied word as
>   `"x: z\n"` in the nested-script case and terminates a non-interactive shell
>   with status 1 before the following command. In an interactive shell it
>   abandons only the erroneous command, so the following `echo hello` runs and
>   the explicit `exit` returns 0.
> - Leaving a temporary redirection scope MUST restore a descriptor that was
>   closed on entry to the closed state, even if the compound command opened it
>   permanently with `exec`. A later duplication from that descriptor fails;
>   `semantics.redir.close.test` has empty stdout and returns 1.
>
> Where one of these imported results collides with a rule this repository
> wrote about its own dialect boundary, the written rule wins and the Smoosh
> result is recorded as a sanctioned divergence in `docs/divergences.md`
> rather than followed: Smoosh's bytes are evidence of what another shell did,
> which is the standing `[spec:nsh:sem:idiom.specified-defects+1]` also gives
> dash's, and evidence does not outrank a contract. Recorded 2026-09-04, with
> the bullets above kept verbatim. The refused `unset` is the first such
> collision resolved: its status is
> `[spec:nsh:req:compat.bash.error-boundary]`'s 2 rather than the 1 above,
> while its stdout and its diagnostic are unchanged. Four bullets above are
> the same collision and still answer 1 where dash answers 2 -- the missing
> dot file, the `export` to a read-only name, the redirection error on a
> directly invoked special built-in, and the unset-parameter `?` expansion --
> and they are not decided by this paragraph;
> `bash.divergences.error-boundary-status-collisions` holds them.

## Adopted extensions

> [spec:nsh:req:compat.smoosh.nonlexical-control]
> The `nonlexicalctrl` shell option MUST be accepted by `set -o`. While enabled,
> a `break n` or `continue n` executed by a function MAY select dynamically
> active caller loops: the count is applied across the function boundary,
> clamped to the available active loops, and the function body stops at the
> control transfer. The pinned break case writes only `"0\n"`; the pinned
> continue case writes `"0\n1\n2\n3\n4\n"`; both return zero.

> [spec:nsh:req:compat.smoosh.history-builtin]
> Interactive shells MUST provide a `history` built-in. With no operands it
> writes retained entries in a form in which the command text remains
> searchable; `history -c` clears those entries. Once `set -o nolog` is
> enabled, subsequently read commands MUST NOT be added to history. History
> storage MUST be created only for an interactive shell; neither this extension
> nor baseline arrow-key navigation may require vi or emacs mode. The pinned
> history case writes exactly `"ok\n"` and returns zero.

> [spec:nsh:req:compat.smoosh.source-builtin]
> `source file` MUST be an alias of dot execution in the current shell
> environment, including persistence of assignments and interaction with
> `set -e`. Unlike an ordinary missing utility, failure to find the source file
> MUST write `"source: <file>: not found\n"`, terminate a non-interactive shell
> immediately with status 1, and execute no following command. The three pinned
> source cases therefore preserve `x=5` and write `"5\n"`, or return 1 with the
> exact missing-file diagnostic and no later output.

> [spec:nsh:req:compat.smoosh.hash-all]
> Enabling `set -h` MUST cause successful external-command resolutions made
> while executing a shell function to be retained in the current shell's
> command-location table. `hash -r` clears that table, and a subsequent
> argument-free `hash` report MUST include the resolved `ls`, `touch`, and `rm`
> entries exercised by `semantics.-h.nonposix.test`; the case returns zero.

## Forced-interactive and job-control behavior

> [spec:nsh:req:compat.smoosh.interactive-job-prompt]
> With monitor mode disabled, this profile does not support job-ID operands for
> non-job-control background jobs: `kill %1 %2` MUST fail while numeric process
> operands continue to work. After `set -m`, background jobs MUST have
> signalable `%1` and `%2` process groups and both jobs MUST terminate promptly.
> A shell forced interactive with `-i` MUST write PS1 to standard error before
> each attempted read even when input is not a terminal. An unset PS1 uses
> `"$ "`, while an explicitly supplied PS1 is preserved. The pinned prompt
> cases produce stderr `"$ "` and `"$ $ $ PS1$ PS1$ PS1$ $ "` respectively;
> the latter writes `"hi\nbye\nhi\nbye\n"` to stdout. All three pinned cases
> return zero.

## Failure ownership inventory

Every failure in the recorded 155/186 baseline appears exactly once below.
The compatibility rule is the closure contract owned by this work; the POSIX
and dash columns identify the existing normative and implementation contracts
that must remain visible while implementing it. A dash anchor marked
"precedence" is intentionally superseded only for the observable result named
by the compatibility rule.

| Recorded failure | Closure owner | Compatibility rule | Existing POSIX mapping | Dash-port anchor |
|---|---|---|---|---|
| `sh.set.ifs.test` | runner fidelity | `[spec:nsh:req:compat.smoosh.ifs-launch]` | `[spec:posix:req:param.ifs-initial-value]` | `[spec:dash:sem:var.initvar-fn]` |
| `builtin.dot.break.test` | control boundaries | `[spec:nsh:req:compat.smoosh.control-boundaries]` | `[spec:posix:def:builtin.break.lexically-enclosing]`, `[spec:posix:sem:builtin.break.non-lexical-loop-unspecified]`, `[spec:posix:req:builtin.dot.execute-in-current-environment]` | `[spec:dash:sem:main.cmdloop-fn]`, `[spec:dash:sem:main.dotcmd-fn]` |
| `semantics.subshell.break.test` | control boundaries | `[spec:nsh:req:compat.smoosh.control-boundaries]` | `[spec:posix:def:builtin.break.lexically-enclosing]`, `[spec:posix:req:builtin.break.exit-nth-loop]` | `[spec:dash:sem:eval.breakcmd-fn]`, `[spec:dash:sem:eval.skiploop-fn]` |
| `builtin.trap.chained.test` | trap status | `[spec:nsh:req:compat.smoosh.trap-status]` | `[spec:posix:req:builtin.trap.action-overrides-and-exit-status]`, `[spec:posix:req:builtin.trap.action-executed-as-eval]` | `[spec:dash:sem:trap.dotrap-fn]` |
| `builtin.trap.exitcode.test` | trap status | `[spec:nsh:req:compat.smoosh.trap-status]` | `[spec:posix:req:builtin.trap.action-overrides-and-exit-status]` | `[spec:dash:sem:trap.dotrap-fn]` |
| `builtin.trap.subshell.false.exit.test` | trap status | `[spec:nsh:req:compat.smoosh.trap-status]` | `[spec:posix:req:builtin.trap.exit-condition]`, `[spec:posix:req:builtin.trap.exit-action-environment]` | `[spec:dash:sem:trap.exitshell-fn]` |
| `builtin.trap.subshell.loud.test` | trap status | `[spec:nsh:req:compat.smoosh.trap-status]` | `[spec:posix:req:builtin.exit.default-n]`, `[spec:posix:req:builtin.exit.exit-trap]` | `[spec:dash:sem:trap.exitshell-fn]` |
| `builtin.trap.subshell.loud2.test` | trap status | `[spec:nsh:req:compat.smoosh.trap-status]` | `[spec:posix:req:builtin.trap.action-overrides-and-exit-status]`, `[spec:posix:req:builtin.exit.default-n]` | `[spec:dash:sem:trap.dotrap-fn]`, `[spec:dash:sem:trap.exitshell-fn]` |
| `builtin.trap.subshell.true.ec1.test` | trap status | `[spec:nsh:req:compat.smoosh.trap-status]` | `[spec:posix:req:builtin.trap.exit-action-environment]` | `[spec:dash:sem:trap.exitshell-fn]` |
| `semantics.return.trap.test` | trap status | `[spec:nsh:req:compat.smoosh.trap-status]` | `[spec:posix:req:builtin.return.stop-function-or-dot-script]`, `[spec:posix:req:builtin.return.exit-status]`, `[spec:posix:req:builtin.trap.exit-condition]` | `[spec:dash:sem:eval.returncmd-fn]`, `[spec:dash:sem:trap.exitshell-fn]` |
| `builtin.command.nospecial.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:builtin.command.special-builtin-properties-suppressed]`, `[spec:posix:req:builtin.command.exit-status-invocation]` | `[spec:dash:sem:eval.parse-command-args-fn]` |
| `builtin.dot.nonexistent.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:builtin.dot.path-search]`, `[spec:posix:req:builtin.dot.stderr]` | `[spec:dash:sem:main.dotcmd-fn]` (precedence) |
| `builtin.exec.badredir.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:builtin.exec.exit-status]`, `[spec:posix:req:exit.shell-error-consequences]` | `[spec:dash:sem:redir.redirectsafe-fn]` (precedence) |
| `builtin.readonly.assign.noninteractive.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:cmd.assign-readonly-error]`, `[spec:posix:req:exit.shell-error-consequences]` | `[spec:dash:sem:var.exportcmd-fn]` (precedence) |
| `builtin.special.redir.error.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:exit.shell-error-consequences]`, `[spec:posix:req:redir.dup-output]` | `[spec:dash:sem:eval.evalcommand-fn]`, `[spec:dash:sem:redir.redirectsafe-fn]` (precedence) |
| `builtin.times.ioerror.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:builtin.times.stderr]`, `[spec:posix:req:builtin.times.exit-status]` | `[spec:dash:sem:eval.evalbltin-fn]` (precedence for exact status/text) |
| `builtin.unset.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:builtin.unset.not-previously-set]`, `[spec:posix:req:builtin.unset.stderr]`, `[spec:posix:req:builtin.unset.exit-status]` | `[spec:dash:sem:var.unsetcmd-fn]` (precedence for exact text/status) |
| `semantics.error.noninteractive.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:expand.param-error-if-unset]`, `[spec:posix:req:exit.shell-error-consequences]` | `[spec:dash:sem:main.cmdloop-fn]` (precedence for exact status/text) |
| `semantics.interactive.expansion.exit.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:expand.param-error-if-unset]`, `[spec:posix:req:exit.interactive-abandons-command]` | `[spec:dash:sem:main.cmdloop-fn]` |
| `semantics.noninteractive.expansion.exit.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:expand.param-error-if-unset]`, `[spec:posix:req:exit.shell-error-consequences]` | `[spec:dash:sem:main.cmdloop-fn]` (precedence for status 1) |
| `semantics.redir.close.test` | error contracts | `[spec:nsh:req:compat.smoosh.error-contracts]` | `[spec:posix:req:redir.dup-input]`, `[spec:posix:req:redir.dup-input-close]` | `[spec:dash:sem:redir.popredir-fn]`, `[spec:dash:sem:redir.update-closed-redirs-fn]` (precedence for status 1) |
| `builtin.break.nonlexical.test` | adopted extensions | `[spec:nsh:req:compat.smoosh.nonlexical-control]` | `[spec:posix:sem:builtin.break.non-lexical-loop-unspecified]` | `[spec:dash:sem:eval.breakcmd-fn]` (extension over dash) |
| `builtin.continue.nonlexical.test` | adopted extensions | `[spec:nsh:req:compat.smoosh.nonlexical-control]` | `[spec:posix:req:builtin.continue.n-operand]`, `[spec:posix:sem:builtin.break.non-lexical-loop-unspecified]` | `[spec:dash:sem:eval.breakcmd-fn]` (extension over dash) |
| `builtin.history.nonposix.test` | adopted extensions | `[spec:nsh:req:compat.smoosh.history-builtin]` | `[spec:posix:req:builtin.set.opt-o-nolog]` | `[spec:dash:sem:histedit.histedit-fn]` (extension over dash) |
| `builtin.source.nonexistent.earlyexit.test` | adopted extensions | `[spec:nsh:req:compat.smoosh.source-builtin]` | `[spec:posix:req:builtin.dot.path-search]` by alias | `[spec:dash:sem:main.dotcmd-fn]` (extension over dash) |
| `builtin.source.nonexistent.test` | adopted extensions | `[spec:nsh:req:compat.smoosh.source-builtin]` | `[spec:posix:req:builtin.dot.path-search]`, `[spec:posix:req:builtin.dot.stderr]` by alias | `[spec:dash:sem:main.dotcmd-fn]` (extension over dash) |
| `builtin.source.setvar.test` | adopted extensions | `[spec:nsh:req:compat.smoosh.source-builtin]` | `[spec:posix:req:builtin.dot.execute-in-current-environment]` by alias | `[spec:dash:sem:main.dotcmd-fn]` (extension over dash) |
| `semantics.-h.nonposix.test` | adopted extensions | `[spec:nsh:req:compat.smoosh.hash-all]` | `[spec:posix:req:builtin.set.opt-h]`, `[spec:posix:req:builtin.hash.remembered-locations]`, `[spec:posix:req:builtin.hash.stdout-report]` | `[spec:dash:sem:eval.prehash-fn]`, `[spec:dash:sem:exec.hashcmd-fn]` |
| `builtin.kill.jobs.test` | interactive/job control | `[spec:nsh:req:compat.smoosh.interactive-job-prompt]` | `[spec:posix:req:builtin.set.opt-m-monitor]`, `[spec:posix:def:builtin.kill.operand-pid-job-id]`, `[spec:posix:req:builtin.kill.exit-status]` | `[spec:dash:sem:jobs.killcmd-fn]`, `[spec:dash:sem:jobs.setjobctl-fn]` (precedence while monitor mode is off) |
| `sh.interactive.ps1.test` | interactive/job control | `[spec:nsh:req:compat.smoosh.interactive-job-prompt]` | `[spec:posix:def:sh.interactive]`, `[spec:posix:req:param.ps1]` | `[spec:dash:sem:main.cmdloop-fn]`, `[spec:dash:sem:parser.getprompt-fn]` |
| `sh.ps1.override.test` | interactive/job control | `[spec:nsh:req:compat.smoosh.interactive-job-prompt]` | `[spec:posix:req:param.ps1]`, `[spec:posix:req:param.ps1-default]` | `[spec:dash:sem:var.initvar-fn]`, `[spec:dash:sem:parser.getprompt-fn]` |
