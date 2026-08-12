//! Literal port of `src/bltin/test.c` — the `test` / `[` builtin.
//! Rules: `docs/spec/port/src/bltin/test.md`.
//!
//! `test_access` and `test_file_access` are also declared in `exec.h` and
//! used by command lookup, so they are `pub` here and carry both the
//! `test.*` and the `exec.*` rule ids.
//!
//! Configuration ported: `HAVE_FACCESSAT` defined (true on Linux/glibc),
//! `HAVE_TRADITIONAL_FACCESSAT` undefined (configure only turns it on for
//! FreeBSD / GNU-kFreeBSD), `HAVE_ST_MTIM` defined, `DEBUG` undefined.
//! The bodies guarded by the other side of each `#if` are kept as comments
//! where they change behaviour, and `test_access` — the `#else` branch of
//! `HAVE_FACCESSAT` — is compiled in regardless because `exec.c` needs it.
//!
//! Cross-module signature assumed (see the port report):
//! `crate::mystring::atomax10(*const c_char) -> intmax_t`.

use core::mem;
use core::ptr;
use libc::{c_char, c_int, c_short, intmax_t};
use bstr::BStr;
use std::ffi::{CStr, CString};

/* test(1) accepts the following grammar:
    oexpr	::= aexpr | aexpr "-o" oexpr ;
    aexpr	::= nexpr | nexpr "-a" aexpr ;
    nexpr	::= primary | "!" primary
    primary	::= unary-operator operand
        | operand binary-operator operand
        | operand
        | "(" oexpr ")"
        ;
    unary-operator ::= "-r"|"-w"|"-x"|"-f"|"-d"|"-c"|"-b"|"-p"|
        "-u"|"-g"|"-k"|"-s"|"-t"|"-z"|"-n"|"-o"|"-O"|"-G"|"-L"|"-S";

    binary-operator ::= "="|"!="|"-eq"|"-ne"|"-ge"|"-gt"|"-le"|"-lt"|
            "-nt"|"-ot"|"-ef";
    operand ::= <any legal UNIX file name>
*/

// [spec:dash:def:test.token]
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum token {
    EOI = 0,
    FILRD,
    FILWR,
    FILEX,
    FILEXIST,
    FILREG,
    FILDIR,
    FILCDEV,
    FILBDEV,
    FILFIFO,
    FILSOCK,
    FILSYM,
    FILGZ,
    FILTT,
    FILSUID,
    FILSGID,
    FILSTCK,
    FILNT,
    FILOT,
    FILEQ,
    FILUID,
    FILGID,
    STREZ,
    STRNZ,
    STREQ,
    STRNE,
    STRLT,
    STRGT,
    INTEQ,
    INTNE,
    INTGE,
    INTGT,
    INTLE,
    INTLT,
    UNOT,
    BAND,
    BOR,
    LPAREN,
    RPAREN,
    OPERAND,
}

// [spec:dash:def:test.token-types]
#[repr(i32)]
#[derive(Clone, Copy, PartialEq, Eq)]
pub enum token_types {
    UNOP = 0,
    BINOP,
    BUNOP,
    BBINOP,
    PAREN,
}

// [spec:dash:def:test.t-op]
#[repr(C)]
#[derive(Clone, Copy)]
pub struct t_op {
    pub op_text: *const c_char,
    pub op_num: c_short,
    pub op_type: c_short,
}

/// Table-entry constructor; only exists because Rust has no designated
/// aggregate initialiser syntax that can call `CStr::as_ptr` inline.
const fn op(text: &'static core::ffi::CStr, num: token, ty: token_types) -> t_op {
    t_op {
        op_text: text.as_ptr(),
        op_num: num as c_short,
        op_type: ty as c_short,
    }
}

