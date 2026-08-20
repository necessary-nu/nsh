//! Literal port of `src/input.c` / `src/input.h`.
//! Rules: `docs/spec/port/src/input.md`.
//!
//! Configuration: `SMALL` is *not* defined, so `IS_DEFINED_SMALL` is false and
//! the `#ifndef SMALL` arms (`lleft`, libedit, history) are the live ones.
//! Both `IS_DEFINED_SMALL` arms are carried, exactly as the C carries them.
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
use core::ffi::c_int;
use nsh_platform::Descriptor;
use std::io::Write;

use crate::fd::LogicalDescriptor;
use crate::options::ShellOption;
// [spec:nsh:def:idiom.shell-options]
use crate::syntax::InputUnit;

/// `MB_LEN_MAX > 16 ? MB_LEN_MAX : 16` — 16 on glibc.
pub const PUNGETC_MAX: usize = 16;
/// stdio's `BUFSIZ`.
pub const BUFSIZ: c_int = 8192;
pub const IBUFSIZ: usize = BUFSIZ as usize + PUNGETC_MAX + 1;

/// `#ifdef SMALL / #define IS_DEFINED_SMALL 1 #else 0` — this port is !SMALL.
pub const IS_DEFINED_SMALL: bool = false;

pub const INPUT_PUSH_FILE: c_int = 1;
pub const INPUT_NOFILE_OK: c_int = 2;
pub const INPUT_DOT_FILE: c_int = 4;

// [spec:dash:def:input.strpush]
/// The C's `struct strpush`.
///
/// `prev` is the `Vec` order and `basestrpush` has no reason to exist, so
/// both are gone. `string` is a copy of the pushed text; in the C it is
/// `ap->name`, the *whole* `name=value` allocation that `ap->val` points
/// into, held so that redefining an alias mid-expansion does not free the
/// text being read. See `plan/decisions/owned-data.md`.
pub struct StrPush {
    /// `sp->prevstring`, as a cursor into the text that was current
    pub prevpos: usize,
    pub prevnleft: c_int,
    /// if push was associated with an alias
    pub alias_name: Option<BString>,
    /// the pushed text, NUL-terminated the way the C's `s` was
    pub string: Vec<u8>,
    /// `sp->spfree`: the pending-free chain hidden while this string is read
    pub spfree: Vec<StrPush>,
    /// Number of outstanding calls to pungetc.
    pub unget: c_int,
}

/*
 * The parsefile structure pointed to by the global variable parsefile
 * contains information about the current file being read.
 */

// [spec:dash:def:input.parsefile]
/// The C's `struct parsefile`. `prev` is an index into the frame stack, not
/// a pointer, so that `Vec` growth cannot invalidate it.
pub struct ParseFile {
    /// preceding file on stack
    pub prev: Option<usize>,
    /// current line
    pub linno: c_int,
    /// Whether this frame reads logical descriptor 0. Keeping the logical
    /// identity separate from the backing descriptor is what lets a later
    /// redirection change stdin without invalidating this parse frame.
    uses_stdin: bool,
    /// Ownership when this frame opened the descriptor itself.
    owned_fd: Option<crate::fd::SharedFd>,
    /// Whether this file is the operand of the `.` special built-in.
    dot_operand: bool,
    /// number of chars left in this line
    pub nleft: c_int,
    /// do not read again once we hit EOF
    pub eof: c_int,
    /// next char in the current text
    pub pos: usize,
    /// input buffer, or the whole text when this level reads a string
    pub buf: Vec<u8>,
    /// for pushing strings at this level
    pub strpush: Vec<StrPush>,
    /// Delay freeing so we can stop nested aliases.
    pub spfree: Vec<StrPush>,
    /* #ifndef SMALL */
    /// number of chars left in this buffer
    pub lleft: c_int,
    /// Number of outstanding calls to pungetc.
    pub unget: c_int,
}

impl ParseFile {
    /// What `memset(pf, 0, sizeof(*pf))` produced.
    pub const EMPTY: ParseFile = ParseFile {
        prev: None,
        linno: 0,
        uses_stdin: false,
        owned_fd: None,
        dot_operand: false,
        nleft: 0,
        eof: 0,
        pos: 0,
        buf: Vec::new(),
        strpush: Vec::new(),
        spfree: Vec::new(),
        lleft: 0,
        unget: 0,
    };

    /// Whether evaluation is still attached to interactive standard input,
    /// rather than a sourced file or an `eval` string.
    pub(crate) const fn uses_stdin(&self) -> bool {
        self.uses_stdin
    }
}

// [spec:dash:def:input.stdin-state]
pub struct stdin_state_t {
    pub seekable: i64,
    pub pip: Option<crate::redir::Pipe>,
    pub pending: c_int,
    pub bufferable: bool,
}

/// `basepf` — top level input file. Index 0 of the frame stack; it is never
/// popped, and `pushstdin` makes it current again by setting its `prev`.
/// The pushed frames. `FRAMES[i]` is frame index `i + 1`.
/// `toppf` — how far `popallfiles` unwinds.
/// `parsefile` — the current input frame.

