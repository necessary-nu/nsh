#!/usr/bin/env python3
"""Generate a randomised differential corpus over the SHELL STATE modules:
var.c options.c input.c redir.c trap.c alias.c cd.c mail.c.
Deterministic (fixed seed) so failures are reproducible."""
import random, sys

random.seed(int(sys.argv[1]) if len(sys.argv) > 1 else 20260803)
N = int(sys.argv[2]) if len(sys.argv) > 2 else 1200

NAMES = ["A", "B", "V1", "vv", "IFS", "PATH", "PS1", "PS2", "PS4", "OPTIND",
         "LINENO", "MAIL", "MAILPATH", "TERM", "HISTSIZE", "LC_ALL",
         "LC_COLLATE", "LC_CTYPE", "LC_NUMERIC", "LANG", "PWD", "OLDPWD",
         "HOME", "CDPATH", "PPID"]
VALS = ["", "1", "abc", "a b", "x=y", "C", "POSIX", ".", "/tmp", "a'b", '"q"',
        "\\t", ":", "0"]
OPTS = ["e", "f", "I", "i", "m", "n", "x", "v", "V", "E", "C", "a", "b", "u"]
ONAMES = ["errexit", "noglob", "ignoreeof", "monitor", "noexec", "xtrace",
          "verbose", "vi", "emacs", "noclobber", "allexport", "notify",
          "nounset", "nolog", "pipefail"]
SIGS = ["USR1", "USR2", "HUP", "TERM", "EXIT", "0", "1", "10", "12", "INT",
        "QUIT", "CHLD", "PIPE", "ALRM"]


def var_stmt(r):
    n = r.choice(NAMES)
    v = r.choice(VALS)
    return r.choice([
        f'{n}={v!r}'.replace("'", "'", 1) if False else f"{n}='{v}'",
        f"unset {n}",
        f"unset -v {n}",
        f"export {n}",
        f"export {n}='{v}'",
        f"readonly {n}='{v}'" if n not in ("PATH", "IFS", "PS1") else f"{n}='{v}'",
        f"echo \"[${{{n}-U}}]\"" if n not in ("PWD", "OLDPWD") else f"echo \"[${{{n}#$B0}}]\"",
        f'echo "[${{{n}}}]"' if n not in ("PWD", "OLDPWD") else f'echo "[${{{n}#$B0}}]"',
        f"export -p | grep -c '{n}' ",
        f"set | grep -c '^{n}='",
        f"env | grep -c '^{n}='",
    ])


def local_stmt(r):
    n = r.choice(["A", "B", "V1", "vv", "IFS", "PATH", "OPTIND", "-"])
    v = r.choice(VALS)
    if n == "-":
        body = r.choice([f"set -{r.choice(OPTS)}", f"set +{r.choice(OPTS)}",
                         f"set -o {r.choice(ONAMES)}"])
        return f"g() {{ local -; {body}; echo \"in:$-\"; }}; g; echo \"out:$-\""
    inner = r.choice([f"echo \"[${{{n}-U}}]\"", f"{n}='{v}'; echo \"[${{{n}}}]\"",
                      f"echo \"[${{{n}}}]\""])
    decl = r.choice([f"local {n}", f"local {n}='{v}'", f"local {n} B"])
    return f"g() {{ {decl}; {inner}; }}; {n}=outer 2>/dev/null; g; echo \"[${{{n}-U}}]\""


def opt_stmt(r):
    return r.choice([
        f"set -{r.choice(OPTS)}; echo \"$-\"",
        f"set +{r.choice(OPTS)}; echo \"$-\"",
        f"set -o {r.choice(ONAMES)}; echo \"$-\"",
        f"set +o {r.choice(ONAMES)}; echo \"$-\"",
        "set -o | md5sum",
        "set +o | md5sum",
        f"set -- {' '.join(r.choice(['a','b','c','1','']) for _ in range(r.randint(0,4)))}; echo \"$#:$*\"",
        f"set -- a b c d e; shift {r.randint(0,6)} 2>&1; echo \"rc=$? $#:$*\"",
        f"set -{r.choice(OPTS)}{r.choice(OPTS)}; echo \"$-\"",
        "set --; echo \"$#\"",
        "set -; echo \"$-\"",
    ])


def getopts_stmt(r):
    optstr = r.choice(["ab", ":ab", "a:b", ":a:b", "abc", ":a:bc:"])
    args = " ".join(r.choice(["-a", "-b", "-c", "-ab", "-a x", "--", "-z",
                              "-a -b", "x", "-bvalue"])
                    for _ in range(r.randint(1, 3)))
    pre = r.choice(["", "OPTIND=1; ", "unset OPTIND; ", "OPTIND=3; ",
                    "unset OPTIND; OPTIND=1; "])
    return (f"set -- {args}; {pre}"
            f"while getopts {optstr} o 2>&1; do echo \"$o|${{OPTARG-U}}|$OPTIND\"; done; "
            f"echo \"end $OPTIND\"")


