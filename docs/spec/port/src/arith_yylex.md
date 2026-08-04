# src/arith_yylex.c

The arithmetic tokeniser. It reads from the global cursor `arith_buf`,
advances it past the token consumed, returns the token code, and leaves
any associated value in the global `yylval`. See `arith_yacc.md` for the
token code table.

> [spec:dash:def:arith-yylex.yylex-fn]
> int yylex()

> [spec:dash:sem:arith-yylex.yylex-fn]
> Scan one token from `arith_buf`.
>
> Skip spaces, tabs and newlines. End of string returns 0. Any character
> not listed below returns `ARITH_BAD`.
>
> A digit starts a number: `strtoimax(buf, &arith_buf, 0)` — base 0, so
> `0x` is hexadecimal and a leading `0` is octal — with the value in
> `yylval.val`, returning `ARITH_NUM`. Note `strtoimax` also accepts a
> sign, but a leading `-`/`+` is tokenised as a unary operator before
> this point, so that path is unreachable.
>
> A letter or `_` starts a variable name: scan while `is_in_name`, copy
> the name onto the shell stack with a NUL terminator into `yylval.name`,
> and return `ARITH_VAR`.
>
> Operators are recognised by a shared trick: the token code is computed
> as `character + (TOKEN - character)`, i.e. the constant folds to the
> token value, and the compiler emits a simple assignment. The
> longest-match logic is:
>
> - `=` — `ARITH_ASS`, upgraded to `ARITH_EQ` by a second `=`.
> - `>` — `>=` gives `ARITH_GE`; `>>` gives `ARITH_RSHIFT` and then
>   checks for a further `=`; otherwise `ARITH_GT`.
> - `<` — symmetrically `ARITH_LE`, `ARITH_LSHIFT` (+`=`), `ARITH_LT`.
> - `|` — `||` gives `ARITH_OR`, otherwise `ARITH_BOR` with an `=` check.
> - `&` — `&&` gives `ARITH_AND`, otherwise `ARITH_BAND` with an `=`
>   check.
> - `!` — `!=` gives `ARITH_NE`, otherwise `ARITH_NOT`.
> - `( ) ~ ? :` — `ARITH_LPAREN`, `ARITH_RPAREN`, `ARITH_BNOT`,
>   `ARITH_QMARK`, `ARITH_COLON`, none of which combine.
> - `* / % + - ^` — the corresponding binary operator, each followed by
>   an `=` check.
>
> The `=` check adds 11 to the token code, which turns a binary operator
> into its compound-assignment form — correct only because the two token
> ranges are laid out in the same order. `checkeq` advances past the
> current character first; `checkeqcur` does not, for the cases that have
> already advanced.
>
> On the way out, `arith_buf` is left just past the token: paths reaching
> the shared tail advance one more character, while paths jumping
> straight to `out` (identifiers, `>`/`<` with no second character, `!`
> alone, end of input) have already positioned the cursor.
