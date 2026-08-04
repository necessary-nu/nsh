#!/usr/bin/env python3
"""Multi-line parser fuzzer: here-documents, backslash-newline, $'..',
nested quoting, redirections.  Emits %%%-separated cases, mode=file."""
import random
import sys

EOFMARKS = ['EOF', "'EOF'", '"EOF"', 'E\\OF', "E'O'F", 'EO"F"', '\\EOF', 'X']
BODYLINES = [
    '$v ${v} ${v:-d} ${#v}',
    '`echo bq` $(echo dp)',
    '$((1+2)) $((v)) $(($#))',
    '\\$ \\` \\\\ \\" \\a \\',
    'plain text',
    'a\\',
    '~ ~/x',
    '"quoted" \'single\'',
    'aéb',
    '${v#a} ${v%b} ${v##*} ${v%%*}',
    '}',
    '${',
    '$',
    '`',
    'EO',
    'EOFX',
    '\tTABBED',
    '',
    '$@ $* $?',
]
WORDS = ['a', 'b', '$v', '"$v"', "'\$v'", '${v:-x}', '$(echo y)', '`echo z`',
         '$((1+1))', 'a\\ b', '"a b"', '*', '?', '[ab]', '~', 'a=b', 'x:y',
         "$'a\\tb'", "$'\\x41'", "$'\\cA'", "$'\\''", '\\']


def heredoc(r, d=0):
    mark = r.choice(EOFMARKS)
    strip = r.choice(['', '-'])
    plain = mark.replace("'", '').replace('"', '').replace('\\', '')
    nlines = r.randrange(0, 4)
    lines = [r.choice(BODYLINES) for _ in range(nlines)]
    pre = '\t' if strip else ''
    body = '\n'.join(pre + l for l in lines)
    if body:
        body += '\n'
    return 'cat <<%s%s\n%s%s\n' % (strip, mark, body, plain)


def cmdline(r, d=0):
    k = r.randrange(12)
    if k == 0:
        return heredoc(r, d)
    if k == 1:
        return 'echo ' + ' '.join(r.choice(WORDS) for _ in range(r.randrange(1, 4))) + '\n'
    if k == 2:
        return 'v=' + r.choice(WORDS) + '\n'
    if k == 3:
        return 'echo a\\\n' + r.choice(['b', '$v', '"c"']) + '\n'
    if k == 4:
        return 'echo "a\\\nb"\n'
    if k == 5:
        return heredoc(r, d) + 'echo after\n'
    if k == 6:
        return 'echo $(cat <<I\ninner $v\nI\n)\n'
    if k == 7:
        return 'echo `cat <<I\ninner\nI\n`\n'
    if k == 8:
        return 'if true; then\n' + heredoc(r, d + 1) + 'fi\n'
    if k == 9:
        return 'for i in 1 2; do\n' + heredoc(r, d + 1) + 'done\n'
    if k == 10:
        return 'cat <<A; cat <<B\n1\nA\n2\nB\n'
    return 'echo ' + r.choice(WORDS) + ' > f.txt\ncat f.txt\n'


def main():
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 300
    r = random.Random(seed)
    out = []
    for _ in range(n):
        nl = r.randrange(1, 4)
        body = 'v=V\n' + ''.join(cmdline(r) for _ in range(nl))
        out.append('#!mode=file\n' + body.rstrip('\n'))
    print('\n%%%\n'.join(out))


main()
