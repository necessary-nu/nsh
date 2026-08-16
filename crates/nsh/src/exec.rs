//! Literal port of `src/exec.c` / `src/exec.h`.
//! Rules: `docs/spec/port/src/exec.md`.
//!
//! Translation notes (literal, bug-for-bug):
//!   * C `goto`s are reproduced with Rust labelled blocks and `loop`s.
//!     The block nesting mirrors the *order* of the C labels, so that
//!     `break 'label` is a forward `goto` and fall-through between two
//!     adjacent labels still happens.
//!   * `TRACE(...)` is a no-op unless `DEBUG` is defined; the default C
//!     build compiles it out, so the calls are dropped and left as
//!     comments where the trace text documented something.
//!   * `errno` is read through `__errno_location()`. `main.c` caches
//!     that pointer in `dash_errno` and the caching is reproduced in
//!     `shellmain`, but reads here go straight to libc, which is
//!     behaviourally identical.
//!
//! `cmdtable` is a `BTreeMap` keyed by command name, not the C's 31
//! chained hash buckets, so `hash` with no operands prints in name order
//! rather than in the order `hashval` happens to chain. Registered in
//! `docs/divergences.md`.

use bstr::{BStr, BString, ByteSlice};
use core::ptr::{addr_of, addr_of_mut, null, null_mut};
use libc::{c_char, c_int, size_t};
use std::collections::BTreeMap;
use std::ffi::CStr;
use std::io::Write as _;
use std::rc::Rc;

use crate::builtins::{BUILTIN_REGULAR, BUILTIN_SPECIAL, builtincmd};
use crate::error::{E_EXEC, Error, INTOFF, INTON};
use crate::nodes::{Node, funcnode};
use crate::output::Output;

// ---------------------------------------------------------------------
// src/exec.h constants
// ---------------------------------------------------------------------

/* values of cmdtype */
pub const CMDUNKNOWN: c_int = -1; /* no entry in table for command */
pub const CMDNORMAL: c_int = 0; /* command is an executable program */
pub const CMDFUNCTION: c_int = 1; /* command is a shell function */
pub const CMDBUILTIN: c_int = 2; /* command is a shell builtin */

/* action to find_command() */
pub const DO_ERR: c_int = 0x01; /* prints errors */
pub const DO_ABS: c_int = 0x02; /* checks absolute paths */
pub const DO_NOFUNC: c_int = 0x04; /* don't return shell functions, for command */
pub const DO_ALTPATH: c_int = 0x08; /* using alternate path */
pub const DO_REGBLTIN: c_int = 0x10; /* regular built-ins and functions only */

const _PATH_BSHELL: &[u8] = b"/bin/sh\0";

// ---------------------------------------------------------------------
// src/exec.h types
// ---------------------------------------------------------------------

// [spec:dash:def:exec.cmdentry.param]
#[repr(C)]
#[derive(Clone, Copy)]
pub union param {
    pub index: c_int,
    pub cmd: *const builtincmd,
    /// A borrowed view of the `Rc<Node>` owned by the command table. The
    /// evaluator increments the strong count before running the body, so a
    /// function can still redefine itself without invalidating this pointer.
    pub func: *const funcnode,
}

// [spec:dash:def:exec.cmdentry]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct cmdentry {
    pub cmdtype: c_int,
    pub u: param,
}

enum Command {
    Unknown,
    Normal(c_int),
    Function(Rc<funcnode>),
    Builtin(*const builtincmd),
}

// [spec:dash:def:exec.tblentry]
/// One cached command resolution.
///
/// The name is the `BTreeMap` key and the command kind is an enum, so the
/// intrusive `next` pointer, flexible array tail and untagged internal union
/// all disappear. Values are boxed because `find_command` keeps their address
/// across operations that can insert another command and rebalance the map.
pub struct tblentry {
    command: Command,
    pub(crate) rehash: bool,
}

impl tblentry {
    pub(crate) fn cmdtype(&self) -> c_int {
        match self.command {
            Command::Unknown => CMDUNKNOWN,
            Command::Normal(_) => CMDNORMAL,
            Command::Function(_) => CMDFUNCTION,
            Command::Builtin(_) => CMDBUILTIN,
        }
    }

    pub(crate) fn path_index(&self) -> c_int {
        match self.command {
            Command::Normal(index) => index,
            _ => unreachable!("only external commands have PATH indices"),
        }
    }

    fn builtin(&self) -> *const builtincmd {
        match self.command {
            Command::Builtin(cmd) => cmd,
            _ => unreachable!("only builtin entries have builtin pointers"),
        }
    }

