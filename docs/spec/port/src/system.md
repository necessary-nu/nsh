# src/system.c, src/system.h

The portability layer. Almost everything here is conditional on a
`HAVE_*` macro from `configure`: on a system that provides the function,
none of this is compiled and the libc version is used. Wave 2 should
treat these as *contracts the target must satisfy* rather than as code to
reproduce — Rust's standard library supplies most of them directly, and
the fallback stubs that merely return -1 exist only so the shell links on
systems lacking the facility.

Note the `ctype` block at the top of `system.c`: on a platform without
`HAVE_ISALPHA` the header's macros are renamed to `_is*` on include, so
the file can then define real functions of the standard names that call
through to them.

`conv_escape` is declared here but defined in `src/bltin/printf.c`; its
rule is anchored to the header, and `printf.md` cross-references it.

**Twenty rules retired.** `delete-gen` removed the twenty exported items
of `crates/nsh/src/system.rs` that had no caller anywhere in the crate:
the twelve `ctype` wrappers, `stpcpy`, `strsignal`, `strtod`, `killpg`,
`sysconf`, `tee`, `memfd_create` and `fnmatch`. Every caller goes to
`libc::` directly, and for all but `fnmatch` the reference build's
`config.h` defines the matching `HAVE_*`, so the C compiles none of them
either; `HAVE_FNMATCH` is undefined, but the sole call site sits behind
`if (FNMATCH_IS_ENABLED)`, which is 0. Their blocks below therefore carry
no `[spec:dash:…]` ids — each is a C signature followed by its semantics
— and are kept because they still describe `src/system.c` and
`src/system.h`. `mempcpy`, `strchrnul`, `bsearch`, `sigclearmask`,
`conv_escape` and the `glob64` group are still implemented and still
carry their rules.

> [spec:dash:def:system.bsearch-fn]
> void *bsearch(const void *key, const void *base, size_t nmemb, size_t size, int (*cmp)(const void *, const void *))

> [spec:dash:sem:system.bsearch-fn]
> Standard binary search, provided where libc lacks it. While `nmemb` is
> non-zero: take `mididx = nmemb / 2` and the object at
> `base + mididx * size`; return it if `cmp` reports 0. When the key is
> greater, move `base` past the midpoint and reduce `nmemb` by
> `mididx + 1`; when smaller, set `nmemb = mididx`. Return NULL when the
> range empties. The array must be sorted by the same comparator.

> [spec:dash:def:system.conv-escape-fn]
> unsigned conv_escape(char *str, char *out, bool mbchar)

> [spec:dash:sem:system.conv-escape-fn]
> Decode one backslash escape. `str` points at the character *after* the
> backslash; the decoded bytes are written to the stack string `out`.
> `mbchar` selects the shell-parser dialect (true, from `parser.c`) over
> the `printf` dialect (false). Returns a packed pair: the number of
> output bytes in the low 4 bits and the number of input characters
> consumed in the remaining bits — `(out - out0) | (str - str0) << 4`.
>
> The simple letters map to control characters: `a`, `b`, `f` are handled
> arithmetically as `value - 'a' + '\a'` (which works because `\a \b`
> and `\f` are consecutive in both ASCII orderings involved), and `e`,
> `n`, `r`, `t`, `v` map to ESC, newline, carriage return, tab and
> vertical tab.
>
> Numeric forms: `x` reads up to 2 hex digits, `u` up to 4, `U` up to 8,
> and a leading octal digit reads up to 3 octal digits. Hex parsing
> accepts either case by masking with `~0x20`, and stops early at the
> first non-hex character. An octal escape with no digits yields a
> literal backslash.
>
> Two post-processing rules apply. For values below 0x80 (and for all
> 1–2 digit hex forms), `check_value` tests whether the byte collides
> with one of the shell's internal control bytes — `SQSYNTAX[value]` being
> `CCTL` — and if so, and `mbchar` is set, emits a `CTLESC` prefix so the
> byte is not later mistaken for a marker. A literal `\\` takes the same
> path unconditionally.
>
> For `u`/`U` values at or above 0x80 and below 0x110000, encode as UTF-8
> wrapped in the parser's multibyte framing: `CTLMBCHAR`, the byte length,
> the 2–4 UTF-8 bytes, the length again, and a closing `CTLMBCHAR` — the
> length appearing on both sides is what lets the parser scan the
> construct backwards as well as forwards. The UTF-8 bytes are assembled
> by packing the code point's 6-bit groups into a 32-bit word with the
> leading-byte and continuation-byte tags, then `htonl`ing it so the bytes
> land in big-endian (i.e. UTF-8) order. `mboff` is `(mbchar - 1) * 2`,
> so in the non-parser dialect the framing bytes are written and then
> stepped back over, leaving only the raw UTF-8. A value at or above
> 0x110000 produces no output at all.
>
> In the `mbchar` dialect a `"` or `'` after the backslash is left alone
> (the caller handles quote removal); otherwise an unrecognised character
> yields a literal backslash followed by that character.

