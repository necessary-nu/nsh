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
use core::ptr::addr_of_mut;
use libc::{c_char, c_int, c_void, off_t, size_t, tcflag_t};
use std::ffi::CStr;
use std::io::Write;

use crate::alias::alias;
use crate::error::{INTOFF, INTON};
use crate::syntax::PEOF;

/* PEOF (the end of file marker) is defined in syntax.h */
pub const PEOA: c_int = PEOF - 1;

/// `MB_LEN_MAX > 16 ? MB_LEN_MAX : 16` — 16 on glibc.
pub const PUNGETC_MAX: usize = 16;
/// stdio's `BUFSIZ`.
pub const BUFSIZ: c_int = 8192;
pub const IBUFSIZ: usize = BUFSIZ as usize + PUNGETC_MAX + 1;

/// `#ifdef SMALL / #define IS_DEFINED_SMALL 1 #else 0` — this port is !SMALL.
pub const IS_DEFINED_SMALL: bool = false;

/*
 * config.h knobs used by this file.  The reference build has `HAVE_TEE 1` /
 * `USE_TEE 1`, so `tee(2)` comes from glibc and system.h's
 * `#ifndef HAVE_TEE` stub is not compiled.
 */
pub const USE_TEE: c_int = 1;

pub const INPUT_PUSH_FILE: c_int = 1;
pub const INPUT_NOFILE_OK: c_int = 2;

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
    pub ap: *mut alias,
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
    /// file descriptor (or -1 if string)
    pub fd: c_int,
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
        fd: 0,
        nleft: 0,
        eof: 0,
        pos: 0,
        buf: Vec::new(),
        strpush: Vec::new(),
        spfree: Vec::new(),
        lleft: 0,
        unget: 0,
    };
}

