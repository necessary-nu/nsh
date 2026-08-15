//! Literal port of `src/options.c` / `src/options.h`.
//! Rules: `docs/spec/port/src/options.md`.
//!
//! `optlist`, `optnames` and `optletters` are three parallel views of the same
//! option and **must stay in the same order**.  The `eflag`/`fflag`/… names of
//! `options.h` become `usize` indices, so a call site reads `optlist[eflag]`
//! and stays assignable exactly like the C macro.

use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString};
use core::ptr::{addr_of, addr_of_mut, null_mut};
use libc::{c_char, c_int, c_uint, size_t};
use std::ffi::{CStr, CString};
use std::io::Write;

use crate::error::{INTOFF, INTON};
use crate::mystring::nullstr;
use crate::var::{VNOFUNC, VUNSET, setvar, setvarint, showvars};

/// The positional parameters when the shell owns them — the C's
/// `shellparam.malloc`.
///
/// `ptrs` is the `char **` that `getopts` walks and `$@` iterates, and it
/// holds the address of each word's bytes, so a word may be dropped or the
/// whole list rebuilt but a word must never be resized in place. Each word
/// carries its own terminator because every reader reads it as a C string.
struct Params {
    words: Vec<BString>,
    ptrs: Vec<*mut c_char>,
}

impl Params {
    fn new(words: Vec<BString>) -> Params {
        let mut p = Params {
            words,
            ptrs: Vec::new(),
        };
        p.reindex();
        p
    }

    fn reindex(&mut self) {
        self.ptrs.clear();
        self.ptrs.reserve(self.words.len() + 1);
        for w in &mut self.words {
            self.ptrs.push(w.as_mut_ptr() as *mut c_char);
        }
        self.ptrs.push(null_mut());
    }
}

// [spec:dash:def:options.shparam]
/// The C's `malloc` flag and `p` pointer are one field: `owned` is `Some`
/// exactly when `malloc` was 1, and `borrowed` is the `argv` `p` pointed
/// into when it was 0.
pub struct shparam {
    pub nparam: c_int, /* # of positional parameters (without $0) */
    pub optind: c_int, /* next parameter to be processed by getopts */
    pub optoff: c_int, /* used by getopts */
    owned: Option<Params>,
    borrowed: *mut *mut c_char,
}

impl shparam {
    pub const fn new() -> shparam {
        shparam {
            nparam: 0,
            optind: 0,
            optoff: 0,
            owned: None,
            borrowed: null_mut(),
        }
    }

    /// Drop the first `n` parameters: what `shift` does, in the module
    /// that knows how they are stored.
    ///
    /// The C shifts the array down whether or not it owns the words, so a
    /// `shift` inside a function rewrites the caller's `argv`;
    /// `evalcommand` discards it when the call returns.
    pub(crate) unsafe fn drop_first(&mut self, n: c_int) {
        self.nparam -= n;
        match &mut self.owned {
            Some(o) => {
                o.words.drain(..n as usize);
                o.reindex();
            }
            None => {
                let mut ap1: *mut *mut c_char = self.borrowed.add(n as usize);
                let mut ap2: *mut *mut c_char = self.borrowed;
                loop {
                    *ap2 = *ap1;
                    let done = (*ap2).is_null();
                    ap2 = ap2.add(1);
                    ap1 = ap1.add(1);
                    if done {
                        break;
                    }
                }
            }
        }
        self.optind = 1;
        self.optoff = -1;
    }

    /// `shellparam.p` — the NULL-terminated array, wherever it lives.
    pub(crate) fn p(&mut self) -> *mut *mut c_char {
        match &mut self.owned {
            Some(o) => o.ptrs.as_mut_ptr(),
            None => self.borrowed,
        }
    }
}

pub const NOPTS: usize = 18;

/*
 * options.h spells these `#define eflag optlist[0]` etc.  The port keeps the
 * names as the *index* into `optlist`, so a call site reads
 * `optlist[iflag]` — assignable exactly like the C macro.
 */
