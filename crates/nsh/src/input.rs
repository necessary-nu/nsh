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
//!
//! What is left in this file is the stack itself: what a frame is made of,
//! where each one's bytes come from, and how one is pushed and popped.
//! Turning those bytes into input units is `read`, and the pushed string a
//! frame reads in front of its own is `overlay`.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString};
use nsh_platform::Descriptor;

use crate::descriptors::LogicalDescriptor;
use crate::options::ShellOption;
// [spec:nsh:def:idiom.shell-options]
use crate::syntax::InputUnit;

mod overlay;
mod read;

pub use overlay::push_string_input;
use overlay::{InputOverlay, clear_input_overlays, release_input_overlays};
pub(crate) use read::{
    forget_standard_input_mode, read_input_unit_preserving_nul, rearm_stdin_after_eof,
};
pub use read::{
    initialize_input, read_input_unit, read_input_unit_or_alias_end, reset_input,
    unread_input_unit, unread_input_units,
};

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
    ///
    /// Readable across the crate because the token log is bound to one
    /// frame: anything that writes to the log has to be able to say which
    /// input it is speaking for, the way `record` and `unrecord` do.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) current: usize,
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
    /// Whether a Bash `[[ ]]` expression is being parsed. Bash enables
    /// extended globs inside one whether or not `shopt -s extglob` is on,
    /// so the lexer has to know it is there.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(crate) parsing_conditional: bool,
    /// `tokpushback` — one token of lookahead, pushed back.
    pub(crate) token_pushed_back: bool,
    /// How many levels the parser is currently nested inside: commands,
    /// the list inside a `$( )`, and a `time` prefix's pipeline alike.
    ///
    /// Recursive descent puts one frame per nesting level on the stack,
    /// and script text is untrusted input: `(((...)))` deep enough
    /// overflows it, with no unwind to catch and nothing executed --
    /// `sh -n` on a hostile file is enough. Bounded rather than trusted;
    /// see `parser::keywords::nested`.
    // [spec:nsh:req:idiom.bounded-recursion]
    pub(crate) nesting_depth: u32,
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
    /// Whether the read in progress may be handed a line that has not
    /// ended yet.
    ///
    /// A parser needs whole lines and everything under
    /// [`read_input_unit`] is built to produce them: a refill keeps
    /// reading until a newline arrives, because a token cannot be
    /// decided from half of one. `read -n1` is the opposite command --
    /// it has to answer on the first character, and a source somebody
    /// is still typing into has no newline in it yet. The bytes already
    /// arrive one at a time from a pipe and from a terminal out of
    /// canonical mode, so it is only the handover that waits; this is
    /// the bit that stops it waiting.
    ///
    /// Set for the duration of one `read` whose record can end before
    /// the line does -- a character count, or a delimiter that is not
    /// the newline -- and put back afterwards rather than cleared,
    /// because a trap action taken at a polling boundary inside that
    /// read may run a `read` of its own.
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    partial_line_delivery: bool,
    /// Set when `popstring` finishes an alias whose text ended in a blank.
    ///
    /// dash spells this `checkkwd |= CHKALIAS` — the input layer reaching
    /// into a parser global. The bit is per-*input-position* state (it
    /// describes the alias that just ended, not the parse that will read
    /// the next token), so it belongs on this side of the seam, and
    /// putting it here is what leaves `input.rs` naming nothing in
    /// `parser.rs`. The parser consults it at the two points it consumed
    /// the flag before: one takes it through
    /// [`InputStack::take_alias_boundary`], and one drops it unread
    /// through [`InputStack::clear_alias_boundary`].
    alias_boundary: bool,
    /// Every byte the parse in progress has consumed, cut into tokens.
    ///
    /// The reader is the only place that knows what was actually read --
    /// input arrives here from strings, files, terminals and alias
    /// expansions interleaved -- so it is where the record is kept.
    // [spec:nsh:def:idiom.token-stream]
    pub(crate) tokens: crate::parser::TokenLog,
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
            parsing_conditional: false,
            token_pushed_back: false,
            nesting_depth: 0,
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
            partial_line_delivery: false,
            alias_boundary: false,
            tokens: crate::parser::TokenLog::new(),
        }
    }

    /// The last word read, as its shell-visible bytes.
    pub(crate) fn word_text(&self) -> &BStr {
        self.word.as_bstr()
    }

    /// Begin a parser entry with the shell's current dialect snapshot.
    pub(crate) fn begin_parse(&mut self, dialect: crate::options::Dialect) {
        self.parse_dialect = dialect;
        /* Bound to the frame the parse starts on, so that a string pushed
         * underneath it -- the text `parsebackq` re-reads a legacy
         * backquote from -- is not recorded a second time. */
        // [spec:nsh:def:idiom.token-stream]
        self.tokens.begin(self.current);
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

    /// Whether a refill may stop short of the newline.
    #[inline]
    pub(crate) const fn partial_line_delivery(&self) -> bool {
        self.partial_line_delivery
    }

    /// Ask for partial lines, and answer with the setting replaced.
    ///
    /// The caller owns putting that answer back, which is why this
    /// returns it rather than offering a clear: reads nest.
    #[inline]
    pub(crate) fn set_partial_line_delivery(&mut self, deliver: bool) -> bool {
        core::mem::replace(&mut self.partial_line_delivery, deliver)
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

/// The bytes the current frame will hand over next without reading, in
/// the order it will hand them over.
///
/// What is left of the line already in the buffer, and nothing past it.
/// A caller looking ahead must not read across the end of that run: the
/// unit after it may be an alias boundary rather than a byte, or may
/// come from a `read` that blocks. Two states are refused outright
/// rather than described, because a caller's fallback is the
/// byte-at-a-time read it would have done anyway -- an outstanding
/// `pungetc`, which is served from *before* the cursor, and a frame
/// whose line is exhausted.
///
/// Empty is always a safe answer, so a caller may treat this as a hint
/// and must not treat a non-empty answer as permission to read further
/// than it is long.
pub(crate) fn buffered_line_bytes(input: &mut InputStack) -> &[u8] {
    let input_frame = current_input_frame(input);
    if input_frame.unread_count != 0 || input_frame.line_remaining == 0 {
        return &[];
    }
    let position = input_frame.position;
    let line_remaining = input_frame.line_remaining;
    let text = text(input_frame);
    let end = position.saturating_add(line_remaining).min(text.len());
    text.get(position..end).unwrap_or_default()
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

/// Push a byte source whose first line continues the caller's numbering.
///
/// A Bash trap action is not a script of its own: it is parsed as if it
/// had been written where the condition was raised, which is what makes
/// `trap 'echo $LINENO' ERR` report the line that failed rather than
/// line 1 of the action.
// [spec:nsh:req:compat.bash.traps-introspection]
pub(crate) fn set_input_string_at_line(shell: &mut Shell, string: &BStr, line: i32) {
    set_input_string(shell, string);
    crate::error::with_interrupts_deferred(shell, |shell| {
        current_input_frame(&mut shell.input).line_number = line;
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

    /// `buffered_line_bytes` answers only for bytes the frame will hand
    /// over next as bytes, and answers empty rather than guessing.
    ///
    /// The three refusals are the whole contract, because a caller uses
    /// this to decide how far it may read ahead: an exhausted line, whose
    /// next unit may be an alias boundary rather than a byte; an
    /// outstanding `pungetc`, which is served from *before* the cursor and
    /// so is not the run starting at it; and a cursor that has advanced,
    /// after which the answer must shrink rather than repeat bytes already
    /// handed out.
    // [spec:nsh:req:shell-locale.operation-binding/test]
    #[test]
    fn only_bytes_the_frame_will_hand_over() {
        let _guard = lock();
        let mut shell = Shell::new(crate::streams::Streams::INHERIT);
        set_input_string(&mut shell, BStr::new(b"abcdef"));

        let frame = current_input_frame(&mut shell.input);
        frame.position = 1;
        frame.line_remaining = 3;
        frame.unread_count = 0;
        assert_eq!(buffered_line_bytes(&mut shell.input), b"bcd");

        /* The cursor moves and the answer shrinks with it. */
        let frame = current_input_frame(&mut shell.input);
        frame.position = 3;
        frame.line_remaining = 1;
        assert_eq!(buffered_line_bytes(&mut shell.input), b"d");

        /* An exhausted line offers nothing, whatever the buffer holds. */
        current_input_frame(&mut shell.input).line_remaining = 0;
        assert_eq!(buffered_line_bytes(&mut shell.input), b"");

        /* An outstanding put-back offers nothing either. */
        let frame = current_input_frame(&mut shell.input);
        frame.line_remaining = 3;
        frame.unread_count = 1;
        assert_eq!(buffered_line_bytes(&mut shell.input), b"");

        /* A count reaching past the text is clamped, not trusted. */
        let frame = current_input_frame(&mut shell.input);
        frame.unread_count = 0;
        frame.position = 4;
        frame.line_remaining = 99;
        assert_eq!(buffered_line_bytes(&mut shell.input), b"ef");
    }
}