    /// `builtinloc` arrives as a value rather than being read here, and
    /// that is not a style choice: the only caller is `clearcmdentry`'s
    /// `retain`, whose closure already holds the table borrowed. Reading
    /// the sibling field from inside it would be a second borrow of the
    /// same `Shell`. Copying the `c_int` out first is the whole fix, and
    /// it is exact -- nothing in the closure can change it.
    pub(crate) fn path_dependent(&self, builtinloc: c_int) -> bool {
        match self.command {
            Command::Normal(_) => true,
            Command::Builtin(cmd) => unsafe {
                ((*cmd).flags & BUILTIN_REGULAR) == 0 && builtinloc > 0
            },
            _ => false,
        }
    }

    pub(crate) unsafe fn write_to(&self, entry: *mut cmdentry) {
        (*entry).cmdtype = self.cmdtype();
        match &self.command {
            Command::Unknown => (*entry).u.index = 0,
            Command::Normal(index) => (*entry).u.index = *index,
            Command::Function(func) => (*entry).u.func = Rc::as_ptr(func),
            Command::Builtin(cmd) => (*entry).u.cmd = *cmd,
        }
    }
}

// ---------------------------------------------------------------------
// module globals
// ---------------------------------------------------------------------

/// The command hash, and where `%builtin` sits in `PATH`.
///
/// The two are one field because they are one question -- "what does
/// this name run" -- and because `clearcmdentry` reads the second while
/// rebuilding the first. `docs/api-design.md` 5 groups them, and
/// function definitions live here too because dash stores them in the
/// same hash.
pub struct CmdTable {
    /// Command names include their trailing NUL because C-shaped
    /// consumers still pass the map key straight to `padvance`. `BStr`
    /// ordering remains ordering by the command's bytes: every key has
    /// the same trailing terminator.
    map: BTreeMap<BString, Box<tblentry>>,
    /// index in path of %builtin, or -1
    builtinloc: c_int,
}

impl CmdTable {
    /// An empty hash and `builtinloc = -1`, which is what the two
    /// statics were declared with.
    pub(crate) const fn new() -> Self {
        CmdTable {
            map: BTreeMap::new(),
            builtinloc: -1,
        }
    }

    /// Whether an entry would be invalidated by a `PATH` change.
    ///
    /// Reads `builtinloc` on the caller's behalf, so a caller holding an
    /// entry does not have to reach for the sibling field itself. The
    /// two in-module walks cannot use this — they hold the map borrowed
    /// and take the `c_int` by value instead.
    pub(crate) fn path_dependent(&self, cmdp: &tblentry) -> bool {
        cmdp.path_dependent(self.builtinloc)
    }

    /// Every entry, in name order — what `hash` with no operand prints.
    pub(crate) fn iter(&self) -> impl Iterator<Item = (&BString, &tblentry)> {
        self.map.iter().map(|(name, cmdp)| (name, &**cmdp))
    }
}

pub static mut pathopt: *const c_char = null(); /* set by padvance */

use crate::system::errno;

// ---------------------------------------------------------------------

/*
 * Exec a program.  Never returns.  If you change this routine, you may
 * have to change the find_command routine as well.
 */

// [spec:dash:def:exec.shellexec-fn]
// [spec:dash:sem:exec.shellexec-fn]
pub unsafe fn shellexec(
    sh: &mut crate::context::Shell,
    argv: *mut *mut c_char,
    path: *const c_char,
    mut idx: c_int,
) -> Result<crate::eval::Flow, crate::error::Error> {
    let mut cmdname: *mut c_char;
    let e: c_int;
    let exerrno: c_int;
    let mut lpath: *const c_char = path;

    /* The C's `environment()` leaves its array in the stack allocator; ours
     * owns it, so the `Vec` has to outlive every `execve` below. */
    let envv = crate::var::environment(sh);
    let envp: *mut *mut c_char = envv.as_ptr() as *mut *mut c_char;
    if CStr::from_ptr(*argv.offset(0)).to_bytes().contains(&b'/') {
        tryexec(*argv.offset(0), argv, envp);
        e = errno();
    } else {
        let mut se: c_int = libc::ENOENT;
        while padvance(&mut lpath, *argv.offset(0)) >= 0 {
            cmdname = padvance_result();
            idx -= 1;
            if idx < 0 && pathopt.is_null() {
                tryexec(cmdname, argv, envp);
                if errno() != libc::ENOENT && errno() != libc::ENOTDIR {
                    se = errno();
                }
            }
        }
        e = se;
    }

    /* Map to POSIX errors */
    match e {
        libc::ELOOP | libc::ENAMETOOLONG | libc::ENOENT | libc::ENOTDIR => {
            exerrno = 127;
        }
        _ => {
            exerrno = 126;
        }
    }
    sh.status = exerrno;
    /* TRACE(("shellexec failed for %s, errno %d, suppressint %d\n", ...)); */
    let mut message = Vec::new();
    message.extend_from_slice(CStr::from_ptr(*argv.offset(0)).to_bytes());
    message.extend_from_slice(b": ");
    message.extend_from_slice(CStr::from_ptr(crate::error::errmsg(e, E_EXEC)).to_bytes());
    /* `exerror(EXEND, msg)`: text *and* control flow, which is why the
     * bridge took the code as a parameter rather than reading it off the
     * value. The text is written here, where dash writes it, and the value
     * it rendered from is dropped -- an `exec` that cannot happen ends the
     * shell, and `docs/api-design.md` 3.3 is explicit that what ends the
     * run is `Flow`, not `Err`. */
    /* Built before the call rather than inside its argument list: the
     * receiver is borrowed for the whole call, so reading the line out of
     * the same shell in an argument is a conflict. */
    let e = crate::error::Error::other(sh.eval.errlinno, exerrno, &message);
    drop(sh.report(e));

    /* The one place a `Result` may not be returned. `vforkexec` runs this
     * in a child that shares the parent's stack, so an `Ok` travelling out
     * of here would return through frames the parent owns and unwind them
     * under it. docs/errors-are-values.md 2.5 calls this a hard boundary
     * rather than a wrinkle, and the ending has to happen at the site.
     * This is the `_exit` that `exraise` performed for every raise and now
     * performs for the only one that can reach a vforked child. */
    if crate::siginbox::signals().vforked() != 0 {
        crate::shell::flush_coverage();
        libc::_exit(sh.status);
    }

    Ok(crate::eval::Flow::END)
}

