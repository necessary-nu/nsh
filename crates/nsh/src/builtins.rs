//! Literal port of `src/builtins.c` / `src/builtins.h`.
//!
//! Both files are *generated* at build time by `src/mkbuiltins` (a shell
//! script) from `src/builtins.def.in`, so nothing here carries
//! `[spec:dash:…]` annotations — there is no C source file to annotate.
//!
//! **`printf` is deliberately absent.** `builtins.def.in:75` lists it and
//! the generator emits it, so this table is not that output byte for byte
//! any more. nsh has no `printf` builtin: the utility's contract is a
//! runtime `%`-conversion interpreter, and this shell does not carry one.
//! `printf` therefore resolves the way any other external utility does,
//! through `PATH`. See `[dec:nsh:no-format-interpreters]`.
//!
//! Otherwise the table is the real generator's output for the default Linux build
//! (`JOBS` = 1, `SMALL` undefined, `HAVE_GETRLIMIT` defined), which is the
//! configuration `plan/.port-manifest.styx` was extracted from — `fc`
//! (`histcmd`) and `ulimit` are therefore present.
//!
//! `mkbuiltins` sorts the table by name with `LC_COLLATE=C`, and
//! `exec.c`'s `find_builtin` binary-searches it, so **the order below is
//! load-bearing**.
//!
//! Flags come from the `-` options in `builtins.def.in`: `-s` (posix special
//! builtin) sets `BUILTIN_SPECIAL | BUILTIN_REGULAR`, `-u` (posix standard
//! utility) sets `BUILTIN_REGULAR`, `-a` (posix assignment builtin) sets
//! `BUILTIN_ASSIGN`, and `-n` (special entry point) makes the function
//! pointer NULL — which is why `eval` has no `builtin` here: `eval.c` calls
//! `evalcmd` directly through its three-argument entry point.

use core::ffi::CStr;

use bstr::BStr;
use libc::{c_char, c_int, c_uint};

/// posix 'special builtin'
pub const BUILTIN_SPECIAL: c_uint = 0x1;
/// posix 'standard utility'
pub const BUILTIN_REGULAR: c_uint = 0x2;
/// posix 'assignment builtin'
pub const BUILTIN_ASSIGN: c_uint = 0x4;

/// A builtin's entry point.
///
/// The C has one shape, `int (*)(int, char **)`, and this has two only
/// while the builtins are moving off it. `Args` is where they all end up:
/// the words the shell expanded, `argv[0]` first, borrowed from the
/// caller's storage rather than from any shell state -- which is the
/// constraint `[dec:nsh:public-surface]` records, because a builtin that
/// re-enters evaluation (`.`, `eval`, `fc`) must be able to hand the shell
/// straight back.
///
/// `Raw` is temporary and goes when the last builtin leaves it.
pub enum Builtin {
    /// Not converted yet: the C's `argc` and `argv`.
    Raw(unsafe fn(c_int, *mut *mut c_char) -> c_int),
    /// The shell's own words, as byte strings.
    Args(unsafe fn(&[&BStr]) -> c_int),
}

pub struct builtincmd {
    pub name: &'static CStr,
    /// `None` is the C `NULL`: the command has a special entry point.
    pub builtin: Option<Builtin>,
    pub flags: c_uint,
}

/// The words a builtin is handed, out of the fields `evalcommand`
/// expanded.
///
/// A field's bytes end with the NUL its C readers need
/// (`strlist::textp`), because every one of them stops at a terminator. A
/// builtin is Rust and stops at a length, so the terminator goes no
/// further than this boundary.
pub fn args(fields: &[crate::expand::strlist]) -> Vec<&BStr> {
    fields
        .iter()
        .map(|field| {
            debug_assert_eq!(field.text.last(), Some(&0), "a field is a C string");
            BStr::new(&field.text[..field.text.len() - 1])
        })
        .collect()
}

pub const NUMBUILTINS: usize = 39;

