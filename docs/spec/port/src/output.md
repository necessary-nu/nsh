# src/output.c, src/output.h

The shell's own buffered output layer, used in preference to stdio so
that buffering, error reporting and interrupt handling are under the
shell's control. A `struct output` is a buffer plus a destination fd:
`buf` is the allocation, `nextc` the write position, `end` the limit,
`bufsize` the capacity, `fd` the target, and `flags` carrying
`OUTPUT_ERR` (01) once a write has failed. Three instances exist:
`output` (fd 1, `OUTBUFSIZ` = `BUFSIZ`), `errout` (fd 2, `bufsize` 0 so
it is effectively unbuffered), and `preverrout`. `out1` and `out2` point
at the first two and are what most code writes through.

`fd == MEM_OUT` (-3) marks an in-memory sink; that support, the `memout`
instance, and the glibc-`FILE`-backed variants are all inside `#ifdef
notyet` / `#ifdef USE_GLIBC_STDIO` and are not compiled in the default
build. Under `USE_GLIBC_STDIO` the struct gains a `stream` member and
most functions delegate to stdio instead.

Errors are recorded in `flags` rather than raised, so a failed write does
not unwind mid-output; callers check later.

**Seven exported items retired.** `output-is-a-writer` made `Output`
an implementation of `std::io::Write` owned by the shell, so the port has
no runtime format-string API for these seven to belong to. `doformat` is
what `write!` does, and `fmtstr`, `out1fmt` and `outfmt` were the three
entry points that reached it — their callers format at the typed call
site now, where the arguments still have types. `xasprintf`,
`xvasprintf` and `xvsnprintf` existed to size and grow a buffer around
`vsnprintf`; the last caller that wanted C formatting at all was the
`printf` builtin, which renders its own conversions. The blocks below
therefore carry no `[spec:dash:…]` ids — each is a C signature followed
by its semantics — and are kept because they still describe
`src/output.c` and `src/output.h`, which still have all seven.

> [spec:dash:def:output.closememout-fn]
> int __closememout(void)

> [spec:dash:sem:output.closememout-fn]
> Close the in-memory output stream: `fclose(memout.stream)`, set
> `memout.stream` to NULL, and return the `fclose` result. Compiled only
> under `USE_GLIBC_STDIO` *and* `notyet`, so it is dead code in every
> shipped configuration; Wave 2 may carry the annotation on an equally
> inactive site.

> void doformat(struct output *dest, const char *f, va_list ap)

> Render a printf-style format straight into `dest`'s buffer when it
> fits, avoiding a copy. Take a stack mark. Point `s` at `dest->nextc`
> and compute `olen`, the space left in the buffer. Call
> `xvasprintf(&s, olen, f, ap)`, which formats in place if the result
> fits in `olen` and otherwise allocates stack space and re-formats
> there, returning the length either way. If `olen > len` the text landed
> in the buffer, so just advance `dest->nextc` by `len`. Otherwise it
> landed on the stack, so push it through `outmem(s, len, dest)`. Pop the
> stack mark.
>
> The comparison is strict (`olen > len`, not `>=`) because
> `xvsnprintf` needs one extra byte for its NUL: a result that exactly
> fills the buffer is treated as not fitting. Under `USE_GLIBC_STDIO`
> this function does not exist and `doformat` is a macro for `vfprintf`.

> [spec:dash:def:output.flushall-fn]
> void flushall(void)

> [spec:dash:sem:output.flushall-fn]
> Flush `output`, and also `errout` when `FLUSHERR` is configured.
> Called before the shell unwinds or exits so buffered text is not lost.

> [spec:dash:def:output.flushout-fn]
> void flushout(struct output *dest)

> [spec:dash:sem:output.flushout-fn]
> Write out whatever is buffered. Compute `len = dest->nextc - dest->buf`
> and return immediately if it is zero or `dest->fd` is negative (a
> memory sink has nothing to flush). Reset `nextc` to `buf` *before*
> writing, so a write that fails or is interrupted does not leave the
> data queued for a second attempt, then `xwrite` the bytes; on failure
> set `OUTPUT_ERR` in `dest->flags`. Under `USE_GLIBC_STDIO` this is
> `fflush(dest->stream)` between `INTOFF`/`INTON`.

> int fmtstr(char *outbuf, size_t length, const char *fmt, ...)

> `snprintf` onto a caller-supplied buffer: collect the variadic
> arguments, call `xvsnprintf(outbuf, length, fmt, ap)`, and return the
> result clamped to `length`. The clamp converts C's "characters that
> *would* have been written" into "characters actually available", so
> callers can use the return value as a length without re-checking for
> truncation. A negative return from `vsnprintf` is passed through
> unclamped.

> [spec:dash:def:output.freestdout-fn]
> static inline void freestdout()

> [spec:dash:sem:output.freestdout-fn]
> Discard anything buffered on `output` without writing it: reset
> `output.nextc` to `output.buf` and clear `output.flags`, dropping any
> recorded `OUTPUT_ERR`. Used after a fork so a child does not re-emit
> output the parent had buffered, and to reset the error state between
> commands.

> [spec:dash:def:output.initstreams-fn]
> void initstreams()

> [spec:dash:sem:output.initstreams-fn]
> Bind the two standard outputs to stdio streams: `output.stream = stdout`
> and `errout.stream = stderr`. Compiled only under `USE_GLIBC_STDIO` and
> `notyet`; the `INIT` block in this file calls it when that
> configuration is active.

