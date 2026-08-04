# src/bltin/test.c

The `test` / `[` builtin, derived from the BSD standalone `test(1)`.

The operator table `ops[]` maps each operator's text to a `struct t_op`
holding its `enum token` code (`op_num`) and its `enum token_types`
class (`op_type`): `UNOP` (a unary file/string test), `BINOP` (a binary
comparison), `BUNOP` (`!`), `BBINOP` (`-a`, `-o`) and `PAREN`.

Parsing walks the argument vector through the module-global cursor
`t_wp`, with `t_wp_op` holding the `struct t_op` of the token most
recently lexed — the lexer's only way of returning the operator's class
alongside its code. The grammar is the usual precedence cascade:
`oexpr` (`-o`) → `aexpr` (`-a`) → `nexpr` (`!`) → `primary`.

Note that `test_access` and `test_file_access` are also declared in
`exec.h` and used by command lookup; their rules live in `exec.md`
(`exec.test-access-fn`, `exec.test-file-access-fn`), and the entries here
are the same functions.

> [spec:dash:def:test.aexpr-fn]
> static int aexpr(enum token n)

> [spec:dash:sem:test.aexpr-fn]
> Parse and evaluate a `-a` (and) chain. Start with `res = 1` and loop:
> evaluate an `nexpr`, clearing `res` if it is false — note it does *not*
> short-circuit, so every operand is evaluated. Stop at the end of the
> arguments, or when the next token is not `BAND`. Otherwise advance two
> words (past the `-a` and onto the next operand) and continue.

> [spec:dash:def:test.binop-fn]
> static int binop(void)

> [spec:dash:sem:test.binop-fn]
> Evaluate a binary operator. The first operand is the current word; lex
> the operator, then take the second operand from the following word,
> raising `syntax(op->op_text, "argument expected")` if there is none.
>
> Dispatch on `op->op_num`: `STREQ`/`STRNE` compare with `strcmp`;
> `STRLT`/`STRGT` compare with `strcoll`, so ordering follows the locale;
> the six `INT*` comparisons convert both operands with `getn` and
> compare as `intmax_t`; `FILNT`/`FILOT`/`FILEQ` compare files with
> `newerf`, `olderf` and `equalf`. An unrecognised operator `abort()`s
> under `DEBUG`.

> [spec:dash:def:test.equalf-fn]
> static int equalf (const char *f1, const char *f2)

> [spec:dash:sem:test.equalf-fn]
> `f1 -ef f2`: true when both files can be `stat64`'d and their `st_dev`
> and `st_ino` both match — i.e. they are the same file, whether reached
> by a link or a different path.

> [spec:dash:def:test.faccessat-confused-about-superuser-fn]
> static inline int faccessat_confused_about_superuser(void)

> [spec:dash:sem:test.faccessat-confused-about-superuser-fn]
> Return whether this platform's `faccessat` wrongly grants execute
> permission to the superuser for a file with no execute bit set: 1 under
> `HAVE_TRADITIONAL_FACCESSAT`, 0 otherwise. Compiled only where
> `faccessat` exists. `test_file_access` uses it to decide whether to
> apply the correction by hand.

> [spec:dash:def:test.filstat-fn]
> static int filstat(char *nm, enum token mode)

> [spec:dash:sem:test.filstat-fn]
> Evaluate a file test. `stat64` the name — or `lstat64` for `FILSYM`
> (`-h`/`-L`), which must not follow the link — and return 0 if that
> fails, so every file test on a nonexistent path is false.
>
> Then by `mode`: `FILRD`/`FILWR`/`FILEX` go through `test_access` (only
> where `faccessat` is unavailable; otherwise `primary` handled them
> already); `FILEXIST` is true by having stat'd successfully; the type
> tests use the `S_IS*` macros (`FILREG`, `FILDIR`, `FILCDEV`, `FILBDEV`,
> `FILFIFO`, `FILSOCK`, `FILSYM`); `FILSUID`, `FILSGID` and `FILSTCK`
> test the `S_ISUID`, `S_ISGID` and `S_ISVTX` bits; `FILGZ` (`-s`) is
> true for a non-zero size; `FILUID` (`-O`) and `FILGID` (`-G`) compare
> the owner against the effective uid and gid. Any other mode returns 1.