pub const eflag: usize = 0;
pub const fflag: usize = 1;
pub const Iflag: usize = 2;
pub const iflag: usize = 3;
pub const mflag: usize = 4;
pub const nflag: usize = 5;
pub const sflag: usize = 6;
pub const xflag: usize = 7;
pub const vflag: usize = 8;
pub const Vflag: usize = 9;
pub const Eflag: usize = 10;
pub const Cflag: usize = 11;
pub const aflag: usize = 12;
pub const bflag: usize = 13;
pub const uflag: usize = 14;
pub const nolog: usize = 15;
pub const pipefail: usize = 16;
pub const debug: usize = 17;

pub static mut arg0: *mut c_char = null_mut(); /* value of $0 */

/* `static const char *const optnames[NOPTS]`.
 *
 * `static` rather than `static mut`: it is never written, and it was only
 * mutable because a `[*const c_char]` is not `Sync`. `&CStr` is, and it
 * carries the NUL the readers below rely on in the type instead of in a
 * comment. Same move `defifsvar` made in `var.rs`. */
static optnames: [&CStr; NOPTS] = [
    c"errexit",
    c"noglob",
    c"ignoreeof",
    c"interactive",
    c"monitor",
    c"noexec",
    c"stdin",
    c"xtrace",
    c"verbose",
    c"vi",
    c"emacs",
    c"noclobber",
    c"allexport",
    c"notify",
    c"nounset",
    c"nolog",
    c"pipefail",
    c"debug",
];

pub static optletters: [c_char; NOPTS] = [
    b'e' as c_char,
    b'f' as c_char,
    b'I' as c_char,
    b'i' as c_char,
    b'm' as c_char,
    b'n' as c_char,
    b's' as c_char,
    b'x' as c_char,
    b'v' as c_char,
    b'V' as c_char,
    b'E' as c_char,
    b'C' as c_char,
    b'a' as c_char,
    b'b' as c_char,
    b'u' as c_char,
    0,
    0,
    0,
];

/// The shell's option flags — `set -e`, `set -x`, `-i` and the rest.
///
/// `docs/api-design.md` 5 calls the field `options`; the type cannot be
/// `Options` because that name is already this module's builtin option
/// *parser*, which is a different thing and stays call-scoped per 5.2.
///
/// The array is private and reached through [`ShellOptions::flag`] and
/// [`ShellOptions::set_flag`]. That is worth the extra characters at 80
/// call sites: an index into a bare `[c_char; NOPTS]` is exactly the
/// shape that let any module write any flag, and the accessors are what
/// make "who sets `-e`" answerable.
pub struct ShellOptions {
    flags: [c_char; NOPTS],
    /// `shellparam` — the positional parameters, `$1` onwards, and
    /// `getopts`' place in them.
    ///
    /// `pub(crate)` rather than private: `getopts.rs` and `shift.rs`
    /// read and write its members directly and there is no invariant
    /// across them the flags array has. It is here because
    /// `docs/api-design.md` §5 puts it here — one row for everything
    /// `set` and the option scan own.
    pub(crate) shellparam: shparam,
    /// `minusc` — the argument to `-c`, or NULL.
    pub(crate) minusc: *mut c_char,
}

impl ShellOptions {
    /// `optlist` was declared all-zero.
    pub(crate) const fn new() -> Self {
        ShellOptions {
            flags: [0; NOPTS],
            shellparam: shparam::new(),
            minusc: null_mut(),
        }
    }

    #[inline]
    pub(crate) fn flag(&self, which: usize) -> c_char {
        self.flags[which]
    }

    #[inline]
    pub(crate) fn set_flag(&mut self, which: usize, to: c_char) {
        self.flags[which] = to;
    }

    /// The whole flag set, copied.
    ///
    /// `local -` saves it and `poplocalvars` puts it back, and that is
    /// the only place the flags move as a unit. It is also the seam
    /// where `options` and `vars` are genuinely entangled -- a
    /// `localvar` holds a copy of this array -- so it is a named pair of
    /// methods rather than a public field, and whoever moves `vars`
    /// reads this comment first.
    pub(crate) fn snapshot(&self) -> [c_char; NOPTS] {
        self.flags
    }

    pub(crate) fn restore(&mut self, saved: [c_char; NOPTS]) {
        self.flags = saved;
    }
}

/*
 * Process the shell command line arguments.
 */

