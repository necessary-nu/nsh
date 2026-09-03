# Embedding safety

These rules specify properties of `nsh` as a Rust library. They are not a
translation of dash internals and they are not shell-language conformance
requirements. They govern what a safe caller may assume about the host process
while constructing and driving one or more `Shell` values.

## Host process boundaries

> [spec:nsh:req:embedding-safety.process-environment-is-read-only]
> Every safe `nsh` library operation MUST treat the host process environment as
> read-only. `Builder::inherit_env` MAY take an owned snapshot. Setting,
> unsetting, or exporting a shell variable MUST update only that `Shell`'s
> variable table. A child environment MUST be assembled explicitly from the
> shell variable table at the process-execution boundary; it MUST NOT be
> published through the host's `environ` first.

> [spec:nsh:req:embedding-safety.host-children-are-not-reaped]
> A `Shell` MUST wait only for processes it forked itself. It MUST NOT reap a
> child of the host process that it did not create, and a child it did create
> MUST cease to be its concern once its status has been collected. A forked
> shell MUST NOT inherit its parent's claim on the parent's other children.
> Waiting MUST name the process waited for; asking the operating system for
> whichever child is ready does not satisfy this rule, because collecting a
> status destroys it and the host cannot then observe its own child.

> [spec:nsh:req:embedding-safety.process-locale-is-unchanged]
> Every safe `nsh` library operation MUST return with the calling thread's
> previously selected C-library locale restored, on success, on ordinary error,
> and while unwinding. The library MUST NOT call process-global `setlocale` or
> configure a locale by mutating the host process environment.

## Owned shell locale

> [spec:nsh:def:shell-locale.owned-locale]
> A Shell Locale is an owned C-library locale object stored by one `Shell`. Its
> raw handle and the operations which select or free it belong exclusively to
> `nsh-platform`; `nsh` observes it only through safe, locale-explicit
> operations.

> [spec:nsh:req:shell-locale.handle-lifetime]
> A Shell Locale MUST remain alive for every operation that selects or reads it,
> MUST be freed exactly once when its owner is dropped, and MUST NOT permit a
> selection guard to move to another thread. Nested selections MUST restore the
> immediately preceding selection in stack order.

## Selection semantics

> [spec:nsh:sem:shell-locale.selection]
> For each locale category, a `Shell` selects the first non-empty value in this
> order: `LC_ALL`, the category's `LC_*` variable, then `LANG`; when none has a
> non-empty value it selects `C`. An explicitly empty variable and an unset
> variable remain observably distinct in the shell variable table and exported
> child environment, while both fall through to the next source for locale
> selection.

> [spec:nsh:sem:shell-locale.invalid-selection]
> If a locale-variable change names a locale object that the platform cannot
> construct, the variable change remains visible but the `Shell` retains its
> previous effective locale. A new `Shell` whose initial locale selection cannot
> be constructed starts with the `C` locale.

> [spec:nsh:req:shell-locale.operation-binding]
> Every locale-sensitive classification, multibyte conversion, collation,
> operating-system error rendering, and signal-description operation performed
> for a `Shell` MUST explicitly use that Shell's Locale. Core-library code MUST
> NOT reach a free function whose result depends on the calling thread's ambient
> locale.

> [spec:nsh:req:shell-locale.instance-isolation]
> Distinct `Shell` values MUST be able to hold different effective locales.
> Alternating them on one thread or driving them concurrently on different
> threads MUST NOT let either shell change the other's results or the embedding
> host's selected locale.

## Reading without running

> [spec:nsh:req:embedding-safety.reading-without-running]
> A safe caller MUST be able to obtain this shell's own reading of script text
> without executing it, and that reading MUST be the one the shell itself would
> act on -- the same parse, from the same parser, in the caller's chosen
> dialect. It MUST carry the distinctions the shell draws when it runs the
> text, including which parts of a word were quoted and how, so that a caller
> can tell apart two spellings the shell tells apart.
>
> The alternative to offering this is that an embedder composing shell text
> writes a second grammar of its own and gets a different answer than the shell
> does on the text it did not anticipate. A reading that silently loses a
> distinction is worse than no reading, because nothing in this repository
> consumes it and so nothing here would notice.
