# src/expand.c, src/expand.h

Word expansion: tilde, parameter, arithmetic and command substitution,
then field splitting, then pathname expansion. Input is a word in the
parser's internal encoding (see `parser.md`); output is a `struct
arglist` of `struct strlist` fields.

**`expandarg` flags** (`expand.h`): `EXP_FULL` (0x1, do field splitting
and globbing), `EXP_TILDE` (0x2), `EXP_VARTILDE` (0x4, tildes in an
assignment), `EXP_REDIR` (0x8, redirection target — one match only),
`EXP_CASE` (0x10, keep quoting for a `case` pattern), `EXP_MBCHAR`
(0x20, keep multibyte markers), `EXP_VARTILDE2` (0x40, tildes after
colons only), `EXP_WORD` (0x80, expanding the word of a parameter
expansion), `EXP_QUOTED` (0x100, inside double quotes), `EXP_KEEPNUL`
(0x200), `EXP_DISCARD` (0x400, parse but produce nothing — used for the
unevaluated branch of `${x-…}`). `QUOTES_ESC` is `EXP_FULL | EXP_CASE`,
the condition under which `CTLESC` must be emitted.

Several flag values are load-bearing and guarded by `#error` checks:
`EXP_QUOTED == EXP_FULL << CHAR_BIT`, and the relative positions of
`QUOTES_ESC`, `EXP_MBCHAR` and `EXP_QUOTED` used by `memtodest`'s
branchless test.

**`_rmescapes` flags**: `RMESCAPE_ALLOC` (0x1), `RMESCAPE_GLOB` (0x2, add
backslashes for glob), `RMESCAPE_GROW` (0x8, grow the stack string
instead of `stalloc`), `RMESCAPE_HEAP` (0x10).

**State.** `expdest` is the stack string being built. `argbackq` walks
the word's command-substitution list. `ifsfirst`/`ifslastp` hold the
`struct ifsregion` list — the byte ranges of the result that came from
expansions and are therefore subject to field splitting, which is what
makes `$x` split but a literal not. `exparg` accumulates the finished
fields.

`changeifs` precomputes the IFS lookup: `ncifs` (the effective IFS
string), `ifsmap[128]` (a byte membership table for the ASCII fast path),
`wcifs` (wide-character form, non-NULL only when IFS contains multibyte
characters), `ifsmb0len` (the byte length of the first IFS character,
used as the join separator for `$*`).

**Build-time variants.** `FNMATCH_IS_ENABLED` and `GLOB_IS_ENABLED`
select libc `fnmatch`/`glob` over the shell's own matcher. The two paths
differ in escape convention — backslash for libc, `CTLESC` for the
built-in matcher — which is why so many routines branch on it.

**`mbnext` encoding.** Several routines use `mbnext`, which returns two
byte counts packed into one unsigned: the low 8 bits are the offset from
`p` to the character's data (skipping markers), and the next 8 bits are
the span *remaining from that data position* — not the total. For framed
multibyte data the remaining span is `length + 2`, covering the trailing
length byte and closing `CTLMBCHAR`; the two leading marker bytes are
already accounted for in the low half. The **total** advance is therefore
`(mb & 0xff) + (mb >> 8)`, which is exactly the expression that appears
at every call site.

> [spec:dash:def:expand.addfname-common-fn]
> static void addfname_common(char *name)

> [spec:dash:sem:expand.addfname-common-fn]
> Append `name` to `exparg` as a new `struct strlist`, without copying
> it — the caller owns the storage.

> [spec:dash:def:expand.addfname-fn]
> STATIC void addfname(char *name)

> [spec:dash:sem:expand.addfname-fn]
> `addfname_common(sstrdup(name))` — append a *copy* of the name, used
> for `glob` results whose storage `globfree64` will reclaim.

> [spec:dash:def:expand.addfnamealt-fn]
> static char *addfnamealt(char *enddir, size_t expdir_len)

