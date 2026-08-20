# src/parser.c, src/parser.h

The command parser: a hand-written recursive-descent grammar over a
tokeniser that is deeply entangled with the shell's quoting rules.

**Token stream.** `readtoken` returns a token code, with a one-token
pushback in `tokpushback`/`lasttoken`. A `TWORD` leaves its text in
`wordtext`, whether any part was quoted in `quoteflag`, and the command
substitutions it contains in `backquotelist`. A `TREDIR` leaves the
redirection node in `redirnode`.

**`checkkwd`** tells `readtoken` what the *current* grammar position
allows, and must be set before each call: `CHKALIAS` (0x1, expand
aliases), `CHKKWD` (0x2, recognise reserved words), `CHKNL` (0x4, skip
newlines), `CHKEOFMARK` (0x8, we are reading a here-document delimiter,
so produce literal text rather than the internal encoding). It is
cleared by every `readtoken` call.

**Output encoding.** A word is not stored as source text but in the
shell's internal form, where markers from `syntax.h` — `CTLESC`,
`CTLVAR`, `CTLENDVAR`, `CTLBACKQ`, `CTLARI`, `CTLENDARI`,
`CTLQUOTEMARK`, `CTLMBCHAR` — record what expansion must do later. This
is what lets `expand.c` know that a `*` came from `$x` (not a glob) or
was written literally.

**Quoting state.** `struct synstack` is a stack of parse contexts, one
per nesting level of `${…}`, `$((…))` or `` ` ``. `syntax` points at the
active syntax table (`BASESYNTAX`, `DQSYNTAX`, `SQSYNTAX`, `ARISYNTAX`),
which classifies each byte; the other fields count nesting
(`varnest`, `parenlevel`, `dqvarnest`) and record quoting
(`dblquote`, `innerdq`, `backq`, `varpushed`). The base level is a
local, and pushed levels are `alloca`'d — and *reused* via the `prev`
link, since `alloca` inside a loop cannot free.

`struct heredoc` queues a here-document whose body has not yet been read:
the parser records the delimiter at the `<<` and reads the body at the
next newline. `heredoclist` is that queue.

`FAKEEOFMARK` (the pointer value 1) is a sentinel meaning "parse like a
here document, but there is no real delimiter" — used by `expandstr`.
`realeofmark` distinguishes it.

**Dash source shape (`parser.andor-fn`):**

    node * andor(void)

> [spec:dash:sem:parser.andor-fn]
> Parse `pipeline (&& | || pipeline)*`. Parse a pipeline, then loop: a
> `TAND` or `TOR` becomes an `NAND`/`NOR` node with the accumulated left
> side and a freshly parsed right side; anything else is pushed back and
> the accumulation returned. Before each right operand set
> `checkkwd = CHKNL | CHKKWD | CHKALIAS`, so a newline may follow the
> operator. Left-associative, since each new node takes the previous
> result as its left child.

**Dash source shape (`parser.command-fn`):**

    node * command(void)

> [spec:dash:sem:parser.command-fn]
> Parse one command — a compound command, a function definition, or a
> simple command — plus any redirections that follow it. Record `plinno`
> as `savelinno` on entry so the node carries the line where the command
> *started*.
>
> Dispatch on the first token:
>
> - `TIF` — `if list; then list [elif …] [else list]; fi`, building a
>   chain of `NIF` nodes where each `elif` becomes the `elsepart` of the
>   previous. A missing `then` is `synexpect(TTHEN)`. Terminator `TFI`.
> - `TWHILE`/`TUNTIL` — `NWHILE`/`NUNTIL` with the condition and body as
>   the two children. A missing `do` is `synexpect(TDO)`. Terminator
>   `TDONE`.
> - `TFOR` — the variable must be an unquoted `TWORD` that is a valid
>   name, else `synerror("Bad for loop variable")`. With `in`, collect
>   the word list, requiring a newline or `;` after it. Without `in`, the
>   list defaults to `dolatstr` — the pre-encoded `"$@"` — and a
>   following `;` is optional (the original Bourne shell required a
>   newline). Then `do … done`.
> - `TCASE` — the subject word, `in`, then clauses until `esac`. Each
>   clause may open with an optional `(`, has `|`-separated patterns, a
>   required `)`, and a body parsed with `list(2)` so it stops at the
>   clause terminator. A `;;` (`TENDCASE`) starts the next clause; `esac`
>   ends. Jumps straight to the redirection step, since `case` has no
>   separate terminator token to consume.
> - `TLP` — `( list )` as an `NSUBSHELL`; terminator `TRP`.
> - `TBEGIN` — `{ list }`; terminator `TEND`. Note no node is created:
>   grouping is purely syntactic, so `{ …; }` produces its list directly.
> - `TWORD`/`TREDIR` — push back and delegate to `simplecmd`, which also
>   handles function definitions.
> - anything else — `synexpect(-1)`.
>
> Consume the terminator token, then read trailing redirections: with
> `checkkwd = CHKKWD | CHKALIAS`, collect `TREDIR` tokens, calling
> `parsefname` for each. If any were found, wrap the command in an
> `NREDIR` node — except for `NSUBSHELL`, which already has a `redirect`
> field of its own and is used directly.

**Dash source shape (`parser.dollarsq-escape-fn`):**

    static char *dollarsq_escape(char *out)

> [spec:dash:sem:parser.dollarsq-escape-fn]
> Decode one escape inside a `$'…'` string. Read up to 9 characters,
> stopping early at a `'` or end of input, into a local buffer — a
> lookahead window large enough for the longest form, `\UXXXXXXXX`.
>
> Unless the first character is `c`, hand the window to
> `conv_escape(p, out, true)`, which decodes the escape and reports how
> much of the input it consumed and how much it produced.
>
> A leading `c` is the control-character form `\cX`: take the next
> character, and let `p += !((c ^ *p) | (c ^ '\\'))` skip a second
> backslash for `\c\\`. The control value is
> `(c & ~((c & 0x40) >> 1) & 0x7f) ^ 0x40` — this upper-cases a lowercase
> letter by clearing bit 5 when bit 6 is set, masks to 7 bits, and then
> flips bit 6, giving `\ca` → 0x01 and `\c[` → 0x1B.
>
> Finally push back the characters of the window that were not consumed,
> and return the updated output pointer.