// [spec:dash:def:options.procargs-fn]
// [spec:dash:sem:options.procargs-fn]
pub unsafe fn procargs(sh: &mut crate::context::Shell, mut xargv: *mut *mut c_char) -> Result<c_int, Error> {
    let mut i: c_int;
    let mut login: c_int;

    login = (!(*xargv.offset(0)).is_null() && **xargv.offset(0) == b'-' as c_char) as c_int;
    arg0 = *xargv.offset(0);
    if !(*xargv.offset(0)).is_null() {
        xargv = xargv.add(1);
    }
    i = 0;
    while i < NOPTS as c_int {
        sh.options.set_flag(i as usize, 2);
        i += 1;
    }
    /* `options` reports what the C left in `argptr` and `minusc`: how
     * far it got, and whether `-c` was given. The pointer the C stores in
     * `minusc` is only ever read as a flag before the line below
     * overwrites it with the command itself. */
    sh.options.minusc = null_mut();
    let words = argv_words(xargv);
    let scan = options(sh, &words, 0, true)?;
    login |= scan.login;
    xargv = xargv.add(scan.next);
    if (*xargv).is_null() {
        if scan.minus_c {
            return Err(crate::error::sh_error_value(b"-c requires an argument"));
        }
        sh.options.set_flag(sflag, 1);
    }
    if sh.options.flag(iflag) == 2 && sh.options.flag(sflag) == 1 {
        crate::input::input_init(sh);
        if sh.input.stdin_istty != 0 && libc::isatty(crate::streams::streams().stderr) != 0 {
            sh.options.set_flag(iflag, 1);
        }
    }
    if sh.options.flag(mflag) == 2 {
        sh.options.set_flag(mflag, sh.options.flag(iflag));
    }
    i = 0;
    while i < NOPTS as c_int {
        if sh.options.flag(i as usize) == 2 {
            sh.options.set_flag(i as usize, 0);
        }
        i += 1;
    }
    /* #if DEBUG == 2 — not selected in this configuration:
     *     debug = 1;
     */
    /* POSIX 1003.2: first arg after -c cmd is $0, remainder $1... */
    let mut setarg0 = false;
    if scan.minus_c {
        sh.options.minusc = *xargv;
        xargv = xargv.add(1);
        if !(*xargv).is_null() {
            setarg0 = true; /* goto setarg0 */
        }
    } else if sh.options.flag(sflag) == 0 {
        crate::input::setinputfile(sh, *xargv, 0)?;
        setarg0 = true;
    }
    if setarg0 {
        arg0 = *xargv;
        xargv = xargv.add(1);
    }

    /* assert(shellparam.malloc == 0 && shellparam.nparam == 0); */
    let mut nparam: c_int = 0;
    let mut count = xargv;
    while !(*count).is_null() {
        nparam += 1;
        count = count.add(1);
    }
    borrowparam(sh, xargv, nparam);
    optschanged(sh)?;

    Ok(login)
}

// [spec:dash:def:options.optschanged-fn]
// [spec:dash:sem:options.optschanged-fn]
/// Returns rather than raising, because `setjobctl` can fail and one of
/// this function's callers is teardown. See `jobs::setjobctl`.
pub unsafe fn optschanged(sh: &mut crate::context::Shell) -> Result<(), crate::error::Error> {
    /* `#ifdef DEBUG opentrace();` — the dash build does not define DEBUG,
     * so `show.c` compiles to nothing and there is no trace file. */
    crate::trap::setinteractive(sh, sh.options.flag(iflag) as c_int);
    /* #ifndef SMALL */
    crate::histedit::histedit(sh);
    crate::jobs::setjobctl(sh, sh.options.flag(mflag) as c_int)
}

/// What a pass of [`options`] found.
///
/// The C reads all three back out of globals: the return value, `argptr`,
/// and `minusc`.
#[derive(Debug)]
pub(crate) struct Scan {
    /// The C's return value.
    pub(crate) login: c_int,
    /// The first word the scan did not consume: the C's `argptr`.
    pub(crate) next: usize,
    /// `-c` was given. The C records it by pointing `minusc` into the
    /// word, but every reader of that pointer treats it as a flag --
    /// `procargs` replaces it with the command before anything reads the
    /// bytes -- so a flag is what it is.
    pub(crate) minus_c: bool,
}

/*
 * Process shell options.  The global variable argptr contains a pointer
 * to the argument list; we advance it past the options.
 */

