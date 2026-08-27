---
id [dec:nsh:no-equivalent-forms]
epitome "One program has one tree: the syntax tree records what a program is, never how it was spelled."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core])
    rules ([spec:nsh:req:idiom.canonical-tree] [spec:nsh:req:idiom.printable-ast+1])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Concrete syntax: the tree records exactly what was written, losslessly, and every normalization moves into the evaluator."
        rejected_because "It makes printing trivially correct by making every consumer of the tree carry distinctions that do not affect behaviour. Evaluation, expansion, assignment classification and the Bash array path would each grow arms for which quote opened a run and whether a byte was made inert by a backslash or by the quoting around it -- none of which changes what the program does."
    }
    {
        option "Keep the tree as it is and state the round-trip property modulo spelling: compare trees under an equality that ignores the spelling variants."
        rejected_because "It writes the lie into the comparison instead of removing it from the data. The equality would have to hold that `Literal(' ')` and `Escaped(' ')` are the same while `Literal('*')` and `Escaped('*')` are not, since one is a glob metacharacter and the other is not -- so the exception list is the spelling table again, in a place where no test can reach it."
    }
    {
        option "Fix the printer defects one at a time and leave the representation alone."
        rejected_because "Measured, and it does not converge. Twenty-eight commits closed real defects and took the corpus from 285 artifacts to 85; a three-minute campaign then produced 341 more crashes reducing to 205 further distinct minimal forms. The generator walks the missing cells of a hand-maintained table faster than they can be named."
    }
)
consequences {
    accepted (
        "`declare -f` does not reproduce the operator's quoting. Already true and recorded in docs/divergences.md -- a quoted run is re-spelled, single quotes when nothing in it expands and double when something does -- verified behaviourally across 35 cases under both shells with byte-identical output. This makes it deliberate rather than incidental, and Bash does not promise it either."
        "`WordToken`, `TokenDecoder`, `from_tokens` and `lexer.output` are deleted rather than canonicalized alongside `WordPart`. `WordToken` is `WordPart` flattened -- ten variants against eight, differing only in that nesting is spelled with Start/End pairs instead of recursion -- and the whole stream has one consumer. Two shapes of one model, both carrying the same spelling, so both admit the same equivalent forms."
        "`Escaped`, `Protected`, `QuoteKind` and `Multibyte.escaped` collapse into bytes plus an inert flag. Two markers survive because they carry meaning rather than spelling: an empty quoted run, since `a=''` and `a=` are different programs, and `$\"...\"`, since locale translation is behaviour."
        "A construct with no spelling is a parse error, not a tree. Bash rejects `$[])a]` and `$[$(a+=)))`; nsh parses them and cannot write them back, and under this decision that is a parser defect rather than a printer one."
        "The round-trip property becomes well-posed. It was asking for a bijection an over-specified tree cannot provide, so five of its eighty-five failures were phantoms -- correct output failing an incoherent test -- and nothing distinguished them from the real ones."
    )
    deferred (
        "A diagnostic that must show the bytes the operator typed has nowhere to read them once `ParameterExpansion.invalid_marker` and `.invalid_prefix` are gone. Those fields are the tree carrying source text so a rejected expansion can be quoted back. Replacing them means a span or slice into the input, which may not exist yet; until it does, they stay, and they are the last thing removed rather than the first."
        "`SourceLine::eq` still returns `true` unconditionally. Canonicity does not settle whether a position belongs in the tree at all -- only that spelling does not."
    )
}
edges {
    requires ([dec:nsh:owned-data])
}
codifies ([spec:nsh:req:idiom.canonical-tree])
---

## Rationale

The tree this shell parses into is neither a concrete syntax tree nor an
abstract one, and the round-trip property cannot be satisfied while that is
true.

It is over-specified. `Literal`, `Escaped`, `Protected` and `Quote(QuoteKind)`
are four ways to say one thing -- this byte, inert or not -- so `echo 'a'`,
`echo "a"` and `echo \a` are one program and three trees, each printing
differently. `a[ ]` and `a[\ ]` are one program, two trees, and one printed
form, so the source that spelled it without the backslash fails a property the
printer satisfied. A representation admitting two forms of one program is not
an abstract syntax tree; recording only some of the spelling means it is not a
concrete one either.

It is also lossy, but narrowly, and mostly in the way a syntax tree should be.
Backquote and `$()`, `1>a` and `>a`, `(a)` and `a)` as a case pattern, `for a;`
and `for a in "$@";` all collapse, and Bash collapses them identically. What
does not collapse keeps its distinction: `;;` against `;&`, all three
function-definition styles, `[[ ]]`, `until`, `&`.

So the fault is the over-specification, and the fix is to remove it rather than
to complete it. A program has one tree; spelling is the renderer's to choose,
exactly as layout already is. The printer stops trying to recover which of four
spellings the source used and instead emits one with the right inertness, which
is a single function of the byte and its context rather than eleven named
quoting contexts over a hand-maintained table of bytes to protect.

The honest accounting is that canonicity fixes five of the eighty-five open
round-trip failures directly. That is not what it buys. It buys a property that
can reach zero: today every remaining failure might be real or might be the
test being wrong, and no measurement from outside the tree can tell which.
