---
id [dec:nsh:no-format-interpreters]
epitome "nsh interprets no format strings at runtime. While this stood, that was read to include the printf builtin, and the shell had none."
state @obsolesced
category @ban
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:conformance])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep the printf builtin and render its conversions with libc, as the C does -- `snprintf` per conversion behind the `PF`/`ASPF` arity switch."
        rejected_because "It is the arrangement the port was trying to leave. It needs a C format string built at runtime, so it needs varargs, so it needs the arity switch -- and the port's version of it shipped two defects that fell straight out of that shape: no INTOFF/INTON around the call, and `%d`/`%i` handing a `uintmax_t` to a `%ld` conversion, which is varargs undefined behaviour that happened to work on LP64."
    }
    {
        option "Keep the printf builtin and render its conversions with Rust's `format!`, parsing the `%` specification into a typed descriptor first."
        rejected_because "This was built and measured: every conversion but `%a`/`%A` came out byte-identical to glibc across 12264 differential cases. It was still rejected, because what it removed was libc, not the thing being objected to. A `%` specification parsed at runtime and dispatched to a renderer is a format-string interpreter however the digits are produced, and having one in the crate is what this decision bans."
    }
    {
        option "Keep the builtin for the conversions Rust can spell, and reject `%a`/`%A`."
        rejected_because "A utility that implements most of its contract is worse than one that implements none of it: scripts would get a shell-specific printf that silently differs from the one in PATH, and the differential harness would be comparing nsh's partial printf against dash's complete one forever."
    }
)
consequences {
    accepted (
        "While this stood, `printf` resolved through PATH like any other external utility, and `type printf` said so. [dec:nsh:printf-is-parsed-not-interpreted] returns it to the builtin table."
        "The builtin table is no longer `mkbuiltins`' output byte for byte. `src/builtins.def.in` lists printf and the generator emits it; `crates/nsh/src/builtins.rs` deliberately does not, and says why."
        "Six differential corpora are retired: they measured the printf utility, and nsh no longer contains it. Comparing nsh-plus-coreutils against dash's builtin measures coreutils against dash, not the shell against the shell."
        "`echo` is unaffected and keeps every escape it had. It shared `print_escape_str` with printf, but only ever passed it `%s`, `%s ` or `%s\\n` -- two bytes of which meant anything -- so it passes those bytes directly now."
        "There is one behavioural difference a corpus can still see: dash runs printf without forking and nsh execs it. `docs/divergences.md` records it and the register refuses anything broader."
    )
    deferred ("Nothing in the shell now formats a value by a pattern chosen at runtime, and nothing should acquire one. The nearest live temptation is a future `read -t`-style option or a prompt escape expander; both should reach for `write!` at the site that knows the types, not for a specification parsed out of a string.")
}
edges {
    requires ([dec:nsh:we-own-the-defects])
    constrains ([dec:nsh:differential-is-the-oracle])
}
---

## Rationale

The shell's output layer became `Output`, a `std::io::Write` the shell
owns, and formatting moved to `write!` at the call sites where the
arguments still have types. That deleted the runtime format-string API
outright — `doformat`, `fmtstr`, `out1fmt`, `outfmt` and the
`xasprintf` family — because with a writer and a trait there is nothing
for them to do.

The `printf` builtin was the one thing left that still wanted them. Its
contract is not "print this value", it is "read a format string the user
supplied, at runtime, and interpret it": find each `%`, parse flags, a
width, a precision, a conversion character, then choose a renderer and
feed it an argument whose type the specification decided. That is an
interpreter for a small language, and where its digits come from is not
what makes it one.

So the intermediate step — parse the specification into a typed `Spec`
and render it with `format!` — was not the destination. It was measured
and it worked: `aud_bltin_printf` 150/1, `printf2` 113/0, the three fuzz
sets 3819/181, 3606/394 and 4000/0, with every one of the 576 failures a
`%a` or `%A` and nothing else diverging in 12264 cases. Rust has no
hexadecimal float formatting, and the two ways to close that gap were to
hand-write a `%a` renderer or to excuse 576 cases in the divergence
register. Both were rejected, and the question they came from was
rejected with them: the shell does not need to be able to answer it.

This decision no longer governs, and the step it got wrong is the one
above: it treated the builtin and the machinery as one thing. They were
separable. Once `Output` was an `io::Write` and the builtins took typed
arguments, `printf` no longer needed a format string built at runtime,
varargs, or an arity switch — it needed a `%` conversion parsed into a
descriptor and a typed argument rendered against it, which is the utility
reading its own operand rather than the shell speaking a language.
`[dec:nsh:printf-is-parsed-not-interpreted]` supersedes this record and
returns the builtin. What that decision keeps is everything below.

## What this does not ban

Formatting. `write!(output, "{name}={value}")` is not a format-string
interpreter: the pattern is a literal in the source, the arguments are
typed at compile time, and `std::fmt` resolves the whole thing before
the program runs. The ban is on *runtime* patterns — a string that
arrives as data and decides what to do with other data.