// [spec:dash:def:exec.tryexec-fn]
// [spec:dash:sem:exec.tryexec-fn]
unsafe fn tryexec(mut cmd: *mut c_char, mut argv: *mut *mut c_char, envp: *mut *mut c_char) {
    let path_bshell: *mut c_char = _PATH_BSHELL.as_ptr() as *mut c_char;

    loop {
        // repeat:
        libc::execve(
            cmd,
            argv as *const *const c_char,
            envp as *const *const c_char,
        );
        if cmd != path_bshell && errno() == libc::ENOEXEC {
            /* *argv-- = cmd; */
            *argv = cmd;
            argv = argv.offset(-1);
            /* *argv = cmd = path_bshell; */
            cmd = path_bshell;
            *argv = cmd;
            continue; // goto repeat
        }
        break;
    }
}

// [spec:dash:def:exec.legal-pathopt-fn]
// [spec:dash:sem:exec.legal-pathopt-fn]
unsafe fn legal_pathopt(
    mut opt: *const c_char,
    term: *const c_char,
    magic: c_int,
) -> *const c_char {
    match magic {
        0 => {
            opt = null();
        }

        1 => {
            /* GNU `?:` — prefix(opt, "builtin") ?: prefix(opt, "func") */
            let p = crate::mystring::prefix(opt, b"builtin\0".as_ptr() as *const c_char);
            opt = if !p.is_null() {
                p as *const c_char
            } else {
                crate::mystring::prefix(opt, b"func\0".as_ptr() as *const c_char) as *const c_char
            };
        }

        _ => {
            /* `strcspn(opt, term)`: the length of the run before the
             * first byte that is in `term`, which is `find_byteset`
             * with the miss meaning "all of it". */
            let rest = CStr::from_ptr(opt).to_bytes();
            let end = rest
                .find_byteset(CStr::from_ptr(term).to_bytes())
                .unwrap_or(rest.len());
            opt = opt.add(end);
        }
    }

    if !opt.is_null() && *opt == b'%' as c_char {
        opt = opt.add(1);
    }

    opt
}

/*
 * Do a path search.  The variable path (passed by reference) should be
 * set to the start of the path before the first call; padvance will update
 * this value as it proceeds.  Successive calls to padvance will return
 * the possible path expansions in sequence.  If an option (indicated by
 * a percent sign) appears in the path entry then the global variable
 * pathopt will be set to point to it; otherwise pathopt will be set to
 * NULL.
 *
 * If magic is 0 then pathopt recognition will be disabled.  If magic is
 * 1 we shall recognise %builtin/%func.  Otherwise we shall accept any
 * pathopt.
 */

