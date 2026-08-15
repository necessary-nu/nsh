/*
 * Shell pattern matching: `case` patterns, the `${v#pat}` family, and the
 * glob matcher, ported from dash's `expand.c`.
 *
 * A pure function over two byte strings with no shell state, which is why
 * it is a file rather than three hundred more lines of `expand.rs`. The
 * unsafe surface is the two `CStr::from_ptr` calls in `pmatch` and
 * `fnmatch`; everything below `pmatch_bytes` is safe and indexed.
 *
 * See plan/decisions/owned-data.md, "What this cost in the port: the
 * pattern matcher", for the one comparison here that is decided rather
 * than transcribed.
 */

use bstr::ByteSlice;
use core::mem;
use libc::{c_char, c_int, c_uint, size_t, wchar_t};
use std::ffi::CStr;

use crate::mystring::{byte_at, ncmp_eq_at, slice_from};
use crate::expand::{
    C_BANG, C_CARET, C_COLON, C_LBRACKET, C_MINUS, C_NUL, C_QUESTION, C_RBRACKET, C_STAR, CTLESC,
    CTLMBCHAR, FNMATCH_IS_ENABLED, iswctype, mbnext_bytes, mbrtowc, preglob, wctype, wctype_t,
    wint_t,
};

/*
 * Returns true if the pattern matches the string.
 */

// [spec:dash:def:expand.patmatch-fn]
// [spec:dash:sem:expand.patmatch-fn]
#[inline]
pub(crate) unsafe fn patmatch(pattern: *mut c_char, string: *const c_char) -> c_int {
    pmatch(preglob(pattern, 0, None), string)
}

// [spec:dash:def:expand.ccmatch-fn]
// [spec:dash:sem:expand.ccmatch-fn]
//
// Returns whether the character matched and, separately, where the pattern
// continues.  The C signalled the second through an out-parameter `char **r`
// left NULL when `p` did not open a well-formed `[:class:]`; `Option<usize>`
// is that signal as a value.  The order the C fixed is kept: the
// continuation is set as soon as the class *name* is good, before the
// character is tested, so a non-matching `[:alpha:]` is still consumed
// rather than re-read as ordinary bracket members.
#[inline(never)]
fn ccmatch_bytes(p: &[u8], mbc: &[u8], ml: usize) -> (bool, Option<usize>) {
    if byte_at(p, 0) != C_COLON {
        return (false, None);
    }
    let body = slice_from(p, 1);
    let close = match body.find(b":]") {
        Some(at) => at,
        None => return (false, None),
    };

    /* The C wrote a NUL over the `:` of `:]`, called `wctype`, and put the
     * `:` back -- it needed a C string and the only one to hand was the
     * pattern itself.  Copying the name costs an allocation on a path that
     * runs once per `[:class:]`, and buys a pattern that `pmatch` never has
     * to be able to write to, which is what lets it take `&[u8]`. */
    let mut name = body[..close].to_vec();
    name.push(0);
    let type_ = unsafe { wctype(name.as_ptr() as *const c_char) };
    if type_ == 0 as wctype_t {
        return (false, None);
    }

    /* Past the `:` skipped above, and past the `:]` just found. */
    let r = 1 + close + 2;

    /* `ml` is what the caller measured of the string's character.  The C
     * passed that count straight to `mbrtowc`; clamping it to what the
     * slice holds can only change a case where the C read past the
     * string's terminator, and a short read fails the `!= ml` test below
     * exactly as a malformed character does. */
    let mut wc: wchar_t = 0;
    let mut mbst: libc::mbstate_t = unsafe { mem::zeroed() };
    let n = ml.min(mbc.len());
    let got = unsafe { mbrtowc(&mut wc, mbc.as_ptr() as *const c_char, n as size_t, &mut mbst) };
    if got != ml as size_t {
        return (false, Some(r));
    }

    (unsafe { iswctype(wc as wint_t, type_) } != 0, Some(r))
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

// [spec:dash:def:expand.pmatch-fn]
// [spec:dash:sem:expand.pmatch-fn]
//
// The unsafe half is this adapter and nothing else: it measures the two C
// strings once and hands the matcher slices that carry their terminators.
// `fnmatch` is the one arm that still wants pointers, and it is libc's.
pub(crate) unsafe fn pmatch(pattern: *mut c_char, string: *const c_char) -> c_int {
    if FNMATCH_IS_ENABLED {
        return (libc::fnmatch(pattern, string, 0) == 0) as c_int;
    }
    pmatch_bytes(
        CStr::from_ptr(pattern).to_bytes_with_nul(),
        CStr::from_ptr(string).to_bytes_with_nul(),
    ) as c_int
}

// The matcher.  `pi`/`qi` are the C's `p`/`q`; every `p++` is `pi += 1` and
// the recursion takes the two tails.  Nothing here is unsafe.
fn pmatch_bytes(pattern: &[u8], string: &[u8]) -> bool {
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
                        if pmatch_bytes(slice_from(pattern, pi), slice_from(string, qi)) {
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
                                let ml = if mb > 1 { mb - 2 } else { mb } as usize;
                                let (hit, r) =
                                    ccmatch_bytes(slice_from(pattern, pi), slice_from(string, qi), ml);
                                found |= hit as c_int;
                                if let Some(r) = r {
                                    pi += r;
                                    break 'cont; /* continue */
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
                    qi += mb as usize;
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