> [spec:dash:sem:expand.addfnamealt-fn]
> Append the completed pathname the stack string holds, then restart the
> stack string with just its directory prefix so the caller can build the
> next candidate. `grabstackstr(enddir)` claims the name, `addfname_common`
> appends it, then a fresh stack string is seeded with the first
> `expdir_len` bytes of that name. Returns the new block base (the
> `- expdir_len` undoes `stnputs`'s advance).

> [spec:dash:def:expand.addglob-fn]
> static void addglob(const glob64_t *pglob)

> [spec:dash:sem:expand.addglob-fn]
> Append every path in `pglob->gl_pathv` with `addfname`. The loop is
> do/while, so it assumes at least one match — guaranteed by the caller,
> which only calls this after a successful `glob64`.

> [spec:dash:def:expand.arglist]
> struct arglist {
>   struct strlist *list;
>   struct strlist **lastp;
> }

> [spec:dash:def:expand.argstr-fn]
> static char *argstr(char *p, int flag)

> [spec:dash:sem:expand.argstr-fn]
> The main expansion loop: walk the encoded word, copying literal text and
> dispatching on the control markers. Returns a pointer just past the
> terminator that ended it (NUL, `CTLENDVAR` or `CTLENDARI`), so nested
> calls can resume.
>
> `reject` is the set of bytes to stop at, taken from `spclchars` with
> the leading entries skipped according to the tilde flags: `=` and `:`
> are only interesting when tilde expansion after them is wanted, so
> `EXP_VARTILDE` includes both and `EXP_VARTILDE2` includes only `:`.
>
> `breakall` is set when expanding the word of a parameter expansion
> outside quotes (`EXP_WORD` without `EXP_QUOTED`), in which case each
> literal run is recorded as a splittable region.
>
> A leading `~` is handed to `exptilde` when `EXP_TILDE` is set; the flag
> is then cleared so only the first one counts.
>
> Each iteration uses `strcspn` to find the next interesting byte, then
> copies the run with `stnputs`. The terminator test is subtle: the byte
> is a terminator when it is NUL, `CTLENDARI` or `CTLENDVAR`, detected by
> `!!((c - 1) & 0x80)` after establishing that it is one of the
> non-high-bit cases; `q[-1] &= end - 1` then clears the copied
> terminator byte to NUL. A splittable region is recorded when `breakall`
> holds and we are not inside quotes.
>
> Then dispatch:
>
> - `=` — enable `EXP_VARTILDE2` and extend `reject` so subsequent
>   colons also trigger tilde expansion; fall through.
> - `:` — if the next character is `~`, go back to tilde expansion. This
>   is what makes `PATH=~/bin:~/sbin` expand both.
> - `CTLQUOTEMARK` — toggle the quoted state. The special case first:
>   when not already in quotes and the remaining text is exactly the
>   encoded `"$@"` (`dolatstr + 1`), expand it directly with
>   `EXP_QUOTED`, which is the hack that gives `"$@"` its
>   one-field-per-parameter behaviour. Otherwise, under `QUOTES_ESC` the
>   marker is retained in the output by rewinding one byte and extending
>   the run.
> - `CTLMBCHAR` — a framed multibyte character. When quoting escapes or
>   `EXP_MBCHAR` are wanted, keep the whole framing by extending the run.
>   Otherwise copy just the character's bytes and skip the framing.
> - `CTLESC` — retained like a quote mark, advancing `startloc` so the
>   escaped byte is not treated as the start of a splittable region.
> - `CTLVAR` — `evalvar`.
> - `CTLBACKQ` — `expbackq` on the current entry of `argbackq`.
> - `CTLARI` — `expari`.

> [spec:dash:def:expand.arith-fn]
> intmax_t arith(const char *)

> [spec:dash:sem:expand.arith-fn]
> Prototype only; the implementation is in `arith_yacc.c`. See
> `arith-yacc.arith-fn`.

> [spec:dash:def:expand.arith-lex-reset-fn]
> void arith_lex_reset(void)

> [spec:dash:sem:expand.arith-lex-reset-fn]
> Reset the arithmetic lexer. Declared as a function only under
> `USE_LEX`, where a generated lexer needs its buffer state cleared
> between expressions; in the shipped build it is a macro expanding to
> nothing, because `arith_yylex.c` keeps no state beyond `arith_buf`.
> Nothing to port unless a generated lexer is used.

> [spec:dash:def:expand.casematch-fn]
> int casematch(union node *pattern, char *val)

> [spec:dash:sem:expand.casematch-fn]
> Test whether a `case` pattern matches `val`. Inside a stack mark: point
> `argbackq` at the pattern's substitutions, expand it with
> `EXP_TILDE | EXP_CASE` — `EXP_CASE` keeps the quoting markers, so that
> a quoted `*` in the pattern stays literal — release the IFS regions
> (the pattern is not split), and `patmatch` the result against `val`.

> [spec:dash:def:expand.ccmatch-fn]
> static __attribute__((noinline)) int ccmatch(char *p, const char *mbc, int ml, char **r)

> [spec:dash:sem:expand.ccmatch-fn]
> Match a POSIX character class such as `[:alpha:]` inside a bracket
> expression. `p` points just past the `[`. Returns whether the character
> matches, and sets `*r` to the position after the closing `:]` — or NULL
> if this is not a character class at all, which tells the caller to
> treat the `[` literally.
>
> Require a leading `:` and a following `:]`. Temporarily NUL-terminate
> the class name to call `wctype`; an unknown class also returns "not a
> class". Then convert the candidate character with `mbrtowc`, requiring
> that it consume exactly `ml` bytes, and test with `iswctype`.

> [spec:dash:def:expand.changeifs-fn]
> void changeifs(const char *ifs)

> [spec:dash:sem:expand.changeifs-fn]
> Recompute the cached IFS representation. Callback on the `IFS`
> variable; an unset `IFS` uses `defifs` (space, tab, newline).
>
> Build `ifsmap`, a 128-entry table marking each ASCII IFS byte, and
> note in `mb` whether any byte has the high bit set. Record the length.
> `ifsmb0len` starts as "1 if IFS is non-empty".
>
> If no high-bit bytes are present, that is all — `wcifs` becomes NULL
> and the fast ASCII path applies everywhere. Otherwise measure the first
> character's byte length with `mbrlen` (treating an invalid or
> incomplete sequence as 1) into `ifsmb0len`, and convert the whole
> string to wide characters into a freshly allocated `wcifs`.
>
> Free the previous `wcifs` and install the new one.

> [spec:dash:def:expand.chtodest-fn]
> static char *chtodest(int c, const char *syntax, char *out)

> [spec:dash:sem:expand.chtodest-fn]
> Append one byte to the output, prefixed by `CTLESC` when the syntax
> table classifies it `CCTL` — i.e. when it would otherwise be mistaken
> for one of the shell's internal markers.

> [spec:dash:def:expand.cvtnum-fn]
> static size_t cvtnum(intmax_t num, int flags)

> [spec:dash:sem:expand.cvtnum-fn]
> Render `num` in decimal into a stack-sized buffer
> (`max_int_length(sizeof(num))`) and append it to the output with
> `memtodest`. Returns the number of bytes appended.

> [spec:dash:def:expand.esclen-fn]
> static size_t esclen(const char *start, const char *p)

> [spec:dash:sem:expand.esclen-fn]
> `mesclen(start, p, CTLESC)` — count the run of `CTLESC` bytes
> immediately before `p`.

> [spec:dash:def:expand.evalvar-fn]
> STATIC char * evalvar(char *p, int flag)

