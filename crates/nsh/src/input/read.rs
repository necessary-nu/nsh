//! Getting bytes out of the source and in front of the parser, one unit
//! at a time.
//!
//! A frame says where input comes from; this is what makes a byte appear.
//! It is layered because the shell's standard input is shared with the
//! commands the script runs: `preadfd` asks for no more than it may keep,
//! so a `read` further down the script still sees the bytes the shell did
//! not eat, and where the descriptor cannot be rewound afterwards the
//! bytes are teed through a pipe instead of being lost. `preadbuffer`
//! drops NULs and stops at the newline, which is also where a line is
//! offered to history. `pgetc` hands out one unit and is the single place
//! a byte crosses into the parser, which is why the token log is written
//! there and nowhere else.
//!
//! The buffer state that outlives a single read is here too -- the
//! seek-back in `flush_input`, the EOF latch `reset_input` clears -- since
//! it is only meaningful to the code that filled the buffer in the first
//! place.

use super::overlay::{clear_input_overlays, pop_string_input};
use super::*;

impl Shell {
    /// Drain the abandoned input record before the command loop continues.
    pub(crate) fn discard_interrupted_input(&mut self) {
        pop_all_input_frames(self);

        /* At least one character past the pushback window has been consumed. */
        let floor_index = self.input.floor_index;
        let floor_frame = input_frame_at(&mut self.input, floor_index);
        let mut input = if floor_frame.position > floor_frame.unread_count {
            InputUnit::Byte(text(floor_frame)[floor_frame.position - floor_frame.unread_count - 1])
        } else {
            InputUnit::EndOfInput
        };
        while !input.is(b'\n')
            && input != InputUnit::EndOfInput
            && !crate::error::interrupt_pending()
        {
            match read_input_unit(self) {
                Ok(next) => input = next,
                Err(error) => {
                    self.status = error.status();
                    drop(error);
                    break;
                }
            }
        }
    }
}

// [spec:dash:sem:input.input-init-fn]
// [spec:nsh:def:idiom.logical-descriptors]
pub fn initialize_input(shell: &mut Shell) {
    let standard_input = shell.descriptors.get(LogicalDescriptor::STDIN);
    if let Some(canonical) = standard_input
        .as_ref()
        .and_then(nsh_platform::terminal_canonical_mode)
    {
        shell.input.standard_input_is_terminal = Some(true);
        shell.input.standard_input_state.bufferable = canonical;
        shell.input.standard_input_state.seekable = false;
    } else {
        shell.input.standard_input_is_terminal = Some(false);
        shell.input.standard_input_state.seekable = standard_input
            .as_ref()
            .is_some_and(nsh_platform::fd_is_seekable);
        shell.input.standard_input_state.bufferable = shell.input.standard_input_state.seekable;
    }
}

// [spec:dash:sem:input.stdin-bufferable-fn]
fn standard_input_is_bufferable(shell: &mut Shell) -> bool {
    if shell.input.standard_input_is_terminal.is_none() {
        initialize_input(shell);
    }
    shell.input.standard_input_state.bufferable
}

// [spec:dash:sem:input.flush-tee-fn]
fn flush_tee(shell: &mut crate::context::Shell, request: usize, mut pending: usize) {
    let mut scratch = [0_u8; INPUT_BUFFER_SIZE];
    let standard_input = shell.descriptors.get(LogicalDescriptor::STDIN);
    while pending > 0 {
        let length = request.min(pending);
        let Some(standard_input) = &standard_input else {
            break;
        };
        match nsh_platform::read_once(standard_input, &mut scratch[..length]) {
            Ok(count) if count > 0 => pending -= count,
            _ => break,
        }
    }
}