/// `static struct t_op const ops[]` (src/bltin/test.c:91-135).
///
/// Rendered as `static mut` rather than `static` only because a `static`
/// holding a raw pointer would have to be `Sync`; the table is never
/// written.
static mut ops: [t_op; 40] = [
    op(c"-r", token::FILRD, token_types::UNOP),
    op(c"-w", token::FILWR, token_types::UNOP),
    op(c"-x", token::FILEX, token_types::UNOP),
    op(c"-e", token::FILEXIST, token_types::UNOP),
    op(c"-f", token::FILREG, token_types::UNOP),
    op(c"-d", token::FILDIR, token_types::UNOP),
    op(c"-c", token::FILCDEV, token_types::UNOP),
    op(c"-b", token::FILBDEV, token_types::UNOP),
    op(c"-p", token::FILFIFO, token_types::UNOP),
    op(c"-u", token::FILSUID, token_types::UNOP),
    op(c"-g", token::FILSGID, token_types::UNOP),
    op(c"-k", token::FILSTCK, token_types::UNOP),
    op(c"-s", token::FILGZ, token_types::UNOP),
    op(c"-t", token::FILTT, token_types::UNOP),
    op(c"-z", token::STREZ, token_types::UNOP),
    op(c"-n", token::STRNZ, token_types::UNOP),
    op(c"-h", token::FILSYM, token_types::UNOP), /* for backwards compat */
    op(c"-O", token::FILUID, token_types::UNOP),
    op(c"-G", token::FILGID, token_types::UNOP),
    op(c"-L", token::FILSYM, token_types::UNOP),
    op(c"-S", token::FILSOCK, token_types::UNOP),
    op(c"=", token::STREQ, token_types::BINOP),
    op(c"!=", token::STRNE, token_types::BINOP),
    op(c"<", token::STRLT, token_types::BINOP),
    op(c">", token::STRGT, token_types::BINOP),
    op(c"-eq", token::INTEQ, token_types::BINOP),
    op(c"-ne", token::INTNE, token_types::BINOP),
    op(c"-ge", token::INTGE, token_types::BINOP),
    op(c"-gt", token::INTGT, token_types::BINOP),
    op(c"-le", token::INTLE, token_types::BINOP),
    op(c"-lt", token::INTLT, token_types::BINOP),
    op(c"-nt", token::FILNT, token_types::BINOP),
    op(c"-ot", token::FILOT, token_types::BINOP),
    op(c"-ef", token::FILEQ, token_types::BINOP),
    op(c"!", token::UNOT, token_types::BUNOP),
    op(c"-a", token::BAND, token_types::BBINOP),
    op(c"-o", token::BOR, token_types::BBINOP),
    op(c"(", token::LPAREN, token_types::PAREN),
    op(c")", token::RPAREN, token_types::PAREN),
    t_op {
        op_text: ptr::null(),
        op_num: 0,
        op_type: 0,
    },
];

static mut t_wp: *mut *mut c_char = ptr::null_mut();
static mut t_wp_op: *const t_op = ptr::null();

/// `enum token` is what `t_lex` and `binop` switch on, but the table stores
/// the code in a `short`; C converts implicitly on return.
#[inline]
unsafe fn token_of(n: c_short) -> token {
    mem::transmute(n as c_int)
}

/// configure: `--enable-test-workaround`, defaulted on only for the
/// FreeBSD / GNU-kFreeBSD kernels (configure.ac:110-126). Off for Linux.
const HAVE_TRADITIONAL_FACCESSAT: bool = false;

// [spec:dash:def:test.faccessat-confused-about-superuser-fn]
// [spec:dash:sem:test.faccessat-confused-about-superuser-fn]
#[inline]
unsafe fn faccessat_confused_about_superuser() -> c_int {
    if HAVE_TRADITIONAL_FACCESSAT { 1 } else { 0 }
}

// [spec:dash:def:test.getn-fn]
// [spec:dash:sem:test.getn-fn]
#[inline]
unsafe fn getn(s: *const c_char) -> intmax_t {
    crate::mystring::atomax10(s)
}