> [spec:dash:sem:expand.evalvar-fn]
> Expand one `${…}` construct. `p` points at the subtype byte the parser
> emitted; the name follows, terminated by `=`, and the word (for the
> forms that take one) after that. Returns a pointer past the construct.
>
> Read the subtype (masking off `VSBIT`) and remember `startloc`, where
> this expansion's output begins.
>
> Evaluate the variable with `varvalue`, adding `EXP_MBCHAR` for the four
> trimming subtypes so their pattern matching sees multibyte markers. A
> `VSNUL` subtype (`${x:-…}` rather than `${x-…}`) decrements the length,
> which turns an empty value into a negative one and so makes it count as
> unset. `discard` is `EXP_DISCARD` exactly when the variable is
> unset/null.
>
> Dispatch on subtype:
>
> - `VSPLUS` — invert `discard`, then fall through, so the word is used
>   when the variable *is* set.
> - `VSNORMAL` (0) and `VSMINUS` — expand the word with `argstr`, passing
>   `discard ^ EXP_DISCARD` so it is only produced when wanted; the word
>   is still parsed either way, which is required for correct nesting.
> - `VSASSIGN`/`VSQUESTION` — hand to `subevalvar`, which assigns or
>   raises. After a successful assignment, clear `VSNUL`, switch to
>   `VSNORMAL` and re-evaluate, so `${x=y}` yields the newly assigned
>   value.
>
> Then: an unset variable raises `varunset` when `set -u` is on — but
> the guard is `(discard & ~flag) && uflag`, so it also requires that the
> *caller* was not already discarding. An unset variable inside an
> already-suppressed branch does not trigger `-u`. `VSLENGTH`
> emits the length (clamped at 0) with `cvtnum` and jumps past the
> `record:` discard test — but not past everything: it still returns
> early when `flag & EXP_DISCARD`, and `really_record:` still applies its
> own `if (quoted)` early return. "Unconditionally" would overstate it.
> `VSNORMAL` records. The trimming subtypes
> NUL-terminate the value, note where the pattern starts, and call
> `subevalvar` to do the trimming.
>
> Finally record the output range as splittable — except that inside
> double quotes nothing is splittable *unless* this is `$@` with at least
> one positional parameter, which is the one construct that splits while
> quoted; the `nulonly` argument to `recordregion` carries that
> distinction.

> [spec:dash:def:expand.expandarg-fn]
> void expandarg(union node *arg, struct arglist *arglist, int flag)

> [spec:dash:sem:expand.expandarg-fn]
> Expand one word node and append the resulting fields to `arglist`. A
> NULL `arglist` means here-document expansion: the result is left on the
> stack and nothing is split or globbed.
>
> Point `argbackq` at the node's substitution list, start a fresh stack
> string, and run `argstr`. Then, with `EXP_FULL`, split the result with
> `ifsbreakup(p, -1, &exparg)` — no argument limit — and glob it with
> `expandmeta`. Without `EXP_FULL` the whole result becomes one field.
> Append whatever was produced to the caller's list (possibly nothing, as
> for an unquoted `$@` with no parameters). Release the IFS regions on
> every path.

> [spec:dash:def:expand.expandmeta-fn]
> STATIC void expandmeta(struct strlist *str)

> [spec:dash:sem:expand.expandmeta-fn]
> Pathname expansion using the shell's own matcher; delegates to
> `expandmeta_glob` when libc `glob` is enabled.
>
> For each field: skip globbing entirely under `set -f` (`fflag`), or
> when the text contains none of `*?]` — or is exactly `]`, which cannot
> be a bracket expression. Otherwise remember the list tail, prepare the
> pattern with `preglob` and run `expmeta`.
>
> If nothing was appended there were no matches, so the field is used
> literally with its escapes removed. Otherwise sort the new entries with
> `expsort` and splice them in — POSIX requires pathname expansion
> results to be sorted.

> [spec:dash:def:expand.expandmeta-glob-fn]
> static void expandmeta_glob(struct strlist *str)

> [spec:dash:sem:expand.expandmeta-glob-fn]
> Pathname expansion via libc `glob64`. For each field: skip under
> `set -f`. Under glibc, install the `GLOB_ALTDIRFUNC` hooks so directory
> reading goes through `opendir_interruptible`, which lets a pending
> SIGINT break out of a large glob. Prepare the pattern with `preglob`
> into heap storage and call
> `glob64(p, GLOB_ALTDIRFUNC | GLOB_NOMAGIC, 0, &pglob)`.
>
> On success, `GLOB_NOMAGIC | GLOB_NOCHECK` both set in the result flags
> means the pattern had no metacharacters, so it is treated as a literal;
> otherwise append the matches. `GLOB_NOMATCH` also falls back to the
> literal, with its escapes removed. `GLOB_NOSPACE` raises
> `sh_error("Out of space")`.
>

> [spec:dash:def:expand.expari-fn]
> static char *expari(char *start, int flag)

> [spec:dash:sem:expand.expari-fn]
> Expand `$(( … ))`. Note where the expansion's output begins, then
> `argstr` the expression body so that any nested expansions inside it
> are performed first — the arithmetic parser sees fully expanded text.
>
> Then rewind the output pointer to the start of the expression, so the
> expression text is about to be overwritten by its value, and discard
> any IFS regions recorded within it (they described text that no longer
> exists). Evaluate with `arith` inside a stack mark that protects the
> expression text, then append the result with `cvtnum`.
>
> Record the digits as splittable unless quoted. Under `EXP_DISCARD`
> nothing is evaluated at all — important, since `$(( 1/0 ))` in an
> unused branch must not error.

> [spec:dash:def:expand.expbackq-fn]
> STATIC void expbackq(union node *cmd, int flag)

> [spec:dash:sem:expand.expbackq-fn]
> Expand a command substitution. Under `EXP_DISCARD` only the
> `argbackq` cursor is advanced.
>
> With interrupts suspended: note the output position, and run the
> command with `evalbackcmd` inside a stack mark, which returns either a
> buffer or a descriptor to read from. Copy the output into the stack
> string with `memtodest`, reading 128 bytes at a time and retrying on
> `EINTR`, until end of file.
>
> Then release the buffer, close the descriptor, and `waitforjob` the
> child — storing its status in `back_exitstatus`, which is what a bare
> assignment command reports as its own status.
>
> Strip *all* trailing newlines from the output, as POSIX requires.
> Record the result as splittable unless quoted. Advance `argbackq`.