// [spec:dash:sem:input.stdin-tee-fn]
// [spec:nsh:req:idiom.platform-errors]
fn tee_standard_input(shell: &mut Shell, request: usize) -> Result<std::io::Result<usize>, Error> {
    if shell.input.standard_input_state.pipe.is_none() {
        let (pipe, _) = crate::redirection::create_pipe(shell, false)?;
        let read = crate::redirection::move_descriptor_above(shell, pipe.read)?;
        let write = crate::redirection::move_descriptor_above(shell, pipe.write)?;
        shell.input.standard_input_state.pipe = Some(crate::redirection::Pipe { read, write });
    }

    if let Some(pending) = shell.input.standard_input_state.pending {
        flush_tee(shell, request, pending);
    }

    let pipe = shell
        .input
        .standard_input_state
        .pipe
        .as_ref()
        .expect("stdin tee pipe exists");
    let result = if nsh_platform::supports_tee() {
        match shell.descriptors.get(LogicalDescriptor::STDIN) {
            Some(standard_input) => nsh_platform::tee(&standard_input, &pipe.write, request),
            None => Err(nsh_platform::platform_error(
                nsh_platform::PlatformErrorKind::BadDescriptor,
            )),
        }
    } else {
        Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
    };
    shell.input.standard_input_state.pending = result.as_ref().ok().copied();
    Ok(result)
}

/*
 * Read one item from the script.
 * Nul characters in the input are silently discarded by the normal entry
 * point; `read -d ''` uses the preserving entry point below.
 */

// [spec:dash:sem:input.pgetc-fn]
// [spec:nsh:req:idiom.lexer-tokens]
pub fn read_input_unit(shell: &mut crate::context::Shell) -> Result<InputUnit, Error> {
    read_input_unit_with_mode(shell, false)
}

/// Read one input byte without applying the parser's normal NUL filtering.
///
/// This is intentionally narrower than [`pgetc`]: shell input remains text,
/// while `read -d ''` needs to observe the NUL that terminates its record.
pub(crate) fn read_input_unit_preserving_nul(
    shell: &mut crate::context::Shell,
) -> Result<InputUnit, Error> {
    read_input_unit_with_mode(shell, true)
}

fn read_input_unit_with_mode(
    shell: &mut crate::context::Shell,
    preserve_nul: bool,
) -> Result<InputUnit, Error> {
    let input: InputUnit;
    /* Re-derived after everything that can push a level, because that is
     * what moves the frames; the C reloads the same global for the same
     * reason. */
    let mut input_frame = current_input_frame(&mut shell.input);

    if !input_frame.deferred_overlays.is_empty() {
        clear_input_overlays(shell);
        input_frame = current_input_frame(&mut shell.input);
    }

    let unit = 'read_next_unit: loop {
        if input_frame.unread_count != 0 {
            let unread_count = input_frame.unread_count;
            input_frame.unread_count -= 1;

            break InputUnit::Byte(text(input_frame)[input_frame.position - unread_count]);
        }

        if input_frame.line_remaining > 0 {
            input_frame.line_remaining -= 1;
            input = InputUnit::Byte(text(input_frame)[input_frame.position]);
            input_frame.position += 1;
        } else if !input_frame.overlays.is_empty() {
            pop_string_input(shell);
            /* The freestrings call must be delayed til the next
             * input read so the alias-end boundary remains observable.
             */
            input_frame = current_input_frame(&mut shell.input);
            continue 'read_next_unit;
        } else {
            input = refill_input_buffer(shell, preserve_nul)?;
        }

        break input;
    };

    /* The one place a byte crosses from the input sources into the parser,
     * and so the one place the record of what was read can be complete. */
    // [spec:nsh:def:idiom.token-stream]
    if let InputUnit::Byte(byte) = unit {
        let frame = shell.input.current;
        shell.input.tokens.record(frame, byte);
    }
    Ok(unit)
}