**Dash source shape (`parser.endofname-fn`):**

    char * endofname(const char *name)

> [spec:dash:sem:parser.endofname-fn]
> Return a pointer to the first character of `name` that cannot be part
> of a variable name. If the *first* character is not a valid name start
> (`is_name`: letter or underscore) return `name` itself, so the caller
> sees an empty name. Otherwise advance while `is_in_name` (letter,
> underscore or digit) holds.

**Dash source shape (`parser.expandstr-fn`):**

    const char * expandstr(const char *ps)

> [spec:dash:sem:parser.expandstr-fn]
> Parse and expand a string as if it were a double-quoted word — used for
> `PS1`/`PS2`/`PS4` and for the startup-file names. Returns the expanded
> text, or the original string unchanged if anything went wrong.
>
> Save the input stack position, make `ps` the input, and stash and clear
> `heredoclist`, `doprompt` and `needprompt` — a prompt must not itself
> prompt, or queue here documents into the real queue. Install a local
> exception handler so a bad prompt cannot kill the shell.
>
> Then `readtoken1(pgetc_eatbnl(), DQSYNTAX, FAKEEOFMARK, 0)`: double-quote
> syntax, with `FAKEEOFMARK` so the here-document code path is used
> (which suppresses the `CTLQUOTEMARK` markers) without a real
> delimiter to look for. Build a temporary `NARG` node from the result
> and `expandarg(&n, NULL, EXP_QUOTED)`, taking the result from
> `stackblock()`.
>
> On the way out — including the error path — restore the handler with
> `restore_handler_expandarg`, restore `doprompt`, unwind the input stack
> and restore `heredoclist`.

**Dash source shape (`parser.findkwd-fn`):**

    const char *const * findkwd(const char *s)

> [spec:dash:sem:parser.findkwd-fn]
> Binary-search the sorted `parsekwd` table for the reserved word `s`,
> returning a pointer to the matching slot or NULL. The caller converts
> the slot address into a token code via `pp - parsekwd + KWDOFFSET`.