> static inline int fnmatch(const char *pattern, const char *string, int flags)

> Stub returning -1, compiled only where libc has no `fnmatch`. The shell
> does its own pattern matching in `expand.c`, so this exists purely to
> satisfy the link.

> [spec:dash:def:system.gl-closedir-fn]
> void (*gl_closedir)(void *)

> [spec:dash:sem:system.gl-closedir-fn]
> A member of the fallback `glob64_t`, not a function of this program —
> the extractor lifted the member declaration out of the struct. In
> glibc's `glob` these are the `GLOB_ALTDIRFUNC` hooks that let a caller
> substitute its own directory access. In the fallback `glob64_t` they
> are declared for layout compatibility only and are never called, since
> the fallback `glob64` returns -1 without doing anything.

> [spec:dash:def:system.gl-lstat-fn]
> int (*gl_lstat)(const char *, struct stat64 *)

> [spec:dash:sem:system.gl-lstat-fn]
> `GLOB_ALTDIRFUNC` hook member of the fallback `glob64_t`; see
> `system.gl-closedir-fn`. Unused.

> [spec:dash:def:system.gl-opendir-fn]
> void *(*gl_opendir)(const char *)

> [spec:dash:sem:system.gl-opendir-fn]
> `GLOB_ALTDIRFUNC` hook member of the fallback `glob64_t`; see
> `system.gl-closedir-fn`. Unused.

> [spec:dash:def:system.gl-readdir-fn]
> struct dirent64 *(*gl_readdir)(void *)

> [spec:dash:sem:system.gl-readdir-fn]
> `GLOB_ALTDIRFUNC` hook member of the fallback `glob64_t`; see
> `system.gl-closedir-fn`. Unused.

> [spec:dash:def:system.gl-stat-fn]
> int (*gl_stat)(const char *, struct stat64 *)

> [spec:dash:sem:system.gl-stat-fn]
> `GLOB_ALTDIRFUNC` hook member of the fallback `glob64_t`; see
> `system.gl-closedir-fn`. Unused.

> [spec:dash:def:system.glob64-fn]
> static inline int glob64(const char *pattern, int flags, int (*errfunc)(const char *epath, int eerrno), glob64_t *restrict pglob)

> [spec:dash:sem:system.glob64-fn]
> Stub returning -1, compiled only where libc has no `glob`. `expand.c`
> uses libc `glob` as a fast path when available and falls back to its own
> matcher otherwise, so returning failure here simply selects the shell's
> own implementation.

> [spec:dash:def:system.glob64-t]
> typedef struct

> [spec:dash:def:system.globfree64-fn]
> static inline void globfree64(glob64_t *pglob)

> [spec:dash:sem:system.globfree64-fn]
> Empty stub, compiled only where libc has no `glob`: the fallback
> `glob64` never allocates, so there is nothing to release.

> int isalnum(int c)

> Out-of-line wrapper around the platform's `_isalnum` macro, compiled
> only without `HAVE_ISALPHA`. Semantics are the C standard's: true for
> letters and digits in the current locale.

> int isalpha(int c)

> Out-of-line wrapper around `_isalpha`; standard C semantics.

> int isblank(int c)

