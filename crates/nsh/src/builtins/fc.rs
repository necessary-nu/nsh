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
//! `fc -s old=new` parses the substitution operand without mutating the
//! expanded argument. The reference splits its `argv` storage in place, but
//! that incidental write is neither POSIX behavior nor part of nsh's command
//! model.

// [spec:nsh:req:idiom.operation-modes]
use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use bstr::{BStr, BString, ByteSlice};
use nsh_platform::NativeStrExt as _;
use std::fs::File;
use std::io::Write;
use std::path::PathBuf;

use crate::editor::{HistoryEvent, history_active, history_mut, record_history_line};
use crate::expand::ExpandedField;
use crate::output::OutputDestination;

/// max recursions through fc
const MAX_HISTORY_LOOPS: usize = 4;
/// default editor *should* be $EDITOR
const DEFAULT_EDITOR: &[u8] = b"ed";

/// What the option scan found: the five flags `fc` reads afterwards.
///
/// Extracted from `histcmd` because it is a self-contained phase -- the
/// scan ends and nothing after it looks at an option again -- and because
/// `histcmd` is long enough without it.
struct Flags {
    editor: Option<BString>,
    list: bool,
    suppress_numbers: bool,
    reverse: bool,
    substitute: bool,
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
fn scan_options(shell: &mut crate::context::Shell, args: &[&BStr]) -> Result<Flags, Error> {
    let mut flags = Flags {
        editor: None,
        list: false,
        suppress_numbers: false,
        reverse: false,
        substitute: false,
        operand_start: 1,
    };

    let mut index = 1;
    while index < args.len() {
        let word = args[index];
        if is_event_number(word) || word.first() != Some(&b'-') || word.len() == 1 {
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
                            return Err(shell
                                .diagnostics()
                                .shell_error(b"option -e expects argument"));
                        };
                        flags.editor = Some(BString::from(*argument));
                    }
                    option = word.len();
                }
                b'l' => flags.list = true,
                b'n' => flags.suppress_numbers = true,
                b'r' => flags.reverse = true,
                b's' => flags.substitute = true,
                unknown => {
                    let mut message = b"unknown option: -".to_vec();
                    message.push(unknown);
                    return Err(shell.diagnostics().shell_error(&message));
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
// [spec:dash:sem:histedit.histcmd-fn]
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
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut fields: Vec<ExpandedField> = args
        .iter()
        .map(|word| ExpandedField::from_bytes(word))
        .collect();
    run_fields(shell, &mut fields)
}

fn split_substitution_operand(word: &BStr) -> Option<(BString, BString)> {
    let at = word.find_byte(b'=')?;
    Some((BString::from(&word[..at]), BString::from(&word[at + 1..])))
}

/// The evaluator's entry point. It receives the owned expanded fields used by
/// the command evaluator and leaves them intact while parsing `old=new`.
// [spec:posix:req:builtin.fc.edit-and-reexecute]
// [spec:posix:req:builtin.fc.exit-status]
// [spec:nsh:sem:idiom.specified-defects+1]
pub(crate) fn run_fields(shell: &mut Shell, fields: &mut [ExpandedField]) -> Result<Flow, Error> {
    let args = crate::builtins::args(fields);
    let flags = scan_options(shell, &args)?;
    drop(args);
    let mut operand_start = flags.operand_start;
    let mut editor = flags.editor;
    let mut list = flags.list;
    let suppress_numbers = flags.suppress_numbers;
    let reverse = flags.reverse;
    let mut substitute = flags.substitute;
    let mut editfile: Option<PathBuf> = None;
    let mut edit_file: Option<File> = None;
    // The `(void) &var` statements at src/histedit.c:196-210 exist only to
    // stop GCC keeping those variables in registers, where longjmp could
    // clobber them; they have no Rust equivalent.

    if !history_active(shell) {
        return Err(shell.diagnostics().shell_error(b"history not active"));
    }
    let discard_input_entry = crate::input::current_input_frame(&mut shell.input).uses_stdin();

    /*
     * If executing...
     *
     * The C arms a handler here (`if (setjmp(jmploc.loc)) { ... }`) and
     * leaves it installed for *the whole rest of the function* — there is
     * no `out:` label, and `handler` is deliberately not restored on the
     * normal path, which leaves it dangling into a returned frame. That
     * existed only while handlers were translated literally; there are none
     * now, so the hazard is gone. What survives is
     * the shape: the guarded body is a closure, and what the C's non-zero
     * arm did is the code after the call.
     */
    let executing = !list || editor.is_some() || substitute;
    if executing {
        list = false;
        editfile = None;
        /*
         * Catch interrupts to reset active counter and
         * cleanup temp files.
         */
    }
    let mut body = || {
        let mut result_status = crate::status::ExitStatus::SUCCESS;
        if executing {
            shell.editor.fc_depth += 1;
            if shell.editor.fc_depth > MAX_HISTORY_LOOPS {
                shell.editor.fc_depth = 0;
                shell.display_history = false;
                return Err(shell
                    .diagnostics()
                    .shell_error(b"called recursively too many times"));
            }
            /*
             * Set editor.
             */
            if !substitute {
                if editor.is_none() {
                    editor = crate::variables::lookup_bytes(shell, BStr::new(b"FCEDIT"))
                        .or_else(|| crate::variables::lookup_bytes(shell, BStr::new(b"EDITOR")));
                    editor.get_or_insert_with(|| BString::from(DEFAULT_EDITOR));
                }
                if editor
                    .as_ref()
                    .is_some_and(|value| value.as_slice() == b"-")
                {
                    substitute = true;
                    editor = None;
                }
            }
        }

        /*
         * If -s is specified, accept [old=new] first only
         */
        let mut pattern: Option<BString> = None;
        let mut replacement = BString::default();
        if substitute {
            if let Some(field) = fields.get(operand_start) {
                if let Some((old, new)) = split_substitution_operand(field.as_bstr()) {
                    pattern = Some(old);
                    replacement = new;
                    operand_start += 1;
                }
            }
            if fields.len().saturating_sub(operand_start) >= 2 {
                return Err(shell.diagnostics().shell_error(b"too many args"));
            }
        }

        let operands: Vec<BString> = fields[operand_start..]
            .iter()
            .map(|field| BString::from(field.as_bstr()))
            .collect();

        /*
         * determine [first] and [last]
         */
        let (firststr, laststr) = match operands.as_slice() {
            [] => (
                BStr::new(if list { &b"-16"[..] } else { &b"-1"[..] }),
                BStr::new(b"-1"),
            ),
            [first] => (
                BStr::new(first.as_slice()),
                if list {
                    BStr::new(b"-1")
                } else {
                    BStr::new(first.as_slice())
                },
            ),
            [first, last] => (BStr::new(first.as_slice()), BStr::new(last.as_slice())),
            _ => {
                return Err(shell.diagnostics().shell_error(b"too many args"));
            }
        };
        /*
         * Turn into event numbers.
         */
        let mut first = resolve_history_event(shell, firststr, false)?;
        let mut last = resolve_history_event(shell, laststr, true)?;

        if reverse {
            core::mem::swap(&mut first, &mut last);
        }
        let editing = editor.is_some();
        let mut run_selected_events =
            |shell: &mut Shell| -> Result<Result<Option<crate::status::ExitStatus>, Flow>, Error> {
                if editing {
                    let Ok((file, path)) = nsh_platform::create_temporary_file("nsh-fc") else {
                        return Err(shell
                            .diagnostics()
                            .shell_error(b"can't create temporary file"));
                    };
                    editfile = Some(path);
                    edit_file = Some(file);
                }

                // Snapshot the semantic range before `evalstring` can re-enter
                // the shell and mutate history.
                let events = history_mut(shell)
                    .map(|history| history.range(first, last))
                    .unwrap_or_default();
                if !list && discard_input_entry {
                    if let Some(history) = history_mut(shell) {
                        history.discard_input_entry();
                    }
                }
                for event in events {
                    if list {
                        let record = listing_record(&event, !suppress_numbers);
                        shell.write_output(OutputDestination::Stdout, &record)?;
                        continue;
                    }

                    let line = replace_history_text(
                        BStr::new(event.line.as_slice()),
                        &mut pattern,
                        BStr::new(replacement.as_slice()),
                    );
                    if substitute {
                        if shell.display_history {
                            shell.write_output(OutputDestination::Stderr, &line)?;
                        }
                        if history_active(shell) {
                            record_history_line(shell, &line, true, false);
                        }
                        match crate::evaluation::evaluate_string(
                            shell,
                            BStr::new(line.as_slice()),
                            crate::evaluation::EvaluationContext::DEFAULT,
                        )? {
                            Flow::Done(status) => result_status = status,
                            control => return Ok(Err(control)),
                        }
                        break;
                    }

                    let file = edit_file
                        .as_mut()
                        .expect("fc edit file must exist while an editor is selected");
                    file.write_all(&line).map_err(|error| {
                        let message = format!(
                            "can't write temporary file: {}",
                            shell.locale.error_message(&error)
                        );
                        shell.diagnostics().shell_error(message.as_bytes())
                    })?;
                }

                let Some(editor) = &editor else {
                    return Ok(Ok(None));
                };
                let path = editfile.as_ref().expect("fc created an edit file");
                let file_bytes = path.to_shell_bytes();
                let mut command = Vec::with_capacity(editor.len() + file_bytes.len() + 1);
                command.extend_from_slice(editor);
                command.push(b' ');
                command.extend_from_slice(&file_bytes);

                drop(edit_file.take());
                /* XXX - should use no JC command */
                match crate::evaluation::evaluate_string(
                    shell,
                    BStr::new(&command),
                    crate::evaluation::EvaluationContext::DEFAULT,
                )? {
                    Flow::Done(status) => Ok(Ok(Some(status))),
                    control => Ok(Err(control)),
                }
            };

        let selected = if editing {
            crate::error::with_interrupts_deferred(shell, |shell| run_selected_events(shell))
        } else {
            run_selected_events(shell)
        }?;
        let editor_status = match selected {
            Ok(status) => status,
            Err(control) => return Ok(control),
        };

        if let Some(editor_status) = editor_status {
            let path = editfile.as_ref().expect("fc created an edit file").clone();
            let file_bytes = path.to_shell_bytes();
            if editor_status.success() {
                let edited = nsh_platform::read_path(&path).map_err(|error| {
                    let mut message = b"can't read temporary file ".to_vec();
                    message.extend_from_slice(&file_bytes);
                    message.extend_from_slice(b": ");
                    message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
                    shell.diagnostics().shell_error(&message)
                })?;
                if let Some(path) = editfile.take() {
                    cleanup_edit_file(&path);
                }
                if edited
                    .iter()
                    .any(|byte| !matches!(*byte, b' ' | b'\t' | b'\n'))
                    && history_active(shell)
                {
                    record_history_line(shell, &edited, true, false);
                }
                result_status = crate::evaluation::flow!(crate::evaluation::evaluate_string(
                    shell,
                    BStr::new(&edited),
                    crate::evaluation::EvaluationContext::DEFAULT,
                ));
            } else {
                result_status = editor_status;
                if let Some(path) = editfile.take() {
                    cleanup_edit_file(&path);
                }
            }
        }

        if !list && shell.editor.fc_depth > 0 {
            shell.editor.fc_depth -= 1;
        }
        if shell.display_history {
            shell.display_history = false;
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
            shell.editor.fc_depth = 0;
            drop(edit_file.take());
            if let Some(path) = editfile.take() {
                cleanup_edit_file(&path);
            }
            return outcome;
        }
        return outcome;
    } else {
        /* `fc -l`: the C runs the same tail with no handler installed, so
         * an error here propagates to whatever frame is outermost. */
        crate::evaluation::flow!(body());
    }
    Ok(Flow::Done((0).into()))
}

fn cleanup_edit_file(path: &std::path::Path) {
    if nsh_platform::remove_file(path).is_err() {
        // Cleanup cannot replace the editor or evaluation outcome being returned.
    }
}

/// Write one listed history event with a tab before its first command line
/// and every continuation line.
// [spec:posix:req:builtin.fc.stdout-list-format]
fn listing_record(event: &HistoryEvent, numbered: bool) -> Vec<u8> {
    let mut output = Vec::new();
    if numbered {
        write!(&mut output, "{}\t", event.number).expect("writing to a Vec cannot fail");
    } else {
        output.push(b'\t');
    }
    if event.line.is_empty() {
        output.push(b'\n');
        return output;
    }
    for (index, line) in event
        .line
        .split_inclusive(|byte| *byte == b'\n')
        .enumerate()
    {
        if index != 0 {
            output.push(b'\t');
        }
        output.extend_from_slice(line);
        if !line.ends_with(b"\n") {
            output.push(b'\n');
        }
    }
    output
}

// [spec:dash:sem:histedit.fc-replace-fn]
//
// The owned result makes both its byte length and lifetime explicit before
// the caller hands it to `evalstring`.
fn replace_history_text(hay: &BStr, pattern: &mut Option<BString>, replacement: &BStr) -> BString {
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

// [spec:dash:sem:histedit.not-fcnumber-fn]
// [spec:dash:sem:myhistedit.not-fcnumber-fn]
pub fn is_event_number(word: &BStr) -> bool {
    let digits = word.strip_prefix(b"-").unwrap_or(word);
    !digits.is_empty() && digits.iter().all(u8::is_ascii_digit)
}

// [spec:dash:sem:histedit.str-to-event-fn]
// [spec:dash:sem:myhistedit.str-to-event-fn]
pub fn resolve_history_event(
    shell: &mut crate::context::Shell,
    word: &BStr,
    use_previous_for_missing_last: bool,
) -> Result<i32, Error> {
    let mut number_bytes = word;
    let mut relative = false;
    match word.first().copied() {
        Some(b'-') => {
            relative = true;
            number_bytes = BStr::new(&word[1..]);
        }
        Some(b'+') => {
            number_bytes = BStr::new(&word[1..]);
        }
        _ => {}
    }
    let numeric = crate::number::parse_decimal(number_bytes);
    let event: Option<HistoryEvent> = if let Some(number) = numeric {
        let event_number = number.min(i32::MAX as u64) as i32;
        if relative {
            history_mut(shell).and_then(|history| {
                usize::try_from(event_number)
                    .ok()
                    .and_then(|offset| history.relative(offset))
                    .or_else(|| history.oldest())
            })
        } else {
            history_mut(shell).and_then(|history| {
                history.numbered(event_number).or_else(|| {
                    if use_previous_for_missing_last {
                        history.relative(1)
                    } else {
                        history.oldest()
                    }
                })
            })
        }
    } else {
        history_mut(shell).and_then(|history| history.prefixed(word))
    };

    match event {
        Some(event) => Ok(event.number),
        None if numeric.is_some() => {
            let mut message = b"history number ".to_vec();
            message.extend_from_slice(word);
            message.extend_from_slice(b" not found (internal error)");
            Err(shell.diagnostics().shell_error(&message))
        }
        None => {
            let mut message = b"history pattern not found: ".to_vec();
            message.extend_from_slice(word);
            Err(shell.diagnostics().shell_error(&message))
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
        is_event_number(BStr::new(s.as_bytes()))
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
            replace_history_text(BStr::new(b"aa aa"), &mut pattern, BStr::new(b"bb")),
            BString::from("bb aa"),
        );
        assert_eq!(
            replace_history_text(BStr::new(b"aa"), &mut pattern, BStr::new(b"bb")),
            BString::from("aa"),
        );
        assert!(pattern.is_none());
    }

    #[test]
    fn an_empty_substitution_pattern_matches_nothing() {
        let mut pattern = Some(BString::from(""));
        assert_eq!(
            replace_history_text(BStr::new(b"abc"), &mut pattern, BStr::new(b"x")),
            BString::from("abc"),
        );
        assert_eq!(pattern, Some(BString::from("")));
    }

    // [spec:nsh:sem:idiom.specified-defects+1/test]
    #[test]
    fn substitution_operand_stays_intact() {
        let fields = [ExpandedField::from_bytes(b"old=new")];
        let (pattern, replacement) = split_substitution_operand(fields[0].as_bstr()).unwrap();

        assert_eq!(pattern, BString::from("old"));
        assert_eq!(replacement, BString::from("new"));
        assert_eq!(fields[0].as_bstr(), BStr::new(b"old=new"));
    }

    // [spec:posix:req:builtin.fc.stdout-list-format/test]
    #[test]
    fn listing_prefixes_first_and_continued_lines() {
        let event = HistoryEvent {
            number: 12,
            line: BString::from("first\nsecond"),
        };
        let numbered = listing_record(&event, true);
        assert_eq!(numbered, b"12\tfirst\n\tsecond\n");

        let plain = listing_record(&event, false);
        assert_eq!(plain, b"\tfirst\n\tsecond\n");
    }
}