> [spec:dash:def:output.openmemout-fn]
> void openmemout(void)

> [spec:dash:sem:output.openmemout-fn]
> Open the in-memory sink with
> `open_memstream(&memout.buf, &memout.bufsize)` between `INTOFF`/`INTON`,
> so writes to `memout` accumulate into a growing buffer. Compiled only
> under `USE_GLIBC_STDIO` and `notyet`.

> void out1fmt(const char *fmt, ...)

> Collect the variadic arguments and `doformat(out1, fmt, ap)` — formatted
> output to standard output.

> [spec:dash:def:output.outc-fn]
> static inline void outc(int ch, struct output *file)

> [spec:dash:sem:output.outc-fn]
> Write one character. The fast path stores `ch` at `file->nextc` and
> advances it; when `nextc` has reached `end` there is no room, so
> delegate to `outcslow`. Under `USE_GLIBC_STDIO` this is `putc` on the
> stream instead.

> [spec:dash:def:output.outcslow-fn]
> void outcslow(int c, struct output *dest)

> [spec:dash:sem:output.outcslow-fn]
> Out-of-line one-character write: place `c` in a one-byte local and call
> `outmem(&buf, 1, dest)`, which handles growing or flushing the buffer.
> Split out of `outc` to keep the inline fast path small.

> void outfmt(struct output *file, const char *fmt, ...)

> Collect the variadic arguments and `doformat(file, fmt, ap)` —
> formatted output to an arbitrary `struct output`.

> [spec:dash:def:output.outmem-fn]
> void outmem(const char *p, size_t len, struct output *dest)

> [spec:dash:sem:output.outmem-fn]
> Append `len` bytes to `dest`, buffering, growing or flushing as needed.
> If at least `len` bytes remain (`dest->end - dest->nextc >= len`),
> `mempcpy` them in, advance `nextc`, and return — the common case.
>
> Otherwise decide how to make room, based on `dest->bufsize`. A
> `bufsize` of 0 means unbuffered: do nothing here and fall through to
> the direct write. A non-zero `bufsize` with no allocation yet
> (`dest->buf == NULL`) means allocate: with interrupts suspended
> `ckrealloc` to `bufsize`, and reset `buf`, `bufsize`, `end` and
> `nextc` (at offset 0). Anything else means the buffer exists but is
> full: `flushout(dest)`.
>
> Re-measure the free space; if it is now *greater* than `len` — strictly
> greater, matching the original's conservative test — buffer the bytes
> as above. Otherwise write them straight to `dest->fd` with `xwrite`,
> setting `OUTPUT_ERR` in `dest->flags` on failure. Under
> `USE_GLIBC_STDIO` the whole body is `fwrite` between `INTOFF`/`INTON`.

> [spec:dash:def:output.output]
> struct output {
>   char *nextc;
>   char *end;
>   char *buf;
>   size_t bufsize;
>   int fd;
>   int flags;
> }

> [spec:dash:def:output.outstr-fn]
> void outstr(const char *p, struct output *file)

> [spec:dash:sem:output.outstr-fn]
> `outmem(p, strlen(p), file)` — write a NUL-terminated string without
> its terminator. Under `USE_GLIBC_STDIO`, `fputs` between
> `INTOFF`/`INTON`.

> int xasprintf(char **sp, const char *f, ...)

> Format onto freshly allocated stack space: collect the variadic
> arguments and call `xvasprintf(sp, 0, f, ap)`. Passing size 0 forces
> the allocating path, so `*sp` always receives stack-allocated storage.
> Returns the formatted length.

> static int xvasprintf(char **sp, size_t size, const char *f, va_list ap)

> Format into `*sp` if the result fits in `size` bytes, otherwise onto
> the shell stack, updating `*sp` to point at it. First attempt through a
> `va_copy` so `ap` survives for a second pass:
> `xvsnprintf(*sp, size, f, ap2)`. A negative result raises
> `sh_error("xvsnprintf failed")`. If `len < size` the text fits — return
> `len` with `*sp` unchanged.
>
> Otherwise allocate `stalloc` space of `max(len, stackblocksize()) + 1`
> bytes — taking at least the current scratch-block size so a subsequent
> stack string is not forced to grow immediately — point `*sp` at it, and
> re-format with the original `ap`. Return the length.
>
> The `len < size` comparison promotes the `int` `len` to `size_t`. That
> is load-bearing rather than incidental: it is exactly why `size == 0`
> — the `xasprintf` path — can never take the "it fits" branch, and so
> why `xasprintf` always returns freshly allocated stack storage. A port
> must preserve that, not "clean up" the mixed comparison.

> static int xvsnprintf(char *outbuf, size_t length, const char *fmt, va_list ap)

> `vsnprintf` with interrupts suspended, so a signal cannot arrive
> partway through formatting. On Solaris, a `length` of 0 is first
> redirected to a one-byte dummy buffer, because older `vsnprintf` there
> returns -1 rather than the needed length when given 0. Returns
> `vsnprintf`'s value: the number of characters the full result would
> need, excluding the NUL.

> [spec:dash:def:output.xwrite-fn]
> int xwrite(int fd, const void *p, size_t n)

> [spec:dash:sem:output.xwrite-fn]
> Write all `n` bytes to `fd`, restarting on interruption and handling
> short writes. Loop while bytes remain: clamp the chunk to `SSIZE_MAX`,
> retry `write` while it fails with `EINTR`, return -1 on any other
> error, and otherwise advance the pointer and decrement the count by
> what was actually written. Return 0 once everything is out.
