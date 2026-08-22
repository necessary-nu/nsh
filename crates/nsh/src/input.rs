//! Literal port of `src/input.c` / `src/input.h`.
//! Rules: `docs/spec/port/src/input.md`.
//!
//! The C's three allocations here — the `parsefile` node, its `IBUFSIZ`
//! buffer and the `strpush` node — are owned Rust values. The frame stack is
//! `FRAMES`, addressed by index rather than by `prev` pointer, because a
//! `Vec` moves its elements and the C compares frame *identity*
//! (`unwindfiles(stop)`, `pf == &basepf`). `nextc` is an index into
//! whichever text the level is reading -- the frame's own `buf`, or the
//! string the innermost `strpush` pushed in front of it.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString};
use nsh_platform::Descriptor;

use crate::descriptors::LogicalDescriptor;
use crate::options::ShellOption;
// [spec:nsh:def:idiom.shell-options]
use crate::syntax::InputUnit;

/// `MB_LEN_MAX > 16 ? MB_LEN_MAX : 16` — 16 on glibc.
pub const MAX_UNREAD_UNITS: usize = 16;
/// stdio's `BUFSIZ`.
pub const INPUT_BUFFER_SIZE: usize = 8192;
pub const INPUT_STORAGE_SIZE: usize = INPUT_BUFFER_SIZE + MAX_UNREAD_UNITS + 1;

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct InputFileOptions {
    pub push: bool,
    pub missing_ok: bool,
    pub dot_operand: bool,
}

impl InputFileOptions {
    pub const CURRENT: Self = Self {
        push: false,
        missing_ok: false,
        dot_operand: false,
    };
    pub const PUSHED: Self = Self {
        push: true,
        ..Self::CURRENT
    };
    pub const OPTIONAL_PUSHED: Self = Self {
        missing_ok: true,
        ..Self::PUSHED
    };
    pub const DOT: Self = Self {
        dot_operand: true,
        ..Self::PUSHED
    };
}

/// The C's `struct strpush`.
///
/// `prev` is the `Vec` order and `basestrpush` has no reason to exist, so
/// both are gone. `string` is a copy of the pushed text; in the C it is
/// `ap->name`, the *whole* `name=value` allocation that `ap->val` points
/// into, held so that redefining an alias mid-expansion does not free the
/// text being read. See `plan/decisions/owned-data.md`.
pub struct InputOverlay {
    /// `sp->prevstring`, as a cursor into the text that was current
    pub previous_position: usize,
    pub previous_line_remaining: usize,
    /// if push was associated with an alias
    pub alias_name: Option<BString>,
    /// the complete pushed text
    pub string: Vec<u8>,
    /// `sp->spfree`: the pending-free chain hidden while this string is read
    pub deferred_overlays: Vec<InputOverlay>,
    /// Number of outstanding calls to pungetc.
    pub unread_count: usize,
}

/*
 * The parsefile structure pointed to by the global variable parsefile
 * contains information about the current file being read.
 */

/// The C's `struct parsefile`. `prev` is an index into the frame stack, not
/// a pointer, so that `Vec` growth cannot invalidate it.
pub struct InputFrame {
    /// preceding file on stack
    pub previous: Option<usize>,
    /// current line
    pub line_number: i32,
    /// Whether this frame reads logical descriptor 0. Keeping the logical
    /// identity separate from the backing descriptor is what lets a later
    /// redirection change stdin without invalidating this parse frame.
    uses_stdin: bool,
    /// Ownership when this frame opened the descriptor itself.
    owned_descriptor: Option<crate::descriptors::SharedDescriptor>,
    /// Whether this file is the operand of the `.` special built-in.
    dot_operand: bool,
    /// number of chars left in this line
    pub line_remaining: usize,
    /// Do not read again once the source reaches EOF.
    pub eof_latched: bool,
    /// The most recent read observed that EOF boundary.
    pub eof_observed: bool,
    /// next char in the current text
    pub position: usize,
    /// input buffer, or the whole text when this level reads a string
    pub buffer: Vec<u8>,
    /// for pushing strings at this level
    pub overlays: Vec<InputOverlay>,
    /// Delay freeing so we can stop nested aliases.
    pub deferred_overlays: Vec<InputOverlay>,
    /// number of chars left in this buffer
    pub buffer_remaining: usize,
    /// Number of outstanding calls to pungetc.
    pub unread_count: usize,
}

