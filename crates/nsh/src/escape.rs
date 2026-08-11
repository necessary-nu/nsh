//! The C escape decoder, shared by `echo` and the parser.
//!
//! Port of the escape half of `src/bltin/printf.c`; `conv_escape` is
//! declared in `system.h` and shared with `parser.c` (which calls it with
//! `mbchar = true`), so it carries both the `printf.*` and the `system.*`
//! rule ids.
//!
//! It lives here rather than inside `builtins::echo` because two callers
//! is what shared means: a decoder the parser needs cannot sit inside a
//! builtin without the parser depending on one.
//!
//! Cross-module signatures assumed (see the port report):
//!   * Nothing from `crate::memalloc`.  The one buffer this file deals in
//!     is owned, so `USTPUTC`/`STADJUST` below are the two macros
//!     `conv_escape` still needs to write through a bare cursor and touch
//!     no region.
//!   * `crate::parser::{CTLESC, CTLMBCHAR}` (src/parser.h:43,47)
//!   * `crate::syntax::{sqsyntax, SYNBASE, CCTL}` -- the generated syntax
//!     tables; `SQSYNTAX` is `sqsyntax + SYNBASE` with `SYNBASE` 129
//!     (src/mksyntax.c:147,152)

use core::ptr;

use libc::{c_char, c_int, c_uint};

// ---------------------------------------------------------------------
// src/memalloc.h:78-97 -- the two stack-string macros `conv_escape` still
// needs.  Both are pure cursor arithmetic; neither touches the region.
// ---------------------------------------------------------------------

///
/// The C guards both of its calls with `CHECKSTRSPACE(4, cp)`, and 4 is not
/// enough. `\U0001F600` takes the `len == 4` arm, which writes the four
/// encoded bytes at `out + mboff` and *then* `USTPUTC(len, out);
/// USTPUTC(CTLMBCHAR, out)` at `out + len` and `out + len + 1` — bytes 5 and
/// 6 with `mbchar` false, bytes 7 and 8 with it true. It returns before them,
/// so they are scratch the next write overwrites; but they are written, and
/// in a 504-byte stack block nobody notices. Spare capacity is exactly as
/// long as it is reserved to be, so the port has to reserve what the C
/// writes rather than what the C says.
///
/// `parser.rs` reaches the `mbchar` arm and its `CHECKSTRSPACE(MAX(MB_LEN_MAX,
/// 16) + 7, out)` already covers 8.
pub const CONV_ESCAPE_SLOP: usize = 8;

/// `#define USTPUTC(c, p) (*p++ = (c))`
macro_rules! USTPUTC {
    ($c:expr, $p:ident) => {{
        *$p = $c as c_char;
        $p = $p.add(1);
    }};
}

/// `#define STADJUST(amount, p) (p += (amount))`
macro_rules! STADJUST {
    ($amount:expr, $p:ident) => {
        $p = $p.offset($amount as isize)
    };
}


/// `#define isodigit(c) ((c) >= '0' && (c) <= '7')`
#[inline]
pub(crate) fn isodigit(c: c_int) -> bool {
    c >= b'0' as c_int && c <= b'7' as c_int
}

/// `#define octtobin(c) ((c) - '0')`
#[inline]
fn octtobin(c: c_int) -> c_int {
    c - b'0' as c_int
}

// Character constants used as `match` patterns; Rust cannot cast inside a
// pattern the way a C `case` label can.
const CH_BACKSLASH: c_int = b'\\' as c_int;
const CH_X: c_int = b'x' as c_int;
const CH_U: c_int = b'u' as c_int;
const CH_A: c_int = b'a' as c_int;
const CH_B: c_int = b'b' as c_int;
const CH_F: c_int = b'f' as c_int;
const CH_E: c_int = b'e' as c_int;
const CH_N: c_int = b'n' as c_int;
const CH_R: c_int = b'r' as c_int;
const CH_T: c_int = b't' as c_int;
const CH_V: c_int = b'v' as c_int;

/*
 * Print "standard" escape characters
 */