// [spec:dash:def:test.getop-fn]
// [spec:dash:sem:test.getop-fn]
unsafe fn getop(s: *const c_char) -> *const t_op {
    let mut op: *const t_op;

    op = ptr::addr_of!(ops) as *const t_op;
    while !(*op).op_text.is_null() {
        if CStr::from_ptr(s).to_bytes() == CStr::from_ptr((*op).op_text).to_bytes() {
            return op;
        }
        op = op.add(1);
    }

    ptr::null()
}

// [spec:dash:def:test.testcmd-fn]
// [spec:dash:sem:test.testcmd-fn]
pub unsafe fn testcmd(args: &[&BStr]) -> c_int {
    let mut op: *const t_op;
    let n: token;
    let mut res: c_int = 1;

    /* The expression walker below is a recursive descent over the words
     * sharing one cursor (`t_wp`), and `[` writes into the array to drop
     * its own `]`. Both want the words as an array for the length of the
     * call, so this builtin builds one -- rather than `evalcommand`
     * building one for every builtin whether it wanted one or not. */
    let words: Vec<CString> = args.iter().map(|w| crate::shell::cstring(w)).collect();
    let mut slots: Vec<*mut c_char> = words.iter().map(|w| w.as_ptr() as *mut c_char).collect();
    slots.push(ptr::null_mut());
    let mut argc: c_int = args.len() as c_int;
    let mut argv: *mut *mut c_char = slots.as_mut_ptr();

    if **argv == b'[' as c_char {
        argc -= 1;
        if *(*argv.add(argc as usize)) != b']' as c_char {
            crate::error::sh_error(b"missing ]");
        }
        *argv.add(argc as usize) = ptr::null_mut();
    }

    t_wp_op = ptr::null();

    // `goto eval` leaves the loop with `n` already chosen.
    'eval: {
        'recheck: loop {
            argv = argv.add(1);
            argc -= 1;

            if argc < 1 {
                return res;
            }

            /*
             * POSIX prescriptions: he who wrote this deserves the Nobel
             * peace prize.
             */
            // switch (argc) { case 3: ... /* fall through */ case 4: ... }
            if argc == 3 {
                op = getop(*argv.add(1));
                if !op.is_null() && (*op).op_type == token_types::BINOP as c_short {
                    n = token::OPERAND;
                    break 'eval;
                }
                /* fall through */
            }
            if argc == 3 || argc == 4 {
                if CStr::from_ptr(*argv).to_bytes() == b"("
                    && CStr::from_ptr(*argv.add((argc - 1) as usize)).to_bytes() == b")"
                {
                    argc -= 1;
                    *argv.add(argc as usize) = ptr::null_mut();
                    argv = argv.add(1);
                    argc -= 1;
                } else if CStr::from_ptr(*argv).to_bytes() == b"!" {
                    res = 0;
                    continue 'recheck;
                }
            }

            n = t_lex(argv);
            break 'recheck;
        }
    }

    // eval:
    t_wp = argv;
    res ^= oexpr(n);
    argv = t_wp;

    if !(*argv).is_null() && !(*argv.add(1)).is_null() {
        syntax(*argv, c"unexpected operator".as_ptr());
    }

    res
}

// [spec:dash:def:test.syntax-fn]
// [spec:dash:sem:test.syntax-fn]
unsafe fn syntax(op: *const c_char, msg: *const c_char) -> ! {
    let mut message = Vec::new();
    if !op.is_null() && *op != 0 {
        message.extend_from_slice(CStr::from_ptr(op).to_bytes());
        message.extend_from_slice(b": ");
    }
    message.extend_from_slice(CStr::from_ptr(msg).to_bytes());
    crate::error::sh_error(&message)
}

// [spec:dash:def:test.oexpr-fn]
// [spec:dash:sem:test.oexpr-fn]
unsafe fn oexpr(mut n: token) -> c_int {
    let mut res: c_int = 0;

    loop {
        res |= aexpr(n);
        if (*t_wp).is_null() {
            break;
        }
        n = t_lex(t_wp.add(1));
        if n != token::BOR {
            break;
        }
        t_wp = t_wp.add(2);
        n = t_lex(t_wp);
    }
    res
}

