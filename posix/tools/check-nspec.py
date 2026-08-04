#!/usr/bin/env python3
"""Lint an nspec spec corpus.

nspec's def-site scanner reports nothing when a marker is malformed — the line
simply isn't a rule, silently. This checker mirrors that scanner's grammar and
flags every line that looks like it was *meant* to be a rule but wouldn't be
seen, plus the corpus-level invariants nspec doesn't enforce (id uniqueness,
non-empty bodies).

    tools/check-nspec.py docs/spec

Exits non-zero if any error-level finding is reported.
"""

import pathlib
import re
import sys
from collections import Counter, defaultdict

VERBS = ('def', 'syn', 'sem', 'req', 'thm')

# Mirrors crates/nspec-syntax/src/grammar.rs: ns is [a-z0-9-]+, id is
# [a-z0-9._-]+, an optional +N pin where N >= 1, an optional /facet.
MARKER = re.compile(
    r'\[spec:(?P<ns>[a-z0-9-]+):(?P<verb>' + '|'.join(VERBS) + r')'
    r':(?P<id>[a-z0-9._-]+)(?:\+(?P<pin>[1-9][0-9]*))?'
    r'(?:/(?P<facet>[a-z0-9/_-]+))?\]'
)
# A def site is the marker alone on a blockquote line, nothing after the ']'.
DEF_SITE = re.compile(r'^>[ \t]*(?P<marker>' + MARKER.pattern + r')[ \t]*$')
# Anything on a blockquote line that was plausibly meant to be a marker.
SUSPECT = re.compile(r'^>[ \t]*\[[a-zA-Z]')

FENCE = re.compile(r'^\s*(```|~~~)')


class Finding:
    def __init__(self, level, path, line, message):
        self.level = level
        self.path = path
        self.line = line
        self.message = message

    def __str__(self):
        return f'{self.level:5} {self.path}:{self.line}: {self.message}'


def scan(path, findings, rules):
    lines = path.read_text(encoding='utf-8').split('\n')
    in_fence = False
    current = None

    for n, line in enumerate(lines, 1):
        if FENCE.match(line):
            in_fence = not in_fence

        match = DEF_SITE.match(line)
        if match:
            if in_fence:
                findings.append(Finding(
                    'ERROR', path, n,
                    'marker inside a fenced code block — the scanner does not '
                    'track fences and would register this as a real rule'))
            if line.startswith('>  '):
                findings.append(Finding(
                    'WARN', path, n,
                    'two spaces after ">" — parses for coverage but renders '
                    'as an ordinary blockquote'))
            rid = match.group('id')
            current = {'id': rid, 'verb': match.group('verb'), 'ns':
                       match.group('ns'), 'path': path, 'line': n, 'body': []}
            rules.append(current)
            continue

        if line.startswith('>'):
            if current is not None:
                current['body'].append(line[1:].strip())
            # A blockquote line that looks marker-ish but did not parse is the
            # dangerous case: it vanishes with no diagnostic anywhere.
            if SUSPECT.match(line) and '[spec' in line:
                findings.append(Finding(
                    'ERROR', path, n,
                    'looks like a rule marker but will NOT be seen as one — '
                    f'check verb, lowercase id, and that nothing follows "]": {line.strip()[:80]}'))
        elif line.strip() == '':
            current = None

    return rules


def main():
    roots = sys.argv[1:] or ['docs/spec']
    paths = sorted(
        p for root in roots for p in pathlib.Path(root).rglob('*.md')
    )
    if not paths:
        print('no spec files found', file=sys.stderr)
        return 2

    findings, rules = [], []
    for path in paths:
        scan(path, findings, rules)

    # Coverage is keyed on the bare id across the whole corpus, so a duplicate
    # silently merges two rules into one.
    by_id = defaultdict(list)
    for rule in rules:
        by_id[rule['id']].append(rule)
    for rid, group in sorted(by_id.items()):
        if len(group) > 1:
            where = ', '.join(f"{r['path']}:{r['line']}" for r in group)
            findings.append(Finding(
                'ERROR', group[1]['path'], group[1]['line'],
                f'duplicate rule id "{rid}" — coverage would merge these: {where}'))

    for rule in rules:
        body = [b for b in rule['body'] if b]
        if not body:
            findings.append(Finding(
                'ERROR', rule['path'], rule['line'],
                f'rule "{rule["id"]}" has an empty body'))
        elif not any(b.startswith('Source:') for b in body):
            findings.append(Finding(
                'WARN', rule['path'], rule['line'],
                f'rule "{rule["id"]}" has no Source: citation'))

    per_file = Counter(str(r['path']) for r in rules)
    verbs = Counter(r['verb'] for r in rules)
    namespaces = Counter(r['ns'] for r in rules)

    print(f'{len(rules)} rules across {len(paths)} files\n')
    for path in paths:
        print(f'  {per_file.get(str(path), 0):4d}  {path}')
    print('\nverbs: ' + '  '.join(f'{v}={verbs.get(v, 0)}' for v in VERBS))
    print('namespaces: ' + ', '.join(f'{k}={v}' for k, v in namespaces.items()))

    errors = [f for f in findings if f.level == 'ERROR']
    warnings = [f for f in findings if f.level == 'WARN']
    if findings:
        print(f'\n{len(errors)} errors, {len(warnings)} warnings')
        for finding in findings:
            print(f'  {finding}')
    else:
        print('\nno findings')

    return 1 if errors else 0


if __name__ == '__main__':
    raise SystemExit(main())