impl InputFrame {
    /// What `memset(pf, 0, sizeof(*pf))` produced.
    pub const EMPTY: InputFrame = InputFrame {
        previous: None,
        line_number: 0,
        uses_stdin: false,
        owned_descriptor: None,
        dot_operand: false,
        line_remaining: 0,
        eof_latched: false,
        eof_observed: false,
        position: 0,
        buffer: Vec::new(),
        overlays: Vec::new(),
        deferred_overlays: Vec::new(),
        buffer_remaining: 0,
        unread_count: 0,
    };

    /// Whether evaluation is still attached to interactive standard input,
    /// rather than a sourced file or an `eval` string.
    pub(crate) const fn uses_stdin(&self) -> bool {
        self.uses_stdin
    }
}

pub struct StandardInputState {
    pub seekable: bool,
    pub pipe: Option<crate::redirection::Pipe>,
    pub pending: Option<usize>,
    pub bufferable: bool,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum PromptKind {
    Primary,
    Continuation,
}

/// The shell's input: where it is reading from, and what it has read.
///
/// `docs/api-design.md` §5 assigns `input.rs`'s statics and `parser.rs`'s
/// eleven parser globals to one `input` field. This is that field, being
/// filled in slices: the independent scalars first, the frame stack next.
///
/// `stdin_state`, `prompt` and `stdin_is_tty` are `pub(crate)` --
/// three unrelated scalars, each meaning one thing, read from
/// `options.rs` and `parser.rs`; accessors would be noise, by the
/// criterion the evaluator's state settled. `alias_boundary` is private
/// because it is a bit this module produces and hands over through
/// [`InputStack::take_alias_boundary`], which is the invariant worth keeping.
// [spec:posix:req:xcu.stdin.input-file-restrictions-apply]
// [spec:posix:req:xcu.stdin.terminal-background]
// [spec:posix:req:xcu.stdin.env-independence]
// [spec:posix:req:xcu.input-files.eight-bit-transparency]
// [spec:posix:req:xcu.input-files.seekable-file-offset]
// [spec:posix:req:xcu.input-files.document-size-limits]
// [spec:posix:req:xcu.input-files.text-file-and-line-continuation]
pub struct InputStack {
    /// `basepf` — the top-level input file, frame 0. Never popped.
    base: InputFrame,
    /// `FRAMES` — the pushed frames. `frames[i]` is frame index `i + 1`.
    frames: Vec<InputFrame>,
    /// `toppf` — how far `popallfiles` unwinds.
    floor_index: usize,
    /// `parsefile` — the current frame, by index. See `cur_pf`.
    current: usize,
    /// Here-document delimiters waiting for their bodies at the next
    /// grammar newline.
    pub(crate) pending_here_documents: Vec<crate::parser::PendingHereDocument>,
    /// Bodies read for the syntax tree currently under construction. They
    /// are moved into that tree before it crosses a parser boundary.
    pub(crate) completed_here_documents: Vec<crate::nodes::WordNode>,
    /// `doprompt` — whether to prompt before the next read.
    pub(crate) prompt_before_read: bool,
    /// `needprompt` — interactive and at the start of a line.
    pub(crate) prompt_needed: bool,
    /// `lasttoken` — the last token read.
    pub(crate) last_token: crate::parser::TokenKind,
    /// Whether the last word token contained quoting. Kept beside
    /// `lasttoken` so pushing a token back preserves the complete token;
    /// ordinary parser code receives it as part of `readtoken`'s result.
    pub(crate) last_token_quoted: bool,
    /// Whether a blank separated this token from the one before it.
    /// Bash's compound array assignment requires `name=` and `(` to be
    /// adjacent, so `a= (1 2)` stays a syntax error.
    pub(crate) last_token_after_blank: bool,
    /// `tokpushback` — one token of lookahead, pushed back.
    pub(crate) token_pushed_back: bool,
    /// The option-derived dialect captured at the current parser entry.
    /// It is a snapshot, not a second setting: every top-level parse unit
    /// replaces it from this shell's [`crate::options::ShellOptions`].
    parse_dialect: crate::options::Dialect,
    // [spec:nsh:def:idiom.word-ir]
    /// The last parsed word, including its substitutions at their lexical
    /// positions.
    pub(crate) word: crate::word::ParsedWord,
    /// The redirection operator the last token opened. Its required operand
    /// is parsed before it becomes an AST redirection.
    pub(crate) pending_redirection: Option<crate::parser::PendingRedirection>,
    /// `heredoc` — the here-document the last token opened.
    pub(crate) pending_here_document: Option<crate::parser::PendingHereDocument>,
    /// `stdin_state` — how the shell's standard input behaves.
    pub(crate) standard_input_state: StandardInputState,
    /// The prompt selected for the next interactive read.
    pub(crate) prompt: Option<PromptKind>,
    /// Whether standard input is a terminal, once queried.
    pub(crate) standard_input_is_terminal: Option<bool>,
    /// See [`InputStack::take_alias_boundary`].
    alias_boundary: bool,
}

impl InputStack {
    /// What the statics were declared with.
    pub(crate) const fn new() -> Self {
        InputStack {
            base: InputFrame::EMPTY,
            frames: Vec::new(),
            floor_index: 0,
            current: 0,
            pending_here_documents: Vec::new(),
            completed_here_documents: Vec::new(),
            prompt_before_read: false,
            prompt_needed: false,
            last_token: crate::parser::TokenKind::Eof,
            last_token_quoted: false,
            last_token_after_blank: false,
            token_pushed_back: false,
            parse_dialect: crate::options::Dialect::Posix,
            word: crate::word::ParsedWord::new(),
            pending_redirection: None,
            pending_here_document: None,
            standard_input_state: StandardInputState {
                seekable: false,
                pipe: None,
                pending: None,
                bufferable: false,
            },
            prompt: None,
            standard_input_is_terminal: None,
            alias_boundary: false,
        }
    }