> Two variants exist. Where the platform declares `isblank`
> (`HAVE_DECL_ISBLANK`) but lacks `HAVE_ISALPHA`, this wraps `_isblank`.
> Where it is not declared at all, it is defined directly as
> `c == ' ' || c == '\t'`.

> int iscntrl(int c)

> Out-of-line wrapper around `_iscntrl`; standard C semantics.

> int isdigit(int c)

> Out-of-line wrapper around `_isdigit`; standard C semantics. Note the
> shell's own `is_digit` in `shell.h` is a separate, locale-independent
> test used in parsing.

> int isgraph(int c)

> Out-of-line wrapper around `_isgraph`; standard C semantics.

> int islower(int c)

> Out-of-line wrapper around `_islower`; standard C semantics.

> int isprint(int c)

> Out-of-line wrapper around `_isprint`; standard C semantics.

> int ispunct(int c)

> Out-of-line wrapper around `_ispunct`; standard C semantics.

> int isspace(int c)

> Out-of-line wrapper around `_isspace`; standard C semantics.

> int isupper(int c)

> Out-of-line wrapper around `_isupper`; standard C semantics.

> int isxdigit(int c)

> Out-of-line wrapper around `_isxdigit`; standard C semantics.

> static inline int killpg(pid_t pid, int signal)

> Send `signal` to the process group `pid`: `kill(-pid, signal)`. Under
> `DEBUG`, `abort()` on a negative `pid`, since negating it again would
> address the wrong target. Compiled only without `HAVE_KILLPG`.

> static inline int memfd_create(const char *name, unsigned int flags)

> Stub returning -1, compiled only without `HAVE_MEMFD_CREATE`.
> `sh_pipe` treats the failure as "no memfd available" and falls back to
> a real pipe, so here documents still work.

> [spec:dash:def:system.mempcpy-fn]
> void *mempcpy(void *dest, const void *src, size_t n)

> [spec:dash:sem:system.mempcpy-fn]
> `memcpy` returning a pointer *past* the copied bytes rather than to
> their start: `memcpy(dest, src, n) + n`. Used throughout the shell to
> chain appends without recomputing lengths.

> [spec:dash:def:system.sigclearmask-fn]
> static inline void sigclearmask(void)

> [spec:dash:sem:system.sigclearmask-fn]
> Unblock all signals. Uses BSD `sigsetmask(0)` where available — with
> the deprecation warning suppressed under glibc, and only for GCC 4.6+
> where the pragma works — and otherwise
> `sigprocmask(SIG_SETMASK, &empty_set, 0)`.

> char *stpcpy(char *dest, const char *src)

> Copy `src` into `dest` and return a pointer to the terminating NUL.
> Implemented as: measure the length, write the NUL at `dest[len]`, then
> `mempcpy` the `len` bytes — so the return value is the address of the
> NUL just written.

> [spec:dash:def:system.strchrnul-fn]
> char *strchrnul(const char *s, int c)

> [spec:dash:sem:system.strchrnul-fn]
> Like `strchr`, but returns a pointer to the terminating NUL instead of
> NULL when `c` is absent, so the result is always dereferenceable. The
> shell relies on this heavily to split `"name=value"` without a
> not-found branch.

> char *strsignal(int sig)

> Return a description of signal `sig`. Use `sys_siglist[sig]` when `sig`
> is in range and the entry is non-NULL; otherwise format
> `"Signal %d"` into a 19-byte static buffer and return that. The static
> buffer means the result is only valid until the next call.

> static inline double strtod(const char *nptr, char **endptr)

> Stub compiled only without `HAVE_STRTOD`: consume nothing (set
> `*endptr = nptr`) and return 0, so every input parses as "no digits
> found".

> long sysconf(int name)

> Stub compiled only without `HAVE_SYSCONF`: raise
> `sh_error("no sysconf for: %d", name)`, which does not return. Reaching
> it means the shell needed a limit the platform cannot report.

> static inline ssize_t tee(int fd_in, int fd_out, size_t len, unsigned int flags)

> Stub returning -1, compiled only without `HAVE_TEE`. `input.c` uses the
> failure — specifically the resulting `EINVAL` from `stdin_tee` — to fall
> back to unbuffered, one-byte-at-a-time reads on a shared stdin.