**Dash source shape (`parser.fixredir-fn`):**

    void fixredir(union node *n, const char *text, int err)

> [spec:dash:sem:parser.fixredir-fn]
> Resolve the operand of a `>&`/`<&` redirection. `err` is set when the
> operand has already been expanded and must therefore be a valid
> descriptor now; when clear, an unresolved operand is kept for later
> expansion and `ndup.vname` is first cleared.
>
> A single digit sets `ndup.dupfd` to its value. A lone `-` sets it to
> -1, the "close this descriptor" marker. Anything else either raises
> `sh_error("Bad fd number: %s", text)` when `err` is set, or is stored
> as `ndup.vname = makename()` so `expredir` can expand it at execution
> time and call back here with `err` set.

**Dash source shape (`parser.getmbc-fn`):**

    unsigned getmbc(int c, char *out, int mode)

> [spec:dash:sem:parser.getmbc-fn]
> Try to read a complete multibyte character starting at `c`, writing it
> to `out`. Returns the number of bytes written, 0 if `c` does not start
> a valid multibyte sequence, or 1 for the special blank case below.
>
> Return 0 immediately for a byte that cannot start one: a non-negative
> `c` (plain ASCII, since bytes are returned sign-extended) or `PEOF` and
> below.
>
> Feed bytes to `mbrtowc` one at a time, reading more input while it
> reports `-2` (incomplete), stopping at `MB_LEN_MAX` or at `PEOA`/`PEOF`.
> Reading uses `pgetc_eoa`, so an alias boundary terminates the sequence
> rather than being crossed.
>
> On success (`mbrtowc` returned 1 and more than one byte was consumed):
> mode 4 asks whether the character is a blank for field-splitting
> purposes, and returns 1 for one without writing anything. Modes 0 and 1
> wrap the bytes in the parser's framing — `CTLMBCHAR`, optionally
> `CTLESC` (mode 1, inside a backslash escape), the length, the bytes,
> the length again, `CTLMBCHAR` — so the sequence can be scanned in
> either direction and its bytes are never mistaken for markers. The
> framing test is `(mode & 3) < 2`, so the raw-bytes-only path is taken
> by modes 2 and 3 — and equally by 6 and 7, which the predicate also
> selects even though no call site reaches them (mode 6 would need
> `fieldsplitting` and `printesc` at once, which the syntax-table
> conditions make mutually exclusive). `mbc` is positioned accordingly so
> the bytes land in the right place either way.
>
> On failure, push back everything but the first byte with
> `pungetn(ml - 1)` and return 0, leaving the caller to treat `c` as a
> single byte.

**Dash source shape (`parser.getprompt-fn`):**

    const char * getprompt(void *unused)

> [spec:dash:sem:parser.getprompt-fn]
> Return the current prompt text, expanded. Select by `whichprompt`: 0
> gives `nullstr`, 1 gives `PS1`, 2 gives `PS2`; any other value returns
> `"<internal prompt error>"` under `DEBUG` and otherwise falls into the
> 0 case. The selected value is passed through `expandstr`. Called by
> libedit as well as by `setprompt`, hence the unused `void *`.

**Dash source shape (`parser.goodname-fn`):**

    static inline int goodname(const char *p)

> [spec:dash:sem:parser.goodname-fn]
> Return whether `p` is entirely a valid variable name:
> `!*endofname(p)` — the scan consumed everything. An empty string
> returns 0, since `endofname` stops immediately at the NUL.

**Dash source shape (`parser.heredoc`):**

    struct heredoc {
      struct heredoc *next;
      union node *here;
      char *eofmark;
      int striptabs;
    }

**Dash source shape (`parser.isassignment-fn`):**

    int isassignment(const char *p)

> [spec:dash:sem:parser.isassignment-fn]
> Return whether `p` has the form `name=…`: `endofname` must have
> consumed at least one character and must have stopped at an `=`.

**Dash source shape (`parser.issimplecmd-fn`):**

    int issimplecmd(union node *n, const char *name)

> [spec:dash:sem:parser.issimplecmd-fn]
> Return whether `n` is a simple command whose first word is exactly
> `name`. Used to recognise the subshell special cases — a subshell that
> is nothing but a `trap` or a `jobs` command needs to inherit state that
> a subshell normally discards.