    /// The last word read, as its shell-visible bytes.
    pub(crate) fn word_text(&self) -> &BStr {
        self.word.as_bstr()
    }

    /// Begin a parser entry with the shell's current dialect snapshot.
    pub(crate) fn begin_parse(&mut self, dialect: crate::options::Dialect) {
        self.parse_dialect = dialect;
    }

    /// The immutable dialect for the parser entry now in progress.
    pub(crate) fn parse_dialect(&self) -> crate::options::Dialect {
        self.parse_dialect
    }

    /// `parsefile`, as a value [`unwindfiles`] will accept.
    ///
    /// Taken before a source is pushed and given back after it is done, so
    /// that the stack depth across a [`crate::context::Shell::run`] is the
    /// depth before it. That is checked rather than asserted in prose --
    /// see `run`'s own `debug_assert`.
    #[inline]
    pub(crate) fn mark(&self) -> usize {
        self.current
    }

    /// `toppf` — the floor [`popallfiles`] unwinds to.
    #[inline]
    pub(crate) fn floor(&self) -> usize {
        self.floor_index
    }

    /// Move the floor.
    ///
    /// `setinputfd` already does this for a file opened without
    /// `INPUT_PUSH_FILE`, and `setinputstring` deliberately does not --
    /// which is exactly the asymmetry `docs/api-design.md` §4.2 makes
    /// [`crate::context::Shell::run`] close. The old value is read with
    /// [`InputStack::floor`] *before* the push, because for one of the two
    /// the push is what moves it.
    #[inline]
    pub(crate) fn set_floor(&mut self, to: usize) {
        self.floor_index = to;
    }