> [spec:dash:def:expand.expcmd-fn]
> int expcmd(int , char **)

> [spec:dash:sem:expand.expcmd-fn]
> Declared in `expand.h` but not defined anywhere in the tree — a vestige
> of a removed builtin. There is nothing to port; Wave 2 should record
> the omission.

> [spec:dash:def:expand.expmeta-fn]
> static char *expmeta(char *name, unsigned name_len, size_t expdir_len)

> [spec:dash:sem:expand.expmeta-fn]
> The shell's own recursive pathname expansion, one path component per
> level. `name` is the remaining pattern, `expdir_len` the length of the
> directory prefix already built on the stack. Returns the (possibly
> moved) stack block base.
>
> The function *appears* to install an exception handler so an interrupt
> during a deep directory walk still closes the open `DIR`: it saves
> `savehandler = handler` and calls `setjmp(jmploc.loc)`. **It never
> assigns `handler = &jmploc`** — there is no such assignment anywhere in
> `expand.c` — so nothing can longjmp to that target. The `err` arm, the
> `handler = savehandler` restore and the trailing
> `if (err) longjmp(...)` are all dead, and the `volatile` on `dirp` and
> `err` is vestigial. Compare `redirectsafe` in `redir.c`, which does
> perform the assignment.
>
> `closedir` at the `out:` label still runs on the normal path, so the
> descriptor is not leaked in ordinary operation; interruption is handled
> by the `int_pending()` check inside the loop, not by unwinding.
>
> Port the dead machinery as written (Wave 2 is bug-for-bug) or omit it
> and record the omission — but do not "repair" it by adding the missing
> assignment, which would change behaviour.
>
> Find the first *unescaped* metacharacter (`*?]`), skipping ones
> preceded by an odd number of escape characters — `\` under `fnmatch`,
> `CTLESC` otherwise.
>
> **No metacharacter**: at the top level there is nothing to do.
> Otherwise this is a fully literal final component, so remove its
> escapes, `lstat64` it, and append it if it exists. `lstat` rather than
> `stat` so a dangling symlink still matches.
>
> **With a metacharacter**: split off the literal directory part before
> the last `/` preceding it, unescape that into the prefix, and open it —
> or the current directory when there is no prefix. Then isolate this
> component by NUL-terminating at the next `/`, again respecting escapes,
> remembering whether more components follow (`c`).
>
> `matchdot` records whether the pattern component starts with a literal
> `.` (checked past a leading escape); when it does not, entries
> beginning with `.` are skipped, which is the rule that hides dotfiles.
>
> For each directory entry: skip dotfiles per `matchdot`; skip
> non-directories when more components follow, using `d_type` where the
> filesystem provides it (`DT_UNKNOWN` falls through to a real match
> attempt). Without libc `fnmatch`, the name is first re-encoded through
> `memtodest` so multibyte characters carry markers the matcher
> understands. On a match, append the name to the prefix and either
> record the complete path or add a `/` and recurse for the next
> component. `int_pending()` is checked each iteration so a large walk
> can be interrupted.
>
> Restore the delimiter byte, close the directory, restore the handler,
> and re-raise if an exception was caught.

> [spec:dash:def:expand.expmeta-rmescapes-fn]
> static char *expmeta_rmescapes(char *enddir, const char *name)

> [spec:dash:sem:expand.expmeta-rmescapes-fn]
> Copy `name` into `enddir` with its escapes removed, returning a pointer
> to the terminating NUL. Without libc `fnmatch` this is just
> `rmescapes` over a copy. With it, the escape character is `\`, so the
> loop copies up to each backslash and then substitutes the character
> that follows — a trailing lone backslash ends the copy.

> [spec:dash:def:expand.expsort-fn]
> strlist * expsort(struct strlist *str)

> [spec:dash:sem:expand.expsort-fn]
> Sort a `strlist` by counting its length and calling `msort`.

> [spec:dash:def:expand.exptilde-fn]
> static char *exptilde(char *startp, int flag)

> [spec:dash:sem:expand.exptilde-fn]
> Expand a leading `~`. Scan the name after it, stopping at `/` or
> `CTLENDVAR`, and at `:` when `EXP_VARTILDE` is set (so
> `PATH=~/x:~/y` works). A `CTLESC` or `CTLQUOTEMARK` anywhere in the
> name means the tilde was quoted, so return `startp` unchanged and let
> it be literal.
>
> An empty name uses `HOME`; otherwise `getpwhome` looks the user up. A
> lookup failure also returns `startp` unchanged — an unknown user leaves
> the tilde literal, as POSIX requires. Otherwise append the home
> directory with `EXP_QUOTED` (so it is not itself split or globbed) and
> return the position after the name.

> [spec:dash:def:expand.getpwhome-fn]
> static inline const char *getpwhome(const char *name)

> [spec:dash:sem:expand.getpwhome-fn]
> Return the home directory of user `name` via `getpwnam`, or NULL if
> unknown — and unconditionally NULL where `HAVE_GETPWNAM` is not
> defined, so `~user` never expands on such systems.

> [spec:dash:def:expand.ifs-state]
> struct ifs_state {
>   const char *ifs;
>   char *start;
>   char *r;
>   int maxargs;
>   int ifsspc;
> }

> [spec:dash:def:expand.ifsbreakup-fn]
> void ifsbreakup(char *string, int maxargs, struct arglist *arglist)

> [spec:dash:sem:expand.ifsbreakup-fn]
> Split `string` into fields on IFS, appending them to `arglist`. Only
> the byte ranges recorded by `recordregion` are examined — literal text
> is never split. A non-negative `maxargs` caps the number of fields, with
> the remainder joined into the last one (used by `read`).
>
> With no recorded regions, the whole string is one field (or none, if it
> is empty).
>
> Otherwise walk the regions. For each, the effective IFS is `nullstr`
> when the region is `nulonly` — the `"$@"` case, where only the
> synthesised NUL separators split — and `ncifs` otherwise. Within a
> region, an eight-byte-at-a-time fast path skips runs containing no
> high-bit byte and no IFS byte (via `ifsmap`); anything else goes to
> `ifsbreakup_slow` one character at a time.
>
> Afterwards, a final field is emitted from `ifst.start` unless it is
> empty — except in the `nulonly` case, where an empty final field is
> still emitted, which is what makes `"$@"` produce an empty last
> parameter. `ifst.r`, if set, marks trailing IFS whitespace to be
> truncated.

> [spec:dash:def:expand.ifsbreakup-slow-fn]
> static char *ifsbreakup_slow(struct ifs_state *ifst, struct arglist *arglist, int nulonly, char *p)

> [spec:dash:sem:expand.ifsbreakup-slow-fn]
> Process one character during field splitting and return the position
> after it. `ifst->ifsspc` records that the previous separator was IFS
> whitespace, which coalesces with adjacent whitespace; a non-whitespace
> IFS character always separates, even when adjacent to whitespace.
>
> Classify the character with `ifsisifs`, which reports both "is an IFS
> character" and "is IFS whitespace".
>
> With `maxargs` exhausted (0), no more fields may be created: remember
> the start of trailing IFS whitespace in `ifst->r` so it can be
> truncated, and clear it as soon as a non-whitespace, non-separator
> character appears — so the last field keeps its embedded separators but
> loses its trailing blanks.
>
> After IFS whitespace, a further IFS character is absorbed into the same
> separator run, and the field start is advanced.
>
> On an IFS character that ends a field: leading IFS whitespace before
> any content is skipped rather than producing an empty field; reaching
> the `maxargs` limit stops splitting; otherwise NUL-terminate the field,
> append it, and start the next one after the separator.
>
> Note the `ifsspc` update in that branch is guarded by `if (!nulonly)`.
> Under `nulonly` — the `"$@"` case, where the separators are synthesised
> NUL bytes — `ifsspc` is therefore never set, so consecutive separators
> do not coalesce and an empty positional parameter survives as an empty
> field. That guard is what makes `"$@"` preserve empty arguments.

> [spec:dash:def:expand.ifsfree-fn]
> void ifsfree(void)

> [spec:dash:sem:expand.ifsfree-fn]
> Release the recorded IFS regions: free every node after the static
> `ifsfirst` with interrupts suspended, and clear `ifslastp` so the list
> reads as empty. Called at the end of every expansion and from the
> `EXITRESET` event.

> [spec:dash:def:expand.ifsisifs-fn]
> static unsigned ifsisifs(const char *p, unsigned ml, const char *ifs)

> [spec:dash:sem:expand.ifsisifs-fn]
> Classify a character as an IFS separator. Returns
> `isifs << 1 | isdefifs`, where `isdefifs` means it is IFS *whitespace*
> (which coalesces).
>
> When IFS contains multibyte characters (`wcifs` non-NULL), convert a
> high-bit character with `mbrtowc` — requiring it to consume exactly
> `ml` bytes — and search `wcifs`. Otherwise, for single-byte input only,
> search `ifs` with `strchr`. Whitespace is decided by `iswspace` on the
> character, or on the first IFS character when the character is NUL —
> the synthesised separator used for `"$@"`.

> [spec:dash:def:expand.ifsregion]
> struct ifsregion {
>   struct ifsregion *next;
>   int begoff;
>   int endoff;
>   int nulonly;
> }

> [spec:dash:def:expand.mbnext-fn]
> static __attribute__((noinline)) unsigned mbnext(const char *p)

> [spec:dash:sem:expand.mbnext-fn]
> Measure the encoded character at `p`, returning `start | end << 8`
> where `start` is the offset of the character's data past any markers
> and `end` is the total span.
>
> A `CTLMBCHAR` introduces framed multibyte data: skip an optional
> `CTLESC`, read the length byte, and report the data offset and a total
> of `length + 2` (the trailing length byte and closing `CTLMBCHAR`; the
> leading two are already consumed). A `CTLESC` means the real character
> is one byte further on, so `start` is 1. Anything else is a plain byte:
> `start` 0, `end` 1.

> [spec:dash:def:expand.mbpair]
> struct mbpair {
>   unsigned ml;
>   unsigned ql;
> }

> [spec:dash:def:expand.mbtodest-fn]
> static struct mbpair mbtodest(const char *p, char *q, const char *syntax, size_t len)

> [spec:dash:sem:expand.mbtodest-fn]
> Copy one multibyte character to the output, adding the parser's framing
> when the syntax table says `CTLMBCHAR` needs escaping. Returns `ml`,
> the number of *additional* input bytes consumed, and `ql`, the number
> of output bytes written.
>
> Note `p` is decremented on entry — the caller has already advanced past
> the first byte. `mbrlen` measures the character; an invalid,
> incomplete or single-byte result falls back to `chtodest` on the single
> byte. Otherwise emit `CTLMBCHAR`, the length, the bytes, the length
> again and a closing `CTLMBCHAR` — or just the bytes when framing is not
> needed.
>
> The framing test is `syntax[CTLMBCHAR] == CCTL`, a *negative* index
> that is only valid because the real tables are passed offset by
> `SYNBASE`. See the hazard note under `expand.memtodest-fn`: in the
> `is_type` mode the same expression is an out-of-bounds read that a port
> must model as an explicit no-escape mode instead.

> [spec:dash:def:expand.memrchr-fn]
> static void *memrchr(const void *s, int c, size_t n)

> [spec:dash:sem:expand.memrchr-fn]
> Find the last occurrence of byte `c` in the `n` bytes at `s`, or NULL.
> Compiled only where libc lacks it.

> [spec:dash:def:expand.memtodest-fn]
> static size_t memtodest(const char *p, size_t len, int flags)

> [spec:dash:sem:expand.memtodest-fn]
> Append `len` bytes to the output, applying whatever escaping the flags
> demand. Returns the number of source characters consumed (not bytes
> written).
>
> Reserve `len * 3` bytes — the worst case, a framed multibyte character
> per input byte. Then pick the syntax table by a branchless test: when
> neither quoting escapes nor multibyte marking is wanted, use an
> eight-byte-at-a-time fast path that copies runs containing no high-bit
> and no zero byte, then continue with `BASESYNTAX` or `is_type`;
> otherwise use `SQSYNTAX`, which classifies every marker as `CCTL`. The
> shift amounts in that test depend on the numeric flag values and are
> guarded by an `#error`.
>
> Per byte: NUL is dropped unless `EXP_KEEPNUL` (it is the `"$@"`
> separator). A high-bit byte goes through `mbtodest`, which may consume
> several. Everything else goes through `chtodest`.
>
> **Hazard — the `is_type` branch is not a real syntax table.**
> `BASESYNTAX` and `SQSYNTAX` are `<table> + SYNBASE`, so they may be
> indexed with a sign-extended `char`. `is_type` here is passed
> **unoffset**. Two consequences, both relied upon:
>
> - `chtodest` reads `syntax[c]` with `c` in `0..=127` (negative bytes go
>   to `mbtodest` first). Those are the entries below `SYNBASE`, which
>   `mksyntax` filled with `0` and never touched, so the test against
>   `CCTL` (12) is never true. In bounds, and deliberate.
> - `mbtodest` reads `syntax[CTLMBCHAR]`, i.e. `syntax[-123]`, and
>   `memtodest`'s own loop reaches `syntax[c]` for any `c < 0`. With a
>   real table those land inside it; with unoffset `is_type` they read up
>   to 129 bytes *before* the array — out of bounds, undefined behaviour.
>   Where they land is decided by link layout, not by the source: in the
>   reference build the bytes below `is_type` are `nodesize` and
>   `defpathvar` (`arisyntax` sits *after* `is_type`, not before it), and
>   the byte at `is_type - 123` is `0xE2`. Non-`CCTL`, so no framing is
>   emitted.
>
> Both cases mean the same thing: "this mode escapes nothing". A port
> must not reproduce the out-of-bounds index. Model the third mode
> explicitly — a no-escape mode alongside the two real tables — and treat
> every `syntax[…] == CCTL` test in it as false. That is the observed
> behaviour, faithfully.
>
> **Why this rule says "do not reproduce" while
> `[spec:dash:sem:expand.expmeta-fn]` says "port the dead machinery as
> written".** The two are not in conflict, because the C constructs
> differ in kind. `expmeta`'s never-installed handler is fully determined
> by the source: it can only ever behave one way, so transcribing it
> reproduces the behaviour exactly. This read is not determined by the
> source at all — its result is a property of one binary's memory layout.
> Transcribing it would give the port its *own* independently arbitrary
> answer, which matches the C only by luck and can diverge silently the
> moment either side is relaid out. Bug-for-bug means reproducing
> observable behaviour; for layout-dependent UB, that means reproducing
> the outcome, not the undefinedness.