// [spec:dash:def:test.aexpr-fn]
// [spec:dash:sem:test.aexpr-fn]
unsafe fn aexpr(mut n: token) -> c_int {
    let mut res: c_int = 1;

    loop {
        if nexpr(n) == 0 {
            res = 0;
        }
        if (*t_wp).is_null() {
            break;
        }
        n = t_lex(t_wp.add(1));
        if n != token::BAND {
            break;
        }
        t_wp = t_wp.add(2);
        n = t_lex(t_wp);
    }
    res
}

// [spec:dash:def:test.nexpr-fn]
// [spec:dash:sem:test.nexpr-fn]
unsafe fn nexpr(mut n: token) -> c_int {
    if n != token::UNOT {
        return primary(n);
    }

    n = t_lex(t_wp.add(1));
    if n != token::EOI {
        t_wp = t_wp.add(1);
    }
    (nexpr(n) == 0) as c_int
}

// [spec:dash:def:test.primary-fn]
// [spec:dash:sem:test.primary-fn]
unsafe fn primary(n: token) -> c_int {
    let nn: token;
    let res: c_int;

    if n == token::EOI {
        return 0; /* missing expression */
    }
    if n == token::LPAREN {
        t_wp = t_wp.add(1);
        nn = t_lex(t_wp);
        if nn == token::RPAREN {
            return 0; /* missing expression */
        }
        res = oexpr(nn);
        t_wp = t_wp.add(1);
        if t_lex(t_wp) != token::RPAREN {
            syntax(ptr::null(), c"closing paren expected".as_ptr());
        }
        return res;
    }
    if !t_wp_op.is_null() && (*t_wp_op).op_type == token_types::UNOP as c_short {
        /* unary expression */
        t_wp = t_wp.add(1);
        if (*t_wp).is_null() {
            syntax((*t_wp_op).op_text, c"argument expected".as_ptr());
        }
        match n {
            token::STREZ => return CStr::from_ptr(*t_wp).to_bytes().is_empty() as c_int,
            token::STRNZ => return (!CStr::from_ptr(*t_wp).to_bytes().is_empty()) as c_int,
            token::FILTT => return libc::isatty(getn(*t_wp) as c_int),
            // #ifdef HAVE_FACCESSAT
            token::FILRD => return test_file_access(*t_wp, libc::R_OK),
            token::FILWR => return test_file_access(*t_wp, libc::W_OK),
            token::FILEX => return test_file_access(*t_wp, libc::X_OK),
            // #endif
            _ => return filstat(*t_wp, n),
        }
    }

    // if (t_lex(t_wp + 1), t_wp_op && t_wp_op->op_type == BINOP)
    t_lex(t_wp.add(1));
    if !t_wp_op.is_null() && (*t_wp_op).op_type == token_types::BINOP as c_short {
        return binop();
    }

    (!CStr::from_ptr(*t_wp).to_bytes().is_empty()) as c_int
}

