//! `read`.
//!
//! Port of `readcmd` from `src/miscbltin.c`.
//!
//! Splitting the line it read into fields is `crate::expand`'s
//! `ifsbreakup` -- the same field splitting an unquoted expansion gets,
//! which is why `read` honours `IFS` without knowing what `IFS` is.

use crate::context::Shell;
use crate::error::Error;
use core::ffi::{c_int, c_uint};
use std::ffi::CString;
use std::io::Write as _;

use bstr::{BStr, BString};

use crate::eval::Flow;
use crate::expand::arglist;

/* glibc <limits.h> */
const MB_LEN_MAX: usize = 16;

/// `readcmd`'s `CHECKSTRSPACE((MB_LEN_MAX > 16 ? MB_LEN_MAX : 16) + 4, p)`.
///
/// `getmbc` no longer writes through a cursor this frame makes room for --
/// it has its own scratch and hands back the bytes to append -- so this is
/// not a reservation any more. It survives as the assertion bound on what
/// `getmbc` may return, and the number is still the C's for the reason it
/// always was: with `mode` 0 it puts the character's bytes at `out + 2`
/// and the closing length and marker at `out + 2 + ml` and `out + 3 + ml`,
/// which for `ml == MB_LEN_MAX` is the twentieth byte and not one fewer.
const READ_MBSLOP: usize = (if MB_LEN_MAX > 16 { MB_LEN_MAX } else { 16 }) + 4;

// ---------------------------------------------------------------------

/** handle one line of the read command.
 *  more fields than variables -> remainder shall be part of last variable.
 *  less fields than variables -> remaining variables unset.
 *
 *  @param line complete line of input
 *  @param ac argument count
 *  @param ap argument (variable) list
 *  @param len length of line including trailing '\0'
 */

// [spec:dash:def:miscbltin.readcmd-handle-line-fn]
// [spec:dash:sem:miscbltin.readcmd-handle-line-fn]
fn readcmd_handle_line(sh: &mut Shell, line: &mut BString, names: &[&BStr]) -> Result<(), Error> {
    let mut arglist: arglist = arglist::new();

    /* `s = grabstackstr(s)`.  The C is handed the cursor one *past* the
     * terminator and turns it into the block's base, which both names the
     * line and reserves it so that `ifsbreakup`'s `stalloc`s land above it.
     * An owned line is already its own base and there is nothing to reserve;
     * the fields `ifsbreakup` builds copy out of it rather than pointing
     * into it, so the line only has to outlive that one call. */
    debug_assert!(!line.is_empty(), "readcmd always pushes the terminator");

    crate::expand::ifsbreakup(sh, line, names.len() as c_int, &mut arglist);
    crate::expand::ifsfree(&mut sh.expand);

    /* The C walks the names and the fields with two cursors that advance
     * together, so the field for a name is the field at its index; a name
     * past the last field is the "nullify remaining arguments" case. */
    for (index, name) in names.iter().enumerate() {
        match arglist.list.get_mut(index) {
            None => {
                crate::var::set_bytes(sh, name, Some(BStr::new(b"")), 0)?;
            }
            Some(field) => {
                /* set variable to field */
                field.rmescapes();
                crate::var::set_bytes(
                    sh,
                    name,
                    Some(crate::mystring::cstr_prefix(&field.text)),
                    0,
                )?;
            }
        }
    }
    Ok(())
}

/*
 * The read builtin.  The -e option causes backslashes to escape the
 * following character. The -p option followed by an argument prompts
 * with the argument.
 *
 * This uses unbuffered input, which may be avoidable in some cases.
 */