> [spec:dash:def:expand.mesclen-fn]
> static size_t mesclen(const char *start, const char *p, char mesc)

> [spec:dash:sem:expand.mesclen-fn]
> Count the run of escape characters `mesc` immediately before `p`,
> stopping at `start`. Used to decide whether a metacharacter is escaped:
> an odd count means it is.

> [spec:dash:def:expand.msort-fn]
> strlist * msort(struct strlist *list, int len)

> [spec:dash:sem:expand.msort-fn]
> Merge sort a `strlist` of known length `len`. Lists of 0 or 1 elements
> are returned as is. Otherwise split at the midpoint by walking `half`
> links and cutting, sort both halves recursively, and merge comparing
> with `strcoll` — the locale's collating order, as POSIX requires for
> pathname expansion results. The merge is stable in the sense that ties
> take from the second half first, since the comparison is strict `< 0`.

> [spec:dash:def:expand.opendir-interruptible-fn]
> static void *opendir_interruptible(const char *pathname)

> [spec:dash:sem:expand.opendir-interruptible-fn]
> `opendir`, but first deliver any pending interrupt: when `int_pending()`
> holds, clear `suppressint` and call `onint()`, which does not return.
> Installed as glibc `glob`'s `gl_opendir` hook so that a glob traversing
> a huge tree can still be interrupted. glibc-only.