// [spec:dash:sem:input.pgetc-eoa-fn]
pub fn read_input_unit_or_alias_end(shell: &mut crate::context::Shell) -> Result<InputUnit, Error> {
    let input_frame = current_input_frame(&mut shell.input);
    if !input_frame.overlays.is_empty()
        && input_frame.line_remaining == 0
        && input_frame.overlays[input_frame.overlays.len() - 1]
            .alias_name
            .is_some()
    {
        Ok(InputUnit::EndOfAlias)
    } else {
        read_input_unit(shell)
    }
}

// [spec:dash:sem:input.stdin-clear-nonblock-fn]
fn clear_standard_input_nonblocking(shell: &mut crate::context::Shell) -> bool {
    shell
        .descriptors
        .get(LogicalDescriptor::STDIN)
        .is_some_and(|descriptor| nsh_platform::set_nonblocking(&descriptor, false).is_ok())
}

/// What one read of the input source produced.
///
/// Three outcomes and not two, because a read a signal cut short is
/// neither bytes nor an end of input, and both mistakes have been made
/// here. Answering "no bytes" for it ended the session on the editor's
/// path -- `2a46bd5` -- because no bytes is what a real end of input
/// says. Retrying instead is this path's mirror of that: the read blocks
/// again inside the deferral scope, so an interrupt that has a value
/// waiting cannot reach the polling boundary that would deliver it, and
/// it is taken one line later against a line the user has already typed.
// [spec:nsh:req:interactive.signal-does-not-end-the-session]
enum DescriptorRead {
    /// Bytes were read, or zero for an end of input.
    Bytes(usize),
    /// A signal with something to deliver cut the read short. The caller
    /// has to leave the deferral scope for the boundary poll to take it.
    Interrupted,
}