// [spec:dash:def:test.binop-fn]
// [spec:dash:sem:test.binop-fn]
unsafe fn binop() -> c_int {
    let opnd1: *const c_char;
    let opnd2: *const c_char;
    let op: *const t_op;

    opnd1 = *t_wp;
    t_wp = t_wp.add(1);
    t_lex(t_wp);
    op = t_wp_op;

    t_wp = t_wp.add(1);
    opnd2 = *t_wp;
    if opnd2.is_null() {
        syntax((*op).op_text, c"argument expected".as_ptr());
    }

    // The C `switch` opens with `default:` (an `abort()` under DEBUG, which
    // is not defined here) falling through into `case STREQ`; the `_` arm
    // below is that same default, moved last because Rust requires it.
    match token_of((*op).op_num) {
        token::STRNE => (CStr::from_ptr(opnd1).to_bytes() != CStr::from_ptr(opnd2).to_bytes()) as c_int,
        token::STRLT => (libc::strcoll(opnd1, opnd2) < 0) as c_int,
        token::STRGT => (libc::strcoll(opnd1, opnd2) > 0) as c_int,
        token::INTEQ => (getn(opnd1) == getn(opnd2)) as c_int,
        token::INTNE => (getn(opnd1) != getn(opnd2)) as c_int,
        token::INTGE => (getn(opnd1) >= getn(opnd2)) as c_int,
        token::INTGT => (getn(opnd1) > getn(opnd2)) as c_int,
        token::INTLE => (getn(opnd1) <= getn(opnd2)) as c_int,
        token::INTLT => (getn(opnd1) < getn(opnd2)) as c_int,
        token::FILNT => newerf(opnd1, opnd2) as c_int,
        token::FILOT => olderf(opnd1, opnd2) as c_int,
        token::FILEQ => equalf(opnd1, opnd2),
        // case STREQ: (and default:)
        _ => (CStr::from_ptr(opnd1).to_bytes() == CStr::from_ptr(opnd2).to_bytes()) as c_int,
    }
}

// [spec:dash:def:test.filstat-fn]
// [spec:dash:sem:test.filstat-fn]
unsafe fn filstat(nm: *mut c_char, mode: token) -> c_int {
    let mut s: libc::stat64 = mem::zeroed();

    if (if mode == token::FILSYM {
        libc::lstat64(nm, &mut s)
    } else {
        libc::stat64(nm, &mut s)
    }) != 0
    {
        return 0;
    }

    match mode {
        // #ifndef HAVE_FACCESSAT
        //   case FILRD: return test_access(&s, R_OK);
        //   case FILWR: return test_access(&s, W_OK);
        //   case FILEX: return test_access(&s, X_OK);
        // #endif
        token::FILEXIST => 1,
        token::FILREG => ((s.st_mode & libc::S_IFMT) == libc::S_IFREG) as c_int,
        token::FILDIR => ((s.st_mode & libc::S_IFMT) == libc::S_IFDIR) as c_int,
        token::FILCDEV => ((s.st_mode & libc::S_IFMT) == libc::S_IFCHR) as c_int,
        token::FILBDEV => ((s.st_mode & libc::S_IFMT) == libc::S_IFBLK) as c_int,
        token::FILFIFO => ((s.st_mode & libc::S_IFMT) == libc::S_IFIFO) as c_int,
        token::FILSOCK => ((s.st_mode & libc::S_IFMT) == libc::S_IFSOCK) as c_int,
        token::FILSYM => ((s.st_mode & libc::S_IFMT) == libc::S_IFLNK) as c_int,
        token::FILSUID => ((s.st_mode & libc::S_ISUID) != 0) as c_int,
        token::FILSGID => ((s.st_mode & libc::S_ISGID) != 0) as c_int,
        // #ifdef S_ISVTX
        token::FILSTCK => ((s.st_mode & libc::S_ISVTX) != 0) as c_int,
        // #endif
        token::FILGZ => (s.st_size != 0) as c_int,
        token::FILUID => (s.st_uid == libc::geteuid()) as c_int,
        token::FILGID => (s.st_gid == libc::getegid()) as c_int,
        _ => 1,
    }
}

// [spec:dash:def:test.t-lex-fn]
// [spec:dash:sem:test.t-lex-fn]
unsafe fn t_lex(tp: *mut *mut c_char) -> token {
    let op: *const t_op;
    let s: *mut c_char = *tp;

    if s.is_null() {
        t_wp_op = ptr::null();
        return token::EOI;
    }

    op = getop(s);
    if !op.is_null()
        && !((*op).op_type == token_types::UNOP as c_short && isoperand(tp) != 0)
        && !((*op).op_num == token::LPAREN as c_short && (*tp.add(1)).is_null())
    {
        t_wp_op = op;
        return token_of((*op).op_num);
    }

    t_wp_op = ptr::null();
    token::OPERAND
}