    /// Take the alias-expansion boundary flag and clear it.
    #[inline]
    pub(crate) fn take_alias_boundary(&mut self) -> bool {
        core::mem::take(&mut self.alias_boundary)
    }

    /// Drop an unread alias-expansion boundary flag.
    #[inline]
    pub(crate) fn clear_alias_boundary(&mut self) {
        self.alias_boundary = false;
    }
}

/// Set when `popstring` finishes an alias whose text ended in a blank.
///
/// dash spells this `checkkwd |= CHKALIAS` — the input layer reaching
/// into a parser global. The bit is per-*input-position* state (it
/// describes the alias that just ended, not the parse that will read the
/// next token), so it belongs on this side of the seam, and putting it
/// here is what leaves `input.rs` naming nothing in `parser.rs`. The
/// parser consults it at the two points it consumed the flag before —
/// Frame `i`. Index 0 is `basepf`, which is not in `FRAMES` because it
/// outlives every push and the C gives it a different `popfile`.
#[inline(always)]
fn input_frame_at(input: &mut InputStack, frame_index: usize) -> &mut InputFrame {
    if frame_index == 0 {
        &mut input.base
    } else {
        &mut input.frames[frame_index - 1]
    }
}

/// The C's `parsefile`, dereferenced.
///
/// Resolved from the index rather than read from a cached pointer. The
/// cache -- `static mut curp`, re-derived by `set_cur` -- was a pointer
/// *into* `basepf`/`FRAMES`, which is a self-reference of the kind that
/// cannot be a field of a movable struct: `Shell::new` returns by value,
/// so the struct moves once and the pointer is left behind. `cur` was
/// already the authoritative half (it is what `unwindfiles` compares) and
/// `cur_pf` already carried a `debug_assert_eq!` claiming the two agree,
/// so deleting the cache removes a duplicated fact rather than a fast
/// path. Same answer as `VarSlot::Builtin`, `owned-jobs` and
/// `owned-input`: name the thing, do not store where it lives.
#[inline(always)]
pub(crate) fn current_input_frame(input: &mut InputStack) -> &mut InputFrame {
    let current = input.current;
    if current == 0 {
        &mut input.base
    } else {
        &mut input.frames[current - 1]
    }
}

/// What `nextc` indexes: the innermost pushed string if there is one, and
/// the level's own buffer otherwise. `preadbuffer` and `preadfd` are reached
/// only with the `strpush` stack empty, so they may assume `buf`.
#[inline(always)]
fn text(input_frame: &InputFrame) -> &[u8] {
    if input_frame.overlays.is_empty() {
        &input_frame.buffer
    } else {
        &input_frame.overlays[input_frame.overlays.len() - 1].string
    }
}

// [spec:dash:sem:input.input-get-lleft-fn]
pub fn remaining_buffer_bytes(input_frame: &InputFrame) -> usize {
    input_frame.buffer_remaining
}

// [spec:dash:sem:input.input-set-lleft-fn]
pub fn set_remaining_buffer_bytes(input_frame: &mut InputFrame, len: usize) {
    input_frame.buffer_remaining = len;
}

impl Shell {
    /// Establish the base input frame for a newly constructed shell.
    pub(crate) fn initialize_input_state(&mut self) {
        let base = input_frame_at(&mut self.input, 0);
        if base.buffer.len() != INPUT_STORAGE_SIZE {
            base.buffer = vec![0u8; INPUT_STORAGE_SIZE];
        }
        base.position = 0;
        base.line_number = 1;
        /* The base frame follows the shell's logical standard input rather
         * than caching a process descriptor number. */
        base.uses_stdin = true;
        base.owned_descriptor = None;
    }

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