// [spec:dash:def:input.stdin-state]
/// `MKINIT struct stdin_state { … }` — absent from the port manifest because
/// the `MKINIT` marker defeated the extractor.
#[repr(C)]
pub struct stdin_state_t {
    pub seekable: off_t,
    pub pip: [c_int; 2],
    pub pending: c_int,
    pub bufferable: tcflag_t,
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
pub struct InputStack {
    /// `basepf` — the top-level input file, frame 0. Never popped.
    base: ParseFile,
    /// `FRAMES` — the pushed frames. `frames[i]` is frame index `i + 1`.
    frames: Vec<ParseFile>,
    /// `toppf` — how far `popallfiles` unwinds.
    top: usize,
    /// `parsefile` — the current frame, by index. See `cur_pf`.
    cur: usize,
    /// `heredoclist` — here-documents read but not yet attached. Filled
    /// by `parseheredoc` and drained by the node builders, across input
    /// positions, which is why it is state and not a local.
    pub(crate) heredoclist: Vec<crate::parser::heredoc>,
    /// `doprompt` — whether to prompt before the next read.
    pub(crate) doprompt: c_int,
    /// `needprompt` — interactive and at the start of a line.
    pub(crate) needprompt: c_int,
    /// `lasttoken` — the last token read.
    pub(crate) lasttoken: c_int,
    /// `tokpushback` — one token of lookahead, pushed back.
    pub(crate) tokpushback: c_int,
    /// `wordtext` — text of the last word, with the terminating NUL
    /// `readtoken1` writes.
    pub(crate) wordtext: bstr::BString,
    /// `backquotelist` — the command substitutions found in the last word.
    pub(crate) backquotelist: Vec<Option<crate::nodes::Node>>,
    /// `redirnode` — the redirection the last token opened.
    pub(crate) redirnode: Option<crate::nodes::Node>,
    /// `heredoc` — the here-document the last token opened.
    pub(crate) heredoc: Option<crate::parser::heredoc>,
    /// `stdin_state` — how the shell's standard input behaves.
    pub(crate) stdin_state: stdin_state_t,
    /// `whichprompt` — 1 == PS1, 2 == PS2.
    pub(crate) whichprompt: c_int,
    /// `stdin_istty` — -1 until asked.
    pub(crate) stdin_istty: c_int,
    /// See [`take_alias_boundary`].
    alias_boundary: c_int,
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
            doprompt: 0,
            needprompt: 0,
            lasttoken: 0,
            tokpushback: 0,
            wordtext: bstr::BString::new(Vec::new()),
            backquotelist: Vec::new(),
            redirnode: None,
            heredoc: None,
            stdin_state: stdin_state_t {
                seekable: 0,
                pip: [0, 0],
                pending: 0,
                bufferable: 0,
            },
            whichprompt: 0,
            stdin_istty: -1,
            alias_boundary: 0,
        }
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
pub unsafe fn take_alias_boundary(sh: &mut Shell) -> c_int {
    let v = sh.input.alias_boundary;
    sh.input.alias_boundary = 0;
    v
}

/// Drop the flag unread: the parser's `checkkwd = 0` while eating
/// newlines, which discarded an alias bit set during that eating.
#[inline]
pub unsafe fn clear_alias_boundary(sh: &mut Shell) {
    sh.input.alias_boundary = 0;
}

/// Frame `i`. Index 0 is `basepf`, which is not in `FRAMES` because it
/// outlives every push and the C gives it a different `popfile`.
#[inline(always)]
pub unsafe fn pf_at(sh: &mut Shell, i: usize) -> &mut ParseFile {
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
pub unsafe fn cur_pf(sh: &mut Shell) -> &mut ParseFile {
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
pub unsafe fn cur_mark(sh: &mut Shell) -> usize {
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
pub unsafe fn input_get_lleft(pf: &ParseFile) -> c_int {
    /* #ifdef SMALL return 0; #else */
    pf.lleft
}

// [spec:dash:def:input.input-set-lleft-fn]
// [spec:dash:sem:input.input-set-lleft-fn]
pub unsafe fn input_set_lleft(pf: &mut ParseFile, len: c_int) {
    /* #ifndef SMALL */
    pf.lleft = len;
}

use crate::system::errno;

#[inline]
unsafe fn set_errno(e: c_int) {
    *libc::__errno_location() = e;
}

/* mkinit INIT fragment from src/input.c:96-99. */
pub unsafe fn mkinit_init(sh: &mut Shell) {
    /* Read before `pf_at` borrows the shell: the base parse file and the
     * streams are different fields, but `pf_at` borrows the whole shell
     * to reach one of them. */
    let stdin_fd = sh.streams.stdin;
    let base = pf_at(sh, 0);
    /* `basebuf` is a static array in the C, so re-entering `init` keeps
     * whatever it held. Only allocate when there is nothing to keep. */
    if base.buf.len() != IBUFSIZ {
        base.buf = vec![0u8; IBUFSIZ];
    }
    base.pos = 0;
    base.linno = 1;
    /* Not in the C: `basepf` is statically `.fd = 0` there because the
     * shell reads descriptor 0 by definition. Here the base parse file
     * reads whatever the frontend gave us -- which is 0 unless it said
     * otherwise. See [dec:nsh:host-owns-streams]. */
    base.fd = stdin_fd;
}

/* mkinit RESET fragment from src/input.c:101-112. */
pub unsafe fn mkinit_reset(sh: &mut crate::context::Shell) {
    let mut c: c_int;

    /* clear input buffer */
    popallfiles(sh);

    /* `toppf->nextc - toppf->buf > toppf->unget` is "at least one character
     * past the pushback window has been consumed". The C subtracts `buf`
     * from a cursor that a live `strpush` has moved into an unrelated
     * allocation; the index says what the difference was meant to say. */
    let top = pf_at(sh, sh.input.top);
    c = PEOF;
    if top.pos > top.unget as usize {
        c = text(top)[top.pos - top.unget as usize - 1] as i8 as c_int;
    }
    while c != b'\n' as c_int && c != PEOF && crate::error::int_pending() == 0 {
        /* Teardown: `reset` drains the rest of the bad line and cannot
         * fail its way out of doing so (§4.3). The loop's own
         * `int_pending` test is what stops it, and it is tested *before*
         * the read rather than after, so an interrupt ends the drain
         * rather than being reported by it. A read that fails for any
         * other reason ends it too, with the diagnostic already
         * written. */
        match pgetc(sh) {
            Ok(next) => c = next,
            Err(e) => {
                sh.status = e.status();
                drop(e);
                break;
            }
        }
    }
}

/* mkinit FORKRESET fragment from src/input.c:114-125. */
pub unsafe fn mkinit_forkreset(sh: &mut crate::context::Shell) {
    popallfiles(sh);
    /* The C tests `> 0`, meaning "an open file that is not stdin". With a
     * frontend-supplied stdin the second half of that is no longer implied
     * by the first, and getting it wrong would close the shell's own
     * input. */
    let sin: c_int = sh.streams.stdin;
    if cur_pf(sh).fd > 0 && cur_pf(sh).fd != sin {
        libc::close(cur_pf(sh).fd);
        cur_pf(sh).fd = sin;
    }
    if sh.input.stdin_state.pip[0] != 0 {
        libc::close(sh.input.stdin_state.pip[0]);
        libc::close(sh.input.stdin_state.pip[1]);
        sh.input.stdin_state.pip = [0; 2];
    }
}

/* mkinit POSTEXITRESET fragment from src/input.c:127-129. */
pub unsafe fn mkinit_postexitreset(sh: &mut Shell) {
    flush_input(sh);
}

// [spec:dash:def:input.input-init-fn]
// [spec:dash:sem:input.input-init-fn]
pub unsafe fn input_init(sh: &mut Shell) {
    let st: *mut stdin_state_t = addr_of_mut!(sh.input.stdin_state);
    let mut tios: libc::termios = core::mem::zeroed();
    let istty: c_int;

    let sin: c_int = sh.streams.stdin;

    istty = libc::tcgetattr(sin, &mut tios) + 1;
    sh.input.stdin_istty = istty;
    if istty != 0 {
        (*st).bufferable = tios.c_lflag & libc::ICANON;
    } else {
        (*st).seekable = libc::lseek(sin, 0, libc::SEEK_CUR) + 1;
        (*st).bufferable = ((*st).seekable != 0) as tcflag_t;
    }
}

// [spec:dash:def:input.stdin-bufferable-fn]
// [spec:dash:sem:input.stdin-bufferable-fn]
unsafe fn stdin_bufferable(sh: &mut Shell) -> bool {
    let st: *mut stdin_state_t = addr_of_mut!(sh.input.stdin_state);

    if sh.input.stdin_istty < 0 {
        input_init(sh);
    }

    (*st).bufferable != 0
}

// [spec:dash:def:input.flush-tee-fn]
// [spec:dash:sem:input.flush-tee-fn]
unsafe fn flush_tee(sh: &mut crate::context::Shell, buf: *mut c_void, nr: c_int, mut pending: c_int) {
    while pending > 0 {
        let err: c_int;

        err = libc::read(
            sh.streams.stdin,
            buf,
            if nr > pending { pending } else { nr } as size_t,
        ) as c_int;
        if err > 0 {
            pending -= err;
        }
    }
}

// [spec:dash:def:input.stdin-tee-fn]
// [spec:dash:sem:input.stdin-tee-fn]
unsafe fn stdin_tee(sh: &mut Shell, buf: *mut c_void, nr: c_int) -> Result<c_int, Error> {
    let err: c_int;

    if sh.input.stdin_state.pip[0] == 0 {
        /* The pointer is taken before the call, not inside its argument
         * list: `sh_pipe` now borrows the shell, and deriving the pointer
         * from the same shell in an argument borrows it twice. */
        let pip = addr_of_mut!(sh.input.stdin_state.pip) as *mut c_int;
        crate::redir::sh_pipe(sh, pip, 0)?;
        if sh.input.stdin_state.pip[0] < 10 {
            sh.input.stdin_state.pip[0] = crate::redir::savefd(sh, sh.input.stdin_state.pip[0], sh.input.stdin_state.pip[0])?;
        }
        if sh.input.stdin_state.pip[1] < 10 {
            sh.input.stdin_state.pip[1] = crate::redir::savefd(sh, sh.input.stdin_state.pip[1], sh.input.stdin_state.pip[1])?;
        }
    }

    flush_tee(sh, buf, nr, sh.input.stdin_state.pending);

    if USE_TEE != 0 {
        err = libc::tee(0, sh.input.stdin_state.pip[1], nr as size_t, 0) as c_int;
    } else {
        set_errno(libc::EINVAL);
        err = -1;
    }
    sh.input.stdin_state.pending = err;
    Ok(err)
}

/// Clear `ALIASINUSE` on everything in `list`, newest first, which is the
/// order the C's `spfree` chain walks in. The `strpush` nodes themselves are
/// dropped with the `Vec`; the C's `ckfree` on each is what that replaces.
unsafe fn release_strpush(sh: &mut crate::context::Shell, mut list: Vec<StrPush>) {
    while let Some(mut sp) = list.pop() {
        if !sp.ap.is_null() {
            (*sp.ap).flag &= !crate::alias::ALIASINUSE;
            if ((*sp.ap).flag & crate::alias::ALIASDEAD) != 0 {
                crate::alias::unalias(sh, (*sp.ap).name);
            }
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
unsafe fn freestrings(sh: &mut crate::context::Shell) {
    INTOFF();
    let list = core::mem::take(&mut cur_pf(sh).spfree);
    release_strpush(sh, list);
    INTON();
}

/*
 * Read a character from the script, returning PEOF on end of file.
 * Nul characters in the input are silently discarded.
 */

// [spec:dash:def:input.pgetc-fn]
// [spec:dash:sem:input.pgetc-fn]
pub unsafe fn pgetc(sh: &mut crate::context::Shell) -> Result<c_int, Error> {
    let mut c: c_int;
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

            return Ok(text(pf)[pf.pos - unget] as i8 as c_int);
        }

        'nextc: loop {
            if pf.nleft > 0 {
                pf.nleft -= 1;
                c = text(pf)[pf.pos] as i8 as c_int;
                pf.pos += 1;
            } else if !pf.strpush.is_empty() {
                popstring(sh);
                /* The freestrings call must be delayed til the next
                 * pgetc call for PEOA to work properly.
                 */
                pf = cur_pf(sh);
                continue 'again;
            } else {
                c = preadbuffer(sh)?;
                pf = cur_pf(sh);
            }

            /* delete nul characters */
            if IS_DEFINED_SMALL && c == 0 {
                let n = pf.nleft as usize;
                pf.buf.copy_within(pf.pos..pf.pos + n, pf.pos - 1);
                pf.pos -= 1;
                continue 'nextc;
            }

            return Ok(c);
        }
    }
}

// [spec:dash:def:input.pgetc-eoa-fn]
// [spec:dash:sem:input.pgetc-eoa-fn]
pub unsafe fn pgetc_eoa(sh: &mut crate::context::Shell) -> Result<c_int, Error> {
    let pf = cur_pf(sh);
    if !pf.strpush.is_empty() && pf.nleft == -1 && !pf.strpush[pf.strpush.len() - 1].ap.is_null() {
        Ok(PEOA)
    } else {
        pgetc(sh)
    }
}

// [spec:dash:def:input.stdin-clear-nonblock-fn]
// [spec:dash:sem:input.stdin-clear-nonblock-fn]
unsafe fn stdin_clear_nonblock(sh: &mut crate::context::Shell) -> c_int {
    let sin: c_int = sh.streams.stdin;
    let mut flags: c_int = libc::fcntl(sin, libc::F_GETFL, 0);

    if flags >= 0 {
        flags &= !libc::O_NONBLOCK;
        flags = libc::fcntl(sin, libc::F_SETFL, flags);
    }

    flags
}

// [spec:dash:def:input.preadfd-fn]
// [spec:dash:sem:input.preadfd-fn]
unsafe fn preadfd(sh: &mut crate::context::Shell) -> Result<c_int, Error> {
    let mut fd: c_int = cur_pf(sh).fd;
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
    let sin: c_int = sh.streams.stdin;

    use_tee = fd == sin
        /* #ifndef SMALL */
        && !crate::histedit::editing_active()
        && !stdin_bufferable(sh);

    pnr = nr;
    'retry: loop {
        nr = pnr;
        /* #ifndef SMALL */
        if fd == sin && crate::histedit::editing_active() {
            /* `docs/api-design.md` §5.5: nothing the shell hands to a
             * callee may borrow from the shell, and `read_edit_line`
             * takes the shell too. The buffer is moved out, filled, and
             * put back -- a `Vec`, so that is a pointer swap rather than
             * a copy. Nothing can reach this frame's buffer while it is
             * out, which is the same thing the borrow used to assert. */
            let mut buf = core::mem::take(&mut cur_pf(sh).buf);
            let result = crate::histedit::read_edit_line(
                sh,
                &mut buf[off..off + nr as usize],
            );
            cur_pf(sh).buf = buf;
            return Ok(match result {
                Ok(count) => count as c_int,
                Err(_) => 0,
            });
        }

        if use_tee {
            /* Hoisted: the receiver cannot appear twice in one argument
             * list, and a raw pointer ends its borrow at the `let`. */
            let dst = cur_pf(sh).buf.as_mut_ptr().add(off) as *mut c_void;
            nr = stdin_tee(sh, dst, nr)?;
            if nr >= 0 {
                fd = sh.input.stdin_state.pip[0];
            } else if errno() == libc::EINVAL {
                use_tee = false;
                pnr = 1;
                nr = 1;
            }
        }

        if nr > 0 {
            nr = libc::read(
                fd,
                cur_pf(sh).buf.as_mut_ptr().add(off) as *mut c_void,
                nr as size_t,
            ) as c_int;
        }

        if nr < 0 {
            if errno() == libc::EINTR && !(pf_at(sh, 0).prev.is_some() && crate::trap::pending_sig != 0)
            {
                continue 'retry;
            }
            if fd == 0 && errno() == libc::EWOULDBLOCK && stdin_clear_nonblock(sh) >= 0 {
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
 * 4) Process input up to the next newline, deleting nul characters.
 */

// [spec:dash:def:input.preadbuffer-fn]
// [spec:dash:sem:input.preadbuffer-fn]
unsafe fn preadbuffer(sh: &mut crate::context::Shell) -> Result<c_int, Error> {
    let first: c_int = (sh.input.whichprompt == 1) as c_int;
    let mut something: c_int;
    let mut savec: u8 = 0;
    let mut more: c_int;
    /* The C's `q`, as an index into `buf`. */
    let mut q: usize;
    let mut nr: c_int;
    let mut save = false;

    if (cur_pf(sh).eof & 2) != 0 {
        /* eof: */
        cur_pf(sh).eof = 3;
        return Ok(PEOF);
    }
    sh.io.flushall();

    q = cur_pf(sh).pos;
    something = (first == 0) as c_int;

    more = input_get_lleft(cur_pf(sh));

    INTOFF();
    'outer: loop {
        if more <= 0 {
            /* again: */
            nr = (q - cur_pf(sh).pos) as c_int;
            input_set_lleft(cur_pf(sh), nr);
            more = preadfd(sh)?;
            q = cur_pf(sh).pos + nr as usize;
            if more <= 0 {
                cur_pf(sh).nleft = 0;
                input_set_lleft(cur_pf(sh), 0);
                if !IS_DEFINED_SMALL && nr > 0 {
                    save = true;
                    break 'outer; /* goto save */
                }
                INTON();
                /* **An interrupted read is not end of input.** The C could
                 * not reach this line with an interrupt pending, because
                 * its delivery left from *inside* the read by longjmp;
                 * with delivery moved to a poll site, a `read` that came
                 * back EINTR arrives here looking exactly like a `read`
                 * that came back 0, and reporting PEOF makes ^C behave
                 * like ^D -- the shell exits instead of printing a fresh
                 * prompt. The pty cases `^C in emacs mode`, `^C in vi
                 * mode` and `^C during a blocked read` are what said so.
                 *
                 * The poll is here rather than inside `preadfd` because
                 * this frame holds INTOFF across the read, so nothing
                 * under it is due; this is the instruction where the
                 * counter comes back to zero, which is where the C
                 * delivered a *deferred* interrupt too. It also covers
                 * the line editor, whose failed read reaches the same
                 * line through `preadfd`'s `Err(_) => 0`. */
                if let Some(e) = crate::error::poll_interrupt(sh) {
                    return Err(e);
                }
                /* goto eof */
                cur_pf(sh).eof = 3;
                return Ok(PEOF);
            }
        }

        if IS_DEFINED_SMALL {
            q += more as usize;
            more = 0;
            break 'outer; /* goto done */
        }

        /* delete nul characters */
        loop {
            let c: c_int;

            more -= 1;
            c = cur_pf(sh).buf[q] as i8 as c_int;

            if c == 0 {
                let pf = cur_pf(sh);
                pf.buf.copy_within(q + 1..q + 1 + more as usize, q);
                /* goto check */
            } else {
                q += 1;

                if c == b'\n' as c_int {
                    break 'outer; /* goto done */
                }
                if c != b'\t' as c_int && c != b' ' as c_int {
                    something = 1;
                }
            }

            /* check: */
            if more <= 0 {
                continue 'outer; /* goto again */
            }
        }
    }

    if !save {
        /* done: */
        input_set_lleft(cur_pf(sh), more);
    }

    /* save: */
    {
        let pf = cur_pf(sh);
        pf.nleft = (q - pf.pos) as c_int - 1;
        if !IS_DEFINED_SMALL {
            savec = pf.buf[q];
        }
        pf.buf[q] = b'\0';
    }

    /* `parsefile->nextc` handed to something that wants a C string: the
     * line was NUL-terminated at `q` two statements ago. */
    let line: *mut c_char = {
        let pf = cur_pf(sh);
        pf.buf.as_mut_ptr().add(pf.pos) as *mut c_char
    };

    if cur_pf(sh).fd == sh.streams.stdin
        && crate::histedit::history_active()
        && something != 0
    {
        let bytes = {
            let pf = cur_pf(sh);
            &pf.buf[pf.pos..q]
        };
        crate::histedit::record_history_line(bytes, first != 0);
    }
    INTON();
    /* This frame brackets the read in INTOFF/INTON, so an interrupt that
     * arrived during it is pending rather than taken, and the C delivered
     * it right here when the counter reached zero. Polling at the call
     * site rather than inside `INTON` is what keeps `INTON` infallible
     * (§4.3) while leaving the delivery point where it was. */
    if let Some(e) = crate::error::poll_interrupt(sh) {
        return Err(e);
    }

    if sh.options.flag(crate::options::vflag) != 0 {
        let _ = sh.io.stderr().write_all(CStr::from_ptr(line).to_bytes());
        /* #ifdef FLUSHERR flushout(out2); */
    }

    if !IS_DEFINED_SMALL {
        cur_pf(sh).buf[q] = savec;
    }

    let pf = cur_pf(sh);
    let r = pf.buf[pf.pos] as i8 as c_int;
    pf.pos += 1;
    Ok(r)
}

// [spec:dash:def:input.pungetn-fn]
// [spec:dash:sem:input.pungetn-fn]
pub unsafe fn pungetn(sh: &mut Shell, n: c_int) {
    cur_pf(sh).unget += n;
}

/*
 * Undo a call to pgetc.  Only two characters may be pushed back.
 * PEOF may be pushed back.
 */

// [spec:dash:def:input.pungetc-fn]
// [spec:dash:sem:input.pungetc-fn]
pub unsafe fn pungetc(sh: &mut Shell) {
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
pub unsafe fn pushstring(sh: &mut Shell, s: *mut c_char, ap: *mut c_void) {
    let len: size_t;

    len = CStr::from_ptr(s).count_bytes();
    INTOFF();
    /*dprintf("*** calling pushstring: %s, %d\n", s, len);*/
    /* The C picks between `basestrpush` and a `ckmalloc` here; a `Vec`
     * needs neither, and the condition it picked on was only ever about
     * whether the inline slot was still spoken for. */
    let pf = cur_pf(sh);
    let mut string: Vec<u8> = Vec::with_capacity(len as usize + 1);
    string.extend_from_slice(core::slice::from_raw_parts(s as *const u8, len as usize));
    string.push(0);
    let sp = StrPush {
        prevpos: pf.pos,
        prevnleft: pf.nleft,
        unget: pf.unget,
        spfree: core::mem::take(&mut pf.spfree),
        ap: ap as *mut alias,
        string,
    };
    if !ap.is_null() {
        (*(ap as *mut alias)).flag |= crate::alias::ALIASINUSE;
    }
    /* The C reads on through `ap->val`, which points into `ap->name`; this
     * reads the copy, so redefining the alias mid-expansion cannot pull the
     * text out from under the cursor and `popstring` has nothing to free. */
    pf.pos = 0;
    pf.nleft = len as c_int;
    pf.unget = 0;
    pf.strpush.push(sp);
    INTON();
}

// [spec:dash:def:input.popstring-fn]
// [spec:dash:sem:input.popstring-fn]
unsafe fn popstring(sh: &mut Shell) {
    INTOFF();
    let pf = cur_pf(sh);
    let mut sp = pf.strpush.pop().unwrap();

    /* The C compares `nextc` against `sp->string`, which is `ap->name` —
     * the base of the allocation `ap->val` points into — so the test reads
     * as "always true" and the byte it then looks at is the one before the
     * cursor. Against the copy the same test means "at least one character
     * consumed", and the two agree: with none consumed the C reads the `=`
     * that ends the alias name, which is neither a space nor a tab. */
    let boundary = !sp.ap.is_null()
        && pf.pos > 0
        && matches!(sp.string[pf.pos - 1], b' ' | b'\t');
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
        sh.input.alias_boundary = 1;
    }
    INTON();
}

/*
 * Set the input to take input from a file.  If push is set, push the
 * old input onto the stack first.
 */

// [spec:dash:def:input.setinputfile-fn]
// [spec:dash:sem:input.setinputfile-fn]
pub unsafe fn setinputfile(sh: &mut crate::context::Shell, fname: *const c_char, flags: c_int) -> Result<c_int, Error> {
    let mut fd: c_int;

    INTOFF();
    fd = crate::redir::sh_open(sh, fname, libc::O_RDONLY, flags & INPUT_NOFILE_OK)?;
    if fd < 0 {
        INTON();
        return Ok(fd); /* goto out */
    }
    if fd < 10 {
        fd = crate::redir::savefd(sh, fd, fd)?;
    }
    setinputfd(sh, fd, flags & INPUT_PUSH_FILE);
    INTON();
    Ok(fd)
}

/*
 * Like setinputfile, but takes an open file descriptor.  Call this with
 * interrupts off.
 */

// [spec:dash:def:input.setinputfd-fn]
// [spec:dash:sem:input.setinputfd-fn]
unsafe fn setinputfd(sh: &mut Shell, fd: c_int, push: c_int) {
    pushfile(sh);
    if push == 0 {
        sh.input.top = sh.input.cur;
    }
    let pf = cur_pf(sh);
    pf.fd = fd;
    pf.buf = vec![0u8; IBUFSIZ];
    pf.pos = 0;
}

/*
 * Like setinputfile, but takes input from a string.
 */

// [spec:dash:def:input.setinputstring-fn]
// [spec:dash:sem:input.setinputstring-fn]
pub unsafe fn setinputstring(sh: &mut Shell, string: *mut c_char) {
    INTOFF();
    pushfile(sh);
    let len: usize = CStr::from_ptr(string).count_bytes();
    let pf = cur_pf(sh);
    /* The C points `nextc` at the caller's string and reads it in place,
     * which is why `evalstring` has to keep its `sstrdup` alive across the
     * `popfile` and why `parsebackq` cannot release the stack block it
     * grabbed. The level owns its text here. */
    pf.buf = Vec::with_capacity(len + 1);
    pf.buf
        .extend_from_slice(core::slice::from_raw_parts(string as *const u8, len));
    pf.buf.push(0);
    pf.pos = 0;
    pf.nleft = len as c_int;
    pf.eof = 2;
    INTON();
}

/*
 * To handle the "." command, a stack of input files is used.  Pushfile
 * adds a new entry to the stack and popfile restores the previous level.
 */

// [spec:dash:def:input.pushfile-fn]
// [spec:dash:sem:input.pushfile-fn]
unsafe fn pushfile(sh: &mut Shell) {
    let prev = sh.input.cur;
    sh.input.frames.push(ParseFile {
        prev: Some(prev),
        linno: 1,
        fd: -1,
        ..ParseFile::EMPTY
    });
    let depth = sh.input.frames.len();
    sh.input.cur = depth;
}

// [spec:dash:def:input.pushstdin-fn]
// [spec:dash:sem:input.pushstdin-fn]
pub unsafe fn pushstdin(sh: &mut Shell) {
    INTOFF();
    let from = sh.input.cur;
    pf_at(sh, 0).prev = Some(from);
    sh.input.cur = 0;
    INTON();
}

// [spec:dash:def:input.popfile-fn]
// [spec:dash:sem:input.popfile-fn]
pub unsafe fn popfile(sh: &mut crate::context::Shell) {
    let dying: usize = sh.input.cur;

    INTOFF();
    /* The C reads `pf->prev` into the global unconditionally, so popping
     * `basepf` when nothing pushed it leaves `parsefile` NULL; there is no
     * such value here and the base frame stays current. */
    let to = pf_at(sh, dying).prev.take().unwrap_or(0);
    sh.input.cur = to;
    if dying == 0 {
        INTON();
        return; /* goto out */
    }

    let frames = &mut *&mut sh.input.frames;
    debug_assert_eq!(dying, frames.len());
    let mut pf = frames.pop().unwrap();
    /* `set_cur(cur)` stood here to re-derive the cached frame pointer,
     * because popping the `Vec` can move the remaining frames. The index
     * does not move with them, so with the cache gone this was a
     * self-assignment and says nothing. */

    if pf.fd >= 0 {
        libc::close(pf.fd);
    }
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

    INTON();
}

// [spec:dash:def:input.unwindfiles-fn]
// [spec:dash:sem:input.unwindfiles-fn]
pub unsafe fn unwindfiles(sh: &mut crate::context::Shell, stop: usize) {
    while pf_at(sh, 0).prev.is_some() || sh.input.cur != stop {
        popfile(sh);
    }
}

/*
 * Return to top level.
 */

// [spec:dash:def:input.popallfiles-fn]
// [spec:dash:sem:input.popallfiles-fn]
pub unsafe fn popallfiles(sh: &mut crate::context::Shell) {
    /* Read out first: `toppf` is a field of the same stack `unwindfiles`
     * unwinds, so the depth is taken as a value before the call. */
    let top = sh.input.top;
    unwindfiles(sh, top);
}

// [spec:dash:def:input.flush-input-fn]
// [spec:dash:sem:input.flush-input-fn]
pub unsafe fn flush_input(sh: &mut Shell) {
    /* The frame's borrow is dropped before the stack's scalars are read:
     * `base` borrows `sh.input`, and so do they. What survives it is
     * `left` (a value) and the scratch pointer (a raw pointer, whose
     * borrow ends at the `let`). */
    let base = pf_at(sh, 0);
    let left: c_int = base.nleft + input_get_lleft(base);
    let scratch = base.buf.as_mut_ptr() as *mut c_void;

    INTOFF();
    if sh.input.stdin_state.seekable != 0 && left != 0 {
        libc::lseek(
            sh.streams.stdin,
            -(left as off_t),
            libc::SEEK_CUR,
        );
    } else if sh.input.stdin_state.pending > left {
        /* `basebuf` is scratch here; the bytes are being discarded. */
        let pending = sh.input.stdin_state.pending;
        flush_tee(sh, scratch, BUFSIZ, pending - left);
        sh.input.stdin_state.pending = 0;
    }
    let base = pf_at(sh, 0);
    base.nleft = 0;
    input_set_lleft(base, 0);
    INTON();
}

// [spec:dash:def:input.reset-input-fn]
// [spec:dash:sem:input.reset-input-fn]
pub unsafe fn reset_input(sh: &mut Shell) {
    sh.input.stdin_istty = -1;
    pf_at(sh, 0).eof = 0;
    flush_input(sh);
}
