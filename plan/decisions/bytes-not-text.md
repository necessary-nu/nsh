---
id [dec:nsh:bytes-not-text]
epitome "A shell value is a byte string, never a `String`. The vocabulary types are `bstr`'s `BStr` and `BString`."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core] [arch:nsh:shell-bin])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "`String` / `&str`, converting lossily at the boundaries."
        rejected_because "It is not merely lossy, it is impossible. dash signals in band: CTLESC, CTLVAR, CTLENDVAR, CTLBACKQ, CTLARI and CTLQUOTEMARK are bytes 0x81-0x88 embedded in every word carrying a quote or a variable reference, and a lone 0x81 is an invalid UTF-8 continuation byte. `String::from_utf8` would reject the parser's own output. Beyond that, a shell handles filenames, arguments and environment variables, none of which the kernel constrains to UTF-8."
    }
    {
        option "Bare `Vec<u8>` and `&[u8]`, no dependency."
        rejected_because "Correct but hostile. Every `Debug` becomes a wall of decimal, and every search, split and trim is hand-rolled. `BStr` is a `#[repr(transparent)]` newtype over `[u8]` -- the representation is identical, so this costs nothing at runtime and buys back the ergonomics."
    }
)
consequences {
    accepted (
        "`bstr` becomes the crate's third dependency, after `libc` and `nshedit`."
        "Interior NUL is excluded by the shell language, not by the type -- see \"On NUL\" below. `BString` permits it, so the invariant becomes ours to hold at the point where a value reaches a syscall."
        "The syscall boundary is explicit. `execve`, `open`, `getenv` and friends want NUL-terminated bytes, so a conversion sits at that edge rather than being implicit in the representation."
    )
    deferred ("Whether the conversion at the syscall edge is a `CString` allocation each time or a type that maintains a trailing-NUL invariant is a real question -- dash today hands `*mut c_char` straight to `execve` with no copy. Measure before choosing; do not assume the allocation matters, and do not assume it does not.")
}
edges {
    requires ([dec:nsh:owned-data])
}
---

## Rationale

The port already knows this in one place, and it was learned the hard
way. `crates/dash/src/main.rs` collects argv as `Vec<Vec<u8>>` rather
than `Vec<String>` with a comment explaining why: `std::env::args()`
unwraps a UTF-8 conversion and panics on any non-UTF-8 argument, so the
port died with status 101 where the C ran normally. `dash -c $'x=\xff;
echo $x'` prints the byte.

That is the same fact appearing at the process boundary that
[dec:nsh:owned-data] is about to hit everywhere else. When `memalloc`
goes and the values stop being `*mut c_char`, whatever replaces them
inherits the constraint: a shell value is a sequence of bytes that is not
text and must not be validated as text.

## The in-band bytes

`parser.rs` defines the control bytes dash embeds in the strings it
produces:

```
CTLESC       -127   escape next character
CTLVAR       -126   variable defn
CTLENDVAR    -125
CTLBACKQ     -124
CTLARI       -122   arithmetic expression
CTLQUOTEMARK -120
```

As bytes those are 0x81 through 0x88. They are not delimiters around the
data, they are *in* it: `"$x"` leaves the parser as CTLQUOTEMARK, CTLVAR,
a flags byte, `x`, CTLENDVAR, CTLQUOTEMARK. Expansion consumes them and
`rmescapes` strips what survives.

So the parser's output is a byte string that is invalid UTF-8 by
construction, and it stays that way through every stage of expansion.
There is no point in the pipeline at which a `String` would be a correct
model, and the one place a lossy conversion would be tolerable -- final
output -- is exactly where the bytes must be passed through untouched.

## A note on the word "word"

POSIX XBD 3 defines a word as "a token other than an operator". Token
recognition (XCU 2.3) splits the input on unquoted blanks, operators and
newlines, and whatever is not an operator token is a `WORD` -- command
names, arguments, assignment prefixes, redirection targets, `case`
patterns, `for` list items, here-document delimiters, function names.

A word is not a field. One word becomes zero, one or many fields once
[spec:posix:req:grammar.word-expansion-timing] applies the expansions,
which happens immediately before the command runs rather than at parse
time. `f $x` with `x='a b'` is one word and two fields; with `x=''` it is
one word and no fields at all.

In this port a word is what a `narg` node holds: the byte string with the
CTL markers still in band, sitting in the tree until `evalcommand`
expands it. That is the thing this decision is about the type of.

## On NUL

A shell value may not contain a NUL, and the reason is not that the C
terminates on one -- that would be arguing the language's constraint from
the implementation's. There are three routes a byte can take into a word,
and none of them admits NUL:

  * **From the input.** The shell's input is a text file, and a text file
    cannot contain NUL.
  * **From a variable.** Values arrive by assignment, by the environment
    or by `read`, all of which are themselves words or NUL-free by the
    same argument -- `execve` takes a NUL-terminated environment.
  * **From command substitution.** This is the only route where POSIX
    declines to forbid it: XCU 2.6.3 says that if the output contains
    null bytes, the behaviour is unspecified. Both shells drop them.
    `x=$(printf 'a\0b'); printf %s "$x"` yields two bytes, not three, in
    the C dash and in the port alike.

So the invariant is real and independent of the representation. What
changes is who enforces it: `*mut c_char` enforced it by construction,
and `BString` does not, so it becomes ours to hold at the edge where a
value becomes a syscall argument. That edge is one to make explicit in
any case -- it is the same edge [dec:nsh:host-owns-streams] made explicit
for descriptors.