// [spec:dash:def:printf.conv-escape-fn]
// [spec:dash:sem:printf.conv-escape-fn]
// [spec:dash:def:system.conv-escape-fn]
// [spec:dash:sem:system.conv-escape-fn]
pub unsafe fn conv_escape(str0: *mut c_char, out0: *mut c_char, mbchar: bool) -> c_uint {
    let mut out: *mut c_char = out0;
    let mut str: *mut c_char = str0;
    let mut value: c_uint;
    let och: c_int;
    let mut ch: c_int;

    ch = *str as c_int;
    value = ch as c_uint;

    // The C switch's `default:` label falls into `check_value:`, which falls
    // into `case '\\':`; `case 'x':` falls into `hex:`, which can jump back
    // to `check_value:` or forward to `out_noput:`. The three flags below
    // encode those gotos; the blocks stay in source order except that the
    // `default:` arm has to move last, as Rust requires.
    let mut goto_hex = false;
    let mut goto_check_value = false;
    let mut goto_backslash = false;

    'out_noput: {
        'sw: {
            match ch {
                CH_BACKSLASH => {
                    goto_backslash = true;
                }

                CH_X => {
                    ch = 2;
                    goto_hex = true;
                }

                CH_U => {
                    ch = 4;
                    goto_hex = true;
                }

                CH_A /* alert */ | CH_B /* backspace */ | CH_F /* form-feed */ => {
                    value = value.wrapping_sub(b'a' as c_uint);
                    value = value.wrapping_add(0x07 /* '\a' */);
                }

                CH_E => value = 0o33,   /* <ESC> */
                CH_N => value = 0o12,   /* newline */
                CH_R => value = 0o15,   /* carriage-return */
                CH_T => value = 0o11,   /* tab */
                CH_V => value = 0o13,   /* vertical-tab */

                _ => {
                    // default:
                    if mbchar && (ch == b'"' as c_int || ch == b'\'' as c_int) {
                        break 'sw;
                    }

                    if ch == b'U' as c_int {
                        ch = 8;
                        goto_hex = true;
                        break 'sw;
                    }

                    value = b'\\' as c_uint;

                    if isodigit(ch) {
                        ch = 3;
                        value = 0;
                        loop {
                            value <<= 3;
                            value = value.wrapping_add(octtobin(*str as c_int) as c_uint);
                            str = str.add(1);
                            ch -= 1;
                            if !(ch != 0 && isodigit(*str as c_int)) {
                                break;
                            }
                        }
                    }

                    str = str.sub(1);

                    goto_check_value = true;
                }
            }
        }

        if goto_hex {
            // hex:
            och = ch;
            value = 0;
            loop {
                str = str.add(1);
                let c: c_int = *str as c_int;
                let d: c_int;

                if c >= b'0' as c_int && c <= b'9' as c_int {
                    d = c - b'0' as c_int;
                } else {
                    let cl: c_int;

                    cl = c & !0x20;
                    if cl >= b'A' as c_int && cl <= b'F' as c_int {
                        d = cl - b'A' as c_int + 10;
                    } else {
                        str = str.sub(1);
                        break;
                    }
                }

                value <<= 4;
                value = value.wrapping_add(d as c_uint);

                ch -= 1;
                if ch == 0 {
                    break;
                }
            }

            if och <= 2 {
                goto_check_value = true;
            } else if value < 0x80 {
                goto_check_value = true;
            } else {
                if value < 0x110000 {
                    let mboff: c_int = (mbchar as c_int - 1) * 2;
                    let uni: c_uint = value;
                    let len: c_int;

                    value = 0x80 << 8 | (value & 0xfc0) << 2 | 0x80 | (value & 0x3f);

                    if uni < 0x800 {
                        value |= 0x40 << 8;
                        len = 2;
                    } else {
                        value |= 0x80 << 16 | (uni & 0x3f000) << 4;
                        if uni < 0x10000 {
                            value |= 0x60 << 16;
                            len = 3;
                        } else {
                            value |= 0xf0 << 24 | (uni & !0x3ffff) << 6;
                            len = 4;
                        }
                    }

                    // htonl(): host order to big-endian, i.e. UTF-8 order.
                    value = (value << ((4 - len) * 8)).to_be();

                    USTPUTC!(crate::parser::CTLMBCHAR, out);
                    USTPUTC!(len, out);
                    STADJUST!(mboff, out);
                    ptr::copy_nonoverlapping(
                        &value as *const c_uint as *const u8,
                        out as *mut u8,
                        4,
                    );
                    STADJUST!(len, out);
                    USTPUTC!(len, out);
                    USTPUTC!(crate::parser::CTLMBCHAR, out);
                    STADJUST!(mboff, out);

                    /* The highest byte the block above touches, counted from
                     * `out0`: the four encoded bytes end at `2 + mboff + 3`
                     * and the closing pair at `2 + mboff + len + 1`.  It is
                     * past the length this returns, so every caller has to
                     * have reserved `CONV_ESCAPE_SLOP` and not the C's 4. */
                    let highest = 2 + mboff + if len + 1 > 3 { len + 1 } else { 3 };
                    debug_assert!(highest >= 0 && (highest as usize) < CONV_ESCAPE_SLOP);
                }

                break 'out_noput; /* goto out_noput */
            }
        }

        if goto_check_value {
            // check_value:
            // if (SQSYNTAX[(signed char)value] != CCTL) break;
            if crate::syntax::sqsyntax
                [(crate::syntax::SYNBASE as isize + (value as i8) as isize) as usize]
                as c_int
                != crate::syntax::CCTL as c_int
            {
                goto_backslash = false;
            } else {
                /* fall through */
                goto_backslash = true;
            }
        }

        if goto_backslash {
            // case '\\':
            if mbchar {
                USTPUTC!(crate::parser::CTLESC, out);
            }
        }

        USTPUTC!(value, out);
    }

    // out_noput:
    str = str.add(1);
    (out.offset_from(out0) as c_uint) | ((str.offset_from(str0) as c_uint) << 4)
}