// [spec:dash:sem:input.preadfd-fn]
// The retry below is the shell's answer to a signal that arrived while it was
// waiting for a line, and the line editor's read owes the same answer.
// [spec:nsh:req:interactive.signal-does-not-end-the-session]
// [spec:posix:req:sh.stdin-used-only-if]
// [spec:posix:req:sh.stdin-no-read-ahead]
// [spec:posix:req:sh.stdin-blocking-reads]
// [spec:posix:req:sh.input-file-contents]
// [spec:posix:req:sh.input-file-blank-or-comments]
// [spec:posix:req:xcurel.file-contents-nbytes]
// [spec:posix:sem:xcurel.file-contents-read-error]
// [spec:posix:req:exit.unrecoverable-read-error]
fn read_input_descriptor(shell: &mut crate::context::Shell) -> Result<DescriptorRead, Error> {
    let uses_stdin = current_input_frame(&mut shell.input).uses_stdin;
    let dot_operand = current_input_frame(&mut shell.input).dot_operand;
    let mut use_standard_input_tee: bool;
    let buffered = remaining_buffer_bytes(current_input_frame(&mut shell.input));
    let unread_count = current_input_frame(&mut shell.input)
        .position
        .min(MAX_UNREAD_UNITS);

    /* Slide the retained pushback window and the partial line already read
     * down to the front, so the read lands after both. */
    {
        let input_frame = current_input_frame(&mut shell.input);
        let retained_start = input_frame.position - unread_count;
        input_frame
            .buffer
            .copy_within(retained_start..retained_start + unread_count + buffered, 0);
        input_frame.position = unread_count;
    }
    /* The C's `buf` walks past both; here it is the offset the read fills
     * from, and it survives a nested `pushfile` because it is not a
     * pointer. */
    let buffer_offset = unread_count + buffered;

    let mut requested = INPUT_BUFFER_SIZE - buffered;
    if requested == 0 {
        return Ok(DescriptorRead::Bytes(0));
    }

    /* The C's `fd == 0` means "this parse file is the shell's standard
     * input", which is the condition for line editing and for teeing --
     * not descriptor 0 for its own sake. */
    use_standard_input_tee =
        uses_stdin && !crate::editor::editing_active(shell) && !standard_input_is_bufferable(shell);

    'retry: loop {
        if uses_stdin && crate::editor::editing_active(shell) {
            /* `docs/api-design.md` §5.5: nothing the shell hands to a
             * callee may borrow from the shell, and `read_edit_line`
             * takes the shell too. The buffer is moved out, filled, and
             * put back -- a `Vec`, so that is a pointer swap rather than
             * a copy. Nothing can reach this frame's buffer while it is
             * out, which is the same thing the borrow used to assert. */
            let mut buffer = core::mem::take(&mut current_input_frame(&mut shell.input).buffer);
            let result = crate::editor::read_edit_line(
                shell,
                &mut buffer[buffer_offset..buffer_offset + requested],
            );
            current_input_frame(&mut shell.input).buffer = buffer;
            return match result {
                Ok(count) => Ok(DescriptorRead::Bytes(count)),
                Err(error) => {
                    let mut message = BString::from("read error: ");
                    message.extend_from_slice(error.to_string().as_bytes());
                    let failure = Error::unrecoverable_read(
                        shell.evaluation.diagnostic_line,
                        &message,
                        dot_operand,
                    );
                    Err(shell.diagnostics().report(failure))
                }
            };
        }

        let mut reading_tee = false;
        let mut immediate_error = None;
        if use_standard_input_tee {
            match tee_standard_input(shell, requested)? {
                Ok(count) => {
                    requested = count;
                    reading_tee = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                    use_standard_input_tee = false;
                    requested = 1;
                }
                Err(error) => immediate_error = Some(error),
            }
        }

        if requested > 0 || immediate_error.is_some() {
            let source = if reading_tee {
                None
            } else if uses_stdin {
                shell.descriptors.get(LogicalDescriptor::STDIN)
            } else {
                current_input_frame(&mut shell.input)
                    .owned_descriptor
                    .clone()
            };
            let mut scratch = [0_u8; INPUT_BUFFER_SIZE];
            let result = if let Some(error) = immediate_error {
                Err(error)
            } else if reading_tee {
                let pipe = shell
                    .input
                    .standard_input_state
                    .pipe
                    .as_ref()
                    .expect("stdin tee pipe exists");
                nsh_platform::read_once(&pipe.read, &mut scratch[..requested])
            } else if let Some(source) = &source {
                nsh_platform::read_once(source, &mut scratch[..requested])
            } else {
                Err(nsh_platform::platform_error(
                    nsh_platform::PlatformErrorKind::BadDescriptor,
                ))
            };
            let count = match result {
                Ok(count) => count,
                Err(error) => {
                    let error_kind = error.kind();
                    if error_kind == std::io::ErrorKind::Interrupted {
                        /* An interrupt already has a value waiting for the
                         * polling boundary above, and this read is inside the
                         * deferral scope that boundary ends -- so retrying
                         * here does not delay the delivery by a moment, it
                         * delays it by a whole line, and the line it is then
                         * taken against is one the user typed after the `^C`.
                         * dash retries in the same place and is right to,
                         * because `onint` had already longjmped out of the
                         * read from inside the handler;
                         * `[dec:nsh:errors-are-values]` removed the jump and
                         * left this the only way out.
                         *
                         * Every other signal has nothing for that boundary --
                         * a `SIGCHLD`, a trapped signal whose action runs at
                         * the next `dotrap` -- and for those the retry is
                         * still the answer, which is what keeps a half-typed
                         * line and runs a trap action after the next line
                         * rather than in place of it. */
                        // [spec:nsh:req:interactive.signal-does-not-end-the-session]
                        if crate::error::interrupt_pending() {
                            return Ok(DescriptorRead::Interrupted);
                        }
                        if !(input_frame_at(&mut shell.input, 0).previous.is_some()
                            && crate::signal_inbox::signals().pending_signal().is_some())
                        {
                            continue 'retry;
                        }
                    }
                    if uses_stdin
                        && error_kind == std::io::ErrorKind::WouldBlock
                        && clear_standard_input_nonblocking(shell)
                    {
                        shell.write_output(
                            crate::output::OutputDestination::Stderr,
                            b"sh: turning off NDELAY mode\n",
                        )?;
                        continue 'retry;
                    }
                    /* The interactive prompt's read, and the one place the C had
                     * no synchronous alternative: `onsig` used to deliver from
                     * inside the handler and the longjmp abandoned this read
                     * where it stood. Now the read returns EINTR and this is
                     * where the shell looks.
                     *
                     * The C's condition -- retry unless a *nested* input has a
                     * signal pending -- is kept underneath, because it is about
                     * something else: abandoning a here-document or a `.` file
                     * when a trapped signal arrives. */
                    if let Some(error) = crate::error::poll_interrupt(shell.interrupt_context()) {
                        return Err(error);
                    }
                    let mut message = BString::from("read error: ");
                    message.extend_from_slice(shell.locale.error_message(&error).as_bytes());
                    let failure = Error::unrecoverable_read(
                        shell.evaluation.diagnostic_line,
                        &message,
                        dot_operand,
                    );
                    return Err(shell.diagnostics().report(failure));
                }
            };
            current_input_frame(&mut shell.input).buffer[buffer_offset..buffer_offset + count]
                .copy_from_slice(&scratch[..count]);
            return Ok(DescriptorRead::Bytes(count));
        }
        return Ok(DescriptorRead::Bytes(0));
    }
}