pub static builtincmd: [builtincmd; NUMBUILTINS] = [
    builtincmd { name: c".", builtin: Some(Builtin::Args(crate::shellmain::dotcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 0
    builtincmd { name: c":", builtin: Some(Builtin::Args(crate::eval::truecmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 1
    builtincmd { name: c"[", builtin: Some(Builtin::Raw(crate::bltin::test::testcmd)), flags: 0 }, // 2
    builtincmd { name: c"alias", builtin: Some(Builtin::Raw(crate::alias::aliascmd)), flags: BUILTIN_REGULAR | BUILTIN_ASSIGN }, // 3
    builtincmd { name: c"bg", builtin: Some(Builtin::Raw(crate::jobs::bgcmd)), flags: BUILTIN_REGULAR }, // 4
    builtincmd { name: c"break", builtin: Some(Builtin::Args(crate::eval::breakcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 5
    builtincmd { name: c"cd", builtin: Some(Builtin::Raw(crate::cd::cdcmd)), flags: BUILTIN_REGULAR }, // 6
    builtincmd { name: c"chdir", builtin: Some(Builtin::Raw(crate::cd::cdcmd)), flags: 0 }, // 7
    builtincmd { name: c"command", builtin: Some(Builtin::Raw(crate::exec::commandcmd)), flags: BUILTIN_REGULAR }, // 8
    builtincmd { name: c"continue", builtin: Some(Builtin::Args(crate::eval::breakcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 9
    builtincmd { name: c"echo", builtin: Some(Builtin::Raw(crate::bltin::printf::echocmd)), flags: 0 }, // 10
    builtincmd { name: c"eval", builtin: None, flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 11
    builtincmd { name: c"exec", builtin: Some(Builtin::Args(crate::eval::execcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 12
    builtincmd { name: c"exit", builtin: Some(Builtin::Args(crate::shellmain::exitcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 13
    builtincmd { name: c"export", builtin: Some(Builtin::Raw(crate::var::exportcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR | BUILTIN_ASSIGN }, // 14
    builtincmd { name: c"false", builtin: Some(Builtin::Args(crate::eval::falsecmd)), flags: BUILTIN_REGULAR }, // 15
    builtincmd { name: c"fc", builtin: Some(Builtin::Raw(crate::histedit::histcmd)), flags: BUILTIN_REGULAR }, // 16
    builtincmd { name: c"fg", builtin: Some(Builtin::Raw(crate::jobs::fgcmd)), flags: BUILTIN_REGULAR }, // 17
    builtincmd { name: c"getopts", builtin: Some(Builtin::Raw(crate::options::getoptscmd)), flags: BUILTIN_REGULAR }, // 18
    builtincmd { name: c"hash", builtin: Some(Builtin::Raw(crate::exec::hashcmd)), flags: BUILTIN_REGULAR }, // 19
    builtincmd { name: c"jobs", builtin: Some(Builtin::Raw(crate::jobs::jobscmd)), flags: BUILTIN_REGULAR }, // 20
    builtincmd { name: c"kill", builtin: Some(Builtin::Raw(crate::jobs::killcmd)), flags: BUILTIN_REGULAR }, // 21
    builtincmd { name: c"local", builtin: Some(Builtin::Raw(crate::var::localcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR | BUILTIN_ASSIGN }, // 22
    builtincmd { name: c"pwd", builtin: Some(Builtin::Raw(crate::cd::pwdcmd)), flags: BUILTIN_REGULAR }, // 23
    builtincmd { name: c"read", builtin: Some(Builtin::Raw(crate::miscbltin::readcmd)), flags: BUILTIN_REGULAR }, // 24
    builtincmd { name: c"readonly", builtin: Some(Builtin::Raw(crate::var::exportcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR | BUILTIN_ASSIGN }, // 25
    builtincmd { name: c"return", builtin: Some(Builtin::Args(crate::eval::returncmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 26
    builtincmd { name: c"set", builtin: Some(Builtin::Raw(crate::options::setcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 27
    builtincmd { name: c"shift", builtin: Some(Builtin::Raw(crate::options::shiftcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 28
    builtincmd { name: c"test", builtin: Some(Builtin::Raw(crate::bltin::test::testcmd)), flags: 0 }, // 29
    builtincmd { name: c"times", builtin: Some(Builtin::Raw(crate::bltin::times::timescmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 30
    builtincmd { name: c"trap", builtin: Some(Builtin::Raw(crate::trap::trapcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 31
    builtincmd { name: c"true", builtin: Some(Builtin::Args(crate::eval::truecmd)), flags: BUILTIN_REGULAR }, // 32
    builtincmd { name: c"type", builtin: Some(Builtin::Raw(crate::exec::typecmd)), flags: BUILTIN_REGULAR }, // 33
    builtincmd { name: c"ulimit", builtin: Some(Builtin::Raw(crate::miscbltin::ulimitcmd)), flags: BUILTIN_REGULAR }, // 34
    builtincmd { name: c"umask", builtin: Some(Builtin::Raw(crate::miscbltin::umaskcmd)), flags: BUILTIN_REGULAR }, // 35
    builtincmd { name: c"unalias", builtin: Some(Builtin::Raw(crate::alias::unaliascmd)), flags: BUILTIN_REGULAR }, // 36
    builtincmd { name: c"unset", builtin: Some(Builtin::Raw(crate::var::unsetcmd)), flags: BUILTIN_SPECIAL | BUILTIN_REGULAR }, // 37
    builtincmd { name: c"wait", builtin: Some(Builtin::Raw(crate::jobs::waitcmd)), flags: BUILTIN_REGULAR }, // 38
];

// The `*CMD` pointers of builtins.h: `#define NAME (builtincmd + n)`.
pub static ALIASCMD: &builtincmd = &builtincmd[3];
pub static BGCMD: &builtincmd = &builtincmd[4];
pub static BREAKCMD: &builtincmd = &builtincmd[5];
pub static CDCMD: &builtincmd = &builtincmd[6];
pub static COMMANDCMD: &builtincmd = &builtincmd[8];
pub static DOTCMD: &builtincmd = &builtincmd[0];
pub static ECHOCMD: &builtincmd = &builtincmd[10];
pub static EVALCMD: &builtincmd = &builtincmd[11];
pub static EXECCMD: &builtincmd = &builtincmd[12];
pub static EXITCMD: &builtincmd = &builtincmd[13];
pub static EXPORTCMD: &builtincmd = &builtincmd[14];
pub static FALSECMD: &builtincmd = &builtincmd[15];
pub static FGCMD: &builtincmd = &builtincmd[17];
pub static GETOPTSCMD: &builtincmd = &builtincmd[18];
pub static HASHCMD: &builtincmd = &builtincmd[19];
pub static HISTCMD: &builtincmd = &builtincmd[16];
pub static JOBSCMD: &builtincmd = &builtincmd[20];
pub static KILLCMD: &builtincmd = &builtincmd[21];
pub static LOCALCMD: &builtincmd = &builtincmd[22];
pub static PWDCMD: &builtincmd = &builtincmd[23];
pub static READCMD: &builtincmd = &builtincmd[24];
pub static RETURNCMD: &builtincmd = &builtincmd[26];
pub static SETCMD: &builtincmd = &builtincmd[27];
pub static SHIFTCMD: &builtincmd = &builtincmd[28];
pub static TESTCMD: &builtincmd = &builtincmd[2];
pub static TIMESCMD: &builtincmd = &builtincmd[30];
pub static TRAPCMD: &builtincmd = &builtincmd[31];
pub static TRUECMD: &builtincmd = &builtincmd[1];
pub static TYPECMD: &builtincmd = &builtincmd[33];
pub static ULIMITCMD: &builtincmd = &builtincmd[34];
pub static UMASKCMD: &builtincmd = &builtincmd[35];
pub static UNALIASCMD: &builtincmd = &builtincmd[36];
pub static UNSETCMD: &builtincmd = &builtincmd[37];
pub static WAITCMD: &builtincmd = &builtincmd[38];
