//! `fc`.
//!
//! Port of `histcmd` and its helpers from `src/histedit.c`.
//!
//! The history list itself stays in `crate::histedit`, which is where
//! the line editor writes it; this is the command that lists, edits and
//! re-runs entries. It re-enters evaluation three ways -- `-s`, the
//! editor it spawns, and the file it reads back -- so like `eval` it
//! depends on its words not borrowing from the shell.
//!
//! `fc -s old=new` still splits that word in place because dash exposes the
//! truncated value through `$_`; the write is made through the owned field,
//! not through a fabricated `char **`.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;
use nsh_platform::NativeStrExt as _;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use crate::error::{INTOFF, INTON};
use crate::expand::strlist;
use crate::histedit::{history_active, history_mut, record_history_line};
use crate::linedit::HistoryEvent;

/// max recursions through fc
const MAXHISTLOOPS: c_int = 4;
/// default editor *should* be $EDITOR
const DEFEDITOR: &[u8] = b"ed";

/// What the option scan found: the five flags `fc` reads afterwards.
///
/// Extracted from `histcmd` because it is a self-contained phase -- the
/// scan ends and nothing after it looks at an option again -- and because
/// `histcmd` is long enough without it.
struct Flags {
    editor: Option<BString>,
    lflg: c_int,
    nflg: c_int,
    rflg: c_int,
    sflg: c_int,
    operand_start: usize,
}

/// Scan `fc` options, stopping at the first operand. A negative decimal
/// number is an operand, not an option (`fc -2`).
// [spec:posix:syn:builtin.fc.synopsis]
// [spec:posix:req:builtin.fc.utility-syntax-guidelines]
// [spec:posix:req:builtin.fc.opt-e]
// [spec:posix:req:builtin.fc.opt-l]
// [spec:posix:req:builtin.fc.opt-n]
// [spec:posix:req:builtin.fc.opt-r]
// [spec:posix:req:builtin.fc.opt-s]
fn scan_options(sh: &mut crate::context::Shell, args: &[&BStr]) -> Result<Flags, Error> {
    let mut flags = Flags {
        editor: None,
        lflg: 0,
        nflg: 0,
        rflg: 0,
        sflg: 0,
        operand_start: 1,
    };

    let mut index = 1;
    while index < args.len() {
        let word = args[index];
        if is_fc_number(word) || word.first() != Some(&b'-') || word.len() == 1 {
            break;
        }
        if word == b"--" {
            index += 1;
            break;
        }

        let mut option = 1;
        while option < word.len() {
            match word[option] {
                b'e' => {
                    if option + 1 < word.len() {
                        flags.editor = Some(BString::from(&word[option + 1..]));
                    } else {
                        index += 1;
                        let Some(argument) = args.get(index) else {
                            return Err(sh.sh_error_value(b"option -e expects argument"));
                        };
                        flags.editor = Some(BString::from(*argument));
                    }
                    option = word.len();
                }
                b'l' => flags.lflg = 1,
                b'n' => flags.nflg = 1,
                b'r' => flags.rflg = 1,
                b's' => flags.sflg = 1,
                unknown => {
                    let mut message = b"unknown option: -".to_vec();
                    message.push(unknown);
                    return Err(sh.sh_error_value(&message));
                }
            }
            option += 1;
        }
        index += 1;
    }
    flags.operand_start = index;
    Ok(flags)
}

/*
 *  This command is provided since POSIX decided to standardize
 *  the Korn shell fc command.  Oh well...
 */
// [spec:dash:def:histedit.histcmd-fn]
// [spec:dash:sem:histedit.histcmd-fn]
// [spec:dash:def:myhistedit.histcmd-fn]
// [spec:dash:sem:myhistedit.histcmd-fn]
// [spec:posix:req:xcu.output-files.tmpdir]
// [spec:posix:req:xcu.output-files.temp-file-naming]
// [spec:posix:req:xcu.output-files.temp-file-removal]
// [spec:posix:req:xcu.output-files.sigquit-bypasses-recovery]
// [spec:posix:req:builtin.fc.list-or-edit]
// [spec:posix:syn:builtin.fc.operand-first-last]
// [spec:posix:req:builtin.fc.operand-default-s]
// [spec:posix:req:builtin.fc.operand-defaults-no-s]
// [spec:posix:req:builtin.fc.operand-range]
// [spec:posix:req:builtin.fc.operand-range-clamping]
// [spec:posix:req:builtin.fc.operand-old-new]
// [spec:posix:req:builtin.fc.env-fcedit]
// [spec:posix:req:builtin.fc.env-locale]
// [spec:posix:sem:builtin.fc.env-nlspath]
// [spec:posix:req:builtin.fc.stderr]
// [spec:posix:req:builtin.fc.interfaces]
pub fn histcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut fields: Vec<strlist> = args.iter().map(|word| strlist::from_cbytes(word)).collect();
    histcmd_fields(sh, &mut fields)
}

