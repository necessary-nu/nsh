#!/usr/bin/env python3
"""Grammar-ish fuzzer for dash parser/expansion/arith differential testing.

Emits %%%-separated cases on stdout.  Deterministic given a seed.
"""
import random
import sys

VARS = ['v', 'u', 'w', 'IFS', 'HOME', 'x1', '_a']
SPECIALS = ['@', '*', '#', '?', '$', '!', '-', '1', '2', '9']
LITS = ['a', 'b', 'abc', 'a b', 'a*b', 'a?b', 'a[b', 'a]b', '', ' ', 'x:y',
        'a=b', '~', '~/z', 'a\\b', "a'b", 'a"b', 'a`b', 'a$b', 'a#b', 'a%b',
        'a{b', 'a}b', 'aa', 'aaa', '/a/b', '.', '..', 'é', 'aéb', '\t', '\n']
PATS = ['*', '?', 'a*', '*a', 'a?c', '[abc]', '[!abc]', '[a-c]', '[]a]',
        '[[:alpha:]]', '**', 'a', '', '\\*', '[', ']', 'a\\*b', '*b*']
OPS = ['-', ':-', '+', ':+', '=', ':=', '?', ':?', '#', '##', '%', '%%']
AOPS = ['+', '-', '*', '/', '%', '<<', '>>', '<', '<=', '>', '>=', '==',
        '!=', '&', '^', '|', '&&', '||']
LENSPECIALS = ['@', '*', '#', '?', '$', '-', '1', '2', '9']
ASSIGNOPS = ['=', '+=', '-=', '*=', '/=', '%=', '<<=', '>>=', '&=', '^=', '|=']


def sq(r, s):
    return "'" + s.replace("'", "'\\''") + "'"


def word(r, d=0):
    k = r.randrange(14)
    if d > 3:
        k = 0
    if k == 0:
        return r.choice(LITS).replace('\n', 'N')
    if k == 1:
        return '$' + r.choice(VARS)
    if k == 2:
        return '${' + r.choice(VARS) + '}'
    if k == 3:
        return '${' + r.choice(VARS + SPECIALS) + r.choice(OPS) + word(r, d + 1) + '}'
    if k == 4:
        return '${#' + r.choice(VARS + LENSPECIALS) + '}'
    if k == 5:
        return '"' + word(r, d + 1) + '"'
    if k == 6:
        return sq(r, r.choice(LITS))
    if k == 7:
        return '$(' + cmd(r, d + 1) + ')'
    if k == 8:
        return '`' + cmd(r, d + 1).replace('`', '') + '`'
    if k == 9:
        return '$((' + arith(r, d + 1) + '))'
    if k == 10:
        return '$' + r.choice(SPECIALS)
    if k == 11:
        return word(r, d + 1) + word(r, d + 1)
    if k == 12:
        return '\\' + r.choice(['a', '$', '`', '"', "'", '\\', '*', ' ', 'n'])
    return r.choice(PATS)


def arith(r, d=0):
    k = r.randrange(9)
    if d > 3:
        k = r.randrange(3)
    if k == 0:
        return str(r.choice([0, 1, 2, 3, 7, 8, 10, 16, 63, 64, -1, -2,
                             9223372036854775807, 8, 31]))
    if k == 1:
        return r.choice(VARS)
    if k == 2:
        return r.choice(['0x10', '017', '08', '0', '1'])
    if k == 3:
        return arith(r, d + 1) + ' ' + r.choice(AOPS) + ' ' + arith(r, d + 1)
    if k == 4:
        return '(' + arith(r, d + 1) + ')'
    if k == 5:
        return r.choice(['-', '+', '!', '~']) + arith(r, d + 1)
    if k == 6:
        return arith(r, d + 1) + ' ? ' + arith(r, d + 1) + ' : ' + arith(r, d + 1)
    if k == 7:
        return r.choice(VARS) + ' ' + r.choice(ASSIGNOPS) + ' ' + arith(r, d + 1)
    return '$' + r.choice(VARS) + ' + 1'


def simple(r, d):
    n = r.randrange(1, 4)
    args = ' '.join(word(r, d) for _ in range(n))
    c = r.choice(['echo', 'printf "<%s>" ', 'echo', 'echo', ':', 'echo'])
    return c + ' ' + args


def cmd(r, d=0):
    k = r.randrange(12)
    if d > 2:
        k = 0
    if k == 0:
        return simple(r, d)
    if k == 1:
        return simple(r, d) + ' ' + r.choice(['&&', '||', ';', '|']) + ' ' + simple(r, d)
    if k == 2:
        return 'if ' + simple(r, d) + '; then ' + simple(r, d) + '; fi'
    if k == 3:
        return 'for i in ' + ' '.join(word(r, d + 1) for _ in range(r.randrange(1, 3))) \
            + '; do echo "$i"; done'
    if k == 4:
        return 'case ' + word(r, d + 1) + ' in ' + r.choice(PATS) + ') echo M;; *) echo N;; esac'
    if k == 5:
        return '( ' + cmd(r, d + 1) + ' )'
    if k == 6:
        return '{ ' + cmd(r, d + 1) + '; }'
    if k == 7:
        return r.choice(VARS) + '=' + word(r, d + 1) + '; ' + simple(r, d)
    if k == 8:
        return 'while false; do :; done; ' + simple(r, d)
    if k == 9:
        return 'set -- ' + ' '.join(word(r, d + 1) for _ in range(r.randrange(0, 3))) \
            + '; ' + simple(r, d)
    if k == 10:
        return 'IFS=' + sq(r, r.choice([':', ' ', '', ' :', 'ab', ':\t\n', 'é'])) \
            + '; ' + simple(r, d)
    return 'echo $((' + arith(r, d + 1) + '))'


def main():
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 500
    r = random.Random(seed)
    out = []
    for _ in range(n):
        body = cmd(r)
        out.append(body)
    print('\n%%%\n'.join(out))


main()