/*
 * Refill the input buffer and return the next input character:
 *
 * 1) If a string was pushed back on the input, pop it;
 * 2) If we are reading from a string we can't refill the buffer, return EOF.
 * 3) If there is more stuff in this buffer, use it else call read to fill it.
 * 4) Process input up to the next newline, normally deleting nul characters.
 */

/// What one refill produced.
///
/// The third value is the one that matters: a read a signal cut short is
/// not an end of input, and the frame must not latch EOF for it. It says
/// only that this attempt has nothing to hand back and that the caller
/// should leave the deferral scope so the interrupt behind it can be
/// taken at the boundary below.
// [spec:nsh:req:interactive.signal-does-not-end-the-session]
enum Refill {
    Line(Vec<u8>),
    EndOfInput,
    Interrupted,
}

// [spec:dash:sem:input.preadbuffer-fn]
fn refill_input_buffer(
    shell: &mut crate::context::Shell,
    preserve_nul: bool,
) -> Result<InputUnit, Error> {
    loop {
        match refill_once(shell, preserve_nul)? {
            /* The read was abandoned so that an interrupt could be taken
             * at the boundary inside `refill_once`, and the boundary
             * found nothing to take -- so there is nothing to deliver and
             * nothing has been read, and the answer is to read again.
             * That is the same answer `read_effect` gives the editor's
             * path for a signal with no value behind it, and it is the
             * half of this that must not become "no line": no line is
             * what an end of input says. */
            // [spec:nsh:req:interactive.signal-does-not-end-the-session]
            None => continue,
            Some(unit) => return Ok(unit),
        }
    }
}