/// The evaluator's entry point. It receives the owned expanded fields so the
/// one observable mutation made by `fc -s old=new` remains a normal indexed
/// write and `$_` sees the same truncated word dash exposes.
// [spec:posix:req:builtin.fc.edit-and-reexecute]
// [spec:posix:req:builtin.fc.exit-status]
pub(crate) fn histcmd_fields(sh: &mut Shell, fields: &mut [strlist]) -> Result<Flow, Error> {
    let args = crate::builtins::args(fields);
    let flags = scan_options(sh, &args)?;
    drop(args);
    let mut operand_start = flags.operand_start;
    let mut editor = flags.editor;
    let mut lflg: c_int = flags.lflg;
    let nflg: c_int = flags.nflg;
    let rflg: c_int = flags.rflg;
    let mut sflg: c_int = flags.sflg;
    let mut editfile: Option<PathBuf> = None;
    let mut edit_file: Option<File> = None;
    // The `(void) &var` statements at src/histedit.c:196-210 exist only to
    // stop GCC keeping those variables in registers, where longjmp could
    // clobber them; they have no Rust equivalent.

    if !history_active(sh) {
        return Err(sh.sh_error_value(b"history not active"));
    }
    let discard_input_entry = crate::input::cur_pf(sh).uses_stdin();

    /*
     * If executing...
     *
     * The C arms a handler here (`if (setjmp(jmploc.loc)) { ... }`) and
     * leaves it installed for *the whole rest of the function* — there is
     * no `out:` label, and `handler` is deliberately not restored on the
     * normal path, which leaves it dangling into a returned frame. That
     * was reproduced bug-for-bug while handlers existed; there are none
     * now, so the hazard is gone rather than preserved. What survives is
     * the shape: the guarded body is a closure, and what the C's non-zero
     * arm did is the code after the call.
     */
    let executing = lflg == 0 || editor.is_some() || sflg != 0;
    if executing {
        lflg = 0; /* ignore */
        editfile = None;
        /*
         * Catch interrupts to reset active counter and
         * cleanup temp files.
         */
    }
    let mut body = || {
        let mut result_status = crate::status::ExitStatus::SUCCESS;
        if executing {
            sh.histedit.fc_depth += 1;
            if sh.histedit.fc_depth > MAXHISTLOOPS {
                sh.histedit.fc_depth = 0;
                sh.displayhist = 0;
                return Err(sh.sh_error_value(b"called recursively too many times"));
            }
            /*
             * Set editor.
             */
            if sflg == 0 {
                if editor.is_none() {
                    editor = crate::var::lookup_bytes(sh, BStr::new(b"FCEDIT"))
                        .or_else(|| crate::var::lookup_bytes(sh, BStr::new(b"EDITOR")));
                    editor.get_or_insert_with(|| BString::from(DEFEDITOR));
                }
                if editor
                    .as_ref()
                    .is_some_and(|value| value.as_slice() == b"-")
                {
                    sflg = 1; /* no edit */
                    editor = None;
                }
            }
        }

        /*
         * If -s is specified, accept [old=new] first only
         */
        let mut pattern: Option<BString> = None;
        let mut replacement = BString::default();
        if sflg != 0 {
            if let Some(field) = fields.get_mut(operand_start) {
                let word = crate::mystring::cstr_prefix(&field.text);
                if let Some(at) = word.find_byte(b'=') {
                    pattern = Some(BString::from(&word[..at]));
                    replacement = BString::from(&word[at + 1..]);
                    field.text[at] = 0;
                    operand_start += 1;
                }
            }
            if fields.len().saturating_sub(operand_start) >= 2 {
                return Err(sh.sh_error_value(b"too many args"));
            }
        }

        let operands: Vec<BString> = fields[operand_start..]
            .iter()
            .map(|field| BString::from(crate::mystring::cstr_prefix(&field.text)))
            .collect();

        /*
         * determine [first] and [last]
         */
        let (firststr, laststr) = match operands.as_slice() {
            [] => (
                BStr::new(if lflg != 0 { &b"-16"[..] } else { &b"-1"[..] }),
                BStr::new(b"-1"),
            ),
            [first] => (
                BStr::new(first.as_slice()),
                if lflg != 0 {
                    BStr::new(b"-1")
                } else {
                    BStr::new(first.as_slice())
                },
            ),
            [first, last] => (BStr::new(first.as_slice()), BStr::new(last.as_slice())),
            _ => {
                return Err(sh.sh_error_value(b"too many args"));
            }
        };
        /*
         * Turn into event numbers.
         */
        let mut first = str_to_event(sh, firststr, 0)?;
        let mut last = str_to_event(sh, laststr, 1)?;

        if rflg != 0 {
            core::mem::swap(&mut first, &mut last);
        }
        /*
         * If editing, grab a temp file.
         */
        if editor.is_some() {
            INTOFF(sh); /* easier */
            let Ok((file, path)) = nsh_platform::create_temporary_file("nsh-fc") else {
                return Err(sh.sh_error_value(b"can't create temporary file"));
            };
            editfile = Some(path);
            edit_file = Some(file);
        }

        // Snapshot the semantic range before `evalstring` can re-enter the
        // shell and mutate history.
        let events = history_mut(sh)
            .map(|history| history.range(first, last))
            .unwrap_or_default();
        if lflg == 0 && discard_input_entry {
            if let Some(history) = history_mut(sh) {
                history.discard_input_entry();
            }
        }
        for event in events {
            if lflg != 0 {
                let _ = write_listing(sh.io.stdout(), &event, nflg == 0);
            } else {
                let line = fc_replace(
                    BStr::new(event.line.as_slice()),
                    &mut pattern,
                    BStr::new(replacement.as_slice()),
                );

                if sflg != 0 {
                    if sh.displayhist != 0 {
                        let _ = sh.io.stderr().write_all(&line);
                    }

                    if history_active(sh) {
                        record_history_line(sh, &line, true, false);
                    }

                    /* `fc -s` runs the recalled line, which can be an
                     * `exit`. It leaves through the cleanup below like
                     * everything else this frame catches. */
                    result_status = crate::eval::flow!(crate::eval::evalstring(
                        sh,
                        BStr::new(line.as_slice()),
                        0,
                    ));

                    break;
                } else {
                    let file = edit_file
                        .as_mut()
                        .expect("fc edit file must exist while an editor is selected");
                    let _ = file.write_all(&line);
                }
            }
        }
        if let Some(editor) = &editor {
            /* The C `stalloc`s `strlen(editor) + strlen(editfile) + 2` —
             * the two strings, the separating space and the terminator —
             * and lets `fccmd`'s enclosing mark release it.  `evalstring`
             * copies what it is given, so the buffer is dead as soon as
             * that call returns and can be this block's. */
            let path = editfile.as_ref().expect("fc created an edit file").clone();
            let file_bytes = path.to_shell_bytes();
            let mut editcmdbuf: Vec<u8> = Vec::with_capacity(editor.len() + file_bytes.len() + 1);
            editcmdbuf.extend_from_slice(editor);
            editcmdbuf.push(b' ');
            editcmdbuf.extend_from_slice(&file_bytes);

            drop(edit_file.take());
            /* XXX - should use no JC command */
            let editor_status =
                crate::eval::flow!(crate::eval::evalstring(sh, BStr::new(&editcmdbuf), 0,));
            INTON(sh);

            if editor_status.success() {
                let edited = nsh_platform::read_path(&path).map_err(|error| {
                    let mut message = b"can't read temporary file ".to_vec();
                    message.extend_from_slice(&file_bytes);
                    message.extend_from_slice(b": ");
                    message.extend_from_slice(sh.locale.error_message(&error).as_bytes());
                    sh.sh_error_value(&message)
                })?;
                if let Some(path) = editfile.take() {
                    let _ = nsh_platform::remove_file(&path);
                }
                if edited
                    .iter()
                    .any(|byte| !matches!(*byte, b' ' | b'\t' | b'\n'))
                    && history_active(sh)
                {
                    record_history_line(sh, &edited, true, false);
                }
                result_status =
                    crate::eval::flow!(crate::eval::evalstring(sh, BStr::new(&edited), 0,));
            } else {
                result_status = editor_status;
                if let Some(path) = editfile.take() {
                    let _ = nsh_platform::remove_file(&path);
                }
            }
        }

        if lflg == 0 && sh.histedit.fc_depth > 0 {
            sh.histedit.fc_depth -= 1;
        }
        if sh.displayhist != 0 {
            sh.displayhist = 0;
        }
        Ok(Flow::Done((result_status).into()))
    };

    if executing {
        /* The C arms a handler here so that an exception out of
         * `evalstring` still lowers `active` and unlinks the temporary
         * file -- the only filesystem side effect on any catch path in
         * the shell. Everything that used to arrive as an exception is a
         * value now, so the cleanup runs on the one path that is not a
         * plain success and the frame itself is gone. */
        let outcome = body();
        if !matches!(outcome, Ok(Flow::Done(_))) {
            sh.histedit.fc_depth = 0;
            drop(edit_file.take());
            if let Some(path) = editfile.take() {
                let _ = nsh_platform::remove_file(&path);
            }
            return outcome;
        }
        return outcome;
    } else {
        /* `fc -l`: the C runs the same tail with no handler installed, so
         * an error here propagates to whatever frame is outermost. */
        crate::eval::flow!(body());
    }
    Ok(Flow::Done((0).into()))
}

