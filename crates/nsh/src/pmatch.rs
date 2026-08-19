/*
 * Shell pattern matching: `case` patterns, the `${v#pat}` family, and the
 * glob matcher, ported from dash's `expand.c`.
 *
 * A pure function over two byte strings with no shell state, which is why
 * it is a file rather than three hundred more lines of `expand.rs`. The
 * The matcher is entirely slice based and indexed.
 *
 * See plan/decisions/owned-data.md, "What this cost in the port: the
 * pattern matcher", for the one comparison here that is decided rather
 * than transcribed.
 */

use bstr::ByteSlice;
use core::ffi::{c_char, c_int, c_uint};

use crate::expand::{
    C_BANG, C_CARET, C_COLON, C_LBRACKET, C_MINUS, C_NUL, C_QUESTION, C_RBRACKET, C_STAR, CTLESC,
    CTLMBCHAR, mbnext_bytes,
};
use crate::mystring::{byte_at, ncmp_eq_at, slice_from};

/// Remove the shell's quote and multibyte framing from a pattern fragment.
fn decode_pattern_bytes(encoded: &[u8]) -> Vec<u8> {
    let mut decoded = Vec::with_capacity(encoded.len());
    let mut at = 0;
    while at < encoded.len() {
        match byte_at(encoded, at) {
            CTLESC => {
                at += 1;
                if let Some(byte) = encoded.get(at) {
                    decoded.push(*byte);
                    at += 1;
                }
            }
            CTLMBCHAR => {
                let frame = mbnext_bytes(slice_from(encoded, at));
                let start = (frame & 0xff) as usize;
                let span = (frame >> 8) as usize;
                let data_len = span.saturating_sub(2);
                let data_start = at + start;
                if data_start >= encoded.len() {
                    break;
                }
                let data_end = data_start.saturating_add(data_len).min(encoded.len());
                decoded.extend_from_slice(&encoded[data_start..data_end]);
                let next = at.saturating_add(start).saturating_add(span);
                if next <= at {
                    break;
                }
                at = next.min(encoded.len());
            }
            _ => {
                decoded.push(encoded[at]);
                at += 1;
            }
        }
    }
    decoded
}

fn is_nested_bracket_delimiter(byte: c_char) -> bool {
    byte == C_COLON || byte == b'.' as c_char || byte == b'=' as c_char
}

// [spec:dash:def:expand.ccmatch-fn]
// [spec:dash:sem:expand.ccmatch-fn]
//
// Returns whether the character matched and, separately, where the pattern
// continues and how many subject bytes the member consumes. The C signalled
// the continuation through an out-parameter `char **r` left NULL when `p`
// did not open a well-formed `[:class:]`; the option is that signal as a
// value. POSIX adds the sibling `[.element.]` and `[=element=]` forms, which
// have the same nested closing-bracket requirement.
#[inline(never)]
fn ccmatch_bytes(
    locale: &nsh_platform::Locale,
    p: &[u8],
    mbc: &[u8],
    ml: usize,
) -> (bool, Option<(usize, usize)>) {
    let delimiter = byte_at(p, 0);
    if delimiter != C_COLON && delimiter != b'.' as c_char && delimiter != b'=' as c_char {
        return (false, None);
    }
    let body = slice_from(p, 1);
    let closing = [delimiter as u8, b']'];
    let close = match body.find(closing) {
        Some(at) => at,
        None => return (false, None),
    };

    let encoded_member = &body[..close];
    if encoded_member.is_empty() {
        return (false, None);
    }

    /* Past the delimiter skipped above, and past the two-byte close. */
    let continuation = 1 + close + 2;
    let first_span = if byte_at(mbc, ml) == ml as c_char && byte_at(mbc, ml + 1) == CTLMBCHAR {
        ml + 2
    } else {
        ml
    };

    if delimiter != C_COLON {
        let member = decode_pattern_bytes(encoded_member);
        let mut expression = Vec::with_capacity(member.len() + 5);
        expression.extend_from_slice(b"[[");
        expression.push(delimiter as u8);
        expression.extend_from_slice(&member);
        expression.extend_from_slice(&[delimiter as u8, b']', b']']);

        if !locale.collating_bracket_matches(&expression, &member) {
            return (false, None);
        }

        for (subject_len, consumed) in [(ml, first_span), (member.len(), member.len())] {
            if subject_len != 0
                && subject_len <= mbc.len()
                && locale.collating_bracket_matches(&expression, &mbc[..subject_len])
            {
                return (true, Some((continuation, consumed)));
            }
        }
        return (false, Some((continuation, first_span)));
    }

    /* The C wrote a NUL over the `:` of `:]`, called `wctype`, and put the
     * `:` back -- it needed a C string and the only one to hand was the
     * pattern itself.  Copying the name costs an allocation on a path that
     * runs once per `[:class:]`, and buys a pattern that `pmatch` never has
     * to be able to write to, which is what lets it take `&[u8]`. */
    let Some(matches) = locale.wide_class_matches(encoded_member, mbc, ml) else {
        return (false, None);
    };

    /* `ml` is what the caller measured of the string's character.  The C
     * passed that count straight to `mbrtowc`; clamping it to what the
     * slice holds can only change a case where the C read past the
     * string's terminator, and a short read fails the `!= ml` test below
     * exactly as a malformed character does. */
    (matches, Some((continuation, first_span)))
}