// [spec:dash:def:exec.padvance-magic-fn]
// [spec:dash:sem:exec.padvance-magic-fn]
pub unsafe fn padvance_magic(path: &mut *const c_char, name: *const c_char, magic: c_int) -> c_int {
    let mut term: *const c_char = b"%:\0".as_ptr() as *const c_char;
    let mut lpathopt: *const c_char;
    let mut p: *const c_char;
    let mut q: *mut c_char;
    let mut start: *const c_char;
    let qlen: size_t;
    let mut len: size_t;

    if (*path).is_null() {
        return -1;
    }

    lpathopt = null();
    start = *path;

    if *start == b'%' as c_char && {
        p = legal_pathopt(start.add(1), term, magic);
        !p.is_null()
    } {
        lpathopt = start.add(1);
        start = p;
        term = b":\0".as_ptr() as *const c_char;
    }

    let rest = CStr::from_ptr(start).to_bytes();
    len = rest
        .find_byteset(CStr::from_ptr(term).to_bytes())
        .unwrap_or(rest.len());
    p = start.add(len);

    if *p == b'%' as c_char {
        /* `strchrnul(p, ':') - p` is the distance to the next colon or to
         * the end, which is what a miss returning the length spells. */
        let rest = CStr::from_ptr(p).to_bytes();
        let extra: size_t = rest.find_byte(b':').unwrap_or(rest.len());

        if !legal_pathopt(p.add(1), term, magic).is_null() {
            lpathopt = p.add(1);
        } else {
            len += extra;
        }

        p = p.add(extra);
    }

    pathopt = lpathopt;
    *path = if *p == b':' as c_char {
        p.add(1)
    } else {
        null()
    };

    /* "2" is for '/' and '\0' -- the name's bytes already carry the
     * second, so what is added here is the separator. */
    let name_bytes = CStr::from_ptr(name).to_bytes_with_nul();
    qlen = len + name_bytes.len() + 1;
    let buf = &mut *addr_of_mut!(pathbuf);
    buf.clear();
    buf.reserve(qlen);

    if len != 0 {
        buf.extend_from_slice(core::slice::from_raw_parts(start as *const u8, len));
        buf.push(b'/');
    }
    q = buf.as_mut_ptr().add(buf.len()) as *mut c_char;
    /* The name and its terminator go into the reserved tail; `qlen` is
     * what the C's `growstackto` guaranteed room for, and it is one more
     * than the bytes written when `len` is zero. */
    core::ptr::copy_nonoverlapping(name_bytes.as_ptr(), q as *mut u8, name_bytes.len());
    let n = buf.len() + name_bytes.len();
    buf.set_len(n);

    qlen as c_int
}

/// The candidate path [`padvance_magic`] builds.
///
/// The C builds it at `stackblock()` and hands the caller the *length* it
/// reserved room for, so a caller that wants to keep the candidate calls
/// `stalloc(len)` to take exactly that block. That is why `len` is the
/// return value and not the string: it is an allocation size, not a
/// strlen, and it is two larger than the path when the path component is
/// empty. Callers that kept the candidate now copy it instead.
static mut pathbuf: BString = BString::new(Vec::new());

/// The candidate path the last `padvance` built, as a C string.
pub unsafe fn padvance_result() -> *mut c_char {
    (*addr_of_mut!(pathbuf)).as_mut_ptr() as *mut c_char
}

// [spec:dash:def:exec.padvance-fn]
// [spec:dash:sem:exec.padvance-fn]
#[inline]
pub unsafe fn padvance(path: &mut *const c_char, name: *const c_char) -> c_int {
    padvance_magic(path, name, 1)
}

/*** Command hashing code ***/

// [spec:dash:def:exec.test-exec-fn]
// [spec:dash:sem:exec.test-exec-fn]
unsafe fn test_exec(fullname: *const c_char, statb: *mut libc::stat64) -> c_int {
    if ((*statb).st_mode & libc::S_IFMT) != libc::S_IFREG {
        return 0;
    }

    if ((*statb).st_mode & 0o111) != 0o111 &&
        /* HAVE_FACCESSAT; the non-faccessat build uses test_access(statb, X_OK) */
        test_file_access(fullname, libc::X_OK) == 0
    {
        return 0;
    }

    1
}

/*
 * Resolve a command name.  If you change this routine, you may have to
 * change the shellexec routine as well.
 */

