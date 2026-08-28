---
id [dec:nsh:tokens-are-the-truth]
epitome "The tokenizer's output is kept and carried into the tree, so printing emits what was read instead of spelling it again."
state @decided
category @existence
scope {
    elements ([arch:nsh:shell-core])
    rules ([spec:nsh:def:idiom.token-stream] [spec:nsh:req:idiom.printable-ast+2])
}
author "brendan@necessary.nu"
alternatives (
    {
        option "Keep the printer and give it the lexer's `SyntaxContext::classify` to consult, so the two stop holding separate copies of the grammar."
        rejected_because "It leaves a printer that still derives spelling, only from a better source. Deriving is the defect: the parser already read the bytes, so any second computation of them can disagree with what was there, and only a fuzzer notices. Emitting what was read cannot disagree."
    }
    {
        option "Discard spelling entirely: collapse the word parts to bytes plus an inert flag and let the printer choose any valid spelling."
        rejected_because "It makes the round-trip property satisfiable by making the tree unable to answer what the source said. `declare -f` then re-spells a function the operator wrote, diagnostics cannot quote the text that failed, and the parser's own record of the input is thrown away one line after it is produced."
    }
    {
        option "Retain source spans into an input buffer rather than owned token text."
        rejected_because "There is no single buffer to span into. Input arrives through `input.rs` from strings, files, terminals and alias expansions, interleaved, and a here-document body is read at a different time from the redirection that named it. Owned token bytes cost memory and are correct under every input source."
    }
)
consequences {
    accepted (
        "The round-trip property becomes a byte comparison. `print(parse(x)) == x` replaces comparing two trees, so `printing_is_reversible`, the `Reversibility` verdict and every tree-equality question the fuzz target asks collapse into one equality on bytes."
        "The printer stops having a grammar. `Quoting`'s eleven variants, the nine byte-sets, `protected`, `operand_needs_apostrophe_protection` and the four region printers exist to re-derive what the tokens already record, and go."
        "`declare -f` reproduces the function as written, which is better than Bash and better than this shell's current re-spelling. docs/divergences.md loses the entry about quoted runs being re-spelled."
        "Every byte is accounted for, so blanks, comments and line continuations are in the tree. That is what makes the property byte-exact rather than approximately-exact, and it is the part that touches the most code."
        "A node built rather than parsed carries no tokens and needs a spelling. That fallback is the only place a renderer decides anything, and it is bounded by the constructs the shell synthesizes rather than by the grammar."
    )
    deferred (
        "Alias substitution replaces source text before the parser sees it, so a printed tree reproduces the expanded text and not what was typed. Recording the pre-expansion bytes as well is a further decision, not this one."
        "Memory. Owned token bytes for every parse are larger than the current word parts. A function definition retained for the life of a shell now retains its whitespace and comments too."
    )
}
edges {
    requires ([dec:nsh:no-equivalent-forms] [dec:nsh:owned-data])
}
codifies ([spec:nsh:def:idiom.token-stream])
---

## Rationale

A printer that spells a construct again is a second implementation of the
grammar, and two implementations of one grammar drift. This one has: the
printer protects `" \ $ ` ` inside double quotes while `SyntaxContext::
DoubleQuoted` also calls `\n` and `}` syntax and gives eleven more bytes their
own class; the printer names eleven quoting contexts where the lexer tracks
five plus a depth. Neither table is wrong on its own terms, and nothing in
either says whether a difference between them is deliberate. Only the fuzzer
finds out, one artifact at a time, forever.

The parser already knows the answer. It read the bytes. Keeping them removes
the second implementation rather than improving it: printing becomes emission,
which cannot disagree with what was there because it is what was there.

That also settles what the tree is, which nothing had. Structure is what the
program means and is canonical -- one shape per program, so `echo 'a'` and
`echo "a"` differ in tokens and not in nodes. Tokens are what the source said
and are exact. The two questions that were tangled together in
`Literal`/`Escaped`/`Protected`/`QuoteKind` -- what is this byte, and how was
it written -- get one home each, and comparison picks the one it means:
programs ignore tokens, text considers nothing else.

Shell resists a tokenizer as a separate pass, and this does not attempt one.
Lexical rules depend on the construct being parsed -- a here-document body, a
`${...}` operand and a `=~` operand each have their own -- so the tokenizer is
mode-driven and the parser drives it, which is what `read_word_token` taking a
`SyntaxContext` already is. The change is that its output is kept rather than
consumed and dropped one line later by `from_tokens`.