/// Write one listed history event with a tab before its first command line
/// and every continuation line. Output errors remain recorded by `Output`
/// for the builtin epilogue to fold into the exit status.
// [spec:posix:req:builtin.fc.stdout-list-format]
fn write_listing(
    output: &mut impl Write,
    event: &HistoryEvent,
    numbered: bool,
) -> std::io::Result<()> {
    if numbered {
        write!(output, "{}\t", event.number)?;
    } else {
        output.write_all(b"\t")?;
    }
    if event.line.is_empty() {
        return output.write_all(b"\n");
    }
    for (index, line) in event
        .line
        .split_inclusive(|byte| *byte == b'\n')
        .enumerate()
    {
        if index != 0 {
            output.write_all(b"\t")?;
        }
        output.write_all(line)?;
        if !line.ends_with(b"\n") {
            output.write_all(b"\n")?;
        }
    }
    Ok(())
}

// [spec:dash:def:histedit.fc-replace-fn]
// [spec:dash:sem:histedit.fc-replace-fn]
//
// The C returns `grabstackstr(dest)`, which reserves the bytes *before* the
// `STACKSTRNUL` — so the terminator sits one past the allocation and the
// caller reads it anyway. An owned string carries its own terminator, and
// returning it makes the lifetime the caller's rather than the enclosing
// stack mark's, which matters because the caller hands it to `evalstring`.
fn fc_replace(hay: &BStr, pattern: &mut Option<BString>, replacement: &BStr) -> BString {
    /* The C walks `s` a byte at a time and asks `*s == *p && strncmp(s,
     * p, plen)` at each position, which is `find`. The leading-byte test
     * is not an optimisation, though: it is also what makes an *empty*
     * pattern match nothing, because the loop only runs while `*s` is
     * non-NUL and `*p` is the NUL. `find` on an empty needle answers 0,
     * so the emptiness is checked rather than inherited. */
    let hit = pattern.as_ref().and_then(|pattern| {
        let pat = pattern.as_slice();
        if pat.is_empty() {
            None
        } else {
            hay.find(pat).map(|at| (at, pat.len()))
        }
    });

    let mut dest: BString = BString::new(Vec::new());
    match hit {
        Some((at, plen)) => {
            dest.extend_from_slice(&hay[..at]);
            dest.extend_from_slice(replacement);
            dest.extend_from_slice(&hay[at + plen..]);
            /* `so no more matches` — the C truncates the pattern in
             * place, and the buffer belongs to the caller, so the
             * suppression carries across the whole range of events and
             * not just the rest of this line. */
            *pattern = None;
        }
        None => dest.extend_from_slice(hay),
    }

    dest
}