// [spec:dash:def:exec.find-command-fn]
// [spec:dash:sem:exec.find-command-fn]
pub unsafe fn find_command(
    sh: &mut crate::context::Shell,
    name: *mut c_char,
    entry: *mut cmdentry,
    mut act: c_int,
    path: *const c_char,
) -> Result<crate::eval::Flow, Error> {
    let mut cmdp: *mut tblentry;
    let mut idx: c_int;
    let mut prev: c_int;
    let mut fullname: *mut c_char;
    let mut statb: libc::stat64 = core::mem::zeroed();
    let mut e: c_int;
    let mut updatetbl: c_int;
    let mut bcmd: *const builtincmd;
    let mut len: c_int;
    let mut lpath: *const c_char = path;

    /* If name contains a slash, don't use PATH or hash table */
    if CStr::from_ptr(name).to_bytes().contains(&b'/') {
        (*entry).u.index = -1;
        'absdone: {
            if (act & DO_ABS) != 0 {
                'absfail: {
                    while libc::stat64(name, &mut statb) < 0 {
                        /* SYSV: retry on EINTR */
                        break 'absfail;
                    }
                    if test_exec(name, &mut statb) == 0 {
                        break 'absfail;
                    }
                    break 'absdone;
                }
                // absfail:
                (*entry).cmdtype = CMDUNKNOWN;
                return Ok(crate::eval::Flow::Done(0));
            }
        }
        (*entry).cmdtype = CMDNORMAL;
        return Ok(crate::eval::Flow::Done(0));
    }

    updatetbl = (path == crate::var::pathval(sh)) as c_int;
    if updatetbl == 0 {
        act |= DO_ALTPATH;
    }

    bcmd = null();

    'success: {
        'builtin_success: {
            'fail: {
                /* If name is in the table, check answer will be ok */
                cmdp = cmdlookup(sh, name, 0);
                if !cmdp.is_null() {
                    let bit: c_int;

                    match (*cmdp).cmdtype() {
                        CMDFUNCTION => {
                            bit = DO_NOFUNC;
                        }
                        CMDBUILTIN => {
                            bit = if ((*(*cmdp).builtin()).flags & BUILTIN_REGULAR) != 0 {
                                0
                            } else {
                                DO_REGBLTIN
                            };
                        }
                        /* `default:` (DEBUG: abort()) falls through to CMDNORMAL */
                        _ => {
                            bit = DO_ALTPATH | DO_REGBLTIN;
                        }
                    }
                    if (act & bit) != 0 {
                        if (act & bit & DO_REGBLTIN) != 0 {
                            break 'fail;
                        }

                        updatetbl = 0;
                        cmdp = null_mut();
                    } else if !(*cmdp).rehash {
                        /* if not invalidated by cd, we're done */
                        break 'success;
                    }
                }

                /* If %builtin not in path, check for builtin next */
                bcmd = find_builtin(name);
                if !bcmd.is_null()
                    && ((((*bcmd).flags & BUILTIN_REGULAR) as c_int)
                        | (act & DO_ALTPATH)
                        | ((sh.commands.builtinloc <= 0) as c_int))
                        != 0
                {
                    break 'builtin_success;
                }

                if (act & DO_REGBLTIN) != 0 {
                    break 'fail;
                }

                /* We have to search path. */
                prev = -1; /* where to start */
                if !cmdp.is_null() && (*cmdp).rehash {
                    /* doing a rehash */
                    if (*cmdp).cmdtype() == CMDBUILTIN {
                        prev = sh.commands.builtinloc;
                    } else {
                        prev = (*cmdp).path_index();
                    }
                }

                e = libc::ENOENT;
                idx = -1;
                'padvloop: loop {
                    // loop:
                    len = padvance(&mut lpath, name);
                    if len < 0 {
                        break 'padvloop;
                    }
                    let lpathopt: *const c_char = pathopt;

                    fullname = padvance_result();
                    idx += 1;
                    if !lpathopt.is_null() {
                        if *lpathopt == b'b' as c_char {
                            if !bcmd.is_null() {
                                break 'builtin_success;
                            }
                            continue 'padvloop;
                        } else if (act & DO_NOFUNC) == 0 {
                            /* handled below */
                        } else {
                            /* ignore unimplemented options */
                            continue 'padvloop;
                        }
                    }
                    /* if rehash, don't redo absolute path names */
                    if *fullname.offset(0) == b'/' as c_char && idx <= prev {
                        if idx < prev {
                            continue 'padvloop;
                        }
                        /* TRACE(("searchexec \"%s\": no change\n", name)); */
                        break 'success;
                    }
                    loop {
                        if libc::stat64(fullname, &mut statb) >= 0 {
                            break;
                        }
                        /* SYSV: retry on EINTR */
                        if errno() != libc::ENOENT && errno() != libc::ENOTDIR {
                            e = errno();
                        }
                        continue 'padvloop; // goto loop
                    }
                    if !lpathopt.is_null() {
                        /* this is a %func directory */
                        /* `stalloc(len)` took the candidate out of the way
                         * because `readcmdfile` runs shell code that can
                         * search the path again; the copy is what keeps it,
                         * and `stunalloc` is the copy going out of scope. */
                        let kept = (*addr_of!(pathbuf)).clone();
                        let fullname = kept.as_ptr() as *mut c_char;
                        /* A `%func` PATH entry is a file of shell code, so
                         * it can `exit`; the C's longjmp took that straight
                         * past `find_command` and its callers, and this
                         * returns it through them instead. It is why
                         * `find_command` carries a `Flow` at all. */
                        match crate::shellmain::readcmdfile(sh, fullname)? {
                            crate::eval::Flow::Done(_) => {}
                            exit @ crate::eval::Flow::Exit { .. } => return Ok(exit),
                        }
                        cmdp = cmdlookup(sh, name, 0);
                        if cmdp.is_null() || (*cmdp).cmdtype() != CMDFUNCTION {
                            let mut message = Vec::new();
                            message.extend_from_slice(CStr::from_ptr(name).to_bytes());
                            message.extend_from_slice(b" not defined in ");
                            message.extend_from_slice(CStr::from_ptr(fullname).to_bytes());
                            return Err(sh.sh_error_value(&message));
                        }
                        break 'success;
                    }
                    e = libc::EACCES; /* if we fail, this will be the error */
                    if test_exec(fullname, &mut statb) == 0 {
                        continue 'padvloop;
                    }
                    /* TRACE(("searchexec \"%s\" returns \"%s\"\n", name, fullname)); */
                    if updatetbl == 0 {
                        (*entry).cmdtype = CMDNORMAL;
                        (*entry).u.index = idx;
                        return Ok(crate::eval::Flow::Done(0));
                    }
                    INTOFF();
                    cmdp = cmdlookup(sh, name, 1);
                    (*cmdp).command = Command::Normal(idx);
                    INTON();
                    break 'success;
                }

                /* We failed.  If there was an entry for this command, delete it */
                if !cmdp.is_null() && updatetbl != 0 {
                    delete_cmd_entry(sh, name);
                }
                if (act & DO_ERR) != 0 {
                    let mut message = Vec::new();
                    message.extend_from_slice(CStr::from_ptr(name).to_bytes());
                    message.extend_from_slice(b": ");
                    message.extend_from_slice(
                        CStr::from_ptr(crate::error::errmsg(e, E_EXEC)).to_bytes(),
                    );
                    sh.sh_warnx(&message);
                }
                // fall through into fail:
            }
            // fail:
            (*entry).cmdtype = CMDUNKNOWN;
            return Ok(crate::eval::Flow::Done(0));
        }
        // builtin_success:
        if updatetbl == 0 {
            (*entry).cmdtype = CMDBUILTIN;
            (*entry).u.cmd = bcmd;
            return Ok(crate::eval::Flow::Done(0));
        }
        INTOFF();
        cmdp = cmdlookup(sh, name, 1);
        (*cmdp).command = Command::Builtin(bcmd);
        INTON();
        // fall through into success:
    }
    // success:
    (*cmdp).rehash = false;
    (*cmdp).write_to(entry);
    Ok(crate::eval::Flow::Done(0))
}