    /// Detach input buffers and owned sources copied from the parent shell.
    pub(crate) fn detach_parent_input(&mut self) {
        pop_all_input_frames(self);
        if !current_input_frame(&mut self.input).uses_stdin
            && current_input_frame(&mut self.input)
                .owned_descriptor
                .is_some()
        {
            let frame = current_input_frame(&mut self.input);
            drop(frame.owned_descriptor.take());
            frame.uses_stdin = true;
        }
        drop(self.input.standard_input_state.pipe.take());
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

/// Clear `ALIASINUSE` on everything in `list`, newest first, which is the
/// order the C's `spfree` chain walks in. The `strpush` nodes themselves are
/// dropped with the `Vec`; the C's `ckfree` on each is what that replaces.
fn release_input_overlays(shell: &mut crate::context::Shell, mut list: Vec<InputOverlay>) {
    while let Some(mut overlay) = list.pop() {
        if let Some(name) = &overlay.alias_name {
            shell.aliases.finish_expansion(BStr::new(name.as_slice()));
        }
        /* Only an entry that is still on `strpush` carries one; `popstring`
         * moves the chain out on the way past. */
        let carry = core::mem::take(&mut overlay.deferred_overlays);
        if !carry.is_empty() {
            release_input_overlays(shell, carry);
        }
    }
}

// [spec:dash:sem:input.freestrings-fn]
fn clear_input_overlays(shell: &mut crate::context::Shell) {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let list = core::mem::take(&mut current_input_frame(&mut shell.input).deferred_overlays);
        release_input_overlays(shell, list);
    });
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