> [spec:dash:def:expand.patmatch-fn]
> STATIC inline int patmatch(char *pattern, const char *string)

> [spec:dash:sem:expand.patmatch-fn]
> `pmatch(preglob(pattern, 0), string)` — prepare the pattern (removing
> quoting while preserving escaped metacharacters) and match.

> [spec:dash:def:expand.pmatch-fn]
> static int pmatch(char *pattern, const char *string)

> [spec:dash:sem:expand.pmatch-fn]
> The shell's own pattern matcher, used when libc `fnmatch` is not
> enabled (where it is, this is just `!fnmatch(pattern, string, 0)`).
> Returns 1 on a full match.
>
> Walk the pattern:
>
> - end of pattern — match iff the string is also exhausted.
> - `CTLESC` — the next byte is literal.
> - `?` — consume one whole character of the string (via `mbnext`, so a
>   multibyte character counts as one); fail at end of string.
> - `*` — collapse a run of `*`; a trailing `*` matches everything.
>   Otherwise take the next pattern character as an anchor and try to
>   match the rest at each subsequent position. When the anchor is a
>   literal byte, `strpbrk` skips directly to candidate positions —
>   searching for the anchor, `CTLESC` or `CTLMBCHAR`, so encoded
>   characters are not skipped over. `?` and `[` are treated as
>   non-anchors by substituting `CTLESC`, which forces the
>   character-by-character path.
> - `[` — a bracket expression. An initial `!` or `^` inverts. Members
>   may be a `[:class:]` (via `ccmatch`), an escaped byte, a framed
>   multibyte character, or a plain byte; `a-z` forms a range, which is
>   compared only for single-byte endpoints. An unterminated bracket
>   expression rewinds and treats the `[` as a literal. On a match, the
>   whole (possibly multibyte) character is consumed.
> - `CTLMBCHAR` — a literal multibyte character in the pattern: compare
>   its bytes against the string's.
> - anything else — must equal the next character of the string, which
>   must be single-byte.

