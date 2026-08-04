# src/mystring.c

String helpers plus the shell's shared string constants: `nullstr` (a
one-byte zero-length string used as a sentinel that callers compare by
address and never free), `spcstr` (`" "`), `snlfmt` (`"%s\n"`),
`illnum` (`"Illegal number: %s"`), `homestr` (`"HOME"`), `dotdir`
(`"."`), `cqchars` (the characters `single_quote`'s callers must treat
as needing protection: `\`, `CTLESC`, `CTLMBCHAR`, `CTLQUOTEMARK`), and
`dolatstr`, the pre-built internal encoding of `"$@"` — the byte sequence
`CTLQUOTEMARK, CTLVAR, VSNORMAL|VSBIT, '@', '=', CTLQUOTEMARK, NUL`.

`equal` and `scopy` are macros in `mystring.h`, not functions here.

> [spec:dash:def:mystring.atomax-fn]
> intmax_t atomax(const char *s, int base)

> [spec:dash:sem:mystring.atomax-fn]
> Parse `s` as an `intmax_t` in the given base, erroring on anything
> malformed. Clear `errno`, call `strtoimax(s, &p, base)`. If nothing was
> consumed (`p == s`) *and* `base` is non-zero, the string is blank or
> junk — call `badnum(s)`. The `base` guard deliberately permits an empty
> string in arithmetic contexts, which pass base 0 and want an empty
> expression to evaluate to 0. Then skip trailing whitespace with
> `isspace` on the unsigned char value, and if anything else remains call
> `badnum(s)`. Leading whitespace and an optional sign are accepted
> because `strtoimax` accepts them. Overflow sets `errno` but is *not*
> checked, so a value out of `intmax_t` range returns the clamped
> `strtoimax` result rather than erroring. Return the parsed value.

> [spec:dash:def:mystring.atomax10-fn]
> intmax_t atomax10(const char *s)

> [spec:dash:sem:mystring.atomax10-fn]
> Convenience wrapper: `atomax(s, 10)`. Base 10 is non-zero, so unlike
> the arithmetic path a blank string is rejected.

> [spec:dash:def:mystring.badnum-fn]
> void badnum(const char *s)

> [spec:dash:sem:mystring.badnum-fn]
> Raise `sh_error(illnum, s)` — `"Illegal number: %s"` — which unwinds
> and does not return.

> [spec:dash:def:mystring.findstring-fn]
> const char *const * findstring(const char *s, const char *const *array, size_t nmemb)

> [spec:dash:sem:mystring.findstring-fn]
> Binary-search a sorted array of `nmemb` string pointers for one equal
> to `s`: `bsearch(&s, array, nmemb, sizeof(const char *), pstrcmp)`.
> Note the key is the *address* of `s`, matching `pstrcmp`'s
> pointer-to-pointer convention. Returns a pointer to the matching array
> slot — so the caller can recover the index — or NULL. The array must
> already be sorted by `strcmp` order.

> [spec:dash:def:mystring.is-number-fn]
> int is_number(const char *p)

> [spec:dash:sem:mystring.is-number-fn]
> Return 1 if `p` consists entirely of digits, 0 otherwise. The loop is
> do/while, testing before advancing, so the empty string returns 0 (its
> NUL fails `is_digit`). No sign, whitespace or base prefix is allowed —
> strictly `[0-9]+`.

> [spec:dash:def:mystring.number-fn]
> int number(const char *s)

> [spec:dash:sem:mystring.number-fn]
> Parse `s` as a non-negative `int`. Call `atomax10(s)`, then reject a
> result below 0 or above `INT_MAX` with `badnum(s)`. Return the value
> narrowed to `int`. Used where the shell needs a plain count or status,
> e.g. the operand of `exit` or `shift`.

> [spec:dash:def:mystring.prefix-fn]
> char * prefix(const char *string, const char *pfx)

> [spec:dash:sem:mystring.prefix-fn]
> Test whether `pfx` is a prefix of `string`. Compare characters while
> `*pfx` is non-NUL, returning 0 on the first mismatch. On success return
> a pointer into `string` just past the matched prefix, so the caller can
> continue parsing there. An empty `pfx` matches and returns `string`
> itself. Since the loop stops at `pfx`'s NUL, `string` may be shorter
> only by mismatching on its own NUL, which correctly fails.

> [spec:dash:def:mystring.pstrcmp-fn]
> int pstrcmp(const void *a, const void *b)

> [spec:dash:sem:mystring.pstrcmp-fn]
> `qsort`/`bsearch` comparator over arrays of `const char *`: cast both
> arguments to `const char *const *`, dereference, and return
> `strcmp` of the two strings.

> [spec:dash:def:mystring.scopyn-fn]
> void scopyn(const char *from, char *to, int size)

> [spec:dash:sem:mystring.scopyn-fn]
> Bounded string copy: copy from `from` into the `size`-byte buffer `to`,
> truncating if needed and always NUL-terminating. Copy while
> `--size > 0`, returning early once a NUL has been transferred;
> otherwise write a NUL at the final position.
>
> Note: the whole function is inside `#if 0` in the source and is not
> compiled. It is in the manifest because the extractor parses the text
> regardless. Wave 2 need not port it; if it does, the annotation should
> ride on equivalently dead code so the port stays 1:1.

> [spec:dash:def:mystring.single-quote-fn]
> char * single_quote(const char *s)

> [spec:dash:sem:mystring.single-quote-fn]
> Quote `s` so the shell would read it back as the same literal string,
> returning the result on the stack via `stackblock()`. The output
> alternates two kinds of chunk, because a single-quoted string cannot
> contain a single quote.
>
> Start a stack string and loop: take `len` as the distance to the next
> `'` (or to the NUL, via `strchrnul`), reserve `len + 3` bytes, and emit
> `'` + those `len` bytes + `'`, advancing `s` past them and committing
> with `STADJUST`. Then measure the run of consecutive `'` characters at
> `s` with `strspn(s, "'")`; if there is none, break. Otherwise reserve
> `len + 3` again and emit `"` + that run of quotes + `"`, since inside
> double quotes a `'` is literal, advance `s` and commit. Continue while
> characters remain. Finally append a NUL and return `stackblock()`.
>
> The do/while shape means the empty string still produces one chunk,
> `''`. A string like `a'b` becomes `'a'"'"'b'`.

> [spec:dash:def:mystring.sstrdup-fn]
> char * sstrdup(const char *p)

> [spec:dash:sem:mystring.sstrdup-fn]
> `strdup` onto the shell's stack allocator rather than the heap:
> compute `strlen(p) + 1` and `memcpy` that many bytes into `stalloc`'d
> space, returning the copy. Freed implicitly by the enclosing stack
> mark, never by `free`.
