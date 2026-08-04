# src/arith_yacc.c, src/arith_yacc.h

Arithmetic expansion (`$(( … ))`). Despite the name there is no yacc
grammar: this is a hand-written recursive-descent parser that consumes
tokens from `yylex` (in `arith_yylex.c`) and evaluates as it goes. All
arithmetic is on `intmax_t`.

Token codes are dense small integers assigned in `arith_yacc.h`, and the
code depends on their exact values and ordering:

- `ARITH_ASS` 1, `ARITH_OR` 2, `ARITH_AND` 3, `ARITH_BAD` 4,
  `ARITH_NUM` 5, `ARITH_VAR` 6, `ARITH_NOT` 7.
- Binary operators occupy `[ARITH_BINOP_MIN, ARITH_BINOP_MAX)` = [8, 24):
  `LE` 8, `GE` 9, `LT` 10, `GT` 11, `EQ` 12, `REM` 13, `BAND` 14,
  `LSHIFT` 15, `RSHIFT` 16, `MUL` 17, `ADD` 18, `BOR` 19, `SUB` 20,
  `BXOR` 21, `DIV` 22, `NE` 23.
- Compound assignments occupy `[ARITH_ASS_MIN, ARITH_ASS_MAX)` = [24, 34)
  in the *same relative order* as the binary operators they derive from,
  which is why `op - 11` converts a compound assignment to its operator.
- `LPAREN` 34, `RPAREN` 35, `BNOT` 36, `QMARK` 37, `COLON` 38.

`prec[]` maps each binary operator to a precedence level, **lower binds
tighter**: 0 for `* / %`, 1 for `+ -`, 2 for shifts, 3 for relational,
4 for equality, 5 for `&`, 6 for `^`, 7 for `|`. `ARITH_MAX_PREC` is 8,
the sentinel meaning "no enclosing operator".

The parser keeps one token of lookahead in the global `last_token`, with
the token's value in `yylval`. Every routine takes the current token, its
value, and the *next* token already fetched — an explicit two-token
window rather than a pushback.

`noeval` is threaded through everything so that short-circuited operands
(`&&`, `||`, `?:`) are parsed for syntax but not evaluated, and in
particular do not assign to variables or trigger division-by-zero errors.

> [spec:dash:def:arith-yacc.and-fn]
> static intmax_t and(int token, union yystype *val, int op, int noeval)

> [spec:dash:sem:arith-yacc.and-fn]
> Parse `&&`. Evaluate the left operand with `binop`. If the following
> token is not `ARITH_AND`, return it unchanged. Otherwise fetch the next
> token pair and recurse on the right side with `noeval | !a`, so a false
> left operand suppresses evaluation of the right. Return `a && b`, which
> normalises to 0 or 1. Right recursion makes the operator
> right-associative, which is harmless for `&&`.

> [spec:dash:def:arith-yacc.arith-fn]
> intmax_t arith(const char *s)

> [spec:dash:sem:arith-yacc.arith-fn]
> Evaluate the arithmetic expression in `s`. Point both `arith_buf` (the
> lexer cursor) and `arith_startbuf` (kept for error messages) at it,
> fetch the first token and parse a full `assignment` with `noeval` 0.
> Anything left over — `last_token` not 0, the end-of-input token — is
> `yyerror("expecting EOF")`. Return the value.

> [spec:dash:def:arith-yacc.arith-prec-fn]
> static inline int arith_prec(int op)

> [spec:dash:sem:arith-yacc.arith-prec-fn]
> Return `prec[op - ARITH_BINOP_MIN]`, the precedence level of a binary
> operator. The caller must already have checked that `op` is in the
> binary-operator range; there is no bounds check.

> [spec:dash:def:arith-yacc.assignment-fn]
> static intmax_t assignment(int var, int noeval)

> [spec:dash:sem:arith-yacc.assignment-fn]
> Parse an assignment, the lowest-precedence production. Capture the
> current token's value from `yylval` and fetch the following token.
>
> If the current token is not a variable, or the following token is
> neither `ARITH_ASS` nor a compound assignment, this is not an
> assignment — hand off to `cond`.
>
> Otherwise recurse to evaluate the right-hand side (right-associative,
> so `a = b = 1` works). Under `noeval` return the value without
> assigning. Otherwise `setvarint` the variable: to the result for a
> plain `=`, and for a compound assignment to
> `do_binop(op - 11, lookupvarint(name), result)` — the `- 11` maps the
> assignment token onto its binary operator, which works only because the
> two ranges are laid out in the same order. `setvarint` returns the
> value, so the assignment is itself an expression.

> [spec:dash:def:arith-yacc.binop-fn]
> static intmax_t binop(int token, union yystype *val, int op, int noeval)

> [spec:dash:sem:arith-yacc.binop-fn]
> Parse a binary-operator expression. Evaluate the left operand with
> `primary`; if the token that follows is not a binary operator, return
> it. Otherwise hand off to `binop2` with `ARITH_MAX_PREC`, the sentinel
> precedence that lets it consume operators of every level.

> [spec:dash:def:arith-yacc.binop2-fn]
> static intmax_t binop2(intmax_t a, int op, int prec, int noeval)