/*
 * Search the table of builtin commands.
 */

// [spec:dash:def:exec.find-builtin-fn]
// [spec:dash:sem:exec.find-builtin-fn]
pub unsafe fn find_builtin(name: *const c_char) -> *const builtincmd {
    let name = BStr::new(CStr::from_ptr(name).to_bytes());
    crate::builtins::builtincmd
        .binary_search_by(|cmd| BStr::new(cmd.name.to_bytes()).cmp(name))
        .map_or(null(), |index| &crate::builtins::builtincmd[index])
}

/*
 * Called when a cd is done.  Marks all commands so the next time they
 * are executed they will be rehashed.
 */

// [spec:dash:def:exec.hashcd-fn]
// [spec:dash:sem:exec.hashcd-fn]
pub unsafe fn hashcd(sh: &mut crate::context::Shell) {
    /* Copied out for the same reason `clearcmdentry` copies it: the
     * walk below holds the table borrowed. */
    let builtinloc = sh.commands.builtinloc;
    for cmdp in sh.commands.map.values_mut() {
        if cmdp.path_dependent(builtinloc) {
            cmdp.rehash = true;
        }
    }
}

/*
 * Fix command hash table when PATH changed.
 * Called before PATH is changed.  The argument is the new value of PATH;
 * pathval() still returns the old value at this point.
 * Called with interrupts off.
 */

// [spec:dash:def:exec.changepath-fn]
// [spec:dash:sem:exec.changepath-fn]
pub unsafe fn changepath(sh: &mut crate::context::Shell, newval: *const c_char) {
    let mut new: *const c_char;
    let mut idx: c_int;
    let mut bltin: c_int;

    new = newval;
    idx = 0;
    bltin = -1;
    loop {
        if *new == b'%' as c_char
            && !crate::mystring::prefix(new.add(1), b"builtin\0".as_ptr() as *const c_char)
                .is_null()
        {
            bltin = idx;
            break;
        }
        match CStr::from_ptr(new).to_bytes().find_byte(b':') {
            Some(at) => new = new.add(at + 1),
            None => break,
        }
        idx += 1;
    }
    sh.commands.builtinloc = bltin;
    clearcmdentry(sh);
}

/*
 * Clear out command entries.  The argument specifies the first entry in
 * PATH which has changed.
 */