// [spec:dash:def:options.options-fn]
// [spec:dash:sem:options.options-fn]
pub(crate) unsafe fn options(
    sh: &mut crate::context::Shell,
    args: &[&BStr],
    start: usize,
    cmdline: bool,
) -> Result<Scan, Error> {
    let mut val: c_int = 0;
    let mut scan = Scan {
        login: 0,
        next: start,
        minus_c: false,
    };

    loop {
        let Some(word) = args.get(scan.next) else {
            break;
        };
        scan.next += 1;
        /* `c = *p++`: the first byte decides, and the cluster starts at
         * the second. An empty word takes the `else` and is put back. */
        let c = word.first().copied().unwrap_or(0);
        if c == b'-' {
            val = 1;
            if word.len() == 1 || &word[..] == b"--" {
                if !cmdline {
                    /* "-" means turn off -x and -v */
                    if word.len() == 1 {
                        sh.options.set_flag(vflag, 0);
                        sh.options.set_flag(xflag, sh.options.flag(vflag));
                    }
                    /* "--" means reset params */
                    else if scan.next >= args.len() {
                        setparam(sh, &args[scan.next..]);
                    }
                }
                break; /* "-" or "--" terminates options */
            }
        } else if c == b'+' {
            val = 0;
        } else {
            scan.next -= 1;
            break;
        }
        let mut i = 1usize;
        loop {
            let Some(&c) = word.get(i) else {
                break;
            };
            i += 1;
            if c == b'c' && cmdline {
                scan.minus_c = true; /* command is after shell args */
            } else if c == b'l' && cmdline {
                scan.login = 1;
            } else if c == b'o' {
                minus_o(sh, args.get(scan.next).copied(), val)?;
                if scan.next < args.len() {
                    scan.next += 1;
                }
            } else {
                setoption(sh, c, val)?;
            }
        }
    }

    Ok(scan)
}

/// The words of a NUL-terminated `char **`, for the one caller that is
/// still handed one: the process's own argv, which the frontend owns.
unsafe fn argv_words<'a>(argv: *mut *mut c_char) -> Vec<&'a BStr> {
    let mut words = Vec::new();
    let mut p = argv;
    while !(*p).is_null() {
        words.push(BStr::new(CStr::from_ptr(*p).to_bytes()));
        p = p.add(1);
    }
    words
}

// [spec:dash:def:options.minus-o-fn]
// [spec:dash:sem:options.minus-o-fn]
unsafe fn minus_o(sh: &mut crate::context::Shell, name: Option<&BStr>, val: c_int) -> Result<(), Error> {
    let mut i: c_int;

    let name = name.map(crate::shell::cstring);
    if name.is_none() {
        if val != 0 {
            let heading = b"Current option settings\n";
            let _ = (*crate::output::stdout()).write_all(heading);
            i = 0;
            while i < NOPTS as c_int {
                let name = optnames[i as usize].to_bytes();
                let mut line = name.to_vec();
                if line.len() < 16 {
                    line.resize(16, b' ');
                }
                line.extend_from_slice(if sh.options.flag(i as usize) != 0 {
                    b"on\n"
                } else {
                    b"off\n"
                });
                let _ = (*crate::output::stdout()).write_all(&line);
                i += 1;
            }
        } else {
            i = 0;
            while i < NOPTS as c_int {
                let mut line = b"set ".to_vec();
                line.extend_from_slice(if sh.options.flag(i as usize) != 0 {
                    b"-o "
                } else {
                    b"+o "
                });
                line.extend_from_slice(optnames[i as usize].to_bytes());
                line.push(b'\n');
                let _ = (*crate::output::stdout()).write_all(&line);
                i += 1;
            }
        }
    } else {
        let name = name.expect("the naming branch");
        i = 0;
        while i < NOPTS as c_int {
            if name.as_bytes() == optnames[i as usize].to_bytes() {
                sh.options.set_flag(i as usize, val as c_char);
                return Ok(());
            }
            i += 1;
        }
        let mut message = b"Illegal option -o ".to_vec();
        message.extend_from_slice(name.as_bytes());
        return Err(crate::error::sh_error_value(&message));
    }
    Ok(())
}