> [spec:dash:sem:arith-yacc.binop2-fn]
> Precedence-climbing loop. `a` is the accumulated left operand, `op` the
> operator just seen, and `prec` the precedence ceiling — the loop stops
> when it meets an operator that binds no tighter than the enclosing
> context.
>
> Each iteration: fetch a token and its lookahead and parse a `primary`
> as the right operand `b`. Look at the operator that follows; if it is a
> binary operator of *higher* precedence (a numerically smaller `prec`,
> per `higher_prec`), recurse to fold it into `b` first, bounded by the
> current operator's precedence, and re-read what follows.
>
> Apply the operator: `do_binop(op, a, b)`, or just `b` under `noeval` —
> no arithmetic is performed at all in that case, which is what keeps a
> short-circuited `1/0` from erroring.
>
> Stop and return when what follows is not a binary operator, or its
> precedence is at or beyond `prec`. Otherwise continue with it as the
> new `op`, so same-precedence operators are applied left to right.

> [spec:dash:def:arith-yacc.cond-fn]
> static intmax_t cond(int token, union yystype *val, int op, int noeval)

> [spec:dash:sem:arith-yacc.cond-fn]
> Parse the ternary `?:`. Evaluate the condition with `or`. If what
> follows is not `?`, return it. Otherwise parse the then-branch as a
> full `assignment` with `noeval | !a`, require a `:` — raising
> `yyerror("expecting ':'")` otherwise — and parse the else-branch by
> recursing with `noeval | !!a`. Exactly one branch is evaluated. Return
> `a ? b : c`.
>
> The then-branch is `assignment` while the else-branch is `cond`, which
> is what makes `a ? b : c ? d : e` group to the right and lets an
> assignment appear unparenthesised in the middle.

> [spec:dash:def:arith-yacc.do-binop-fn]
> static intmax_t do_binop(int op, intmax_t a, intmax_t b)

> [spec:dash:sem:arith-yacc.do-binop-fn]
> Apply one binary operator to two `intmax_t` values. Division and
> remainder first reject a zero divisor, and also `INTMAX_MIN / -1` whose
> quotient is unrepresentable, with `yyerror("division error")`.
>
> The remaining operators are the plain C ones: `* + - << >> & ^ |`, and
> the six relational and two equality operators, which yield 0 or 1. Note
> `default` falls into the division case, so an out-of-range operator is
> treated as division rather than diagnosed. Signed overflow, and shift
> counts that are negative or too large, are C undefined behaviour and
> are not checked — a Rust port must decide explicitly (wrapping is the
> closest match to the behaviour observed on the platforms dash targets).

> [spec:dash:def:arith-yacc.higher-prec-fn]
> static inline int higher_prec(int op1, int op2)

> [spec:dash:sem:arith-yacc.higher-prec-fn]
> Return whether `op1` binds tighter than `op2`:
> `arith_prec(op1) < arith_prec(op2)`, since smaller numbers mean tighter
> binding.

> [spec:dash:def:arith-yacc.or-fn]
> static intmax_t or(int token, union yystype *val, int op, int noeval)

> [spec:dash:sem:arith-yacc.or-fn]
> Parse `||`, mirroring `and`: evaluate the left side with `and`, and if
> `||` follows, recurse with `noeval | !!a` so a true left operand
> suppresses the right. Return `a || b`, normalised to 0 or 1.

> [spec:dash:def:arith-yacc.primary-fn]
> static intmax_t primary(int token, union yystype *val, int op, int noeval)

> [spec:dash:sem:arith-yacc.primary-fn]
> Parse a primary expression: a parenthesised expression, a literal, a
> variable, or a unary operator applied to a primary.
>
> - `(` — parse a full `assignment`, require `)` (else
>   `yyerror("expecting ')'")`), and fetch the next token into
>   `last_token`.
> - number — set `last_token` to the lookahead and return the value.
> - variable — likewise, but the value is `lookupvarint(name)`, or the
>   raw `val->val` under `noeval` so an unevaluated branch does not read
>   variables.
> - unary `+` — has no effect: shift the lookahead into place, fetch a
>   new one, and loop rather than recursing, so a run of `+` costs no
>   stack.
> - unary `-`, `!`, `~` — recurse on the following primary and negate,
>   logically negate, or complement it.
> - anything else — `yyerror("expecting primary")`.

> [spec:dash:def:arith-yacc.yyerror-fn]
> static void yyerror(const char *s)

> [spec:dash:sem:arith-yacc.yyerror-fn]
> Raise `sh_error("arithmetic expression: %s: \"%s\"", s,
> arith_startbuf)` — the reason plus the whole original expression, since
> the cursor position is not tracked. Does not return.

> [spec:dash:def:arith-yacc.yylex-fn]
> int yylex(void)

> [spec:dash:sem:arith-yacc.yylex-fn]
> The tokeniser prototype used by this file; the implementation lives in
> `arith_yylex.c`. See `arith-yylex.yylex-fn` for its semantics. It
> returns the token code and leaves any associated value in the global
> `yylval`.

> [spec:dash:def:arith-yacc.yystype]
> union yystype {
>   intmax_t val;
>   char *name;
> }
