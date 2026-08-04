#!/usr/bin/env python3
"""Byte-soup fuzzer: short strings drawn from shell-significant bytes,
including the CTL* range dash uses internally, fed as `sh -c`."""
import random
import sys

# shell metacharacters + dash's internal CTL bytes (0x81..0x88 as raw
# high bytes, which is how CTLESC..CTLQUOTEMARK appear on the wire) +
# assorted whitespace/letters.
POOL = list(b"$'\"`\\(){}[]<>|&;#~*?!:=-+%^/@0123456789abcxyzEOF \t\n")
CTLBYTES = [0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88]
HIGH = [0xc3, 0xa9, 0x80, 0xff, 0xc0, 0xe2, 0x98]
FRAGS = [b'$(', b'${', b'$((', b'))', b'}', b')', b'<<', b'<<-', b'`',
         b'"', b"'", b'$@', b'$*', b'$#', b'${#', b':-', b':+', b':=',
         b':?', b'##', b'%%', b'[[:alpha:]]', b'\\\n', b'&&', b'||',
         b';;', b'>&', b'<&', b'2>', b'echo ', b'case ', b' in ', b'esac',
         b'for ', b'do ', b'done', b'if ', b'then ', b'fi', b'while ',
         b'until ', b'{', b'}', b'!', b'$\'', b'\\x41', b'\\c', b'\\u']


def gen(r):
    n = r.randrange(2, 26)
    out = bytearray()
    for _ in range(n):
        k = r.randrange(10)
        if k < 4:
            out.append(r.choice(POOL))
        elif k < 7:
            out += r.choice(FRAGS)
        elif k < 8:
            out.append(r.choice(CTLBYTES))
        elif k < 9:
            out.append(r.choice(HIGH))
        else:
            out += b'echo '
    return bytes(out)


def main():
    seed = int(sys.argv[1]) if len(sys.argv) > 1 else 1
    n = int(sys.argv[2]) if len(sys.argv) > 2 else 500
    r = random.Random(seed)
    outs = []
    for _ in range(n):
        b = gen(r)
        # keep cases single-"line" for the %%% splitter unless they carry
        # a deliberate backslash-newline; a bare newline is fine too since
        # %%% is a whole-line marker.
        if b.strip() == b'' or b'\x00' in b:
            b = b'echo x'
        mode = b'#!mode=file\n' if r.randrange(2) else b'#!mode=stdin\n'
        outs.append(mode + b)
    sys.stdout.buffer.write(b'\n%%%\n'.join(outs) + b'\n')


main()
