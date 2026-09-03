# What the shell may spend

Every other rule here says what the shell must answer. These say what it may
spend getting there, because nothing else in this specification does, and for
a while that showed. Four defects of the same family were found and closed in
one week:

| what | measured |
|---|---|
| `${v#pattern}` and its three siblings | quadratic in the value's length -- 120 ms for one trim of a 2,000-byte value against a pattern matching nothing |
| `while [ $i -lt 3 ]` | eight `getdents64` calls, where both references make none: `[` is a bracket-expression metacharacter, so the word was globbed |
| one glob match | 482,532 thread-locale selections, against 10 after the fix |
| one substitution | 2,102,274 units of matcher work, against 2,050 after |

Each was fixed, and nothing was written down that would notice its return.

A cost rule is unusual in that it constrains an implementation rather than an
answer: the fast shell and the slow shell print the same bytes, so no
differential corpus can tell them apart and the survey gate passes either. It
belongs in the specification anyway. `[dec:nsh:shell-as-library]` makes this a
shell an embedder calls in a loop, and a cost an embedder cannot predict is
part of what it was handed. The alternative to writing the bound down is
measuring it once and hoping.

## How a cost claim is made

> [spec:nsh:req:cost.asserted-as-work]
> A check that asserts a cost MUST assert counted work -- system calls, locale
> selections, comparisons, allocations, or another unit the shell can be asked
> to report -- and MUST NOT assert elapsed time. A reported measurement MUST
> carry the machine's load beside it.
>
> This is not fastidiousness about benchmarking. A wall-clock assertion on a
> shared machine is a check whose verdict is a property of whatever else is
> running, which is the shape `[spec:nsh:req:oracle.cannot-measure-is-a-failure]`
> exists to refuse; it will fail when the shell is innocent and pass when it is
> not. One artifact in this repository moved from 1.16 s to 25.27 s within an
> hour, unchanged, under load.

## What an operation may spend

> [spec:nsh:req:cost.proportional-to-the-input]
> Removing a prefix or a suffix, matching a pattern, splitting a field, and
> substituting within a value MUST cost work proportional to the length of the
> value and of the pattern -- not to their product, and not to the square of
> either. A bound that holds only for the inputs some corpus happens to contain
> is not this bound, and MUST NOT be claimed as it.

> [spec:nsh:req:cost.only-the-work-the-command-needs]
> A command MUST NOT perform an expansion, a filesystem lookup, or a C-library
> query that its own semantics do not require. A word whose pattern cannot
> match MUST NOT cause a directory to be read. A byte position no caller can
> ask about MUST NOT be computed. State a caller derives once per value MUST
> NOT be rebuilt per use.
>
> Work whose result is discarded is not a question of degree: the answer is the
> same either way, so the only thing the work can do is cost.

> [spec:nsh:req:cost.locale-selected-per-operation]
> A locale-sensitive operation MUST select its Shell Locale once for the
> operation -- not once per byte, character, or table entry it handles. Where
> an operation walks a value, the walk MUST happen inside one selection.
>
> This is the cost half of `[spec:nsh:req:shell-locale.operation-binding]`,
> which says which locale an operation uses and is silent about how often it
> says so. Selection is a thread-global state change that every other
> locale-sensitive operation must then be ordered against, so how many times it
> happens is a property of this library's structure rather than of the script,
> and the shell can be asked to count it.