// [spec:dash:def:exec.clearcmdentry-fn]
// [spec:dash:sem:exec.clearcmdentry-fn]
pub(crate) unsafe fn clearcmdentry(sh: &mut crate::context::Shell) {
    INTOFF();
    let builtinloc = sh.commands.builtinloc;
    sh.commands.map.retain(|_, cmdp| !cmdp.path_dependent(builtinloc));
    INTON();
}

/*
 * Locate a command in the command hash table.  If "add" is nonzero,
 * add the command to the table if it is not already present.  The
 * Interrupts must be off if called with add != 0.
 */

// [spec:dash:def:exec.cmdlookup-fn]
// [spec:dash:sem:exec.cmdlookup-fn]
pub(crate) unsafe fn cmdlookup(sh: &mut crate::context::Shell, name: *const c_char, add: c_int) -> *mut tblentry {
    let name = BStr::new(CStr::from_ptr(name).to_bytes_with_nul());
    if add != 0 {
        &mut **sh.commands.map.entry(name.to_owned()).or_insert_with(|| {
            Box::new(tblentry {
                command: Command::Unknown,
                rehash: false,
            })
        })
    } else {
        sh.commands.map
            .get_mut(name)
            .map_or(null_mut(), |cmdp| &mut **cmdp)
    }
}

/*
 * Delete a command table entry by name.
 */

// [spec:dash:def:exec.delete-cmd-entry-fn]
// [spec:dash:sem:exec.delete-cmd-entry-fn]
pub(crate) unsafe fn delete_cmd_entry(sh: &mut crate::context::Shell, name: *const c_char) {
    INTOFF();
    /* Own the lookup key before mutating the map. This also makes deletion
     * sound if a future caller passes a pointer into the stored key itself. */
    let name = BStr::new(CStr::from_ptr(name).to_bytes_with_nul()).to_owned();
    sh.commands.map.remove(BStr::new(name.as_slice()));
    INTON();
}

// [spec:dash:def:exec.getcmdentry-fn]
// [spec:dash:sem:exec.getcmdentry-fn]
//
// The whole function lives inside `#ifdef notdef` in `src/exec.c`
// (lines 698-712) and is not compiled into the shell. It is carried
// here as an annotated, never-compiled stub so the manifest symbol has
// a target site; `#[cfg(any())]` is the Rust equivalent of the
// unsatisfiable `#ifdef notdef` guard, and the body is the literal
// translation of the dead C.
#[cfg(any())]
pub unsafe fn getcmdentry(name: *mut c_char, entry: *mut cmdentry) {
    let cmdp: *mut tblentry = cmdlookup(sh, name, 0);

    if !cmdp.is_null() {
        (*cmdp).write_to(entry);
    } else {
        (*entry).cmdtype = CMDUNKNOWN;
        (*entry).u.index = 0;
    }
}

/*
 * Add a new command entry, replacing any existing command entry for
 * the same name - except special builtins.
 */

// [spec:dash:def:exec.addcmdentry-fn]
// [spec:dash:sem:exec.addcmdentry-fn]
unsafe fn addcmdentry(sh: &mut crate::context::Shell, name: *mut c_char, command: Command) {
    let cmdp: *mut tblentry;

    cmdp = cmdlookup(sh, name, 1);
    (*cmdp).command = command;
    (*cmdp).rehash = false;
}

/*
 * Define a shell function.
 */

// [spec:dash:def:exec.defun-fn]
// [spec:dash:sem:exec.defun-fn]
pub unsafe fn defun(sh: &mut crate::context::Shell, func: &Node) {
    INTOFF();
    addcmdentry(sh, 
        func.ndefun().text.as_ptr(),
        Command::Function(Rc::new(func.clone())),
    );
    INTON();
}

/*
 * Delete a function if it exists.
 */

// [spec:dash:def:exec.unsetfunc-fn]
// [spec:dash:sem:exec.unsetfunc-fn]
pub unsafe fn unsetfunc(sh: &mut crate::context::Shell, name: *const c_char) {
    let cmdp: *mut tblentry;

    cmdp = cmdlookup(sh, name, 0);
    if !cmdp.is_null() && (*cmdp).cmdtype() == CMDFUNCTION {
        delete_cmd_entry(sh, name);
    }
}

/*
 * Locate and print what a word is...
 */

// ---------------------------------------------------------------------
// src/exec.h declarations whose definitions live in src/bltin/test.c.
//
// The manifest attributes these two symbols to `src/exec.h` (the
// declaration site), while the bodies are owned by the `test.*` rules
// in `src/bltin/test.c`. Rust has no separate declaration form, so the
// `exec.*` annotation is carried on a forwarding wrapper.
// ---------------------------------------------------------------------