/*
 * A bracket member that is a single byte, tested against a string
 * character that may not be.
 *
 * The C compared `mb` bytes starting at `&c` -- the address of a *local*,
 * one byte long -- whenever the member was a plain byte.  `strncmp` stops
 * at the first difference and at a NUL, so it only reads past that one
 * byte when `c == q[0]` and `c != 0`; from there its answer is whatever
 * the compiler had put on the stack beside `c`.  The port carried the
 * read forward with a `NOTE (bug-for-bug)` and a hand-written comparison
 * loop written to keep it from being an unconditional `from_raw_parts`.
 *
 * A slice cannot reproduce that and should not try, so this decides it: a
 * one-byte member is not a multibyte character and does not match one.
 * The two agree everywhere the C was defined -- `mb == 1` is plain byte
 * equality, and a differing first byte is a miss under both -- and can
 * differ only where the C's answer was stack contents, so there is
 * nothing for the divergence register to hold: an entry there must be
 * able to say which side is right about an observable difference, and
 * there is no observable difference to be right about.
 *
 * The precedent is [`IS_TYPE_UNBIASED`] in this file, which refused the
 * same trade for the same reason -- an out-of-bounds read yields a
 * property of one binary's layout, not a behaviour.  Recorded in
 * plan/decisions/owned-data.md.
 */
fn single_byte_member(c: c_char, sc: c_char, mb: c_uint) -> bool {
    c == sc && (mb <= 1 || c == C_NUL)
}

// The same entry for a caller that already holds both strings as bytes.
//
// Neither slice has to carry a terminator: `pmatch_bytes` reads past the
// end as NUL, which is the same answer a terminator would give, and that
// is what lets `expmeta` hand over a *sub*-slice of the pattern instead of
// terminating it in place and putting the byte back afterwards.
//
// Both inputs are borrowed slices; the matcher has no pointer adapter or
// alternate libc implementation.
pub(crate) fn pmatch_slices(locale: &nsh_platform::Locale, pattern: &[u8], string: &[u8]) -> c_int {
    pmatch_bytes(locale, pattern, string) as c_int
}