// [spec:dash:def:options.setoption-fn]
// [spec:dash:sem:options.setoption-fn]
unsafe fn setoption(sh: &mut crate::context::Shell, flag: u8, val: c_int) -> Result<(), Error> {
    let mut i: c_int;

    i = 0;
    while i < NOPTS as c_int {
        if optletters[i as usize] as u8 == flag {
            sh.options.set_flag(i as usize, val as c_char);
            if val != 0 {
                /* #%$ hack for ksh semantics */
                if flag == b'V' {
                    sh.options.set_flag(Eflag, 0);
                } else if flag == b'E' {
                    sh.options.set_flag(Vflag, 0);
                }
            }
            return Ok(());
        }
        i += 1;
    }
    let mut message = b"Illegal option -".to_vec();
    message.push(flag);
    Err(crate::error::sh_error_value(&message))
}

/*
 * Set the shell parameters.
 */

// [spec:dash:def:options.setparam-fn]
// [spec:dash:sem:options.setparam-fn]
pub unsafe fn setparam(sh: &mut Shell, argv: &[&BStr]) {
    let nparam: c_int = argv.len() as c_int;

    /* Copied out in full before the old list goes, as the C's
     * `savestr` loop is: `freeparam` comes after the copy there too. */
    let words: Vec<BString> = argv
        .iter()
        .map(|w| BString::from(crate::shell::cstring(w).into_bytes_with_nul()))
        .collect();
    let param = &mut *addr_of_mut!(sh.options.shellparam);
    param.owned = Some(Params::new(words));
    param.borrowed = null_mut();
    param.nparam = nparam;
    param.optind = 1;
    param.optoff = -1;
}

/// `shellparam.malloc = 0; shellparam.p = argv` — the parameters a function
/// call installs are its caller's `argv`, borrowed for the length of the
/// call and rewritten in place by `shift`.
pub unsafe fn borrowparam(sh: &mut Shell, argv: *mut *mut c_char, nparam: c_int) {
    let param = &mut *addr_of_mut!(sh.options.shellparam);
    param.owned = None;
    param.borrowed = argv;
    param.nparam = nparam;
    param.optind = 1;
    param.optoff = -1;
}

/// `saveparam = shellparam`, which is a copy in the C only because
/// `shellparam.malloc = 0` on the next line disarms the `freeparam` that
/// would otherwise free what the copy still points at. One move says both.
pub unsafe fn takeparam(sh: &mut Shell) -> shparam {
    core::mem::replace(&mut *addr_of_mut!(sh.options.shellparam), shparam::new())
}

/// `freeparam(&shellparam); shellparam = saveparam;`
pub unsafe fn restoreparam(sh: &mut Shell, saved: shparam) {
    freeparam(addr_of_mut!(sh.options.shellparam));
    sh.options.shellparam = saved;
}

/*
 * Free the list of positional parameters.
 */

// [spec:dash:def:options.freeparam-fn]
// [spec:dash:sem:options.freeparam-fn]
pub unsafe fn freeparam(param: *mut shparam) {
    (*param).owned = None;
}

/*
 * The shift builtin command.
 */

/*
 * The set command builtin.
 */

// [spec:dash:def:options.getoptsreset-fn]
// [spec:dash:sem:options.getoptsreset-fn]
pub unsafe fn getoptsreset(sh: &mut crate::context::Shell, value: *const c_char) {
    sh.options.shellparam.optind = 1;
    sh.options.shellparam.optoff = -1;
}

/*
 * The getopts builtin.  Shellparam.optnext points to the next argument
 * to be processed.  Shellparam.optptr points to the next character to
 * be processed in the current argument.  If shellparam.optnext is NULL,
 * then it's the first time getopts has been called.
 */