> [spec:dash:def:test.getn-fn]
> static inline intmax_t getn(const char *s)

> [spec:dash:sem:test.getn-fn]
> `atomax10(s)` — parse a base-10 integer, raising a shell error on
> anything malformed.

> [spec:dash:def:test.getop-fn]
> static const struct t_op *getop(const char *s)

> [spec:dash:sem:test.getop-fn]
> Linear-search `ops[]` for an entry whose `op_text` equals `s`, returning
> it or NULL. Exact match only — this is why `test -x` sees `-x` as an
> operator but `test -xy` does not.

> [spec:dash:def:test.has-exec-bit-set-fn]
> static int has_exec_bit_set(const char *path)

> [spec:dash:sem:test.has-exec-bit-set-fn]
> Return whether any of `S_IXUSR`, `S_IXGRP`, `S_IXOTH` is set on `path`,
> and 0 if it cannot be stat'd. Used to correct `faccessat`'s
> over-permissive answer for the superuser.

> [spec:dash:def:test.isoperand-fn]
> static int isoperand(char **tp)

> [spec:dash:sem:test.isoperand-fn]
> Decide whether the word at `tp` — which *looks* like a unary operator —
> should instead be treated as a plain operand. Return 1 when it is the
> last word (so `test -x` tests the string `-x` for non-emptiness); 0 when
> exactly one word follows (so `test -x foo` is a real unary test);
> and otherwise 1 only if the word after next is a binary operator, which
> makes `test -x = y` compare the strings rather than parse `-x` as an
> operator. This is what implements POSIX's argument-count-driven
> disambiguation.

> [spec:dash:def:test.newerf-fn]
> static bool newerf(const char *f1, const char *f2)

> [spec:dash:sem:test.newerf-fn]
> `f1 -nt f2`: false if `f1` cannot be stat'd; true if `f2` cannot be —
> so a file is newer than one that does not exist. Otherwise compare
> modification times, using nanosecond precision (`st_mtim`) where
> available and whole seconds otherwise.

> [spec:dash:def:test.nexpr-fn]
> static int nexpr(enum token n)

> [spec:dash:sem:test.nexpr-fn]
> Handle `!`. A token other than `UNOT` goes straight to `primary`.
> Otherwise lex the next token, advance the cursor unless the arguments
> are exhausted, and return the logical negation of a recursive `nexpr` —
> so a run of `!` alternates correctly. Not advancing at `EOI` is what
> lets a trailing `!` be treated as an operand rather than running off
> the end.

> [spec:dash:def:test.oexpr-fn]
> static int oexpr(enum token n)

> [spec:dash:sem:test.oexpr-fn]
> Parse and evaluate a `-o` (or) chain, mirroring `aexpr`: start with
> `res = 0`, OR in each `aexpr`, and continue only while the next token
> is `BOR`. Also does not short-circuit.

> [spec:dash:def:test.olderf-fn]
> static bool olderf(const char *f1, const char *f2)

> [spec:dash:sem:test.olderf-fn]
> `f1 -ot f2`: the mirror of `newerf` — false if `f2` cannot be stat'd,
> true if `f1` cannot be, otherwise compares modification times with
> nanosecond precision where available.

> [spec:dash:def:test.primary-fn]
> static int primary(enum token n)

> [spec:dash:sem:test.primary-fn]
> Evaluate a primary. `EOI` — a missing expression — is false.
>
> `LPAREN` recurses: an immediately following `)` is an empty expression
> and is false; otherwise evaluate an `oexpr` and require a closing `)`,
> raising `syntax(NULL, "closing paren expected")` if absent.
>
> If the token lexed as a unary operator, advance to its operand —
> raising `syntax(op_text, "argument expected")` if there is none — and
> evaluate: `STREZ` (`-z`) and `STRNZ` (`-n`) test string length; `FILTT`
> (`-t`) is `isatty` of the operand read as a number; `FILRD`/`FILWR`/
> `FILEX` go through `test_file_access` where `faccessat` exists; and
> everything else goes to `filstat`.
>
> Otherwise lex the *next* word; if that is a binary operator, evaluate
> the comparison with `binop`. Failing all of that, the primary is a bare
> string, true when non-empty.