    'read_next_unit: loop {
        if input_frame.unread_count != 0 {
            let unread_count = input_frame.unread_count;
            input_frame.unread_count -= 1;

            return Ok(InputUnit::Byte(
                text(input_frame)[input_frame.position - unread_count],
            ));
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

        return Ok(input);
    }
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

// [spec:dash:sem:input.preadfd-fn]
// [spec:posix:req:sh.stdin-used-only-if]
// [spec:posix:req:sh.stdin-no-read-ahead]
// [spec:posix:req:sh.stdin-blocking-reads]
// [spec:posix:req:sh.input-file-contents]
// [spec:posix:req:sh.input-file-blank-or-comments]
// [spec:posix:req:xcurel.file-contents-nbytes]
// [spec:posix:sem:xcurel.file-contents-read-error]
// [spec:posix:req:exit.unrecoverable-read-error]
fn read_input_descriptor(shell: &mut crate::context::Shell) -> Result<usize, Error> {
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
        return Ok(0);
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
                Ok(count) => Ok(count),
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
                    if error_kind == std::io::ErrorKind::Interrupted
                        && !(input_frame_at(&mut shell.input, 0).previous.is_some()
                            && crate::signal_inbox::signals().pending_signal().is_some())
                    {
                        continue 'retry;
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
            return Ok(count);
        }
        return Ok(0);
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

// [spec:dash:sem:input.preadbuffer-fn]
fn refill_input_buffer(
    shell: &mut crate::context::Shell,
    preserve_nul: bool,
) -> Result<InputUnit, Error> {
    let first = shell.input.prompt == Some(PromptKind::Primary);

    if current_input_frame(&mut shell.input).eof_latched {
        /* eof: */
        current_input_frame(&mut shell.input).eof_observed = true;
        return Ok(InputUnit::EndOfInput);
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
                remaining = read_input_descriptor(shell)?;
                line_end = current_input_frame(&mut shell.input).position + preserved_count;
                if remaining == 0 {
                    current_input_frame(&mut shell.input).line_remaining = 0;
                    set_remaining_buffer_bytes(current_input_frame(&mut shell.input), 0);
                    if preserved_count != 0 {
                        preserve_buffer = true;
                        break 'outer;
                    }
                    return Ok(None);
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
        Ok::<_, Error>(Some(line))
    })?;

    /* A read interrupted while this scope was active becomes deliverable at
     * this explicit polling boundary, after the prior deferral depth has been
     * restored. */
    if let Some(error) = crate::error::poll_interrupt(shell.interrupt_context()) {
        return Err(error);
    }

    let Some(line) = buffered else {
        let input_frame = current_input_frame(&mut shell.input);
        input_frame.eof_latched = true;
        input_frame.eof_observed = true;
        return Ok(InputUnit::EndOfInput);
    };

    if shell.options.enabled(ShellOption::Verbose) {
        shell.write_output(crate::output::OutputDestination::Stderr, &line)?;
    }

    let input_frame = current_input_frame(&mut shell.input);
    let byte = input_frame.buffer[input_frame.position];
    input_frame.position += 1;
    Ok(InputUnit::Byte(byte))
}

// [spec:dash:sem:input.pungetn-fn]
pub fn unread_input_units(shell: &mut Shell, count: usize) {
    current_input_frame(&mut shell.input).unread_count += count;
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

/*
 * Push a string back onto the input at this current parsefile level.
 * We handle aliases this way.
 */

// [spec:dash:sem:input.pushstring-fn]
pub fn push_string_input(shell: &mut Shell, string: &BStr, alias_name: Option<BString>) {
    let string_length = string.len();
    crate::error::with_interrupts_deferred(shell, |shell| {
        if let Some(name) = &alias_name {
            shell.aliases.begin_expansion(BStr::new(name.as_slice()));
        }
        /*dprintf("*** calling pushstring: %s, %d\n", s, len);*/
        /* The C picks between `basestrpush` and a `ckmalloc` here; a `Vec`
         * needs neither, and the condition it picked on was only ever about
         * whether the inline slot was still spoken for. */
        let input_frame = current_input_frame(&mut shell.input);
        let string = string.to_vec();
        let overlay = InputOverlay {
            previous_position: input_frame.position,
            previous_line_remaining: input_frame.line_remaining,
            unread_count: input_frame.unread_count,
            deferred_overlays: core::mem::take(&mut input_frame.deferred_overlays),
            alias_name,
            string,
        };
        /* The C reads on through `ap->val`, which points into `ap->name`; this
         * reads the copy, so redefining the alias mid-expansion cannot pull the
         * text out from under the cursor and `popstring` has nothing to free. */
        input_frame.position = 0;
        input_frame.line_remaining = string_length;
        input_frame.unread_count = 0;
        input_frame.overlays.push(overlay);
    });
}

// [spec:dash:sem:input.popstring-fn]
// [spec:posix:req:token.alias-trailing-blank-chaining]
fn pop_string_input(shell: &mut Shell) {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let input_frame = current_input_frame(&mut shell.input);
        let mut overlay = input_frame.overlays.pop().unwrap();

        /* The C compares `nextc` against `sp->string`, which is `ap->name` —
         * the base of the allocation `ap->val` points into — so the test reads
         * as "always true" and the byte it then looks at is the one before the
         * cursor. Against the copy the same test means "at least one character
         * consumed", and the two agree: with none consumed the C reads the `=`
         * that ends the alias name, which is neither a space nor a tab. */
        let boundary = overlay.alias_name.is_some()
            && input_frame.position > 0
            && matches!(overlay.string[input_frame.position - 1], b' ' | b'\t');
        input_frame.position = overlay.previous_position;
        input_frame.line_remaining = overlay.previous_line_remaining;
        input_frame.unread_count = overlay.unread_count;
        /*dprintf("*** calling popstring: restoring to '%s'\n", parsenextc);*/
        /* `parsefile->spfree = sp` with `sp->spfree` already holding the chain
         * that was hidden when `sp` was pushed. Anything the current chain still
         * held is dropped, which is what the C's assignment does to it. */
        input_frame.deferred_overlays = core::mem::take(&mut overlay.deferred_overlays);
        input_frame.deferred_overlays.push(overlay);
        /* Set after the frame's borrow ends; it is a flag on the stack, not
         * on the frame, and nothing between here and there reads it. */
        if boundary {
            shell.input.alias_boundary = true;
        }
    });
}

/*
 * Set the input to take input from a file.  If push is set, push the
 * old input onto the stack first.
 */

// [spec:dash:sem:input.setinputfile-fn]
pub fn set_input_file(
    shell: &mut crate::context::Shell,
    path: &BStr,
    options: InputFileOptions,
) -> Result<bool, Error> {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let Some(descriptor) =
            crate::redirection::open_file_for_reading(shell, path, options.missing_ok)?
        else {
            return Ok(false);
        };
        install_input_file(shell, descriptor, options)?;
        Ok(true)
    })
}