// [spec:dash:def:exec.test-file-access-fn]
// [spec:dash:sem:exec.test-file-access-fn]
#[inline]
pub unsafe fn test_file_access(path: *const c_char, mode: c_int) -> c_int {
    crate::builtins::test::test_file_access(path, mode)
}

// [spec:dash:def:exec.test-access-fn]
// [spec:dash:sem:exec.test-access-fn]
#[inline]
pub unsafe fn test_access(sp: *const libc::stat64, stmode: c_int) -> c_int {
    crate::builtins::test::test_access(sp, stmode)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `%builtin`'s position in `PATH` is what makes a cached entry
    /// stale, and `changepath` is the variable hook that finds it. This
    /// is the field the hook could not reach before it carried a
    /// receiver.
    // [spec:dash:sem:exec.changepath-fn/test]
    #[test]
    fn changepath_files_the_builtin_slot() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut owned = crate::context::Shell::new();
            let sh = &mut owned;

            changepath(sh, c"/bin:%builtin:/usr/bin".as_ptr());
            assert_eq!(sh.commands.builtinloc, 1);

            changepath(sh, c"%builtin:/bin".as_ptr());
            assert_eq!(sh.commands.builtinloc, 0);

            changepath(sh, c"/bin:/usr/bin".as_ptr());
            assert_eq!(sh.commands.builtinloc, -1, "no %builtin is -1, not 0");
        }
    }

    /// What `clearcmdentry` keeps, which is the predicate the walk runs
    /// while it holds the table borrowed. An external command is always
    /// invalidated by a `PATH` change; an entry that names nothing is
    /// not. Pinned because the `builtinloc` the predicate reads is now
    /// copied out before the walk rather than read inside it, and a
    /// wrong copy would show up here as the wrong survivor.
    // [spec:dash:sem:exec.clearcmdentry-fn/test]
    #[test]
    fn clearing_drops_only_path_dependent_entries() {
        let _g = crate::testutil::lock();
        unsafe {
            let mut owned = crate::context::Shell::new();
            let sh = &mut owned;

            let external = c"Texternal";
            let unknown = c"Tunknown";
            addcmdentry(sh, external.as_ptr() as *mut c_char, Command::Normal(0));
            cmdlookup(sh, unknown.as_ptr(), 1);

            /* The lookup is hoisted because writing it inline is the
             * very borrow this commit exists to avoid -- `cmdlookup`
             * takes `&mut sh` while `path_dependent` holds `&sh`. A raw
             * pointer parked in a local is the way through, here and at
             * the call site in `hashcmd`. */
            let e = cmdlookup(sh, external.as_ptr(), 0);
            assert!(sh.commands.path_dependent(&*e));
            let u = cmdlookup(sh, unknown.as_ptr(), 0);
            assert!(!sh.commands.path_dependent(&*u));

            clearcmdentry(sh);

            assert!(
                cmdlookup(sh, external.as_ptr(), 0).is_null(),
                "an external command does not survive a PATH change"
            );
            assert!(
                !cmdlookup(sh, unknown.as_ptr(), 0).is_null(),
                "an entry naming nothing has nothing to invalidate"
            );
        }
    }

    // [spec:dash:sem:exec.find-builtin-fn/test]
    #[test]
    fn generated_builtin_lookup_round_trips() {
        unsafe {
            for expected in &crate::builtins::builtincmd {
                assert!(core::ptr::eq(
                    find_builtin(expected.name.as_ptr()),
                    expected,
                ));
            }

            for absent in [c"", c"/", c"alia", c"aliasx", c"waitx", c"zz"] {
                assert!(find_builtin(absent.as_ptr()).is_null());
            }
        }
    }

    /// `printf` is a builtin, and finding it here is what keeps a script
    /// off the PATH search: with `PATH` empty or `printf` missing from
    /// it, the utility is still there. See
    /// `[dec:nsh:printf-is-parsed-not-interpreted]`.
    #[test]
    fn printf_is_a_builtin() {
        unsafe {
            let found = find_builtin(c"printf".as_ptr());
            assert!(!found.is_null());
            assert!(core::ptr::eq(found, crate::builtins::PRINTFCMD));
        }
        /* `echo` shares printf.c with it and is the neighbouring row. */
        unsafe {
            assert!(!find_builtin(c"echo".as_ptr()).is_null());
        }
    }

    /// The table is binary-searched, so its order is load-bearing —
    /// adding a row must not have disturbed it.
    #[test]
    fn the_builtin_table_stays_sorted() {
        let names: Vec<&[u8]> = crate::builtins::builtincmd
            .iter()
            .map(|cmd| cmd.name.to_bytes())
            .collect();
        let mut sorted = names.clone();
        sorted.sort();
        assert_eq!(names, sorted);
        assert_eq!(names.len(), crate::builtins::NUMBUILTINS);
    }
}