// [spec:dash:def:histedit.not-fcnumber-fn]
// [spec:dash:sem:histedit.not-fcnumber-fn]
// [spec:dash:def:myhistedit.not-fcnumber-fn]
// [spec:dash:sem:myhistedit.not-fcnumber-fn]
pub fn is_fc_number(word: &BStr) -> bool {
    let digits = word.strip_prefix(b"-").unwrap_or(word);
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

// [spec:dash:def:histedit.str-to-event-fn]
// [spec:dash:sem:histedit.str-to-event-fn]
// [spec:dash:def:myhistedit.str-to-event-fn]
// [spec:dash:sem:myhistedit.str-to-event-fn]
pub fn str_to_event(
    sh: &mut crate::context::Shell,
    word: &BStr,
    last: c_int,
) -> Result<c_int, Error> {
    let mut number_bytes = word;
    let mut relative: c_int = 0;
    match word.first().copied() {
        Some(b'-') => {
            relative = 1;
            number_bytes = BStr::new(&word[1..]);
        }
        Some(b'+') => {
            number_bytes = BStr::new(&word[1..]);
        }
        _ => {}
    }
    let numeric = crate::mystring::decimal_digits(number_bytes);
    let event: Option<HistoryEvent> = if let Some(number) = numeric {
        let i = number.min(c_int::MAX as u64) as c_int;
        if relative != 0 {
            history_mut(sh).and_then(|history| {
                usize::try_from(i)
                    .ok()
                    .and_then(|offset| history.relative(offset))
                    .or_else(|| history.oldest())
            })
        } else {
            history_mut(sh).and_then(|history| {
                history.numbered(i).or_else(|| {
                    if last != 0 {
                        history.relative(1)
                    } else {
                        history.oldest()
                    }
                })
            })
        }
    } else {
        history_mut(sh).and_then(|history| history.prefixed(word))
    };

    match event {
        Some(event) => Ok(event.number),
        None if numeric.is_some() => {
            let mut message = b"history number ".to_vec();
            message.extend_from_slice(word);
            message.extend_from_slice(b" not found (internal error)");
            Err(sh.sh_error_value(&message))
        }
        None => {
            let mut message = b"history pattern not found: ".to_vec();
            message.extend_from_slice(word);
            Err(sh.sh_error_value(&message))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The option scan stops at the first word that could be a history
    /// number, which is what lets `fc -2` name an entry two back rather
    /// than fail as an unknown option.
    fn is_number_word(s: &str) -> bool {
        is_fc_number(BStr::new(s.as_bytes()))
    }

    #[test]
    fn a_negative_number_is_an_event() {
        assert!(is_number_word("-2"));
        assert!(is_number_word("-10"));
    }

    /// A plain number is one too.
    #[test]
    fn plain_number_is_an_event() {
        assert!(is_number_word("2"));
    }

    #[test]
    fn an_option_is_not_an_event() {
        assert!(!is_number_word("-l"));
        assert!(!is_number_word("-e"));
        assert!(!is_number_word("--"));
    }

    /// A bare `-` is not a number, and neither is a name.
    #[test]
    fn a_word_is_not_an_event() {
        assert!(!is_number_word("-"));
        assert!(!is_number_word("echo"));
    }

    #[test]
    fn substitution_replaces_only_the_first_match_in_the_range() {
        let mut pattern = Some(BString::from("aa"));
        assert_eq!(
            fc_replace(BStr::new(b"aa aa"), &mut pattern, BStr::new(b"bb")),
            BString::from("bb aa"),
        );
        assert_eq!(
            fc_replace(BStr::new(b"aa"), &mut pattern, BStr::new(b"bb")),
            BString::from("aa"),
        );
        assert!(pattern.is_none());
    }

    #[test]
    fn an_empty_substitution_pattern_matches_nothing() {
        let mut pattern = Some(BString::from(""));
        assert_eq!(
            fc_replace(BStr::new(b"abc"), &mut pattern, BStr::new(b"x")),
            BString::from("abc"),
        );
        assert_eq!(pattern, Some(BString::from("")));
    }

    // [spec:posix:req:builtin.fc.stdout-list-format/test]
    #[test]
    fn listing_prefixes_first_and_continued_lines() {
        let event = HistoryEvent {
            number: 12,
            line: BString::from("first\nsecond"),
        };
        let mut numbered = Vec::new();
        write_listing(&mut numbered, &event, true).unwrap();
        assert_eq!(numbered, b"12\tfirst\n\tsecond\n");

        let mut plain = Vec::new();
        write_listing(&mut plain, &event, false).unwrap();
        assert_eq!(plain, b"\tfirst\n\tsecond\n");
    }
}
