#!/usr/bin/env python3
"""Report how much of the source's normative text reached the rule corpus.

For each (source slice, spec file) pair this extracts every normative sentence
from the source — skipping informative regions — and checks whether a
distinctive run of words from it survives somewhere in the spec file. Rules get
split and merged during authoring, so this matches on word runs rather than
whole sentences.

A miss is not automatically a defect: some normative-sounding sentences are
pure cross-references. It is a worklist to review.

    tools/coverage-report.py            # all pairs
    tools/coverage-report.py expansion  # one pair, listing every miss
"""

import pathlib
import re
import sys
import unicodedata

ROOT = pathlib.Path(__file__).resolve().parent.parent
UNITS = ROOT / 'build' / 'units'
SPECS = ROOT / 'docs' / 'spec'

# Source slice -> spec file. Several slices were merged into one spec file.
PAIRS = [
    ('quoting', 'quoting'),
    ('tokens', 'tokens'),
    ('parameters', 'parameters'),
    ('expansion', 'expansion'),
    ('redirection', 'redirection'),
    ('exit-status', 'exit-status'),
    ('commands', 'commands'),
    ('grammar', 'grammar'),
    ('execution', 'execution'),
    ('pattern-matching', 'pattern-matching'),
    ('builtins-control', 'builtins-control'),
    ('builtins-variables', 'builtins-variables'),
    ('builtins-set-trap', 'builtins-set-trap'),
    ('invocation', 'invocation'),
    ('line-editing', 'line-editing'),
    # Intrinsic utilities (XCU 1.7) and the chapter-1 baseline they defer to.
    ('builtins-command', 'builtins-command'),
    ('builtins-process', 'builtins-process'),
    ('builtins-jobs', 'builtins-jobs'),
    ('builtins-signals', 'builtins-signals'),
    ('builtins-alias', 'builtins-alias'),
    ('builtins-input', 'builtins-input'),
    ('utility-defaults', 'utility-defaults'),
    ('xcu-relationship', 'relationship'),
]

NORMATIVE = re.compile(r'\b(shall|should|may|must)\b', re.I)
INFORMATIVE = re.compile(
    r'<!--\s*INFORMATIVE-START\s*-->.*?<!--\s*INFORMATIVE-END\s*-->', re.S)
HTML_TAG = re.compile(
    r'</?(?:table|thead|tbody|tr|td|th|caption|col(?:group)?|p|div|span|sup|sub'
    r'|a|br|hr|em|strong|b|i|tt|code|pre|ul|ol|li|dl|dt|dd|blockquote|img'
    r'|h[1-6])\b[^>]*>', re.I)

WINDOW = 8


def normalise(text):
    text = unicodedata.normalize('NFKC', text)
    text = re.sub(r'\[Option (?:Start|End)\]', ' ', text)
    text = re.sub(r'[^A-Za-z0-9]+', ' ', text)
    return re.sub(r'\s+', ' ', text).strip().lower()


def source_sentences(path):
    text = path.read_text(encoding='utf-8')
    text = INFORMATIVE.sub(' ', text)
    # Drop link targets so URLs don't become "words", then real HTML tags: two
    # tables survived conversion as raw HTML, and their markup would otherwise
    # split every cell into its own bogus "sentence". Match known tag names
    # only — POSIX character names like <space> and <newline> are prose.
    text = re.sub(r'\]\([^)]*\)', '] ', text)
    text = re.sub(HTML_TAG, ' ', text)
    out = []
    for raw in re.split(r'(?<=[.:;])\s+', text):
        if not NORMATIVE.search(raw):
            continue
        words = normalise(raw).split()
        if len(words) >= WINDOW:
            out.append((raw.strip(), words))
    return out


def covered(words, haystack):
    """True if any WINDOW-long run of the sentence appears in the corpus text."""
    for i in range(0, len(words) - WINDOW + 1):
        if ' '.join(words[i:i + WINDOW]) in haystack:
            return True
    return False


def main():
    only = sys.argv[1] if len(sys.argv) > 1 else None
    total_hits = total_all = 0
    print(f'{"slice":24} {"normative":>9} {"covered":>8} {"":>6}')
    for unit, spec in PAIRS:
        if only and unit != only:
            continue
        upath, spath = UNITS / f'{unit}.md', SPECS / f'{spec}.md'
        if not upath.exists() or not spath.exists():
            print(f'{unit:24} {"— missing —":>9}')
            continue
        sentences = source_sentences(upath)
        haystack = normalise(spath.read_text(encoding='utf-8'))
        misses = [s for s, w in sentences if not covered(w, haystack)]
        hits = len(sentences) - len(misses)
        total_hits += hits
        total_all += len(sentences)
        pct = (100 * hits / len(sentences)) if sentences else 100.0
        print(f'{unit:24} {len(sentences):9d} {hits:8d} {pct:5.0f}%')
        if only:
            print()
            for miss in misses:
                flat = re.sub(r'\s+', ' ', miss).strip()
                print(f'  MISS  {flat[:160]}')
    if not only and total_all:
        print(f'\n{"TOTAL":24} {total_all:9d} {total_hits:8d} '
              f'{100 * total_hits / total_all:5.0f}%')
    return 0


if __name__ == '__main__':
    raise SystemExit(main())