// The matcher.  `pi`/`qi` are the C's `p`/`q`; every `p++` is `pi += 1` and
// the recursion takes the two tails.
// [spec:posix:def:pattern.notation-purpose]
// [spec:posix:req:pattern.invalid-byte-sequence-unspecified]
// [spec:posix:req:pattern.match-by-bit-pattern]
// [spec:posix:syn:pattern.single-character-patterns]
// [spec:posix:def:pattern.ordinary-character]
// [spec:posix:def:pattern.special-pattern-characters]
// [spec:posix:sem:pattern.question-mark]
// [spec:posix:sem:pattern.asterisk]
// [spec:posix:syn:pattern.bracket-expression]
// [spec:posix:sem:pattern.left-bracket-literal]
// [spec:posix:sem:pattern.asterisk-matches-any-string]
// [spec:posix:syn:pattern.concatenation]
// [spec:posix:sem:pattern.asterisk-longest-match]
fn pmatch_bytes(locale: &nsh_platform::Locale, pattern: &[u8], string: &[u8]) -> bool {
    let mut pi: usize = 0;
    let mut qi: usize = 0;
    let mut mb: c_uint;
    let mut c: c_char;

    'forever: loop {
        'dft: {
            c = byte_at(pattern, pi);
            pi += 1;
            match c {
                C_NUL => break 'forever, /* goto breakloop */
                CTLESC => {
                    c = byte_at(pattern, pi);
                    pi += 1;
                    /* break -- fall through to dft */
                }
                C_QUESTION => {
                    if byte_at(string, qi) == C_NUL {
                        return false;
                    }
                    mb = mbnext_bytes(slice_from(string, qi));
                    qi += ((mb >> 8) + (mb & 0xff)) as usize;
                    continue 'forever;
                }
                C_STAR => {
                    c = byte_at(pattern, pi);
                    while c == C_STAR {
                        pi += 1;
                        c = byte_at(pattern, pi);
                    }
                    if c == C_NUL {
                        return true;
                    }
                    if c == C_QUESTION || c == C_LBRACKET {
                        c = CTLESC;
                    }
                    loop {
                        if c != CTLESC {
                            /* The C's comment here is `Stop should be
                             * null-terminated as it is passed as a string
                             * to strpbrk(3)`, and the terminator was the
                             * whole reason for the fourth element. The
                             * set is the three bytes; the scan stops at
                             * the string's own NUL, which is a miss.
                             *
                             * Walked rather than taken as a slice: this
                             * runs once per candidate position under a
                             * `*`, and measuring the whole tail each time
                             * would cost a pass per position. */
                            let stop: [u8; 3] = [c as u8, CTLESC as u8, CTLMBCHAR as u8];
                            let mut k: usize = 0;
                            loop {
                                let b = byte_at(string, qi + k) as u8;
                                if b == 0 || stop.contains(&b) {
                                    break;
                                }
                                k += 1;
                            }
                            if byte_at(string, qi + k) == C_NUL {
                                return false;
                            }
                            qi += k;
                        }
                        if pmatch_bytes(locale, slice_from(pattern, pi), slice_from(string, qi)) {
                            return true;
                        }
                        if byte_at(string, qi) == C_NUL {
                            break;
                        }
                        mb = mbnext_bytes(slice_from(string, qi));
                        qi += ((mb >> 8) + (mb & 0xff)) as usize;
                    }
                    return false;
                }
                C_LBRACKET => {
                    let startp: usize;
                    let invert: c_int;
                    let mut found: c_int;
                    let mut matched_span: usize;
                    let chr: c_char;

                    startp = pi;
                    invert = if byte_at(pattern, pi) == C_BANG || byte_at(pattern, pi) == C_CARET {
                        pi += 1;
                        1
                    } else {
                        0
                    };
                    found = 0;
                    mb = mbnext_bytes(slice_from(string, qi));
                    qi += (mb & 0xff) as usize;
                    mb >>= 8;
                    matched_span = mb as usize;
                    chr = byte_at(string, qi);
                    if chr == C_NUL {
                        return false;
                    }
                    c = byte_at(pattern, pi);
                    pi += 1;
                    loop {
                        'cont: {
                            let mut mbp: c_uint = 0;
                            /* The C's `mbs` is `&c` by default -- the
                             * address of the local, so it follows `c`'s
                             * later assignment in the CTLESC arm.  Naming
                             * the two sources instead of taking an address
                             * keeps that, and makes the undefined case
                             * below something that can be decided. */
                            let mut mbs: Option<usize> = None;

                            if c == C_NUL {
                                pi = startp;
                                c = C_LBRACKET;
                                break 'dft; /* goto dft */
                            }
                            if c == C_LBRACKET {
                                let nested_delimiter = byte_at(pattern, pi);
                                let ml = if mb > 1 { mb - 2 } else { mb } as usize;
                                let (hit, nested) = ccmatch_bytes(
                                    locale,
                                    slice_from(pattern, pi),
                                    slice_from(string, qi),
                                    ml,
                                );
                                found |= hit as c_int;
                                if let Some((continuation, span)) = nested {
                                    if hit {
                                        matched_span = span;
                                    }
                                    pi += continuation;
                                    break 'cont; /* continue */
                                }
                                if is_nested_bracket_delimiter(nested_delimiter) {
                                    pi = startp;
                                    c = C_LBRACKET;
                                    break 'dft;
                                }
                            } else if c == CTLESC {
                                c = byte_at(pattern, pi);
                                pi += 1;
                            } else if c == CTLMBCHAR {
                                pi -= 1;
                                mbp = mbnext_bytes(slice_from(pattern, pi));
                                pi += (mbp & 0xff) as usize;
                                mbs = Some(pi);
                                mbp >>= 8;
                                pi += mbp as usize;
                            }
                            if byte_at(pattern, pi) == C_MINUS
                                && byte_at(pattern, pi + 1) != C_NUL
                                && byte_at(pattern, pi + 1) != C_RBRACKET
                            {
                                pi += 1;
                                if byte_at(pattern, pi) == CTLESC {
                                    pi += 1;
                                } else if byte_at(pattern, pi) == CTLMBCHAR {
                                    mbp = mbnext_bytes(slice_from(pattern, pi));
                                    pi += (mbp & 0xff) as usize;
                                    pi += (mbp >> 8) as usize;
                                    break 'cont; /* continue */
                                }
                                if (mbp | mb.wrapping_sub(1)) == 0
                                    && chr >= c
                                    && chr <= byte_at(pattern, pi)
                                {
                                    found = 1;
                                }
                                pi += 1;
                            } else {
                                let hit = match mbs {
                                    Some(i) => ncmp_eq_at(
                                        pattern,
                                        i as isize,
                                        string,
                                        qi as isize,
                                        mb as usize,
                                    ),
                                    None => single_byte_member(c, byte_at(string, qi), mb),
                                };
                                if hit {
                                    found = 1;
                                }
                            }
                        }
                        /* } while ((c = *p++) != ']'); */
                        c = byte_at(pattern, pi);
                        pi += 1;
                        if c == C_RBRACKET {
                            break;
                        }
                    }
                    if found == invert {
                        return false;
                    }
                    qi += matched_span;
                    continue 'forever;
                }
                CTLMBCHAR => {
                    pi -= 1;
                    mb = mbnext_bytes(slice_from(pattern, pi));
                    pi += (mb & 0xff) as usize;
                    mb = mbnext_bytes(slice_from(string, qi));
                    qi += (mb & 0xff) as usize;
                    mb >>= 8;

                    /* Both `-1`s land on the length byte of a CTLMBCHAR
                     * frame when both sides are encoded, which is the case
                     * every caller sets up.  When only the pattern is, the
                     * C read the byte before the string's buffer; here that
                     * reads as NUL and mismatches the pattern's nonzero
                     * length byte, which is the answer a multibyte member
                     * against a single-byte character should have. */
                    if !ncmp_eq_at(
                        pattern,
                        pi as isize - 1,
                        string,
                        qi as isize - 1,
                        (mb + 1) as usize,
                    ) {
                        return false;
                    }

                    pi += mb as usize;
                    qi += mb as usize;
                    continue 'forever;
                }
                _ => {}
            }
        }
        /* dft: */
        mb = mbnext_bytes(slice_from(string, qi));
        if (mb >> 8) > 1 {
            return false;
        }
        qi += (mb & 0xff) as usize;
        if byte_at(string, qi) != c {
            return false;
        }
        qi += (mb >> 8) as usize;
    }
    /* breakloop: */
    byte_at(string, qi) == C_NUL
}

#[cfg(test)]
mod tests {
    use super::*;

    fn matches(locale: &nsh_platform::Locale, pattern: &[u8], subject: &[u8]) -> bool {
        pmatch_slices(locale, pattern, subject) != 0
    }

    #[test]
    fn nested_bracket_members_are_atomic() {
        let locale = nsh_platform::Locale::c().unwrap();
        for (pattern, subject) in [
            (b"[[.-.]]".as_slice(), b"-".as_slice()),
            (b"[[.].]]", b"]"),
            (b"[[=-=]]", b"-"),
            (b"[[=]=]]", b"]"),
            (b"[[:alpha:]]", b"a"),
        ] {
            assert!(matches(&locale, pattern, subject));
        }
    }

    #[test]
    fn bracket_members_preserve_pattern_continuation() {
        let locale = nsh_platform::Locale::c().unwrap();
        assert!(matches(&locale, b"[[.-.]]x", b"-x"));
        assert!(matches(&locale, b"*[[=]=]]", b"prefix]"));
        assert!(matches(&locale, b"[![:digit:]]", b"a"));
        assert!(!matches(&locale, b"[![:digit:]]", b"7"));
        assert!(!matches(&locale, b"[[.zz.]]", b"zz"));
    }
}
