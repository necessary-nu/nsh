---
id [dec:nsh:no-equivalent-forms]
epitome "One program has one tree: the syntax tree records what a program is, never how it was spelled."
state @decided
category @property
scope {
    elements ([arch:nsh:shell-core])
    rules ([spec:nsh:req:idiom.canonical-tree+1])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Concrete syntax: the structure itself records exactly what was written, so a quote's kind and a backslash's presence pick between node shapes."
        rejected_because "It makes printing trivially correct by making every consumer of the tree carry distinctions that do not affect behaviour. Evaluation, expansion, assignment classification and the Bash array path would each grow arms for which quote opened a run and whether a byte was made inert by a backslash or by the quoting around it -- none of which changes what the program does. Recording the same facts as tokens beside a canonical structure buys the losslessness without the arms; see [dec:nsh:tokens-are-the-truth]."
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
        "Canonicity is a property of the structure alone. Where the source said something the structure does not distinguish, that is recorded as the tokens the node was read as -- see [dec:nsh:tokens-are-the-truth] -- so nothing is lost by making the structure canonical, and `declare -f` reproduces the function as written."
        "`WordToken`, `TokenDecoder`, `from_tokens` and `lexer.output` are deleted rather than canonicalized alongside `WordPart`. `WordToken` is `WordPart` flattened -- ten variants against eight, differing only in that nesting is spelled with Start/End pairs instead of recursion -- and the whole stream has one consumer. Two shapes of one model, both carrying the same spelling, so both admit the same equivalent forms."
        "`Escaped`, `Protected`, `QuoteKind` and `Multibyte.escaped` stop being structural alternatives. What survives in the structure is what changes the program: whether a byte is inert, an empty quoted run since `a=''` and `a=` differ, and `$\"...\"` since locale translation is behaviour. Which quote and which escape wrote it are the node's tokens."
        "A construct with no spelling is a parse error, not a tree. Bash rejects `$[])a]` and `$[$(a+=)))`; nsh parses them and cannot write them back, and under this decision that is a parser defect rather than a printer one."
        "The round-trip property becomes well-posed. It was asking for a bijection an over-specified tree cannot provide, so five of its eighty-five failures were phantoms -- correct output failing an incoherent test -- and nothing distinguished them from the real ones."
    )
    deferred (
        "`ParameterExpansion.invalid_marker` and `.invalid_prefix` are the tree carrying source text so a rejected expansion can be quoted back. They stop being needed once the node holds its tokens, so they are removed with that change rather than before it."
        "`SourceLine::eq` still returns `true` unconditionally. Canonicity does not settle whether a position belongs in the tree at all -- only that spelling does not."
    )
}
edges {
    requires ([dec:nsh:owned-data])
}
codifies ([spec:nsh:req:idiom.canonical-tree+1])
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

So the fault is that one axis is smeared across the other, and the fix is to
give each a home. Structure answers what the program is and is canonical: one
shape per program. Tokens answer what the source said and are exact. Comparison
picks the one it means -- programs ignore tokens, text considers nothing else --
and neither question has to be asked of a representation that half-answers both.
What that leaves for the renderer is emission, not choice; [dec:nsh:tokens-are-
the-truth] carries that half.

The honest accounting is that canonicity fixes five of the eighty-five open
round-trip failures directly. That is not what it buys. It buys a property that
can reach zero: today every remaining failure might be real or might be the
test being wrong, and no measurement from outside the tree can tell which.