def redir_stmt(r):
    fd = r.choice([3, 4, 5, 6, 9])
    return r.choice([
        f"exec {fd}>r{fd}.txt; echo hi >&{fd}; exec {fd}>&-; cat r{fd}.txt",
        f"{{ echo a; echo b >&2; }} > o.txt 2>&1; cat o.txt",
        f"{{ echo a; echo b >&2; }} 2>&1 > o.txt; cat o.txt",
        f"exec {fd}>&1; echo x >&{fd}; exec {fd}>&-; echo y >&{fd} 2>&1; echo rc=$?",
        f"f() {{ echo F; }}; f >f.txt 2>&1; cat f.txt",
        f"(exec {fd}>s.txt; echo s >&{fd}); cat s.txt; echo t >&{fd} 2>&1; echo rc=$?",
        f"cat <<E\nline1\nline2\nE",
        f"cat <<'E'\n$notexpanded\nE",
        f"cat <<-E\n\tindented\n\tE",
        f"read v <<E\nvalue\nE\necho \"[$v]\"",
        f"exec {fd}<<E\nfromfd\nE\nread v <&{fd}; echo \"[$v]\"; exec {fd}<&-",
        f"set -C; echo a > nc.txt; echo b > nc.txt 2>&1; echo rc=$?; cat nc.txt",
        f"echo z > nz.txt; echo y >> nz.txt; cat nz.txt",
        f"exec {fd}<&-; read v <&{fd} 2>&1; echo rc=$?",
        f"{{ echo p; }} {fd}>&- ; echo rc=$?",
    ])


def trap_stmt(r):
    s = r.choice(SIGS)
    return r.choice([
        f"trap 'echo T' {s} 2>&1; trap; echo rc=$?",
        f"trap '' {s} 2>&1; trap; echo rc=$?",
        f"trap 'echo A' {s} 2>&1; trap - {s} 2>&1; trap; echo rc=$?",
        f"trap 'echo X' {s} 2>&1; (trap); echo ---; trap",
        f"trap {s} 2>&1; trap; echo rc=$?",
        f"trap 'echo E' EXIT; echo body",
        f"(trap 'echo SE' EXIT; echo sub); echo main",
    ])


def cd_stmt(r):
    return r.choice([
        'mkdir -p c1/c2; cd c1/c2; echo "${PWD#$B0}"; cd ../..; echo "R${PWD#$B0}"',
        'mkdir -p l1; ln -s l1 s1; cd -L s1; echo "${PWD#$B0}"; cd ..; echo "R${PWD#$B0}"',
        'mkdir -p l2; ln -s l2 s2; cd -P s2; echo "${PWD#$B0}"',
        'mkdir -p cp/t; CDPATH=./cp; cd t >/dev/null 2>&1; echo "rc=$? ${PWD#$B0}"',
        'cd /nope 2>&1; echo rc=$?',
        'cd .; echo "R${PWD#$B0}"; cd ./.; echo "R${PWD#$B0}"',
        'mkdir -p q; cd q; cd - >/dev/null; echo "R${PWD#$B0}"',
        'pwd -L > /dev/null; pwd -P > /dev/null; echo pwdok',
        'mkdir -p aa/bb; cd aa/../aa/bb; echo "${PWD#$B0}"',
        'unset CDPATH; mkdir -p dd; cd dd; echo "${PWD#$B0}"',
        'mkdir -p o1 o2; cd o1; cd ../o2; echo "${OLDPWD#$B0}|${PWD#$B0}"',
    ])


def alias_stmt(r):
    return r.choice([
        "alias a='echo A'; a; alias a",
        "alias a='b'; alias b='echo B'; a",
        "alias a='b '; alias b='echo B'; alias c='echo C'; a c",
        "alias a='unalias a; echo A'; a; a 2>&1; echo rc=$?",
        "alias a='x'; unalias a; unalias a 2>&1; echo rc=$?",
        "alias a=1 b=2 c=3; unalias -a; alias; echo rc=$?",
        "alias 'a b'=1 2>&1; echo rc=$?",
        "alias echo='echo pre'; echo hi",
        "alias a='echo A'; f() { a; }; f; unalias a; f 2>&1; echo rc=$?",
    ])


GENS = [var_stmt, local_stmt, opt_stmt, getopts_stmt, redir_stmt,
        trap_stmt, cd_stmt, alias_stmt]

out = []
for i in range(N):
    r = random
    k = r.randint(1, 3)
    body = "\n".join(r.choice(GENS)(r) for _ in range(k))
    out.append(f"#!name gen{i}\n#!mode=file\nB0=$PWD\n{body}")

sys.stdout.write("\n%%%\n".join(out) + "\n")