**Dash source shape (`parser.list-fn`):**

    node * list(int nlflag)

> [spec:dash:sem:parser.list-fn]
> Parse a list of and-or expressions separated by `;`, `&` or newline,
> returning a left-leaning chain of `NSEMI` nodes (or the single command
> when there is only one). `nlflag` encodes two things: bit 0 clear means
> newlines may be skipped (`chknl` becomes `CHKNL`), and bit 1 is set
> once at least one command has been parsed, after which a token in
> `tokendlist` — a closing keyword like `done`, `fi`, `esac` — ends the
> list. `list(2)` therefore parses a body that stops at its terminator
> without consuming it, which is what `case` clauses and command
> substitution need.
>
> Each iteration sets `checkkwd = chknl | CHKKWD | CHKALIAS` and reads a
> token. `TNL` reads any queued here documents and ends the list. `TEOF`
> does the same, and additionally yields `NEOF` when nothing was parsed
> and newlines were not being skipped — that is how end of input is
> distinguished from a blank line. Both push `TEOF` back so the caller
> sees it again.
>
> Otherwise push the token back and parse an and-or. A following
> `TBACKGND` (`&`) marks it background: an `NPIPE` gets its `backgnd`
> flag set, and anything else is retyped `NBACKGND`, first being wrapped
> in an `NREDIR`-shaped node if it is not already one, since `NBACKGND`
> uses that layout. Chain the result onto the accumulation with `NSEMI`.
>
> Then the separator decides: `TEOF` ends as above; `TNL` is pushed back
> and the loop continues (so the next iteration handles the here
> documents); `;` and `&` continue directly; anything else is a syntax
> error when newlines are not being skipped, and otherwise is pushed back
> and ends the list.

**Dash source shape (`parser.makename-fn`):**

    node * makename(void)

> [spec:dash:sem:parser.makename-fn]
> Build an `NARG` node from the token just read: `wordtext` as the text,
> `backquotelist` as its command substitutions, `next` NULL.

**Dash source shape (`parser.nlnoprompt-fn`):**

    static void nlnoprompt(void)

> [spec:dash:sem:parser.nlnoprompt-fn]
> Account for a newline that ends a command: increment `plinno` and set
> `needprompt` to `doprompt`, deferring the prompt until the parser
> actually needs more input rather than issuing it now.

**Dash source shape (`parser.nlprompt-fn`):**

    static void nlprompt(void)

> [spec:dash:sem:parser.nlprompt-fn]
> Account for a newline in the middle of a construct: increment `plinno`
> and, when prompting, issue the `PS2` continuation prompt immediately —
> the parser is about to read more input as part of the same command.

**Dash source shape (`parser.parsecmd-fn`):**

    union node * parsecmd(int interact)

> [spec:dash:sem:parser.parsecmd-fn]
> Parse one complete command. Reset the parser: clear the pushback,
> `checkkwd` and the here-document queue, set `doprompt` from `interact`,
> issue the `PS1` prompt when interactive, and clear `needprompt`. Then
> `list(1)` — bit 0 set, so newlines are not skipped and a leading
> end-of-input yields `NEOF`. Returns `NEOF` at end of input; NULL is a
> valid result meaning a blank line.

**Dash source shape (`parser.parsefname-fn`):**

    STATIC void parsefname(void)

> [spec:dash:sem:parser.parsefname-fn]
> Read the operand of the redirection in `redirnode`. For a here document
> set `CHKEOFMARK` first, so the delimiter is tokenised literally rather
> than into the internal encoding, and clear it after. The operand must
> be a `TWORD`.
>
> For a here document: an unquoted delimiter means the body is subject to
> expansion, so retype the node `NXHERE`. Strip the quoting from the
> delimiter with `rmescapes`, store it in the queued `struct heredoc`,
> and append that to the end of `heredoclist` — order matters, since
> several here documents on one line are read in the order written.
>
> For `>&`/`<&`, hand the text to `fixredir` with `err` 0. Otherwise
> store the filename node from `makename`.

**Dash source shape (`parser.parseheredoc-fn`):**

    STATIC void parseheredoc(void)