// [spec:dash:def:test.isoperand-fn]
// [spec:dash:sem:test.isoperand-fn]
unsafe fn isoperand(tp: *mut *mut c_char) -> c_int {
    let op: *const t_op;
    let s: *mut c_char;

    s = *tp.add(1);
    if s.is_null() {
        return 1;
    }
    if (*tp.add(2)).is_null() {
        return 0;
    }

    op = getop(s);
    (!op.is_null() && (*op).op_type == token_types::BINOP as c_short) as c_int
}

// [spec:dash:def:test.newerf-fn]
// [spec:dash:sem:test.newerf-fn]
unsafe fn newerf(f1: *const c_char, f2: *const c_char) -> bool {
    let mut b1: libc::stat64 = mem::zeroed();
    let mut b2: libc::stat64 = mem::zeroed();

    if libc::stat64(f1, &mut b1) != 0 {
        return false;
    }
    if libc::stat64(f2, &mut b2) != 0 {
        return true;
    }

    // #ifdef HAVE_ST_MTIM — libc names the members st_mtime/st_mtime_nsec.
    b1.st_mtime > b2.st_mtime || (b1.st_mtime == b2.st_mtime && b1.st_mtime_nsec > b2.st_mtime_nsec)
    // #else return b1.st_mtime > b2.st_mtime;
}

// [spec:dash:def:test.olderf-fn]
// [spec:dash:sem:test.olderf-fn]
unsafe fn olderf(f1: *const c_char, f2: *const c_char) -> bool {
    let mut b1: libc::stat64 = mem::zeroed();
    let mut b2: libc::stat64 = mem::zeroed();

    if libc::stat64(f2, &mut b2) != 0 {
        return false;
    }
    if libc::stat64(f1, &mut b1) != 0 {
        return true;
    }

    // #ifdef HAVE_ST_MTIM
    b1.st_mtime < b2.st_mtime || (b1.st_mtime == b2.st_mtime && b1.st_mtime_nsec < b2.st_mtime_nsec)
    // #else return b1.st_mtime < b2.st_mtime;
}

// [spec:dash:def:test.equalf-fn]
// [spec:dash:sem:test.equalf-fn]
unsafe fn equalf(f1: *const c_char, f2: *const c_char) -> c_int {
    let mut b1: libc::stat64 = mem::zeroed();
    let mut b2: libc::stat64 = mem::zeroed();

    (libc::stat64(f1, &mut b1) == 0
        && libc::stat64(f2, &mut b2) == 0
        && b1.st_dev == b2.st_dev
        && b1.st_ino == b2.st_ino) as c_int
}

// #ifdef HAVE_FACCESSAT

// [spec:dash:def:test.has-exec-bit-set-fn]
// [spec:dash:sem:test.has-exec-bit-set-fn]
unsafe fn has_exec_bit_set(path: *const c_char) -> c_int {
    let mut st: libc::stat64 = mem::zeroed();

    if libc::stat64(path, &mut st) != 0 {
        return 0;
    }
    (st.st_mode & (libc::S_IXUSR | libc::S_IXGRP | libc::S_IXOTH)) as c_int
}

// [spec:dash:def:test.test-file-access-fn]
// [spec:dash:sem:test.test-file-access-fn]
// [spec:dash:def:exec.test-file-access-fn]
// [spec:dash:sem:exec.test-file-access-fn]
pub unsafe fn test_file_access(path: *const c_char, mode: c_int) -> c_int {
    if faccessat_confused_about_superuser() != 0
        && mode == libc::X_OK
        && libc::geteuid() == 0
        && has_exec_bit_set(path) == 0
    {
        return 0;
    }
    (libc::faccessat(libc::AT_FDCWD, path, mode, libc::AT_EACCESS) == 0) as c_int
}