/// The option scan a builtin runs over its own arguments.
///
/// This is `nextopt` with its state made local. dash keeps that state in
/// three globals -- `argptr`, `optptr` and `optionarg` -- which
/// `evalbltin` reinitialises before every builtin, and which the builtin
/// reads back after the scan to find its operands. The reinitialisation
/// is the tell: the state was never shared, only ambient, so it belongs to
/// the builtin that is scanning.
///
/// The C's comment above `nextopt` asks for it to be replaced by
/// `getopt(3)`, and says why it cannot be: the library's version keeps
/// *its* state in a process global, which a shell cannot reset portably.
/// Neither can this one, which is why it is a value.
///
/// [`Options::operands`] is the `argptr` a builtin reads afterwards.
pub struct Options<'a> {
    args: &'a [&'a BStr],
    /// The next word to look at: dash's `argptr`.
    next: usize,
    /// How far a run of clustered options has got through a word already
    /// consumed: dash's `optptr`. `None` is its NULL.
    run: Option<(usize, usize)>,
    /// dash's `optionarg`.
    optionarg: Option<&'a BStr>,
}

impl<'a> Options<'a> {
    /// Scan `args` from the first word after the command name, which is
    /// where `evalbltin`'s `argptr = argv + 1` starts.
    pub fn new(args: &'a [&'a BStr]) -> Self {
        Self::from(args, 1)
    }

    /// Scan from `start`, for the one caller whose word list does not
    /// begin with a command name: `procargs` reads the shell's own.
    pub fn from(args: &'a [&'a BStr], start: usize) -> Self {
        Options {
            args,
            next: start.min(args.len()),
            run: None,
            optionarg: None,
        }
    }

    /// The next option, or `None` at the end of the options -- dash's
    /// `'\0'`.
    ///
    /// `optstring` is the C's, minus its terminator: a letter, optionally
    /// followed by `:` to say the option takes an argument.
    ///
    /// # Safety
    ///
    /// An unrecognised option or a missing option argument raises, so this
    /// unwinds through the caller's frame like every other `sh_error`.
    // [spec:dash:def:options.nextopt-fn]
    // [spec:dash:sem:options.nextopt-fn]
    pub unsafe fn next(&mut self, optstring: &[u8]) -> Result<Option<u8>, Error> {
        /* `p = optptr; if (p == NULL || *p == '\0')` -- the run in
         * progress is exhausted, so the next word starts a new one. */
        let (word, mut off) = match self.run {
            Some((w, off)) if off < self.args[w].len() => (w, off),
            _ => {
                let w = self.next;
                /* `p == NULL || *p != '-' || *++p == '\0'`: the end of
                 * the list, a word that is not an option, or a lone `-`.
                 * None of the three is consumed. The `?` this used to take
                 * on the `Option` is spelled out now that the scan can
                 * fail for a second reason. */
                let Some(&word) = self.args.get(w) else {
                    return Ok(None);
                };
                if word.first() != Some(&b'-') || word.len() < 2 {
                    return Ok(None);
                }
                self.next = w + 1; /* argptr++ */
                if &word[..] == b"--" {
                    /* consumed, and it ends the options */
                    return Ok(None);
                }
                (w, 1)
            }
        };

        let c = self.args[word][off];
        off += 1;

        /* Find `c` in the option string.  A `:` belongs to the option
         * before it, so the scan steps over one; running off the end is
         * the C reading its terminator, and the option is not ours. */
        let mut q = 0usize;
        loop {
            let cur = optstring.get(q).copied().unwrap_or(0);
            if cur == c {
                break;
            }
            if cur == 0 {
                let mut message = b"Illegal option -".to_vec();
                message.push(c);
                /* A stop: the loop would spin on the terminator. */
                return Err(crate::error::sh_error_value(&message));
            }
            q += 1;
            if optstring.get(q) == Some(&b':') {
                q += 1;
            }
        }

        q += 1;
        if optstring.get(q) == Some(&b':') {
            /* The option takes an argument: the rest of this word if
             * there is any, otherwise the next word. */
            let bytes = self.args[word];
            if off < bytes.len() {
                self.optionarg = Some(BStr::new(&bytes[off..]));
            } else {
                match self.args.get(self.next) {
                    Some(a) => {
                        self.optionarg = Some(a);
                        self.next += 1;
                    }
                    None => {
                        let mut message = b"No arg for -".to_vec();
                        message.push(c);
                        message.extend_from_slice(b" option");
                        /* A stop: `arg()` would otherwise be asked for an
                         * `optionarg` that was never set. */
                        return Err(crate::error::sh_error_value(&message));
                    }
                }
            }
            self.run = None; /* p = NULL */
        } else {
            self.run = Some((word, off));
        }

        Ok(Some(c))
    }

    /// The argument of the option just returned: dash's `optionarg`.
    ///
    /// Only an option the option string marked with `:` has one, and
    /// [`Options::next`] raises rather than return such an option without
    /// it, so a caller that asks in the right place always gets it.
    pub fn arg(&self) -> &'a BStr {
        self.optionarg
            .expect("an option marked `:` has an argument or does not return")
    }

    /// The words the scan stopped in front of: dash's `argptr`, read back
    /// after `nextopt` has returned `'\0'`.
    pub fn operands(&self) -> &'a [&'a BStr] {
        &self.args[self.next..]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /* The scanner's two complaints, as values. The corpus sees the bytes
     * on stderr; only this sees which of the two produced them, and that
     * the scan stopped rather than carried on with a half-applied set of
     * options. */

    #[test]
    fn an_unknown_letter_returns_its_complaint() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new();
        let sh = &mut owned;
        let args = [BStr::new("set"), BStr::new("-Q")];

        let e = unsafe { options(sh, &args, 1, false) }.expect_err("-Q is not an option");

        assert_eq!(e.message().to_vec(), b"Illegal option -Q".to_vec());
        assert_eq!(e.status(), 2);
    }

    #[test]
    fn an_unknown_name_returns_its_complaint() {
        let _g = crate::testutil::lock();
        let mut owned = crate::context::Shell::new();
        let sh = &mut owned;
        let args = [BStr::new("set"), BStr::new("-o"), BStr::new("nosuchopt")];

        let e = unsafe { options(sh, &args, 1, false) }.expect_err("-o nosuchopt is not an option");

        assert_eq!(e.message().to_vec(), b"Illegal option -o nosuchopt".to_vec());
    }

    /// `Options` is `nextopt` with its state made local, so what it has to
    /// agree with is the C's walk, edge for edge. These are the edges:
    /// which words the scan consumes is what decides where the operands
    /// start, and every builtin reads its operands from there.
    fn scan<'a>(args: &'a [&'a BStr], optstring: &[u8]) -> (Vec<u8>, Vec<&'a BStr>) {
        let mut opts = Options::new(args);
        let mut seen = Vec::new();
        /* `Ok(Some(c))` would end the scan silently on an error and make
         * a failure look like a short option list, so the error is taken
         * loudly: every option string these cases use accepts every
         * option they hand it. */
        while let Some(c) = unsafe { opts.next(optstring) }
            .expect("the scan's cases never pass an option the string rejects")
        {
            seen.push(c);
        }
        (seen, opts.operands().to_vec())
    }

    fn words<'a>(raw: &'a [&'a [u8]]) -> Vec<&'a BStr> {
        raw.iter().map(|w| BStr::new(*w)).collect()
    }

    #[test]
    fn non_option_word_stops_scan() {
        let args = words(&[b"jobs", b"%1", b"-l"]);
        let (seen, operands) = scan(&args, b"lp");
        assert!(seen.is_empty());
        assert_eq!(operands, words(&[b"%1", b"-l"]));
    }

    #[test]
    fn options_cluster_within_one_word() {
        let args = words(&[b"jobs", b"-lp", b"%1"]);
        let (seen, operands) = scan(&args, b"lp");
        assert_eq!(seen, b"lp");
        assert_eq!(operands, words(&[b"%1"]));
    }

    #[test]
    fn option_arg_from_same_word() {
        let args = words(&[b"read", b"-pPROMPT", b"var"]);
        let mut opts = Options::new(&args);
        assert_eq!(unsafe { opts.next(b"p:r") }.unwrap(), Some(b'p'));
        assert_eq!(opts.arg(), BStr::new(b"PROMPT"));
        assert_eq!(unsafe { opts.next(b"p:r") }.unwrap(), None);
        assert_eq!(opts.operands(), words(&[b"var"]));
    }

    #[test]
    fn option_arg_from_next_word() {
        let args = words(&[b"read", b"-p", b"PROMPT", b"var"]);
        let mut opts = Options::new(&args);
        assert_eq!(unsafe { opts.next(b"p:r") }.unwrap(), Some(b'p'));
        assert_eq!(opts.arg(), BStr::new(b"PROMPT"));
        assert_eq!(unsafe { opts.next(b"p:r") }.unwrap(), None);
        assert_eq!(opts.operands(), words(&[b"var"]));
    }

    /// A `:` in the option string belongs to the option in front of it, so
    /// the search for a letter has to step over one. `r` is reachable only
    /// if it does.
    #[test]
    fn search_skips_arg_marker() {
        let args = words(&[b"read", b"-r", b"var"]);
        let (seen, operands) = scan(&args, b"p:r");
        assert_eq!(seen, b"r");
        assert_eq!(operands, words(&[b"var"]));
    }

    #[test]
    fn double_dash_ends_scan_consumed() {
        let args = words(&[b"unalias", b"--", b"-a"]);
        let (seen, operands) = scan(&args, b"a");
        assert!(seen.is_empty());
        assert_eq!(operands, words(&[b"-a"]));
    }

    /// A lone `-` ends the scan like `--` does, but the C returns before
    /// `argptr++`, so it stays an operand. `cd -` is the case that cares.
    #[test]
    fn lone_dash_ends_scan_unconsumed() {
        let args = words(&[b"cd", b"-"]);
        let (seen, operands) = scan(&args, b"LP");
        assert!(seen.is_empty());
        assert_eq!(operands, words(&[b"-"]));
    }

    #[test]
    fn options_spread_over_words() {
        let args = words(&[b"jobs", b"-l", b"-p", b"%1", b"%2"]);
        let (seen, operands) = scan(&args, b"lp");
        assert_eq!(seen, b"lp");
        assert_eq!(operands, words(&[b"%1", b"%2"]));
    }

    #[test]
    fn scan_to_end_leaves_no_operands() {
        let args = words(&[b"jobs", b"-l"]);
        let (seen, operands) = scan(&args, b"lp");
        assert_eq!(seen, b"l");
        assert!(operands.is_empty());
    }

    /// The empty option string is what a builtin that takes no options
    /// passes: it accepts nothing and exists to eat a `--`.
    /// The scan `set` and the shell's command line share. What it reports
    /// is where it stopped, which is what decides the positional
    /// parameters -- so the boundary between options and operands is the
    /// property worth pinning.
    fn scan_options(raw: &[&[u8]], cmdline: bool) -> (usize, bool, c_int) {
        let _guard = crate::testutil::lock();
        let mut owned = crate::context::Shell::new();
        let sh = &mut owned;
        let args = words(raw);
        let scan = unsafe { options(sh, &args, 0, cmdline) }.expect("these cases scan cleanly");
        (scan.next, scan.minus_c, scan.login)
    }

    #[test]
    fn scan_stops_at_the_first_operand() {
        let (next, _, _) = scan_options(&[b"-x", b"file", b"-y"], false);
        assert_eq!(next, 1);
    }

    #[test]
    fn scan_consumes_a_double_dash() {
        let (next, _, _) = scan_options(&[b"--", b"a"], false);
        assert_eq!(next, 1);
    }

    /// A lone `-` ends the options and is consumed -- unlike the builtin
    /// scan, where it stays an operand.
    #[test]
    fn scan_consumes_a_lone_dash() {
        let (next, _, _) = scan_options(&[b"-", b"a"], false);
        assert_eq!(next, 1);
    }

    #[test]
    fn minus_o_takes_next_word() {
        let (next, _, _) = scan_options(&[b"-o", b"noglob", b"rest"], false);
        assert_eq!(next, 2);
    }

    /// `-c` and `-l` are command-line only: as a `set` option `-l` is an
    /// ordinary letter, and `set -c` is an error rather than a command.
    #[test]
    fn minus_c_is_command_line_only() {
        let (_, minus_c, login) = scan_options(&[b"-c", b"echo hi"], true);
        assert!(minus_c);
        let (_, _, login_off) = scan_options(&[b"-l"], true);
        assert_eq!((login, login_off), (0, 1));
    }

    #[test]
    fn empty_word_is_not_an_option() {
        let (next, _, _) = scan_options(&[b"", b"-x"], false);
        assert_eq!(next, 0);
    }

    #[test]
    fn empty_optstring_eats_double_dash() {
        let args = words(&[b".", b"--", b"file"]);
        let (seen, operands) = scan(&args, b"");
        assert!(seen.is_empty());
        assert_eq!(operands, words(&[b"file"]));
    }
}