/// Set the top-level input from the command file named on `sh`'s command line.
// [spec:posix:req:sh.exit-status-values]
pub fn set_command_input_file(shell: &mut crate::context::Shell, path: &BStr) -> Result<(), Error> {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let descriptor = crate::redirection::open_command_file(shell, path)?;
        install_input_file(shell, descriptor, InputFileOptions::CURRENT)
    })
}

fn install_input_file(
    shell: &mut Shell,
    mut descriptor: Descriptor,
    options: InputFileOptions,
) -> Result<(), Error> {
    descriptor = crate::redirection::move_descriptor_above(shell, descriptor)?;
    set_input_descriptor(shell, descriptor, options.push, options.dot_operand);
    Ok(())
}

/*
 * Like setinputfile, but takes an open file descriptor.  Call this with
 * interrupts off.
 */

// [spec:dash:sem:input.setinputfd-fn]
// [spec:nsh:req:idiom.no-raw-fd-core]
fn set_input_descriptor(shell: &mut Shell, descriptor: Descriptor, push: bool, dot_operand: bool) {
    push_input_frame(shell);
    if !push {
        shell.input.floor_index = shell.input.current;
    }
    let input_frame = current_input_frame(&mut shell.input);
    input_frame.uses_stdin = false;
    input_frame.owned_descriptor = Some(crate::descriptors::SharedDescriptor::from(descriptor));
    input_frame.dot_operand = dot_operand;
    input_frame.buffer = vec![0u8; INPUT_STORAGE_SIZE];
    input_frame.position = 0;
}

/*
 * Like setinputfile, but takes input from a string.
 */

// [spec:dash:sem:input.setinputstring-fn]
pub fn set_input_string(shell: &mut Shell, string: &BStr) {
    crate::error::with_interrupts_deferred(shell, |shell| {
        push_input_frame(shell);
        let string_length = string.len();
        let input_frame = current_input_frame(&mut shell.input);
        /* The C points `nextc` at the caller's string and reads it in place,
         * which is why `evalstring` has to keep its `sstrdup` alive across the
         * `popfile` and why `parsebackq` cannot release the stack block it
         * grabbed. The level owns its text here. */
        input_frame.buffer = string.to_vec();
        input_frame.position = 0;
        input_frame.line_remaining = string_length;
        input_frame.eof_latched = true;
        input_frame.eof_observed = false;
    });
}

/*
 * To handle the "." command, a stack of input files is used.  Pushfile
 * adds a new entry to the stack and popfile restores the previous level.
 */

// [spec:dash:sem:input.pushfile-fn]
fn push_input_frame(shell: &mut Shell) {
    let previous = shell.input.current;
    shell.input.frames.push(InputFrame {
        previous: Some(previous),
        line_number: 1,
        uses_stdin: false,
        ..InputFrame::EMPTY
    });
    let depth = shell.input.frames.len();
    shell.input.current = depth;
}