/// One attempt at a line, ending at the polling boundary.
///
/// `None` is "nothing read and nothing to deliver", which only a signal
/// produces; every other outcome is a unit for the parser.
// [spec:dash:sem:input.preadbuffer-fn]
fn refill_once(
    shell: &mut crate::context::Shell,
    preserve_nul: bool,
) -> Result<Option<InputUnit>, Error> {
    let first = shell.input.prompt == Some(PromptKind::Primary);

    if current_input_frame(&mut shell.input).eof_latched {
        /* eof: */
        current_input_frame(&mut shell.input).eof_observed = true;
        return Ok(Some(InputUnit::EndOfInput));
    }
    shell.flush_output()?;

    let buffered = crate::error::with_interrupts_deferred(shell, |shell| {
        let mut line_end = current_input_frame(&mut shell.input).position;
        let mut has_content = !first;
        let mut remaining = remaining_buffer_bytes(current_input_frame(&mut shell.input));
        let mut preserve_buffer = false;

        'outer: loop {
            if remaining == 0 {
                /* again: */
                let preserved_count = line_end - current_input_frame(&mut shell.input).position;
                set_remaining_buffer_bytes(current_input_frame(&mut shell.input), preserved_count);
                remaining = match read_input_descriptor(shell)? {
                    DescriptorRead::Bytes(count) => count,
                    /* Leaving here is the delivery: the scope this runs
                     * in answers `poll_interrupt` with `None` by
                     * construction, so the only way an interrupt taken
                     * during the read reaches a poll site is for the read
                     * to stop and the scope to end. Whatever partial line
                     * the buffer held goes with it, exactly as dash's
                     * longjmp out of `onint` discarded it. */
                    // [spec:nsh:req:interactive.signal-does-not-end-the-session]
                    DescriptorRead::Interrupted => return Ok(Refill::Interrupted),
                };
                line_end = current_input_frame(&mut shell.input).position + preserved_count;
                if remaining == 0 {
                    current_input_frame(&mut shell.input).line_remaining = 0;
                    set_remaining_buffer_bytes(current_input_frame(&mut shell.input), 0);
                    if preserved_count != 0 {
                        preserve_buffer = true;
                        break 'outer;
                    }
                    return Ok(Refill::EndOfInput);
                }
            }

            /* delete nul characters */
            loop {
                remaining -= 1;
                let byte = current_input_frame(&mut shell.input).buffer[line_end];

                if byte == 0 && !preserve_nul {
                    let input_frame = current_input_frame(&mut shell.input);
                    input_frame
                        .buffer
                        .copy_within(line_end + 1..line_end + 1 + remaining, line_end);
                    /* goto check */
                } else {
                    line_end += 1;

                    if byte == b'\n' {
                        let previous = {
                            let input_frame = current_input_frame(&mut shell.input);
                            (line_end - input_frame.position >= 2)
                                .then(|| input_frame.buffer[line_end - 2])
                        };
                        if nsh_platform::input_newline_width(previous) == 2 {
                            // Keep the unread tail contiguous when the platform
                            // treats the preceding CR as part of this newline.
                            let input_frame = current_input_frame(&mut shell.input);
                            input_frame
                                .buffer
                                .copy_within(line_end - 1..line_end + remaining, line_end - 2);
                            line_end -= 1;
                        }
                        break 'outer;
                    }
                    if byte != b'\t' && byte != b' ' {
                        has_content = true;
                    }
                }

                /* check: */
                if remaining == 0 {
                    continue 'outer;
                }
            }
        }

        if !preserve_buffer {
            set_remaining_buffer_bytes(current_input_frame(&mut shell.input), remaining);
        }

        {
            let input_frame = current_input_frame(&mut shell.input);
            input_frame.line_remaining = (line_end - input_frame.position).saturating_sub(1);
        }

        let line = {
            let input_frame = current_input_frame(&mut shell.input);
            input_frame.buffer[input_frame.position..line_end].to_vec()
        };

        // A forced-interactive command file is the shell's top-level input even
        // though it is not descriptor 0. Retain it, but not nested `source`,
        // dot, eval, or command-substitution frames.
        // [spec:nsh:req:compat.smoosh.history-builtin]
        let top_level_history_input = current_input_frame(&mut shell.input).uses_stdin
            || shell.input.current == shell.input.floor_index;
        if top_level_history_input
            && crate::editor::history_active(shell)
            && !shell.options.enabled(ShellOption::NoLog)
            && has_content
        {
            let bytes = {
                let input_frame = current_input_frame(&mut shell.input);
                &input_frame.buffer[input_frame.position..line_end]
            };
            let bytes = bytes.to_vec();
            crate::editor::record_history_line(shell, &bytes, first, true);
        }
        Ok::<_, Error>(Refill::Line(line))
    })?;

    /* A read interrupted while this scope was active becomes deliverable at
     * this explicit polling boundary, after the prior deferral depth has been
     * restored. */
    if let Some(error) = crate::error::poll_interrupt(shell.interrupt_context()) {
        return Err(error);
    }

    let line = match buffered {
        Refill::Line(line) => line,
        Refill::EndOfInput => {
            let input_frame = current_input_frame(&mut shell.input);
            input_frame.eof_latched = true;
            input_frame.eof_observed = true;
            return Ok(Some(InputUnit::EndOfInput));
        }
        Refill::Interrupted => return Ok(None),
    };

    if shell.options.enabled(ShellOption::Verbose) {
        shell.write_output(crate::output::OutputDestination::Stderr, &line)?;
    }

    let input_frame = current_input_frame(&mut shell.input);
    let byte = input_frame.buffer[input_frame.position];
    input_frame.position += 1;
    Ok(Some(InputUnit::Byte(byte)))
}

