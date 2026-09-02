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