// #else	/* HAVE_FACCESSAT */
/*
 * The manual, and IEEE POSIX 1003.2, suggests this should check the mode
 * bits, not use access():
 *
 *	True shall indicate only that the write flag is on.  The file is not
 *	writable on a read-only file system even if this test indicates true.
 *
 * [... src/bltin/test.c:540-661 carries a long rationale for testing the
 * mode bits directly rather than calling access(); it is not reproduced
 * here in full ...]
 *
 * The ksh93 implementation uses access() for '-r' and '-w' if
 * (euid==uid&&egid==gid), but uses st_mode for '-x' iff running as root.
 * i.e. it does strictly conform to 1003.1-2001 (and presumably 1003.2b).
 */

// [spec:dash:def:test.test-access-fn]
// [spec:dash:sem:test.test-access-fn]
// [spec:dash:def:exec.test-access-fn]
// [spec:dash:sem:exec.test-access-fn]
pub unsafe fn test_access(sp: *const libc::stat64, mut stmode: c_int) -> c_int {
    let groups: *mut libc::gid_t;
    let mut n: c_int;
    let euid: libc::uid_t;
    let maxgroups: c_int;

    /*
     * I suppose we could use access() if not running as root and if we are
     * running with ((euid == uid) && (egid == gid)), but we've already
     * done the stat() so we might as well just test the permissions
     * directly instead of asking the kernel to do it....
     */
    euid = libc::geteuid();
    if euid == 0 {
        if stmode != libc::X_OK {
            return 1;
        }

        /* any bit is good enough */
        stmode = (stmode << 6) | (stmode << 3) | stmode;
    } else if (*sp).st_uid == euid {
        stmode <<= 6;
    } else if (*sp).st_gid == libc::getegid() {
        stmode <<= 3;
    } else {
        /* XXX stolen almost verbatim from ksh93.... */
        /* on some systems you can be in several groups */
        maxgroups = libc::getgroups(0, ptr::null_mut());
        /* The C `stalloc`s the array and leaves it to the enclosing mark;
         * nothing reads it after the scan below, so it is a local. */
        let mut groupbuf: Vec<libc::gid_t> = vec![0; maxgroups.max(0) as usize];
        groups = groupbuf.as_mut_ptr();
        n = libc::getgroups(maxgroups, groups);
        debug_assert!(n <= maxgroups);
        loop {
            n -= 1;
            if n < 0 {
                break;
            }
            if *groups.add(n as usize) == (*sp).st_gid {
                stmode <<= 3;
                break;
            }
        }
    }

    ((*sp).st_mode & stmode as libc::mode_t) as c_int
}
// #endif	/* HAVE_FACCESSAT */

#[cfg(test)]
mod tests {
    use super::*;

    /// `test` is a pure expression evaluator over its words, so it can be
    /// asked a question directly. The differential corpus covers it far
    /// more widely than this; what these pin is the shape of the answer
    /// for each arity the POSIX prescription branches on, because that
    /// branching is what a refactor here would break first.
    ///
    /// `t_wp` is a module global, so the lock is the crate's usual one.
    fn eval(words: &[&[u8]]) -> c_int {
        let _guard = crate::testutil::lock();
        let args: Vec<&BStr> = words.iter().map(|w| BStr::new(*w)).collect();
        unsafe { testcmd(&args) }
    }

    #[test]
    fn no_expression_is_false() {
        assert_eq!(eval(&[b"test"]), 1);
        assert_eq!(eval(&[b"[", b"]"]), 1);
    }

    /// One word is true when it is not empty -- the `argc == 2` arm.
    #[test]
    fn one_word_tests_for_emptiness() {
        assert_eq!(eval(&[b"test", b"x"]), 0);
        assert_eq!(eval(&[b"test", b""]), 1);
        /* An operator name is still just a word here. */
        assert_eq!(eval(&[b"test", b"-n"]), 0);
        assert_eq!(eval(&[b"test", b"="]), 0);
    }