> [spec:dash:sem:parser.parseheredoc-fn]
> Read the bodies of all queued here documents. Detach `heredoclist`
> first, so a here document appearing inside one of these bodies queues
> separately.
>
> For each: issue the continuation prompt if one is pending, then read
> the body with `readtoken1`. A quoted delimiter (`NHERE`) uses
> `SQSYNTAX` and plain `pgetc`, so nothing is expanded and backslashes
> are literal; an unquoted one (`NXHERE`) uses `DQSYNTAX` and
> `pgetc_eatbnl`, so expansions and line continuations apply. The
> delimiter and `striptabs` are passed through to the `checkend` logic
> inside `readtoken1`. Wrap the resulting text in an `NARG` node and
> attach it to the redirection node.

**Dash source shape (`parser.parser-eof-fn`):**

    static inline int parser_eof(void)

> [spec:dash:sem:parser.parser-eof-fn]
> Return whether the parser has hit end of input: `tokpushback &&
> lasttoken == TEOF`. `evalstring` uses this to decide whether the
> command it is about to run is genuinely the last one and may therefore
> `exec` in place.

**Dash source shape (`parser.pgetc-eatbnl-fn`):**

    static int pgetc_eatbnl(void)

> [spec:dash:sem:parser.pgetc-eatbnl-fn]
> `pgetc` with backslash-newline line continuations removed: while the
> character read is `\`, look at the next one — if it is a newline,
> account for it with `nlprompt` and read again; otherwise push it back
> and return the backslash. Used everywhere except inside single quotes,
> where a backslash is literal.

**Dash source shape (`parser.pgetc-top-fn`):**

    static int pgetc_top(struct synstack *stack)

> [spec:dash:sem:parser.pgetc-top-fn]
> Read the next character honouring the current quoting context: plain
> `pgetc` under `SQSYNTAX`, where backslash-newline is not a
> continuation, and `pgetc_eatbnl` otherwise.

**Dash source shape (`parser.pipeline-fn`):**

    node * pipeline(void)

> [spec:dash:sem:parser.pipeline-fn]
> Parse `[!] command (| command)*`. A leading `TNOT` sets `negate`
> (toggling it, though only one `!` is reachable) and sets
> `checkkwd = CHKKWD | CHKALIAS`. Parse a command; if a `TPIPE` follows,
> build an `NPIPE` node with a `nodelist` chain, parsing each subsequent
> command with `checkkwd = CHKNL | CHKKWD | CHKALIAS` so a newline may
> follow the `|`. Push back the token that ended the pipeline. Wrap the
> result in an `NNOT` node when negated.

**Dash source shape (`parser.readtoken-fn`):**

    STATIC int readtoken(void)

> [spec:dash:sem:parser.readtoken-fn]
> Read one token, applying the `checkkwd` policy captured on entry.
> `checkkwd` is cleared as a side effect, so it must be set before every
> call.
>
> Get a raw token from `xxreadtoken`. Under `CHKNL`, skip newline tokens,
> reading any here documents queued by each — this is where a here
> document body is actually consumed. `checkkwd` is re-read after the
> skip and OR-ed in, because `xxreadtoken` may have set it (an alias
> ending in a space sets `CHKALIAS`).
>
> A non-word, or a word any part of which was quoted, is returned as is —
> quoting suppresses both keyword recognition and alias expansion.
>
> Under `CHKKWD`, look the word up in the reserved words and, on a hit,
> convert it to its token code (`pp - parsekwd + KWDOFFSET`).
>
> Under `CHKALIAS`, look it up in the aliases with the in-use check, and
> on a hit push the replacement text back into the input with
> `pushstring` and restart from the top — so the alias body is
> re-tokenised. An alias whose value is empty is consumed without
> pushing, which still suppresses the word.

**Dash source shape (`parser.readtoken1-fn`):**

    STATIC int readtoken1(int firstc, char const *syntax, char *eofmark, int striptabs)

> [spec:dash:sem:parser.readtoken1-fn]
> Read a word, a redirection operator, or a here-document body, building
> the internal encoded form on the stack. `firstc` is the first character,
> `syntax` the initial syntax table. A non-NULL `eofmark` means "read a
> here document terminated by this delimiter", with `striptabs` requesting
> that leading tabs be stripped from each line; `FAKEEOFMARK` means
> here-document *style* with no delimiter. Returns `TWORD`, `TREDIR` or
> `TBLANK`.
>
> The body is written as one function with `goto`-linked internal
> "subroutines" (`CHECKEND`, `PARSEREDIR`, `PARSESUB`, `PARSEBACKQOLD`,
> `PARSEBACKQNEW`, `PARSEARITH`) because C has no nested functions; each
> jumps to a labelled block at the end and back to a return label.
>
> **Outer loop**, once per line: `CHECKEND` tests for the here-document
> delimiter. **Inner loop**, once per character, reading via
> `pgetc_top`: reserve worst-case space, then compute `fieldsplitting` —
> non-zero only at the base syntax level outside any variable nesting or
> backquotes, i.e. where an unquoted blank would separate words. Offer
> the character to `getmbc`; a return of 1 means a multibyte blank, which
> ends the word (or yields `TBLANK` if the word is empty so far); a
> return above 1 means the character was consumed as multibyte.
>
> Otherwise dispatch on the syntax class of the byte:
>
> - `CNL` (newline) — ends the word where field splitting applies;
>   otherwise it is part of the word, and the outer loop restarts with a
>   continuation prompt.
> - `CWORD` — an ordinary byte, emitted as is.
> - `CCTL` — a byte that collides with an internal marker. Inside `$'…'`
>   it is the escape introducer and goes to `dollarsq_escape`. Otherwise
>   it is emitted preceded by `CTLESC`, except in an unquoted here
>   document where markers cannot arise.
> - `CBACK` (backslash) — read the next character. At end of input, emit
>   an escaped literal backslash. Inside double quotes or backquotes, a
>   backslash before anything other than `` \ ` $ `` (and `"`/`}` in the
>   relevant nesting contexts) is itself literal and is emitted escaped.
>   Set `quotef`. Then emit the escaped character, via `getmbc` mode 1
>   for a multibyte one.
> - `CSQUOTE` / `CDQUOTE` — switch to single- or double-quote syntax and
>   emit `CTLQUOTEMARK` (suppressed in a here document). Entering double
>   quotes inside a variable expansion toggles `innerdq`.
> - `CENDQUOTE` — leave quoting, but only at `dqvarnest` 0; finish any
>   `$'…'` accumulation by re-deriving the output pointer from the
>   string's length (the escapes may have written NULs); set `quotef`.
> - `CVAR` (`$`) — `PARSESUB`.
> - `CENDVAR` (`}`) — close a variable expansion when one is open and we
>   are not inside a nested double quote: decrement `varnest`, pop the
>   syntax stack if this level pushed one, decrement `dqvarnest`, and
>   emit `CTLENDVAR` instead of the brace.
> - `CLP`/`CRP` — parenthesis nesting inside arithmetic; a `)` at nesting
>   0 followed by another `)` closes the arithmetic expansion, popping the
>   stack and emitting `CTLENDARI`. A single `)` at nesting 0 is left
>   alone rather than diagnosed.
> - `CBQUOTE` (`` ` ``) — `PARSEBACKQOLD`, or ends the word when already
>   inside an old-style backquote at the outer level.
> - `CEOF` — ends the word.
> - default — a `)` closes a new-style `$( … )`; otherwise ends the word
>   where field splitting applies, and is emitted literally elsewhere.
>
> **At end of word**: unterminated arithmetic is `synerror("Missing
> '))'")`, an unterminated quote or backquote is `"Unterminated quoted
> string"`, and an unclosed `${` is `"Missing '}'"`. NUL-terminate.
>
> Then, for a real word (not a here document), check whether it is
> actually a redirection operator: the terminating character is `>` or
> `<`, nothing was quoted, and the accumulated text is at most two
> characters and is empty or a digit — i.e. an optional descriptor
> number. If so, `PARSEREDIR` and return `TREDIR`; otherwise push the
> terminator back. Publish `quoteflag`, `backquotelist` and `wordtext`,
> claim the stack space, and return `TWORD`.
>
> **`checkend`** (here documents only): optionally strip leading tabs,
> then match the delimiter character by character against the input,
> writing each to the output as it goes so nothing is lost. A full match
> followed by newline or end of input sets `c = PEOF`, ending the
> document. A partial match rewinds: the characters already consumed are
> pushed back as a string via `pushstring` so they are re-read as
> document text, and the output pointer is restored to the saved mark.
>
> **`parseredir`**: build an `nfile` node. For `>`: `>>` is `NAPPEND`,
> `>|` is `NCLOBBER`, `>&` is `NTOFD`, otherwise `NTO`; default
> descriptor 1. For `<`: `<<` is `NHERE` (allocating a `struct heredoc`
> and honouring a following `-` as `striptabs`), `<&` is `NFROMFD`, `<>`
> is `NFROMTO`, otherwise `NFROM`; default descriptor 0. An explicit
> leading digit overrides the default descriptor.
>
> **`parsesub`**: having read the `$`, decide what follows. `$(` is
> either arithmetic (a second `(` → `PARSEARITH`) or command
> substitution (`PARSEBACKQNEW`). `$'` starts a dollar-single-quoted
> string where the base syntax allows it. A `{`, a name character or a
> special parameter starts a parameter expansion: parse the name
> (letters/digits/underscore, or a run of digits for `${10}`, or one
> special character), then the operator — `:` sets `VSNUL`, and the
> operator itself is one of `} - + ? =` (indexed out of the `types`
> string), or `#`/`##`/`%`/`%%` for the trimming forms, or `${#name}`
> for `VSLENGTH`. The encoded form written is `CTLVAR`, the subtype byte
> (with `VSBIT`), the name, `=`, and later `CTLENDVAR`; under
> `CHKEOFMARK` the literal source characters are written instead. A
> non-`VSNORMAL` expansion pushes a syntax level and increments
> `varnest`, and the trimming forms switch to `BASESYNTAX` because their
> operand is a pattern rather than a string. Anything else pushes the
> character back, leaving a literal `$`.
>
> **`parsebackq`**: shared by both substitution syntaxes, selected by
> `oldstyle`. Under `CHKEOFMARK` it merely pushes a syntax level marked
> `backq` and returns, since a here-document delimiter is not parsed.
> Otherwise emit `CTLBACKQ`, save the word built so far off the stack,
> and start a fresh stack string. For the old style, read raw up to the
> closing `` ` ``, keeping backslash escapes except before `` \ ` $ ``
> and — inside double quotes — `"`, then push the collected text back as
> input so it can be parsed normally; end of input there is
> `synerror("EOF in backquote substitution")`. Then append a `nodelist`
> entry, save and clear `heredoclist` (a here document inside a
> substitution belongs to the substitution), parse with `list(2)`,
> require `)` for the new style, read the substitution's own here
> documents, restore the outer queue, pop the input, and restore the
> saved word text.
>
> **`parsearith`**: push an `ARISYNTAX` level with `dblquote` set and
> replace the `$((` already emitted with a single `CTLARI` marker (or
> keep it literal under `CHKEOFMARK`).

**Dash source shape (`parser.realeofmark-fn`):**

    static inline int realeofmark(const char *eofmark)

> [spec:dash:sem:parser.realeofmark-fn]
> Return whether `eofmark` is a genuine here-document delimiter — non-NULL
> and not the `FAKEEOFMARK` sentinel — and therefore whether `checkend`
> should actually look for it.

**Dash source shape (`parser.setprompt-fn`):**

    static void __attribute__((noinline)) setprompt(int which)

> [spec:dash:sem:parser.setprompt-fn]
> Issue prompt number `which` (1 for `PS1`, 2 for `PS2`). Clear
> `needprompt` and record `whichprompt`. Print it only when line editing
> is not in use — libedit prints its own prompt by calling `getprompt` —
> and only when the input buffer is empty (`!parsefile->nleft`), so a
> prompt is not emitted in the middle of a line already read. Write
> `getprompt(NULL)` to `out2` inside a stack mark, since prompt expansion
> allocates.

**Dash source shape (`parser.simplecmd-fn`):**

    node * simplecmd(void)

> [spec:dash:sem:parser.simplecmd-fn]
> Parse a simple command: leading assignments, arguments and
> redirections, in any interleaving; or a function definition. Record
> `plinno` on entry as the node's line.
>
> `savecheckkwd` starts as `CHKALIAS` and is cleared once the first
> non-assignment word is seen — so alias expansion applies to the command
> word and to the assignments that precede it, but not to arguments.
>
> Loop reading tokens: a `TWORD` becomes an `NARG`, appended to the
> assignment list when `savecheckkwd` is still set and the word looks
> like `name=…`, and to the argument list otherwise. A `TREDIR` is
> appended to the redirection list, with `parsefname` reading its
> operand. Any other token is pushed back and ends the command.
>
> A `TLP` is a function definition, but only when exactly one word has
> been seen and there are no assignments or redirections. It requires a
> closing `)`; the name must be a valid identifier and must not be a
> special builtin, else `synerror("Bad function name")`. The node is
> retyped `NDEFUN` and its body parsed with `command()` after setting
> `checkkwd = CHKNL | CHKKWD | CHKALIAS`. Any other `TLP` falls through
> and ends the command.
>
> Terminate the three lists and build an `NCMD` node.

**Dash source shape (`parser.synerror-fn`):**

    STATIC void synerror(const char *msg)

> [spec:dash:sem:parser.synerror-fn]
> Set `errlinno` to the current parse line and raise
> `sh_error("Syntax error: %s", msg)`. Does not return.

**Dash source shape (`parser.synexpect-fn`):**

    STATIC void synexpect(int token)

> [spec:dash:sem:parser.synexpect-fn]
> Report an unexpected token. Format into a 64-byte buffer either
> `"<got> unexpected (expecting <want>)"` when `token` is a specific
> expected token, or `"<got> unexpected"` when `token` is -1 meaning
> several would have been valid, using `tokname[]` for both. Then
> `synerror`. Does not return.

**Dash source shape (`parser.synstack`):**

    struct synstack {
      const char *syntax;
      struct synstack *prev;
      struct synstack *next;
      int innerdq;
      int varpushed;
      int dblquote;
      int backq;
      int varnest;
      int parenlevel;
      int dqvarnest;
    }

**Dash source shape (`parser.synstack-pop-fn`):**

    static void synstack_pop(struct synstack **stack)

> [spec:dash:sem:parser.synstack-pop-fn]
> Make the enclosing level current: `*stack = (*stack)->next`. The popped
> level is *not* discarded — it stays reachable through the new top's
> `prev` link so `synstack_push` can reuse its storage, which is what
> makes the `alloca`-based allocation safe inside a loop.

**Dash source shape (`parser.synstack-push-fn`):**

    static void synstack_push(struct synstack **stack, struct synstack *next, const char *syntax)

> [spec:dash:sem:parser.synstack-push-fn]
> Push a new parse context. Zero the supplied storage, set its syntax
> table, link it in front of the current top (setting the old top's
> `prev` to it), and make it current. The caller passes either the
> previously popped level (via `prev`) or fresh `alloca` space.

**Dash source shape (`parser.xxreadtoken-fn`):**

    STATIC int xxreadtoken(void)

> [spec:dash:sem:parser.xxreadtoken-fn]
> Read one raw token, before keyword and alias processing. A pending
> pushback is returned immediately. Issue a continuation prompt if one is
> pending.
>
> Then loop until a token is found: skip spaces and tabs; a `#` starts a
> comment, consumed to just before the newline (which is pushed back so
> it becomes a `TNL`). A newline is `TNL` via `nlnoprompt`; end of input
> is `TEOF`. The operators are longest-match by lookahead: `&&`/`&` give
> `TAND`/`TBACKGND`, `||`/`|` give `TOR`/`TPIPE`, `;;`/`;` give
> `TENDCASE`/`TSEMI`, and `(`/`)` give `TLP`/`TRP`. Reading uses
> `pgetc_eatbnl`, so a line continuation may appear anywhere.
>
> Anything else starts a word: delegate to
> `readtoken1(c, BASESYNTAX, NULL, 0)` and return its result, unless that
> is `TBLANK` — a word that turned out to be only a multibyte blank — in
> which case keep looping.