// [spec:dash:sem:input.pungetn-fn]
pub fn unread_input_units(shell: &mut Shell, count: usize) {
    current_input_frame(&mut shell.input).unread_count += count;
    // [spec:nsh:def:idiom.token-stream]
    let frame = shell.input.current;
    shell.input.tokens.unrecord(frame, count);
}

/*
 * Undo a call to pgetc.  Only two characters may be pushed back.
 * End-of-input may be pushed back.
 */

// [spec:dash:sem:input.pungetc-fn]
pub fn unread_input_unit(shell: &mut Shell) {
    let observed_eof = current_input_frame(&mut shell.input).eof_observed;
    if !observed_eof {
        unread_input_units(shell, 1);
    }
    current_input_frame(&mut shell.input).eof_observed = false;
}

impl Shell {
    /// Discard buffered standard input while preserving the underlying source.
    // [spec:dash:sem:input.flush-input-fn]
    // [spec:dash:sem:init.postexitreset-fn]
    pub(crate) fn flush_input(&mut self) {
        let base = input_frame_at(&mut self.input, 0);
        let left = base.line_remaining + remaining_buffer_bytes(base);
        crate::error::with_interrupts_deferred(self, |shell| {
            if shell.input.standard_input_state.seekable && left != 0 {
                if let Some(standard_input) = shell.descriptors.get(LogicalDescriptor::STDIN) {
                    let offset = i64::try_from(left).unwrap_or(i64::MAX);
                    if nsh_platform::seek_relative(&standard_input, -offset).is_err() {
                        // The descriptor stopped supporting rewind; future reads use tee state.
                        shell.input.standard_input_state.seekable = false;
                    }
                }
            } else if let Some(pending) = shell
                .input
                .standard_input_state
                .pending
                .filter(|pending| *pending > left)
            {
                flush_tee(shell, INPUT_BUFFER_SIZE, pending - left);
                shell.input.standard_input_state.pending = None;
            }
            let base = input_frame_at(&mut shell.input, 0);
            base.line_remaining = 0;
            set_remaining_buffer_bytes(base, 0);
        });
    }
}

// [spec:dash:sem:input.reset-input-fn]
pub fn reset_input(shell: &mut Shell) {
    shell.input.standard_input_is_terminal = None;
    let base = input_frame_at(&mut shell.input, 0);
    base.eof_latched = false;
    base.eof_observed = false;
    shell.flush_input();
}

/// Let the interactive command loop try standard input again after EOF.
///
/// The parser latches EOF on its input frame so files and strings cannot be
/// polled forever. `ignoreeof` is the one boundary that deliberately asks a
/// terminal for a new record, without discarding any bytes that arrived in
/// the meantime.
pub(crate) fn rearm_stdin_after_eof(shell: &mut Shell) {
    let base = input_frame_at(&mut shell.input, 0);
    base.eof_latched = false;
    base.eof_observed = false;
}