> [spec:dash:def:test.syntax-fn]
> static void syntax(const char *op, const char *msg)

> [spec:dash:sem:test.syntax-fn]
> Raise a syntax error, prefixed with the operator when one is given and
> non-empty: `"<op>: <msg>"` or just `"<msg>"`. Does not return.

> [spec:dash:def:test.t-lex-fn]
> static enum token t_lex(char **tp)

> [spec:dash:sem:test.t-lex-fn]
> Classify the word at `tp`, setting `t_wp_op` to its operator entry or
> NULL and returning its token code. A NULL word is `EOI`.
>
> A word that matches an operator is treated as one *unless* it is a
> unary operator that `isoperand` says should be an operand, or it is a
> `(` with nothing after it — the latter making a lone `(` a string
> rather than an unterminated group. Anything else is `OPERAND`.

> [spec:dash:def:test.t-op]
> struct t_op {
>   const char *op_text;
>   short op_num, op_type;
> }

> [spec:dash:def:test.test-access-fn]
> int test_access(const struct stat64 *sp, int stmode)

> [spec:dash:sem:test.test-access-fn]
> Permission test performed directly on the `struct stat64` bits rather
> than via `access()`. Specified in full at `exec.test-access-fn`, since
> `exec.h` declares it and command lookup uses it; this is the same
> function.

> [spec:dash:def:test.test-file-access-fn]
> int test_file_access(const char *path, int mode)

> [spec:dash:sem:test.test-file-access-fn]
> Permission test via `faccessat` with the superuser correction.
> Specified in full at `exec.test-file-access-fn`; this is the same
> function.

> [spec:dash:def:test.testcmd-fn]
> int testcmd(int argc, char **argv)

> [spec:dash:sem:test.testcmd-fn]
> The `test` / `[` builtin. Note the return convention is inverted from
> the expression's truth value: `res` starts at 1 and the expression
> result is XOR-ed into it, so a true expression yields exit status 0.
>
> When invoked as `[`, the last argument must be `]` — else
> `error("missing ]")` — and is removed.
>
> Then apply the POSIX argument-count rules, which depend on how many
> operands remain. With exactly 3, if the middle word is a binary
> operator the expression is that comparison, regardless of what the
> operands look like — so `test ! = x` compares the strings. With 3 or 4,
> a surrounding `(` … `)` pair is stripped, and a leading `!` **assigns**
> `res = 0` — it does not invert it — then restarts the analysis with one
> fewer argument. The distinction is observable: because a second `!` at
> the same level re-assigns rather than flipping back, `test ! ! -n x`
> exits 1, not 0. Zero remaining arguments returns `res` — false for a
> plain `test`, true after a `!`.
>
> Otherwise lex the first token and evaluate an `oexpr`. Anything left
> over beyond a single trailing word is
> `syntax(argv[0], "unexpected operator")`.

> [spec:dash:def:test.token]
> enum token {
>   EOI;
>   FILRD;
>   FILWR;
>   FILEX;
>   FILEXIST;
>   FILREG;
>   FILDIR;
>   FILCDEV;
>   FILBDEV;
>   FILFIFO;
>   FILSOCK;
>   FILSYM;
>   FILGZ;
>   FILTT;
>   FILSUID;
>   FILSGID;
>   FILSTCK;
>   FILNT;
>   FILOT;
>   FILEQ;
>   FILUID;
>   FILGID;
>   STREZ;
>   STRNZ;
>   STREQ;
>   STRNE;
>   STRLT;
>   STRGT;
>   INTEQ;
>   INTNE;
>   INTGE;
>   INTGT;
>   INTLE;
>   INTLT;
>   UNOT;
>   BAND;
>   BOR;
>   LPAREN;
>   RPAREN;
>   OPERAND;
> }

> [spec:dash:def:test.token-types]
> enum token_types {
>   UNOP;
>   BINOP;
>   BUNOP;
>   BBINOP;
>   PAREN;
> }