    #[test]
    fn unary_string_operators() {
        assert_eq!(eval(&[b"test", b"-n", b"x"]), 0);
        assert_eq!(eval(&[b"test", b"-n", b""]), 1);
        assert_eq!(eval(&[b"test", b"-z", b""]), 0);
        assert_eq!(eval(&[b"test", b"-z", b"x"]), 1);
    }

    #[test]
    fn binary_string_operators() {
        assert_eq!(eval(&[b"test", b"a", b"=", b"a"]), 0);
        assert_eq!(eval(&[b"test", b"a", b"=", b"b"]), 1);
        assert_eq!(eval(&[b"test", b"a", b"!=", b"b"]), 0);
    }

    /// The `argc == 3` arm prefers a binary operator in the middle, so a
    /// word that looks like one wins over the `!` negation reading.
    #[test]
    fn a_middle_operator_wins() {
        assert_eq!(eval(&[b"test", b"!", b"=", b"!"]), 0);
        assert_eq!(eval(&[b"test", b"!", b"=", b"x"]), 1);
    }

    #[test]
    fn integer_comparison() {
        assert_eq!(eval(&[b"test", b"1", b"-eq", b"1"]), 0);
        assert_eq!(eval(&[b"test", b"1", b"-ne", b"1"]), 1);
        assert_eq!(eval(&[b"test", b"1", b"-lt", b"2"]), 0);
        assert_eq!(eval(&[b"test", b"2", b"-gt", b"1"]), 0);
        assert_eq!(eval(&[b"test", b"-1", b"-lt", b"0"]), 0);
    }

    #[test]
    fn the_remaining_integer_operators() {
        assert_eq!(eval(&[b"test", b"1", b"-le", b"1"]), 0);
        assert_eq!(eval(&[b"test", b"2", b"-le", b"1"]), 1);
        assert_eq!(eval(&[b"test", b"1", b"-ge", b"1"]), 0);
        assert_eq!(eval(&[b"test", b"1", b"-ge", b"2"]), 1);
    }

    /// The file predicates that ask about permission rather than about
    /// type. `/` is readable and searchable to every uid that can run
    /// this suite at all.
    #[test]
    fn permission_predicates() {
        assert_eq!(eval(&[b"test", b"-r", b"/"]), 0);
        assert_eq!(eval(&[b"test", b"-x", b"/"]), 0);
        assert_eq!(eval(&[b"test", b"-s", b"/nonexistent-for-nsh-tests"]), 1);
    }

    #[test]
    fn negation_and_connectives() {
        assert_eq!(eval(&[b"test", b"!", b"-n", b""]), 0);
        assert_eq!(eval(&[b"test", b"x", b"-a", b"y"]), 0);
        assert_eq!(eval(&[b"test", b"x", b"-a", b""]), 1);
        assert_eq!(eval(&[b"test", b"", b"-o", b"y"]), 0);
        assert_eq!(eval(&[b"test", b"", b"-o", b""]), 1);
    }

    /// Parentheses at the ends of a three- or four-word expression are
    /// stripped by the same arm that handles `!`.
    #[test]
    fn parentheses_group() {
        assert_eq!(eval(&[b"test", b"(", b"x", b")"]), 0);
        assert_eq!(eval(&[b"test", b"(", b"", b")"]), 1);
    }

    #[test]
    fn file_operators_read_the_filesystem() {
        assert_eq!(eval(&[b"test", b"-e", b"/"]), 0);
        assert_eq!(eval(&[b"test", b"-d", b"/"]), 0);
        assert_eq!(eval(&[b"test", b"-f", b"/"]), 1);
        assert_eq!(eval(&[b"test", b"-e", b"/nonexistent-for-nsh-tests"]), 1);
    }

    /// `[` checks its own closing bracket, and the check is on the last
    /// word rather than on any word.
    #[test]
    fn bracket_requires_its_bracket() {
        assert_eq!(eval(&[b"[", b"x", b"]"]), 0);
        assert!(crate::testutil::raises(|| {
            let args = [BStr::new(b"["), BStr::new(b"x")];
            unsafe { testcmd(&args) };
        }));
    }
}