// [spec:dash:def:miscbltin.readcmd-fn]
// [spec:dash:sem:miscbltin.readcmd-fn]
pub fn readcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut prompt: Option<CString>;
    let mut startloc: c_int = 0;
    let mut newloc: c_int = 0;
    let mut status: c_int;
    let mut rflag: c_int;

    rflag = 0;
    prompt = None;
    let mut opts = crate::options::Options::new(args);
    while let Some(i) = opts.next(sh, b"p:r")? {
        if i == b'p' {
            prompt = Some(crate::shell::cstring(opts.arg()));
        } else {
            rflag = 1;
        }
    }
    if let Some(prompt) = &prompt {
        if nsh_platform::is_terminal(sh.streams.stdin) {
            let _ = sh.io.stderr().write_all(prompt.as_bytes());
        }
    }
    let names = opts.operands();
    if names.is_empty() {
        return Err(sh.sh_error_value(b"arg count"));
    }

    status = 0;
    /* `STARTSTACKSTR(p)`.  The line is an owned buffer, so the C's cursor is
     * its length and `stackblock()` its base: every `p - stackblock()` below
     * is `line.len()`, and `USTPUTC` is `push`. */
    let mut line = BString::default();

    crate::input::pushstdin(sh);

    /* The C body is a `for (;;)` entered by `goto start`, with the
     * labels `put`, `record` and `start` inside it. The label graph is
     * reproduced with an explicit program counter. */
    const L_BODY: c_int = 0;
    const L_PUT: c_int = 1;
    const L_RECORD: c_int = 2;
    const L_START: c_int = 3;

    let mut pc: c_int = L_START; /* goto start */
    let mut c: c_int = 0;

    loop {
        if pc == L_BODY {
            let ml: c_uint;

            /* `CHECKSTRSPACE((MB_LEN_MAX > 16 ? MB_LEN_MAX : 16) + 4, p)`
             * bought the C the room `getmbc` writes into. `getmbc` has its
             * own scratch now, so there is nothing to reserve on its
             * behalf -- the reservation left here would be a guess about
             * another function's internals. */
            c = crate::input::pgetc(sh)?;
            if c == crate::syntax::PEOF {
                status = 1;
                break;
            }
            if c == '\0' as c_int {
                pc = L_BODY;
                continue;
            }
            let mut scratch: [u8; crate::parser::MBSLOP] = [0; crate::parser::MBSLOP];
            ml = crate::parser::getmbc(sh, c, &mut scratch, 0)?;
            if ml != 0 {
                /* `p += ml` is the commit of what `getmbc` wrote; a zero
                 * return commits nothing, and the scribble it left behind
                 * stays in the scratch rather than in `line`. */
                debug_assert!(ml as usize <= READ_MBSLOP);
                line.extend_from_slice(&scratch[..ml as usize]);
                pc = L_RECORD; /* goto record */
            } else if newloc >= startloc {
                if c == '\n' as c_int {
                    pc = L_RECORD; /* goto record */
                } else {
                    pc = L_PUT; /* goto put */
                }
            } else if rflag == 0 && c == '\\' as c_int {
                newloc = line.len() as c_int;
                pc = L_BODY;
                continue;
            } else if c == '\n' as c_int {
                break;
            } else {
                pc = L_PUT; /* fall through to put: */
            }
        }
        if pc == L_PUT {
            // put:
            /* `strchr` matches the terminator too, so the set the C
             * scans is `cqchars[1..]` *including* its NUL -- which is
             * how a NUL read from the input gets escaped. */
            if crate::mystring::cqchars[1..]
                .iter()
                .any(|&b| b as c_int == c)
            {
                /* USTPUTC(CTLESC, p) */
                line.push(crate::parser::CTLESC as u8);
            }
            /* USTPUTC(c, p) */
            line.push(c as u8);
            pc = L_RECORD;
        }
        if pc == L_RECORD {
            // record:
            if newloc >= startloc {
                crate::expand::recordregion(&mut sh.expand, startloc, newloc, 0);
                pc = L_START;
            } else {
                pc = L_BODY; /* end of the for body */
                continue;
            }
        }
        if pc == L_START {
            // start:
            startloc = line.len() as c_int;
            newloc = startloc - 1;
            pc = L_BODY; /* end of the for body */
        }
    }
    crate::input::popfile(sh);
    crate::expand::recordregion(&mut sh.expand, startloc, line.len() as c_int, 0);
    /* `STACKSTRNUL(p)` writes the terminator without advancing, and the call
     * below then passes `p + 1` — the length *including* it.  Pushing is both
     * halves at once. */
    line.push(b'\0');
    readcmd_handle_line(sh, &mut line, names)?;
    Ok(Flow::Done(status))
}
