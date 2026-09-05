# Interactive shell behavior

These rules describe interactive conveniences provided by `nsh` beyond the
portable shell-language requirements. They apply to the command-line frontend;
an embedder remains in control of the streams it supplies.

## Baseline line editing

> [spec:nsh:req:interactive.default-history-navigation]
> An interactive `nsh` session connected to a terminal whose declared
> capabilities support line-oriented redisplay MUST initialize its native line
> editor even when neither the `vi` nor the `emacs` shell option is enabled. A
> non-interactive shell MUST NOT initialize the editor. In baseline mode, the
> up-arrow key MUST select older retained command lines and the down-arrow key
> MUST select newer lines, restoring the live line after the newest retained
> command. The `vi` and `emacs` options MAY select their respective extended
> keymaps, but MUST NOT be prerequisites for baseline history navigation.

## A prompt a program generates

`[dec:nsh:bash-compatibility-is-scripts]` left one thing open: whether nsh
should have a designed interactive surface of its own. It settled only that
Bash is not the specification for one, and that if the work happens it needs
its own rules. These are those rules, for the one part of that surface a user
notices first.

The motivating consumer is `starship`, which is a program that prints a prompt
when given the shell's state. Nothing below names it. A shell that can hand its
state to a program and render what comes back serves any such generator, and a
rule written around one vendor's command line would be a rule about that
vendor.

Measured 2026-09-05 against starship 1.22.1: its Bash integration needs
`PROMPT_COMMAND`, `PS0`, a `DEBUG` trap, `PIPESTATUS`, a `jobs -p` count and
Bash's `\[`/`\]` non-printing markers. A shell that computes its own prompt
needs none of those, because it already holds every value they exist to
recover.

> [spec:nsh:req:interactive.prompt-hook]
> An interactive `nsh` MUST run a caller-supplied hook immediately before each
> primary prompt is rendered, in the shell's own execution environment, so that
> what the hook assigns is what the prompt expansion then reads.
>
> The exit status of the last command MUST be observable to the hook as `$?`
> and MUST be restored afterwards, so that a hook cannot change the status the
> next command sees. A hook that fails MUST NOT end the session, and its
> diagnostic MUST NOT be mistaken for the failed command's own.
>
> The hook is named `PROMPT_COMMAND` because a user's existing configuration
> already spells it that way. That is a choice about a name and not an
> obligation to Bash's semantics: nothing here promises Bash's array form, its
> ordering with `PS0`, or its `DEBUG` interactions.

> [spec:nsh:req:interactive.prompt-state]
> The state a prompt generator needs MUST be readable by the hook without
> reconstructing it from the outside: at least the last command's exit status,
> the number of jobs the shell is tracking, and how long the last command took
> to run.
>
> Duration MUST be measured by the shell around the command it reports, not
> inferred by a hook sampling a clock at each prompt. A hook cannot see the
> boundary a pipeline begins at, which is why the shells that do it that way
> need a pre-execution hook as well and still mis-time `slow | slow | fast`.

> [spec:nsh:req:interactive.terminal-width]
> An interactive `nsh` attached to a terminal MUST make that terminal's current
> width readable as `COLUMNS`, and MUST refresh it when the terminal is
> resized. A shell that is not attached to a terminal MUST leave it alone,
> because there is no width to report and a fabricated one is worse than none.

> [spec:nsh:req:interactive.prompt-display-width]
> The line editor MUST place the cursor by the prompt's *display* width, not by
> its byte length: a sequence that moves no cursor column MUST contribute
> nothing to that width, and a character wider than one column MUST contribute
> its own width.
>
> This is what makes a coloured prompt usable, and it is required whether or
> not the colour came from a generator -- today nothing computes it, so any
> prompt carrying an escape sequence mis-wraps and redraws over itself. It MUST
> NOT be satisfied by adopting another shell's convention for marking
> non-printing runs: those markers exist because a shell would otherwise have to
> parse the sequences, and parsing them is the requirement.

## Signals while the prompt is waiting

A shell spends nearly all of an interactive session blocked in one read, so
every signal a user's session receives arrives there. What the read does with
the interruption is therefore what the session does with the signal.

> [spec:nsh:req:interactive.signal-does-not-end-the-session]
> A caught signal delivered while an interactive `nsh` session is waiting for
> a command line MUST NOT be reported to the parser as an end of input, and
> MUST NOT end the session. Reading the line MUST resume, retaining any text
> already typed. This applies however the line is being read, so a shell whose
> native line editor is active MUST behave here as one reading the terminal
> directly does.
>
> An action the signal has under `trap` MUST still run, at the next point the
> shell takes delivery -- which is once the line has been entered, as it is in
> dash and in GNU Bash 5.3, and not at the prompt.
>
> An untrapped `SIGINT` remains the one signal that ends the *line*: the shell
> abandons what was typed and prompts again. It MUST NOT end the session
> either.
>
> That abandoning MUST happen where the signal arrives, and not once a further
> line has been entered. A shell reading the terminal directly retries an
> interrupted read, which is correct for every signal that has nothing to
> deliver and wrong for this one: the retry blocks again, so the interrupt is
> taken against the *next* line the user types, which is discarded in place of
> the empty one and leaves the buffered input misaligned behind it. The read
> MUST therefore end when an interrupt is waiting, so that delivery happens at
> the shell's own polling boundary rather than a line later.