// [spec:dash:sem:input.pushstdin-fn]
pub fn push_standard_input(shell: &mut Shell) {
    crate::error::with_interrupts_deferred(shell, |shell| {
        let previous_frame = shell.input.current;
        input_frame_at(&mut shell.input, 0).previous = Some(previous_frame);
        shell.input.current = 0;
    });
}

// [spec:dash:sem:input.popfile-fn]
// [spec:nsh:sem:idiom.specified-defects+1]
pub fn pop_input_frame(shell: &mut crate::context::Shell) {
    let popped_index = shell.input.current;

    crate::error::with_interrupts_deferred(shell, |shell| {
        /* The C reads `pf->prev` into the global unconditionally, so popping
         * `basepf` when nothing pushed it leaves `parsefile` NULL; there is no
         * such value here and the base frame stays current. */
        let previous_index = input_frame_at(&mut shell.input, popped_index)
            .previous
            .take()
            .unwrap_or(0);
        shell.input.current = previous_index;
        if popped_index == 0 {
            return;
        }

        let frames = &mut shell.input.frames;
        debug_assert_eq!(popped_index, frames.len());
        let mut input_frame = frames.pop().unwrap();
        /* `set_cur(cur)` stood here to re-derive the cached frame pointer,
         * because popping the `Vec` can move the remaining frames. The index
         * does not move with them, so with the cache gone this was a
         * self-assignment and says nothing. */

        drop(input_frame.owned_descriptor.take());
        /* `ckfree(pf->buf)` */
        drop(core::mem::take(&mut input_frame.buffer));
        if !current_input_frame(&mut shell.input)
            .deferred_overlays
            .is_empty()
        {
            clear_input_overlays(shell);
        }
        /* Release every alias expansion owned by the dying frame before the
         * frame disappears. The reference switches to the outer frame too
         * early, walks that frame's string stack, and can both leave aliases
         * permanently marked in-use and dereference a null link. Ownership
         * identifies the intended frame without either failure mode. */
        let mut overlays = core::mem::take(&mut input_frame.deferred_overlays);
        overlays.extend(core::mem::take(&mut input_frame.overlays));
        release_input_overlays(shell, overlays);
        drop(input_frame);
    });
}

// [spec:dash:sem:input.unwindfiles-fn]
pub fn unwind_input_frames(shell: &mut crate::context::Shell, stop: usize) {
    while input_frame_at(&mut shell.input, 0).previous.is_some() || shell.input.current != stop {
        pop_input_frame(shell);
    }
}

/*
 * Return to top level.
 */

// [spec:dash:sem:input.popallfiles-fn]
pub fn pop_all_input_frames(shell: &mut crate::context::Shell) {
    /* Read out first: `toppf` is a field of the same stack `unwindfiles`
     * unwinds, so the depth is taken as a value before the call. */
    let floor_index = shell.input.floor_index;
    unwind_input_frames(shell, floor_index);
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::test_support::lock;

    // [spec:nsh:sem:idiom.specified-defects+1/test]
    #[test]
    fn popped_frame_releases_aliases() {
        let _guard = lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        let name = BStr::new(b"again");
        crate::alias::set_alias(&mut shell, name, BStr::new(b"echo yes")).unwrap();
        shell.aliases.begin_expansion(name);
        assert!(shell.aliases.lookup(name, true).is_none());

        push_input_frame(&mut shell);
        current_input_frame(&mut shell.input)
            .deferred_overlays
            .push(InputOverlay {
                previous_position: 0,
                previous_line_remaining: 0,
                alias_name: Some(name.to_owned()),
                string: b"echo yes".to_vec(),
                deferred_overlays: Vec::new(),
                unread_count: 0,
            });

        pop_input_frame(&mut shell);

        assert_eq!(
            shell.aliases.lookup(name, true),
            Some(BString::from("echo yes"))
        );
    }
}