/// The shell's input: where it is reading from, and what it has read.
///
/// `docs/api-design.md` §5 assigns `input.rs`'s statics and `parser.rs`'s
/// eleven parser globals to one `input` field. This is that field, being
/// filled in slices: the independent scalars first, the frame stack next.
///
/// `stdin_state`, `whichprompt` and `stdin_istty` are `pub(crate)` --
/// three unrelated scalars, each meaning one thing, read from
/// `options.rs` and `parser.rs`; accessors would be noise, by the
/// criterion the evaluator's state settled. `alias_boundary` is private
/// because it is a bit this module produces and hands over through
/// [`take_alias_boundary`], which is the invariant worth keeping.
// [spec:posix:req:xcu.stdin.input-file-restrictions-apply]
// [spec:posix:req:xcu.stdin.terminal-background]
// [spec:posix:req:xcu.stdin.env-independence]
// [spec:posix:req:xcu.input-files.eight-bit-transparency]
// [spec:posix:req:xcu.input-files.seekable-file-offset]
// [spec:posix:req:xcu.input-files.document-size-limits]
// [spec:posix:req:xcu.input-files.text-file-and-line-continuation]
pub struct InputStack {
    /// `basepf` — the top-level input file, frame 0. Never popped.
    base: ParseFile,
    /// `FRAMES` — the pushed frames. `frames[i]` is frame index `i + 1`.
    frames: Vec<ParseFile>,
    /// `toppf` — how far `popallfiles` unwinds.
    top: usize,
    /// `parsefile` — the current frame, by index. See `cur_pf`.
    cur: usize,
    /// Here-document delimiters waiting for their bodies at the next
    /// grammar newline.
    pub(crate) heredoclist: Vec<crate::parser::heredoc>,
    /// Bodies read for the syntax tree currently under construction. They
    /// are moved into that tree before it crosses a parser boundary.
    pub(crate) completed_heredocs: Vec<crate::nodes::WordNode>,
    /// `doprompt` — whether to prompt before the next read.
    pub(crate) doprompt: c_int,
    /// `needprompt` — interactive and at the start of a line.
    pub(crate) needprompt: c_int,
    /// `lasttoken` — the last token read.
    pub(crate) lasttoken: crate::parser::TokenKind,
    /// Whether the last word token contained quoting. Kept beside
    /// `lasttoken` so pushing a token back preserves the complete token;
    /// ordinary parser code receives it as part of `readtoken`'s result.
    pub(crate) last_quoteflag: bool,
    /// `tokpushback` — one token of lookahead, pushed back.
    pub(crate) tokpushback: bool,
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
    pub(crate) redirnode: Option<crate::parser::PendingRedirection>,
    /// `heredoc` — the here-document the last token opened.
    pub(crate) heredoc: Option<crate::parser::heredoc>,
    /// `stdin_state` — how the shell's standard input behaves.
    pub(crate) stdin_state: stdin_state_t,
    /// `whichprompt` — 1 == PS1, 2 == PS2.
    pub(crate) whichprompt: c_int,
    /// `stdin_istty` — -1 until asked.
    pub(crate) stdin_istty: c_int,
    /// See [`take_alias_boundary`].
    alias_boundary: bool,
}

impl InputStack {
    /// What the statics were declared with.
    pub(crate) const fn new() -> Self {
        InputStack {
            base: ParseFile::EMPTY,
            frames: Vec::new(),
            top: 0,
            cur: 0,
            heredoclist: Vec::new(),
            completed_heredocs: Vec::new(),
            doprompt: 0,
            needprompt: 0,
            lasttoken: crate::parser::TokenKind::Eof,
            last_quoteflag: false,
            tokpushback: false,
            parse_dialect: crate::options::Dialect::Posix,
            word: crate::word::ParsedWord::new(),
            redirnode: None,
            heredoc: None,
            stdin_state: stdin_state_t {
                seekable: 0,
                pip: None,
                pending: 0,
                bufferable: false,
            },
            whichprompt: 0,
            stdin_istty: -1,
            alias_boundary: false,
        }
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
        self.cur
    }