> [spec:dash:def:expand.preglob-fn]
> STATIC inline char * preglob(const char *pattern, int flag)

> [spec:dash:sem:expand.preglob-fn]
> Prepare a pattern for matching: `_rmescapes` with `RMESCAPE_GLOB`
> added, which removes the shell's quoting markers while converting
> quoted metacharacters into the escape form the matcher expects. Under
> libc `fnmatch`, `RMESCAPE_ALLOC` is forced (and `RMESCAPE_GROW`
> defaulted) because the backslash escaping can make the result longer
> than the input. Returns stack-allocated storage.

> [spec:dash:def:expand.recordregion-fn]
> void recordregion(int start, int end, int nulonly)

> [spec:dash:sem:expand.recordregion-fn]
> Record that bytes `[start, end)` of the result came from an expansion
> and are therefore subject to field splitting. The first region uses the
> static `ifsfirst`; later ones are allocated with interrupts suspended
> and chained. `nulonly` marks a region where only NUL bytes separate —
> the `"$@"` case.

> [spec:dash:def:expand.removerecordregions-fn]
> void removerecordregions(int endoff)

> [spec:dash:sem:expand.removerecordregions-fn]
> Discard recorded regions at or beyond `endoff`, and truncate a region
> that straddles it. Used when output is rewound — for instance when
> `$(( … ))` replaces its expression text with a number, or when a
> `${x%pat}` trim shortens the value.
>
> If the first region already ends beyond `endoff`, free the whole rest
> of the chain, then either drop the first region entirely (when it also
> *begins* beyond `endoff`) or truncate it. Otherwise walk to the last
> region beginning before `endoff`, free everything after it, and
> truncate it if needed.

> [spec:dash:def:expand.restore-handler-expandarg-fn]
> void restore_handler_expandarg(struct jmploc *savehandler, int err)

> [spec:dash:sem:expand.restore-handler-expandarg-fn]
> Shared epilogue for code that runs an expansion under its own exception
> handler. Restore `handler`. If an exception was caught and it is *not*
> `EXERROR` — i.e. not an ordinary error that this caller is allowed to
> absorb — re-raise it with `longjmp`. Otherwise release the IFS regions,
> which the abandoned expansion would have leaked.

> [spec:dash:def:expand.rmescapes-fn]
> char * _rmescapes(char *str, int flag)

