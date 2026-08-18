# Interactive shell behavior

These rules describe interactive conveniences provided by `nsh` beyond the
portable shell-language requirements. They apply to the command-line frontend;
an embedder remains in control of the streams it supplies.

## Baseline line editing

> [spec:nsh:req:interactive.default-history-navigation]
> An interactive `nsh` session connected to a terminal MUST initialize its
> native line editor even when neither the `vi` nor the `emacs` shell option is
> enabled. In that baseline mode, the up-arrow key MUST select older retained
> command lines and the down-arrow key MUST select newer lines, restoring the
> live line after the newest retained command. The `vi` and `emacs` options MAY
> select their respective extended keymaps, but MUST NOT be prerequisites for
> baseline history navigation.
