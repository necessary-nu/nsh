# src/myhistedit.h

The history/editing interface, with a complete set of no-op stubs for
`SMALL` builds. Under `SMALL` the three libedit types become `void`,
`void` and `int`, `hist` is `#define`d to `NULL`, and `history()` is an
empty inline function — so every call site compiles away without
`#ifdef`s of its own. Otherwise the real `<histedit.h>` is included and
`hist` is the real cookie.

`el` is the editline cookie (NULL when editing is off, which the rest of
the shell tests to decide whether to print its own prompt) and
`displayhist` tells `fc -s` to echo what it re-executes.

The functions declared here are defined in `histedit.c`; their rules are
in `histedit.md`, and the entries below cross-reference them.

**Dash source shape (`myhistedit.edit-line`):**

    typedef void EditLine

**Dash source shape (`myhistedit.hist-event`):**

    typedef int HistEvent

    The extracted `def` above is the **`SMALL`** form. It is not usable for
    the normal build: `histcmd` and `str_to_event` read `he.num` and
    `he.str`, so outside `SMALL` this is libedit's
    `struct histevent { int num; const char *str; }`, pulled in from
    `<histedit.h>`. A port needs the struct; the `int` alias only serves
    the `SMALL` stubs, which never inspect it.

**Dash source shape (`myhistedit.histcmd-fn`):**

    int histcmd(int, char **)

> [spec:dash:sem:myhistedit.histcmd-fn]
> Declaration of the `fc` builtin; defined in `histedit.c`. See
> `histedit.histcmd-fn`.

**Dash source shape (`myhistedit.histedit-fn`):**

    void histedit(void)

> [spec:dash:sem:myhistedit.histedit-fn]
> Declaration of the history/editing reconfiguration routine; defined in
> `histedit.c`. See `histedit.histedit-fn`.

**Dash source shape (`myhistedit.history`):**

    typedef void History

**Dash source shape (`myhistedit.history-fn`):**

    static inline void history(History *h, HistEvent *he, int action, char *p)

> [spec:dash:sem:myhistedit.history-fn]
> The `SMALL`-build stub for libedit's `history()`: accepts the same
> arguments and does nothing. Its presence is what lets `input.c` and
> `histedit.c` call `history(...)` unconditionally.
>
> Note the stub's fixed four-argument signature is *not* the interface
> the normal build uses: real libedit `history()` is variadic and
> returns an `int` that `histcmd` and `str_to_event` both test, and
> `histedit.c` calls it with three arguments as well as four. A port
> needs the variadic form for the real path and may keep this exact
> signature only for the `SMALL` stub. In a normal build
> this declaration does not exist and the real variadic libedit function
> is used, whose `action` selects the operation (`H_ENTER`, `H_APPEND`,
> `H_FIRST`, `H_LAST`, `H_NEXT`, `H_PREV`, `H_NEXT_EVENT`, `H_PREV_STR`,
> `H_SETSIZE`) and whose remaining arguments depend on it. The `SMALL`
> build defines only `H_APPEND` and `H_ENTER`, the two `input.c` uses.

**Dash source shape (`myhistedit.not-fcnumber-fn`):**

    int not_fcnumber(char *)

> [spec:dash:sem:myhistedit.not-fcnumber-fn]
> Declaration; defined in `histedit.c`. See `histedit.not-fcnumber-fn`.

**Dash source shape (`myhistedit.sethistsize-fn`):**

    void sethistsize(const char *)

> [spec:dash:sem:myhistedit.sethistsize-fn]
> Declaration of the `HISTSIZE` change callback; defined in
> `histedit.c`. See `histedit.sethistsize-fn`.

**Dash source shape (`myhistedit.setterm-fn`):**

    void setterm(const char *)

> [spec:dash:sem:myhistedit.setterm-fn]
> Declaration; defined in `histedit.c`. See `histedit.setterm-fn`.

**Dash source shape (`myhistedit.str-to-event-fn`):**

    int str_to_event(const char *, int)

> [spec:dash:sem:myhistedit.str-to-event-fn]
> Declaration; defined in `histedit.c`. See `histedit.str-to-event-fn`.