> [spec:dash:sem:expand.rmescapes-fn]
> Remove the shell's quoting markers from `str`, optionally converting
> them into glob escapes. Returns the result, which is `str` itself when
> nothing needed changing, and otherwise fresh storage (stack, grown
> stack string, or heap, per the `RMESCAPE_*` flags).
>
> Return immediately when the string contains none of `cqchars`
> (backslash, `CTLESC`, `CTLMBCHAR`, `CTLQUOTEMARK`). With
> `RMESCAPE_ALLOC`, size the destination as the prefix plus the rest —
> doubled under libc `fnmatch` with globbing, since each character may
> gain a backslash — and copy the untouched prefix.
>
> Then walk the remainder tracking `inquotes` (inside a `CTLQUOTEMARK`
> pair) and `notescaped`:
>
> - `CTLQUOTEMARK` — dropped; toggles `inquotes` when globbing.
> - `\` — a backslash the user wrote. Outside quotes it toggles the
>   escape state; without libc `fnmatch` it becomes a `CTLESC`.
> - `CTLESC` — the following byte is literal. When globbing, an escape
>   character is emitted so the matcher does not treat it as a
>   metacharacter; the exact form (`\` or `CTLESC`) and whether a
>   preceding byte is overwritten depend on the quoting state.
> - `CTLMBCHAR` — a framed multibyte character: copy its bytes, keeping
>   or dropping the framing according to whether globbing is wanted and
>   which matcher is in use.
>
> Finally NUL-terminate, and when the result was built on the stack
> update `expdest` and commit the space with `STADJUST`.

> [spec:dash:def:expand.scanleft-fn]
> static char *scanleft(char *startp, char *endp, char *rmesc, char *rmescend, char *str, int quotes, int zero )

> [spec:dash:sem:expand.scanleft-fn]
> Find a prefix or suffix match by scanning left to right, for
> `${x#pat}` (shortest prefix) and `${x%%pat}` (longest suffix). Returns
> the boundary position, or NULL if nothing matched.
>
> At each position, temporarily NUL-terminate — at that position when
> `zero` is set (testing a *prefix* ending there) or leaving the string
> whole otherwise (testing a *suffix* starting there) — call `pmatch`,
> and restore the byte. Two parallel cursors are maintained, `loc` into
> the escaped text and `loc2` into the unescaped copy, and which one is
> returned depends on `quotes`; advancing uses `mbnext` so multibyte
> characters move both cursors correctly.
>
> Scanning left to right yields the shortest match for a prefix and the
> longest for a suffix, which is why the caller picks this direction for
> exactly those two subtypes.

> [spec:dash:def:expand.scanright-fn]
> static char *scanright(char *startp, char *endp, char *rmesc, char *rmescend, char *str, int quotes, int zero )

> [spec:dash:sem:expand.scanright-fn]
> The mirror of `scanleft`, scanning right to left for `${x##pat}`
> (longest prefix) and `${x%pat}` (shortest suffix). Same
> NUL-terminate/match/restore approach and same dual-cursor convention.
>
> Moving backwards over the encoded text is the harder direction: an
> escape run is counted with `esclen` so that an escaped byte is stepped
> over as a unit, and a framed multibyte character is recognised by its
> trailing `CTLMBCHAR` and length byte, which is precisely why the parser
> writes the length on both sides of the character.

> [spec:dash:def:expand.strlist]
> struct strlist {
>   struct strlist *next;
>   char *text;
> }

> [spec:dash:def:expand.strtodest-fn]
> static size_t strtodest(const char *p, int flags)

> [spec:dash:sem:expand.strtodest-fn]
> `memtodest(p, strlen(p), flags)`.

> [spec:dash:def:expand.subevalvar-fn]
> static char *subevalvar(char *start, char *str, int strloc, int startloc, int varflags, int flag)

> [spec:dash:sem:expand.subevalvar-fn]
> Handle the `${x=word}`, `${x?word}` and the four trimming forms.
> `start` is the word text, `str` the variable name (for assign/error) or
> NULL (for trimming), `strloc` where the pattern begins, `startloc` where
> the variable's value begins.
>
> First expand the word with `argstr`, adding `EXP_TILDE`, and `EXP_CASE`
> for the trimming forms so the pattern keeps its quoting.
>
> `VSASSIGN` assigns the expanded word to the variable and uses it as the
> result. `VSQUESTION` raises `varunset` and does not return.
>
> For trimming, normalise the subtype to 0–3 and derive two booleans from
> its bits: `zero` (bit 1) distinguishes prefix removal from suffix
> removal, and `(subtype & 1) ^ zero` selects `scanleft` versus
> `scanright` — the combination that gives shortest-match for `#`/`%` and
> longest-match for `##`/`%%`.
>
> Prepare the pattern with `preglob`, and — when libc `fnmatch` is in use
> or quoting escapes are not being emitted — build an unescaped copy of
> the value with `_rmescapes`, since the matcher needs plain text while
> the result must keep its escapes. All the offsets are re-derived from
> `stackblock()` after each of these calls because they may move the
> stack.
>
> Run the scan. Depending on whether it matched and on `quotes`, choose
> the surviving range: no match leaves the value untouched; otherwise the
> part before or after the boundary is kept. `memmove` it to the front,
> NUL-terminate, and shrink the stack string to fit with `STADJUST`.
>
> Finally `removerecordregions(startloc)`, since the recorded splittable
> ranges described text that has just moved.

> [spec:dash:def:expand.varunset-fn]
> STATIC void varunset(const char *end, const char *var, const char *umsg, int varflags)

> [spec:dash:sem:expand.varunset-fn]
> Raise the "parameter not set" error and do not return. With no
> user-supplied message, or when the word was empty (`*end` is
> `CTLENDVAR`), use `"parameter not set"` — appending `" or null"` when
> `VSNUL` makes null count as unset. Otherwise use the user's message.
> The variable name is printed with an explicit length, since it is
> terminated by `=` rather than NUL.

> [spec:dash:def:expand.varvalue-fn]
> static ssize_t varvalue(char *name, int varflags, unsigned flags)

> [spec:dash:sem:expand.varvalue-fn]
> Append a variable's value to the output and return its length, or -1 if
> it is unset.
>
> `discard` is set for `VSPLUS` and `VSLENGTH` — both need to know
> whether the variable is set but not its text — and for `EXP_DISCARD`;
> in those cases quoting escapes are suppressed and the output is rewound
> at the end, so the function is used purely for its length and set-ness.
> A subtype of 0 with no discard is `sh_error("Bad substitution")`.
>
> Special parameters: `$` gives `rootpid`, `?` gives `exitstatus`, `#`
> gives the parameter count, `!` gives `backgndpid` (reported unset when
> 0), each rendered with `cvtnum`. `-` gives the option letters of every
> set option that has one, in reverse table order.
>
> `@` and `*`: `"$@"` — quoted *and* splitting — goes straight to the
> parameter loop with `seps`/`seplen` untouched, and the recorded
> region's `nulonly` flag makes each parameter its own field.
>
> Every other case computes the separator branchlessly. `seps` starts as
> `nullstr` and `seplen` as the `EXP_FULL` bit. Then
> `seplen &= ~(flags >> CHAR_BIT)` clears it when `EXP_QUOTED` is set —
> which works only because `EXP_QUOTED == EXP_FULL << CHAR_BIT`, enforced
> by an `#error`. So `seplen` survives exactly when splitting *and* not
> quoted. Two outcomes:
>
> - **Unquoted with `EXP_FULL`** — `seplen` stays 1 and `seps` stays
>   `nullstr`, so the separator is a single **NUL byte**. That is what
>   makes unquoted `$*` split into fields; it is not "no separator".
> - **Quoted, or not splitting** — `seplen` is 0, so `seps` becomes
>   `ncifs` and the rescale `seplen = ((seplen - 1) & (ifsmb0len - 1)) + 1`
>   yields `ifsmb0len`: the separator is the **first IFS character**,
>   however many bytes it occupies. An empty `IFS` makes `ifsmb0len` 0
>   and the same expression wraps back to 0, giving no separator at all.
>
> The rescale relies on unsigned wraparound in both directions; a port
> must use wrapping arithmetic, not checked.
>
> A digit selects a positional parameter — 0 being `arg0` — with an
> out-of-range index reported unset. Anything else is an ordinary
> variable via `lookupvar`.

> [spec:dash:def:expand.yylex-fn]
> int yylex(void)

> [spec:dash:sem:expand.yylex-fn]
> Prototype only, declared in `expand.h` for the arithmetic parser's
> benefit; the implementation is in `arith_yylex.c`. See
> `arith-yylex.yylex-fn`.