    /// `toppf` — the floor [`popallfiles`] unwinds to.
    #[inline]
    pub(crate) fn floor(&self) -> usize {
        self.top
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
        self.top = to;
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
/// see [`take_alias_boundary`] and [`clear_alias_boundary`].
/// Take the flag and clear it: the parser's `kwd |= checkkwd`.
#[inline]
pub fn take_alias_boundary(sh: &mut Shell) -> bool {
    core::mem::take(&mut sh.input.alias_boundary)
}

/// Drop the flag unread: the parser's `checkkwd = 0` while eating
/// newlines, which discarded an alias bit set during that eating.
#[inline]
pub fn clear_alias_boundary(sh: &mut Shell) {
    sh.input.alias_boundary = false;
}

/// Frame `i`. Index 0 is `basepf`, which is not in `FRAMES` because it
/// outlives every push and the C gives it a different `popfile`.
#[inline(always)]
pub fn pf_at(sh: &mut Shell, i: usize) -> &mut ParseFile {
    if i == 0 {
        &mut sh.input.base
    } else {
        &mut sh.input.frames[i - 1]
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
pub fn cur_pf(sh: &mut Shell) -> &mut ParseFile {
    let i = sh.input.cur;
    pf_at(sh, i)
}

/// What `nextc` indexes: the innermost pushed string if there is one, and
/// the level's own buffer otherwise. `preadbuffer` and `preadfd` are reached
/// only with the `strpush` stack empty, so they may assume `buf`.
#[inline(always)]
fn text(pf: &ParseFile) -> &[u8] {
    if pf.strpush.is_empty() {
        &pf.buf
    } else {
        &pf.strpush[pf.strpush.len() - 1].string
    }
}

/// The C's `parsefile`, as a value `unwindfiles` can be given later.
#[inline]
pub fn cur_mark(sh: &mut Shell) -> usize {
    sh.input.cur
}

/// `#define plinno (parsefile->linno)`
#[macro_export]
macro_rules! plinno {
    ($sh:expr) => {
        $crate::input::cur_pf($sh).linno
    };
}

// [spec:dash:def:input.input-get-lleft-fn]
// [spec:dash:sem:input.input-get-lleft-fn]
pub fn input_get_lleft(pf: &ParseFile) -> c_int {
    /* #ifdef SMALL return 0; #else */
    pf.lleft
}

// [spec:dash:def:input.input-set-lleft-fn]
// [spec:dash:sem:input.input-set-lleft-fn]
pub fn input_set_lleft(pf: &mut ParseFile, len: c_int) {
    /* #ifndef SMALL */
    pf.lleft = len;
}

impl Shell {
    /// Establish the base input frame for a newly constructed shell.
    pub(crate) fn initialize_input_state(&mut self) {
        let base = pf_at(self, 0);
        if base.buf.len() != IBUFSIZ {
            base.buf = vec![0u8; IBUFSIZ];
        }
        base.pos = 0;
        base.linno = 1;
        /* The base frame follows the shell's logical standard input rather
         * than caching a process descriptor number. */
        base.uses_stdin = true;
        base.owned_fd = None;
    }

    /// Drain the abandoned input record before the command loop continues.
    pub(crate) fn discard_interrupted_input(&mut self) {
        popallfiles(self);

        /* At least one character past the pushback window has been consumed. */
        let top_index = self.input.top;
        let top = pf_at(self, top_index);
        let mut input = if top.pos > top.unget as usize {
            InputUnit::Byte(text(top)[top.pos - top.unget as usize - 1])
        } else {
            InputUnit::EndOfInput
        };
        while !input.is(b'\n') && input != InputUnit::EndOfInput && crate::error::int_pending() == 0
        {
            match pgetc(self) {
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
        popallfiles(self);
        if !cur_pf(self).uses_stdin && cur_pf(self).owned_fd.is_some() {
            let frame = cur_pf(self);
            drop(frame.owned_fd.take());
            frame.uses_stdin = true;
        }
        drop(self.input.stdin_state.pip.take());
    }
}

// [spec:dash:def:input.input-init-fn]
// [spec:dash:sem:input.input-init-fn]
// [spec:nsh:def:idiom.logical-descriptors]
pub fn input_init(sh: &mut Shell) {
    let stdin = sh.fds.get(LogicalDescriptor::STDIN);
    if let Some(canonical) = stdin
        .as_ref()
        .and_then(|fd| nsh_platform::terminal_canonical_mode(fd))
    {
        sh.input.stdin_istty = 1;
        sh.input.stdin_state.bufferable = canonical;
        sh.input.stdin_state.seekable = 0;
    } else {
        sh.input.stdin_istty = 0;
        sh.input.stdin_state.seekable = stdin
            .as_ref()
            .is_some_and(|fd| nsh_platform::fd_is_seekable(fd))
            as i64;
        sh.input.stdin_state.bufferable = sh.input.stdin_state.seekable != 0;
    }
}

// [spec:dash:def:input.stdin-bufferable-fn]
// [spec:dash:sem:input.stdin-bufferable-fn]
fn stdin_bufferable(sh: &mut Shell) -> bool {
    if sh.input.stdin_istty < 0 {
        input_init(sh);
    }
    sh.input.stdin_state.bufferable
}

// [spec:dash:def:input.flush-tee-fn]
// [spec:dash:sem:input.flush-tee-fn]
fn flush_tee(sh: &mut crate::context::Shell, nr: c_int, mut pending: c_int) {
    let mut scratch = [0_u8; BUFSIZ as usize];
    let stdin = sh.fds.get(LogicalDescriptor::STDIN);
    while pending > 0 {
        let length = nr.min(pending).max(0) as usize;
        let Some(stdin) = &stdin else {
            break;
        };
        match nsh_platform::read_once(stdin, &mut scratch[..length]) {
            Ok(count) if count > 0 => pending -= count as c_int,
            _ => break,
        }
    }
}

// [spec:dash:def:input.stdin-tee-fn]
// [spec:dash:sem:input.stdin-tee-fn]
// [spec:nsh:req:idiom.platform-errors]
fn stdin_tee(sh: &mut Shell, nr: c_int) -> Result<std::io::Result<usize>, Error> {
    if sh.input.stdin_state.pip.is_none() {
        let (pipe, _) = crate::redir::sh_pipe(sh, false)?;
        let read = crate::redir::move_fd_above(sh, pipe.read)?;
        let write = crate::redir::move_fd_above(sh, pipe.write)?;
        sh.input.stdin_state.pip = Some(crate::redir::Pipe { read, write });
    }

    flush_tee(sh, nr, sh.input.stdin_state.pending);

    let pipe = sh
        .input
        .stdin_state
        .pip
        .as_ref()
        .expect("stdin tee pipe exists");
    let result = if nsh_platform::supports_tee() {
        match sh.fds.get(LogicalDescriptor::STDIN) {
            Some(stdin) => nsh_platform::tee(&stdin, &pipe.write, nr as usize),
            None => Err(nsh_platform::platform_error(
                nsh_platform::PlatformErrorKind::BadDescriptor,
            )),
        }
    } else {
        Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
    };
    sh.input.stdin_state.pending = result.as_ref().map_or(-1, |count| *count as c_int);
    Ok(result)
}

/// Clear `ALIASINUSE` on everything in `list`, newest first, which is the
/// order the C's `spfree` chain walks in. The `strpush` nodes themselves are
/// dropped with the `Vec`; the C's `ckfree` on each is what that replaces.
fn release_strpush(sh: &mut crate::context::Shell, mut list: Vec<StrPush>) {
    while let Some(mut sp) = list.pop() {
        if let Some(name) = &sp.alias_name {
            crate::alias::finish_expansion(sh, BStr::new(name.as_slice()));
        }
        /* Only an entry that is still on `strpush` carries one; `popstring`
         * moves the chain out on the way past. */
        let carry = core::mem::take(&mut sp.spfree);
        if !carry.is_empty() {
            release_strpush(sh, carry);
        }
    }
}

// [spec:dash:def:input.freestrings-fn]
// [spec:dash:sem:input.freestrings-fn]
fn freestrings(sh: &mut crate::context::Shell) {
    crate::error::with_interrupts_deferred(sh, |sh| {
        let list = core::mem::take(&mut cur_pf(sh).spfree);
        release_strpush(sh, list);
    });
}

/*
 * Read one item from the script.
 * Nul characters in the input are silently discarded by the normal entry
 * point; `read -d ''` uses the preserving entry point below.
 */

// [spec:dash:def:input.pgetc-fn]
// [spec:dash:sem:input.pgetc-fn]
// [spec:nsh:req:idiom.lexer-tokens]
pub fn pgetc(sh: &mut crate::context::Shell) -> Result<InputUnit, Error> {
    pgetc_inner(sh, false)
}

/// Read one input byte without applying the parser's normal NUL filtering.
///
/// This is intentionally narrower than [`pgetc`]: shell input remains text,
/// while `read -d ''` needs to observe the NUL that terminates its record.
pub(crate) fn pgetc_preserve_nul(sh: &mut crate::context::Shell) -> Result<InputUnit, Error> {
    pgetc_inner(sh, true)
}

fn pgetc_inner(sh: &mut crate::context::Shell, preserve_nul: bool) -> Result<InputUnit, Error> {
    let mut input: InputUnit;
    /* Re-derived after everything that can push a level, because that is
     * what moves the frames; the C reloads the same global for the same
     * reason. */
    let mut pf = cur_pf(sh);

    if !pf.spfree.is_empty() {
        freestrings(sh);
        pf = cur_pf(sh);
    }

    'again: loop {
        if pf.unget != 0 {
            let unget = pf.unget as usize;
            pf.unget -= 1;

            return Ok(InputUnit::Byte(text(pf)[pf.pos - unget]));
        }

        'nextc: loop {
            if pf.nleft > 0 {
                pf.nleft -= 1;
                input = InputUnit::Byte(text(pf)[pf.pos]);
                pf.pos += 1;
            } else if !pf.strpush.is_empty() {
                popstring(sh);
                /* The freestrings call must be delayed til the next
                 * input read so the alias-end boundary remains observable.
                 */
                pf = cur_pf(sh);
                continue 'again;
            } else {
                input = preadbuffer(sh, preserve_nul)?;
                pf = cur_pf(sh);
            }

            /* delete nul characters */
            if IS_DEFINED_SMALL && !preserve_nul && input.is(0) {
                let n = pf.nleft as usize;
                pf.buf.copy_within(pf.pos..pf.pos + n, pf.pos - 1);
                pf.pos -= 1;
                continue 'nextc;
            }

            return Ok(input);
        }
    }
}

// [spec:dash:def:input.pgetc-eoa-fn]
// [spec:dash:sem:input.pgetc-eoa-fn]
pub fn pgetc_eoa(sh: &mut crate::context::Shell) -> Result<InputUnit, Error> {
    let pf = cur_pf(sh);
    if !pf.strpush.is_empty()
        && pf.nleft == -1
        && pf.strpush[pf.strpush.len() - 1].alias_name.is_some()
    {
        Ok(InputUnit::EndOfAlias)
    } else {
        pgetc(sh)
    }
}

// [spec:dash:def:input.stdin-clear-nonblock-fn]
// [spec:dash:sem:input.stdin-clear-nonblock-fn]
fn stdin_clear_nonblock(sh: &mut crate::context::Shell) -> bool {
    sh.fds
        .get(LogicalDescriptor::STDIN)
        .is_some_and(|fd| nsh_platform::set_nonblocking(&fd, false).is_ok())
}

// [spec:dash:def:input.preadfd-fn]
// [spec:dash:sem:input.preadfd-fn]
// [spec:posix:req:sh.stdin-used-only-if]
// [spec:posix:req:sh.stdin-no-read-ahead]
// [spec:posix:req:sh.stdin-blocking-reads]
// [spec:posix:req:sh.input-file-contents]
// [spec:posix:req:sh.input-file-blank-or-comments]
// [spec:posix:req:xcurel.file-contents-nbytes]
// [spec:posix:sem:xcurel.file-contents-read-error]
// [spec:posix:req:exit.unrecoverable-read-error]
fn preadfd(sh: &mut crate::context::Shell) -> Result<c_int, Error> {
    let uses_stdin = cur_pf(sh).uses_stdin;
    let dot_operand = cur_pf(sh).dot_operand;
    let mut use_tee: bool;
    let mut unget: c_int;
    let mut pnr: c_int;
    let mut nr: c_int;

    nr = input_get_lleft(cur_pf(sh));

    unget = cur_pf(sh).pos as c_int;
    if unget > PUNGETC_MAX as c_int {
        unget = PUNGETC_MAX as c_int;
    }

    /* Slide the retained pushback window and the partial line already read
     * down to the front, so the read lands after both. */
    {
        let pf = cur_pf(sh);
        let from = pf.pos - unget as usize;
        pf.buf.copy_within(from..from + (unget + nr) as usize, 0);
        pf.pos = unget as usize;
    }
    /* The C's `buf` walks past both; here it is the offset the read fills
     * from, and it survives a nested `pushfile` because it is not a
     * pointer. */
    let off: usize = unget as usize + nr as usize;

    nr = BUFSIZ - nr;
    if !IS_DEFINED_SMALL && nr == 0 {
        return Ok(nr);
    }

    /* The C's `fd == 0` means "this parse file is the shell's standard
     * input", which is the condition for line editing and for teeing --
     * not descriptor 0 for its own sake. */
    use_tee = uses_stdin
        /* #ifndef SMALL */
        && !crate::histedit::editing_active(sh)
        && !stdin_bufferable(sh);

    pnr = nr;
    'retry: loop {
        nr = pnr;
        /* #ifndef SMALL */
        if uses_stdin && crate::histedit::editing_active(sh) {
            /* `docs/api-design.md` §5.5: nothing the shell hands to a
             * callee may borrow from the shell, and `read_edit_line`
             * takes the shell too. The buffer is moved out, filled, and
             * put back -- a `Vec`, so that is a pointer swap rather than
             * a copy. Nothing can reach this frame's buffer while it is
             * out, which is the same thing the borrow used to assert. */
            let mut buf = core::mem::take(&mut cur_pf(sh).buf);
            let result = crate::histedit::read_edit_line(sh, &mut buf[off..off + nr as usize]);
            cur_pf(sh).buf = buf;
            return match result {
                Ok(count) => Ok(count as c_int),
                Err(error) => {
                    let mut message = BString::from("read error: ");
                    message.extend_from_slice(error.to_string().as_bytes());
                    let failure =
                        Error::unrecoverable_read(sh.eval.errlinno, &message, dot_operand);
                    Err(sh.report(failure))
                }
            };
        }

        let mut reading_tee = false;
        let mut read_error = None;
        if use_tee {
            match stdin_tee(sh, nr)? {
                Ok(count) => {
                    nr = count as c_int;
                    reading_tee = true;
                }
                Err(error) if error.kind() == std::io::ErrorKind::InvalidInput => {
                    use_tee = false;
                    pnr = 1;
                    nr = 1;
                }
                Err(error) => {
                    nr = -1;
                    read_error = Some(error);
                }
            }
        }

        if nr > 0 {
            let source = if reading_tee {
                None
            } else if uses_stdin {
                sh.fds.get(LogicalDescriptor::STDIN)
            } else {
                cur_pf(sh).owned_fd.clone()
            };
            let mut scratch = [0_u8; BUFSIZ as usize];
            let result = if reading_tee {
                let pipe = sh
                    .input
                    .stdin_state
                    .pip
                    .as_ref()
                    .expect("stdin tee pipe exists");
                nsh_platform::read_once(&pipe.read, &mut scratch[..nr as usize])
            } else if let Some(source) = &source {
                nsh_platform::read_once(source, &mut scratch[..nr as usize])
            } else {
                Err(nsh_platform::platform_error(
                    nsh_platform::PlatformErrorKind::BadDescriptor,
                ))
            };
            match result {
                Ok(count) => {
                    cur_pf(sh).buf[off..off + count].copy_from_slice(&scratch[..count]);
                    nr = count as c_int;
                }
                Err(error) => {
                    nr = -1;
                    read_error = Some(error);
                }
            }
        }

        if nr < 0 {
            let error_kind = read_error
                .as_ref()
                .map(std::io::Error::kind)
                .unwrap_or(std::io::ErrorKind::Other);
            if error_kind == std::io::ErrorKind::Interrupted
                && !(pf_at(sh, 0).prev.is_some()
                    && crate::siginbox::signals().pending_signal().is_some())
            {
                continue 'retry;
            }
            if uses_stdin
                && error_kind == std::io::ErrorKind::WouldBlock
                && stdin_clear_nonblock(sh)
            {
                let _ = sh.io.stderr().write_all(b"sh: turning off NDELAY mode\n");
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
            if let Some(e) = crate::error::poll_interrupt(sh) {
                return Err(e);
            }
            let error = read_error.expect("a failed read retains its error");
            let mut message = BString::from("read error: ");
            message.extend_from_slice(sh.locale.error_message(&error).as_bytes());
            let failure = Error::unrecoverable_read(sh.eval.errlinno, &message, dot_operand);
            return Err(sh.report(failure));
        }
        break 'retry;
    }
    Ok(nr)
}

/*
 * Refill the input buffer and return the next input character:
 *
 * 1) If a string was pushed back on the input, pop it;
 * 2) If we are reading from a string we can't refill the buffer, return EOF.
 * 3) If there is more stuff in this buffer, use it else call read to fill it.
 * 4) Process input up to the next newline, normally deleting nul characters.
 */

// [spec:dash:def:input.preadbuffer-fn]
// [spec:dash:sem:input.preadbuffer-fn]
fn preadbuffer(sh: &mut crate::context::Shell, preserve_nul: bool) -> Result<InputUnit, Error> {
    let first: c_int = (sh.input.whichprompt == 1) as c_int;

    if (cur_pf(sh).eof & 2) != 0 {
        /* eof: */
        cur_pf(sh).eof = 3;
        return Ok(InputUnit::EndOfInput);
    }
    sh.io.flushall();

    let buffered = crate::error::with_interrupts_deferred(sh, |sh| {
        let mut q = cur_pf(sh).pos;
        let mut something = (first == 0) as c_int;
        let mut more = input_get_lleft(cur_pf(sh));
        let mut save = false;
        let mut savec = 0;

        'outer: loop {
            if more <= 0 {
                /* again: */
                let nr = (q - cur_pf(sh).pos) as c_int;
                input_set_lleft(cur_pf(sh), nr);
                more = preadfd(sh)?;
                q = cur_pf(sh).pos + nr as usize;
                if more <= 0 {
                    cur_pf(sh).nleft = 0;
                    input_set_lleft(cur_pf(sh), 0);
                    if !IS_DEFINED_SMALL && nr > 0 {
                        save = true;
                        break 'outer;
                    }
                    return Ok(None);
                }
            }

            if IS_DEFINED_SMALL {
                q += more as usize;
                more = 0;
                break 'outer;
            }

            /* delete nul characters */
            loop {
                let byte: u8;

                more -= 1;
                byte = cur_pf(sh).buf[q];

                if byte == 0 && !preserve_nul {
                    let pf = cur_pf(sh);
                    pf.buf.copy_within(q + 1..q + 1 + more as usize, q);
                    /* goto check */
                } else {
                    q += 1;

                    if byte == b'\n' {
                        let previous = {
                            let pf = cur_pf(sh);
                            (q - pf.pos >= 2).then(|| pf.buf[q - 2])
                        };
                        if nsh_platform::input_newline_width(previous) == 2 {
                            // Keep the unread tail contiguous when the platform
                            // treats the preceding CR as part of this newline.
                            let pf = cur_pf(sh);
                            pf.buf.copy_within(q - 1..q + more as usize, q - 2);
                            q -= 1;
                        }
                        break 'outer;
                    }
                    if byte != b'\t' && byte != b' ' {
                        something = 1;
                    }
                }

                /* check: */
                if more <= 0 {
                    continue 'outer;
                }
            }
        }

        if !save {
            input_set_lleft(cur_pf(sh), more);
        }

        {
            let pf = cur_pf(sh);
            pf.nleft = (q - pf.pos) as c_int - 1;
            if !IS_DEFINED_SMALL {
                savec = pf.buf[q];
            }
            pf.buf[q] = b'\0';
        }

        let line = {
            let pf = cur_pf(sh);
            pf.buf[pf.pos..q].to_vec()
        };

        // A forced-interactive command file is the shell's top-level input even
        // though it is not descriptor 0. Retain it, but not nested `source`,
        // dot, eval, or command-substitution frames.
        // [spec:nsh:req:compat.smoosh.history-builtin]
        let top_level_history_input = cur_pf(sh).uses_stdin || sh.input.cur == sh.input.top;
        if top_level_history_input
            && crate::histedit::history_active(sh)
            && !sh.options.enabled(ShellOption::NoLog)
            && something != 0
        {
            let bytes = {
                let pf = cur_pf(sh);
                &pf.buf[pf.pos..q]
            };
            let bytes = bytes.to_vec();
            crate::histedit::record_history_line(sh, &bytes, first != 0, true);
        }
        Ok::<_, Error>(Some((line, q, savec)))
    })?;

    /* A read interrupted while this scope was active becomes deliverable at
     * this explicit polling boundary, after the prior deferral depth has been
     * restored. */
    if let Some(e) = crate::error::poll_interrupt(sh) {
        return Err(e);
    }

    let Some((line, q, savec)) = buffered else {
        cur_pf(sh).eof = 3;
        return Ok(InputUnit::EndOfInput);
    };

    if sh.options.enabled(ShellOption::Verbose) {
        let _ = sh.io.stderr().write_all(&line);
        /* #ifdef FLUSHERR flushout(out2); */
    }

    if !IS_DEFINED_SMALL {
        cur_pf(sh).buf[q] = savec;
    }

    let pf = cur_pf(sh);
    let byte = pf.buf[pf.pos];
    pf.pos += 1;
    Ok(InputUnit::Byte(byte))
}

// [spec:dash:def:input.pungetn-fn]
// [spec:dash:sem:input.pungetn-fn]
pub fn pungetn(sh: &mut Shell, n: c_int) {
    cur_pf(sh).unget += n;
}

/*
 * Undo a call to pgetc.  Only two characters may be pushed back.
 * End-of-input may be pushed back.
 */

// [spec:dash:def:input.pungetc-fn]
// [spec:dash:sem:input.pungetc-fn]
pub fn pungetc(sh: &mut Shell) {
    let n = 1 - (cur_pf(sh).eof & 1);
    pungetn(sh, n);
    cur_pf(sh).eof &= !1;
}

/*
 * Push a string back onto the input at this current parsefile level.
 * We handle aliases this way.
 */

// [spec:dash:def:input.pushstring-fn]
// [spec:dash:sem:input.pushstring-fn]
pub fn pushstring(sh: &mut Shell, s: &BStr, alias_name: Option<BString>) {
    let len = s.len();
    crate::error::with_interrupts_deferred(sh, |sh| {
        if let Some(name) = &alias_name {
            crate::alias::begin_expansion(sh, BStr::new(name.as_slice()));
        }
        /*dprintf("*** calling pushstring: %s, %d\n", s, len);*/
        /* The C picks between `basestrpush` and a `ckmalloc` here; a `Vec`
         * needs neither, and the condition it picked on was only ever about
         * whether the inline slot was still spoken for. */
        let pf = cur_pf(sh);
        let mut string: Vec<u8> = Vec::with_capacity(len + 1);
        string.extend_from_slice(s);
        string.push(0);
        let sp = StrPush {
            prevpos: pf.pos,
            prevnleft: pf.nleft,
            unget: pf.unget,
            spfree: core::mem::take(&mut pf.spfree),
            alias_name,
            string,
        };
        /* The C reads on through `ap->val`, which points into `ap->name`; this
         * reads the copy, so redefining the alias mid-expansion cannot pull the
         * text out from under the cursor and `popstring` has nothing to free. */
        pf.pos = 0;
        pf.nleft = len as c_int;
        pf.unget = 0;
        pf.strpush.push(sp);
    });
}

// [spec:dash:def:input.popstring-fn]
// [spec:dash:sem:input.popstring-fn]
// [spec:posix:req:token.alias-trailing-blank-chaining]
fn popstring(sh: &mut Shell) {
    crate::error::with_interrupts_deferred(sh, |sh| {
        let pf = cur_pf(sh);
        let mut sp = pf.strpush.pop().unwrap();

        /* The C compares `nextc` against `sp->string`, which is `ap->name` —
         * the base of the allocation `ap->val` points into — so the test reads
         * as "always true" and the byte it then looks at is the one before the
         * cursor. Against the copy the same test means "at least one character
         * consumed", and the two agree: with none consumed the C reads the `=`
         * that ends the alias name, which is neither a space nor a tab. */
        let boundary =
            sp.alias_name.is_some() && pf.pos > 0 && matches!(sp.string[pf.pos - 1], b' ' | b'\t');
        pf.pos = sp.prevpos;
        pf.nleft = sp.prevnleft;
        pf.unget = sp.unget;
        /*dprintf("*** calling popstring: restoring to '%s'\n", parsenextc);*/
        /* `parsefile->spfree = sp` with `sp->spfree` already holding the chain
         * that was hidden when `sp` was pushed. Anything the current chain still
         * held is dropped, which is what the C's assignment does to it. */
        pf.spfree = core::mem::take(&mut sp.spfree);
        pf.spfree.push(sp);
        /* Set after the frame's borrow ends; it is a flag on the stack, not
         * on the frame, and nothing between here and there reads it. */
        if boundary {
            sh.input.alias_boundary = true;
        }
    });
}

/*
 * Set the input to take input from a file.  If push is set, push the
 * old input onto the stack first.
 */

// [spec:dash:def:input.setinputfile-fn]
// [spec:dash:sem:input.setinputfile-fn]
pub fn setinputfile(
    sh: &mut crate::context::Shell,
    fname: &BStr,
    flags: c_int,
) -> Result<bool, Error> {
    crate::error::with_interrupts_deferred(sh, |sh| {
        let Some(fd) = crate::redir::sh_open_read(sh, fname, flags & INPUT_NOFILE_OK)? else {
            return Ok(false);
        };
        install_input_file(sh, fd, flags)?;
        Ok(true)
    })
}

/// Set the top-level input from the command file named on `sh`'s command line.
// [spec:posix:req:sh.exit-status-values]
pub fn set_command_input_file(sh: &mut crate::context::Shell, fname: &BStr) -> Result<(), Error> {
    crate::error::with_interrupts_deferred(sh, |sh| {
        let fd = crate::redir::sh_open_command_file(sh, fname)?;
        install_input_file(sh, fd, 0)
    })
}

fn install_input_file(sh: &mut Shell, mut fd: Descriptor, flags: c_int) -> Result<(), Error> {
    fd = crate::redir::move_fd_above(sh, fd)?;
    setinputfd(sh, fd, flags & INPUT_PUSH_FILE, flags & INPUT_DOT_FILE != 0);
    Ok(())
}

/*
 * Like setinputfile, but takes an open file descriptor.  Call this with
 * interrupts off.
 */

// [spec:dash:def:input.setinputfd-fn]
// [spec:dash:sem:input.setinputfd-fn]
// [spec:nsh:req:idiom.no-raw-fd-core]
fn setinputfd(sh: &mut Shell, fd: Descriptor, push: c_int, dot_operand: bool) {
    pushfile(sh);
    if push == 0 {
        sh.input.top = sh.input.cur;
    }
    let pf = cur_pf(sh);
    pf.uses_stdin = false;
    pf.owned_fd = Some(crate::fd::SharedFd::from(fd));
    pf.dot_operand = dot_operand;
    pf.buf = vec![0u8; IBUFSIZ];
    pf.pos = 0;
}

/*
 * Like setinputfile, but takes input from a string.
 */

// [spec:dash:def:input.setinputstring-fn]
// [spec:dash:sem:input.setinputstring-fn]
pub fn setinputstring(sh: &mut Shell, string: &BStr) {
    crate::error::with_interrupts_deferred(sh, |sh| {
        pushfile(sh);
        let len = string.len();
        let pf = cur_pf(sh);
        /* The C points `nextc` at the caller's string and reads it in place,
         * which is why `evalstring` has to keep its `sstrdup` alive across the
         * `popfile` and why `parsebackq` cannot release the stack block it
         * grabbed. The level owns its text here. */
        pf.buf = Vec::with_capacity(len + 1);
        pf.buf.extend_from_slice(string);
        pf.buf.push(0);
        pf.pos = 0;
        pf.nleft = len as c_int;
        pf.eof = 2;
    });
}

/*
 * To handle the "." command, a stack of input files is used.  Pushfile
 * adds a new entry to the stack and popfile restores the previous level.
 */

// [spec:dash:def:input.pushfile-fn]
// [spec:dash:sem:input.pushfile-fn]
fn pushfile(sh: &mut Shell) {
    let prev = sh.input.cur;
    sh.input.frames.push(ParseFile {
        prev: Some(prev),
        linno: 1,
        uses_stdin: false,
        ..ParseFile::EMPTY
    });
    let depth = sh.input.frames.len();
    sh.input.cur = depth;
}

// [spec:dash:def:input.pushstdin-fn]
// [spec:dash:sem:input.pushstdin-fn]
pub fn pushstdin(sh: &mut Shell) {
    crate::error::with_interrupts_deferred(sh, |sh| {
        let from = sh.input.cur;
        pf_at(sh, 0).prev = Some(from);
        sh.input.cur = 0;
    });
}

// [spec:dash:def:input.popfile-fn]
// [spec:dash:sem:input.popfile-fn]
pub fn popfile(sh: &mut crate::context::Shell) {
    let dying: usize = sh.input.cur;

    crate::error::with_interrupts_deferred(sh, |sh| {
        /* The C reads `pf->prev` into the global unconditionally, so popping
         * `basepf` when nothing pushed it leaves `parsefile` NULL; there is no
         * such value here and the base frame stays current. */
        let to = pf_at(sh, dying).prev.take().unwrap_or(0);
        sh.input.cur = to;
        if dying == 0 {
            return;
        }

        let frames = &mut *&mut sh.input.frames;
        debug_assert_eq!(dying, frames.len());
        let mut pf = frames.pop().unwrap();
        /* `set_cur(cur)` stood here to re-derive the cached frame pointer,
         * because popping the `Vec` can move the remaining frames. The index
         * does not move with them, so with the cache gone this was a
         * self-assignment and says nothing. */

        drop(pf.owned_fd.take());
        /* `ckfree(pf->buf)` */
        drop(core::mem::take(&mut pf.buf));
        if !cur_pf(sh).spfree.is_empty() {
            freestrings(sh);
        }
        /* `ckfree(pf)` takes the dying level's `spfree` chain with it, and the
         * `ALIASINUSE` bits on it are never cleared: an alias expanded inside an
         * old-style backquote, or any other level that ends with the alias
         * already popped but not yet freed, stays marked in use for the rest of
         * the shell's life and never expands again. That is observable, so the
         * chain is dropped here rather than released.
         *
         * The C's `while (pf->strpush) { popstring(); … }` above the free reads
         * `parsefile->strpush`, and `parsefile` was moved to the outer level two
         * lines earlier — so the loop pops the wrong stack and then walks into a
         * NULL `strpush`. It cannot run in any case that survives; these go the
         * same way as the chain. */
        drop(pf);
    });
}

// [spec:dash:def:input.unwindfiles-fn]
// [spec:dash:sem:input.unwindfiles-fn]
pub fn unwindfiles(sh: &mut crate::context::Shell, stop: usize) {
    while pf_at(sh, 0).prev.is_some() || sh.input.cur != stop {
        popfile(sh);
    }
}

/*
 * Return to top level.
 */

// [spec:dash:def:input.popallfiles-fn]
// [spec:dash:sem:input.popallfiles-fn]
pub fn popallfiles(sh: &mut crate::context::Shell) {
    /* Read out first: `toppf` is a field of the same stack `unwindfiles`
     * unwinds, so the depth is taken as a value before the call. */
    let top = sh.input.top;
    unwindfiles(sh, top);
}

impl Shell {
    /// Discard buffered standard input while preserving the underlying source.
    // [spec:dash:def:input.flush-input-fn]
    // [spec:dash:sem:input.flush-input-fn]
    // [spec:dash:def:init.postexitreset-fn]
    // [spec:dash:sem:init.postexitreset-fn]
    pub(crate) fn flush_input(&mut self) {
        let base = pf_at(self, 0);
        let left: c_int = base.nleft + input_get_lleft(base);
        crate::error::with_interrupts_deferred(self, |shell| {
            if shell.input.stdin_state.seekable != 0 && left != 0 {
                if let Some(stdin) = shell.fds.get(LogicalDescriptor::STDIN) {
                    let _ = nsh_platform::seek_relative(&stdin, -(left as i64));
                }
            } else if shell.input.stdin_state.pending > left {
                let pending = shell.input.stdin_state.pending;
                flush_tee(shell, BUFSIZ, pending - left);
                shell.input.stdin_state.pending = 0;
            }
            let base = pf_at(shell, 0);
            base.nleft = 0;
            input_set_lleft(base, 0);
        });
    }
}

// [spec:dash:def:input.reset-input-fn]
// [spec:dash:sem:input.reset-input-fn]
pub fn reset_input(sh: &mut Shell) {
    sh.input.stdin_istty = -1;
    pf_at(sh, 0).eof = 0;
    sh.flush_input();
}

/// Let the interactive command loop try standard input again after EOF.
///
/// The parser latches EOF on its input frame so files and strings cannot be
/// polled forever. `ignoreeof` is the one boundary that deliberately asks a
/// terminal for a new record, without discarding any bytes that arrived in
/// the meantime.
pub(crate) fn rearm_stdin_after_eof(sh: &mut Shell) {
    pf_at(sh, 0).eof = 0;
}
