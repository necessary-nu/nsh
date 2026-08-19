//! The builtins: one module per builtin, and the table that names them.
//!
//! `builtins::<name>` is the whole organising idea -- a builtin's entry
//! point and the helpers only it uses live in the module named after it,
//! while the machinery it drives (the variable table, job control, the
//! alias table, the parser) stays in the module that owns that machinery
//! and is called from here. Where a builtin's name is a Rust keyword the
//! module is a raw identifier, so `type` is `builtins::r#type`: the module
//! is named after the builtin even when the language would rather it were
//! not.
//!
//! This file is the port of `src/builtins.c` / `src/builtins.h`.
//!
//! Both files are *generated* at build time by `src/mkbuiltins` (a shell
//! script) from `src/builtins.def.in`, so nothing here carries
//! `[spec:dash:…]` annotations — there is no C source file to annotate.
//!
//! The table is the real generator's output for the default Linux build
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
use core::ffi::c_uint;

/// posix 'special builtin'
pub const BUILTIN_SPECIAL: c_uint = 0x1;
/// posix 'standard utility'
pub const BUILTIN_REGULAR: c_uint = 0x2;
/// posix 'assignment builtin'
pub const BUILTIN_ASSIGN: c_uint = 0x4;

/// A builtin's entry point.
///
/// The words the shell expanded, `argv[0]` first, borrowed from the
/// caller's storage rather than from any shell state -- which is the
/// constraint `[dec:nsh:public-surface]` records, because a builtin that
/// re-enters evaluation (`.`, `eval`, `fc`) has to be able to hand the
/// shell straight back.
///
/// The C's `int (*)(int, char **)` is gone, and with it the count: a
/// slice carries its own length, and no builtin has to be told twice.
///
/// The status is a `Result` because a builtin that fails hands its
/// diagnostic back rather than jumping out with it
/// ([dec:nsh:errors-are-values]). The `Err` has already been reported: the
/// bytes went to stderr where dash writes them, and the value is what the
/// caller -- and eventually an embedder -- gets to inspect.
///
/// `[dec:nsh:public-surface]` records the destination as
/// `fn(&mut Shell, &[&BStr]) -> Result<ExitStatus, Error>`. This is that
/// signature's receiver and `Result`; the status type belongs to
/// `public-api`.
///
/// The receiver owns all mutable shell state
/// ([dec:nsh:no-ambient-state]), so builtin entry points are ordinary safe
/// functions rather than callbacks into ambient globals.
///
/// The `Ok` side is a [`Flow`] rather than a status because `exit` is a
/// built-in. `exitcmd` used to leave by `exraise(EXEXIT)`, and a table of
/// one function-pointer type is what makes that everybody's business:
/// either every entry can say "the shell is exiting" or `exit` has to
/// keep jumping. Three others need it too, and they need it for the same
/// reason -- `.`, `fc` and `eval` re-enter evaluation, so an `exit` or a
/// `set -e` abort inside them has to travel back out through them. The
/// remaining thirty produce `Flow::Done` and nothing else, which is what
/// the C's `int` said.
pub type Builtin =
    fn(&mut crate::context::Shell, &[&BStr]) -> Result<crate::eval::Flow, crate::error::Error>;

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

pub mod alias;
pub mod r#break;
pub mod cd;
pub mod command;
pub mod dot;
pub mod echo;
pub mod eval;
pub mod exec;
pub mod exit;
pub mod export;
pub mod r#false;
pub mod fc;
pub mod fg;
pub mod getopts;
pub mod hash;
pub mod history;
pub mod jobs;
pub mod kill;
pub mod local;
pub mod printf;
pub mod pwd;
pub mod read;
pub mod r#return;
pub mod set;
pub mod shift;
pub mod shopt;
pub mod test;
pub mod times;
pub mod trap;
pub mod r#true;
pub mod r#type;
pub mod ulimit;
pub mod umask;
pub mod unalias;
pub mod unset;
pub mod wait;

/// The nameless row: a command that is only assignments and
/// redirections still runs a builtin, and this is it.
///
/// The C keeps it in `eval.c` beside `evalcommand`, which is the only
/// thing that reaches for it. It is a table row, so it lives with the
/// table.
pub(crate) static bltin: builtincmd = builtincmd {
    name: c"",
    builtin: Some(bltincmd),
    flags: BUILTIN_REGULAR,
};

// [spec:dash:def:eval.bltincmd-fn]
// [spec:dash:sem:eval.bltincmd-fn]
fn bltincmd(
    sh: &mut crate::context::Shell,
    _args: &[&BStr],
) -> Result<crate::eval::Flow, crate::error::Error> {
    /*
     * Preserve exitstatus of a previous possible redirection
     * as POSIX mandates
     */
    Ok(crate::eval::Flow::Done(sh.eval.back_exitstatus))
}

pub const NUMBUILTINS: usize = 42;

// [spec:posix:req:builtin.special.supported-and-output]
// [spec:posix:def:builtin.special.term-built-in]
// [spec:posix:req:builtin.special.not-exec-accessible]
// [spec:posix:req:xcu.builtin.regular-permitted]
// [spec:posix:req:xcu.builtin.exec-accessible]
// [spec:posix:req:xcu.intrinsic-utilities]
// [spec:posix:req:xcu.intrinsic.additional-implementation-defined]
pub static builtincmd: [builtincmd; NUMBUILTINS] = [
    builtincmd {
        name: c".",
        builtin: Some(dot::dotcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 0
    builtincmd {
        name: c":",
        builtin: Some(r#true::truecmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 1
    builtincmd {
        name: c"[",
        builtin: Some(test::testcmd),
        flags: 0,
    }, // 2
    builtincmd {
        name: c"alias",
        builtin: Some(alias::aliascmd),
        flags: BUILTIN_REGULAR | BUILTIN_ASSIGN,
    }, // 3
    builtincmd {
        name: c"bg",
        builtin: Some(fg::fgcmd),
        flags: BUILTIN_REGULAR,
    }, // 4
    builtincmd {
        name: c"break",
        builtin: Some(r#break::breakcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 5
    builtincmd {
        name: c"cd",
        builtin: Some(cd::cdcmd),
        flags: BUILTIN_REGULAR,
    }, // 6
    builtincmd {
        name: c"chdir",
        builtin: Some(cd::cdcmd),
        flags: 0,
    }, // 7
    builtincmd {
        name: c"command",
        builtin: Some(command::commandcmd),
        flags: BUILTIN_REGULAR,
    }, // 8
    builtincmd {
        name: c"continue",
        builtin: Some(r#break::breakcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 9
    builtincmd {
        name: c"echo",
        builtin: Some(echo::echocmd),
        flags: 0,
    }, // 10
    builtincmd {
        name: c"eval",
        builtin: None,
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 11
    builtincmd {
        name: c"exec",
        builtin: Some(exec::execcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 12
    builtincmd {
        name: c"exit",
        builtin: Some(exit::exitcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 13
    builtincmd {
        name: c"export",
        builtin: Some(export::exportcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR | BUILTIN_ASSIGN,
    }, // 14
    builtincmd {
        name: c"false",
        builtin: Some(r#false::falsecmd),
        flags: BUILTIN_REGULAR,
    }, // 15
    builtincmd {
        name: c"fc",
        builtin: Some(fc::histcmd),
        flags: BUILTIN_REGULAR,
    }, // 16
    builtincmd {
        name: c"fg",
        builtin: Some(fg::fgcmd),
        flags: BUILTIN_REGULAR,
    }, // 17
    builtincmd {
        name: c"getopts",
        builtin: Some(getopts::getoptscmd),
        flags: BUILTIN_REGULAR,
    }, // 18
    builtincmd {
        name: c"hash",
        builtin: Some(hash::hashcmd),
        flags: BUILTIN_REGULAR,
    }, // 19
    builtincmd {
        name: c"history",
        builtin: Some(history::historycmd),
        flags: BUILTIN_REGULAR,
    }, // 20
    builtincmd {
        name: c"jobs",
        builtin: Some(jobs::jobscmd),
        flags: BUILTIN_REGULAR,
    }, // 21
    builtincmd {
        name: c"kill",
        builtin: Some(kill::killcmd),
        flags: BUILTIN_REGULAR,
    }, // 22
    builtincmd {
        name: c"local",
        builtin: Some(local::localcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR | BUILTIN_ASSIGN,
    }, // 23
    builtincmd {
        name: c"printf",
        builtin: Some(printf::printfcmd),
        flags: 0,
    }, // 24
    builtincmd {
        name: c"pwd",
        builtin: Some(pwd::pwdcmd),
        flags: BUILTIN_REGULAR,
    }, // 25
    builtincmd {
        name: c"read",
        builtin: Some(read::readcmd),
        flags: BUILTIN_REGULAR,
    }, // 26
    builtincmd {
        name: c"readonly",
        builtin: Some(export::exportcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR | BUILTIN_ASSIGN,
    }, // 27
    builtincmd {
        name: c"return",
        builtin: Some(r#return::returncmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 28
    builtincmd {
        name: c"set",
        builtin: Some(set::setcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 29
    builtincmd {
        name: c"shift",
        builtin: Some(shift::shiftcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 30
    builtincmd {
        name: c"source",
        builtin: Some(dot::sourcecmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 31
    builtincmd {
        name: c"test",
        builtin: Some(test::testcmd),
        flags: 0,
    }, // 32
    builtincmd {
        name: c"times",
        builtin: Some(times::timescmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 33
    builtincmd {
        name: c"trap",
        builtin: Some(trap::trapcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 34
    builtincmd {
        name: c"true",
        builtin: Some(r#true::truecmd),
        flags: BUILTIN_REGULAR,
    }, // 35
    builtincmd {
        name: c"type",
        builtin: Some(r#type::typecmd),
        flags: BUILTIN_REGULAR,
    }, // 36
    builtincmd {
        name: c"ulimit",
        builtin: Some(ulimit::ulimitcmd),
        flags: BUILTIN_REGULAR,
    }, // 37
    builtincmd {
        name: c"umask",
        builtin: Some(umask::umaskcmd),
        flags: BUILTIN_REGULAR,
    }, // 38
    builtincmd {
        name: c"unalias",
        builtin: Some(unalias::unaliascmd),
        flags: BUILTIN_REGULAR,
    }, // 39
    builtincmd {
        name: c"unset",
        builtin: Some(unset::unsetcmd),
        flags: BUILTIN_SPECIAL | BUILTIN_REGULAR,
    }, // 40
    builtincmd {
        name: c"wait",
        builtin: Some(wait::waitcmd),
        flags: BUILTIN_REGULAR,
    }, // 41
];

/// Bash-only built-ins, searched before the baseline table only while the
/// current shell has Bash Compatibility Mode enabled. Keeping a separate
/// sorted table prevents profile-only names from leaking into default mode
/// and permits a future Bash implementation to override a baseline entry.
pub static bash_builtincmd: [builtincmd; 1] = [builtincmd {
    name: c"shopt",
    builtin: Some(shopt::shoptcmd),
    flags: 0,
}];

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
pub static JOBSCMD: &builtincmd = &builtincmd[21];
pub static KILLCMD: &builtincmd = &builtincmd[22];
pub static LOCALCMD: &builtincmd = &builtincmd[23];
pub static PRINTFCMD: &builtincmd = &builtincmd[24];
pub static PWDCMD: &builtincmd = &builtincmd[25];
pub static READCMD: &builtincmd = &builtincmd[26];
pub static RETURNCMD: &builtincmd = &builtincmd[28];
pub static SETCMD: &builtincmd = &builtincmd[29];
pub static SHIFTCMD: &builtincmd = &builtincmd[30];
pub static TESTCMD: &builtincmd = &builtincmd[2];
pub static TIMESCMD: &builtincmd = &builtincmd[33];
pub static TRAPCMD: &builtincmd = &builtincmd[34];
pub static TRUECMD: &builtincmd = &builtincmd[1];
pub static TYPECMD: &builtincmd = &builtincmd[36];
pub static ULIMITCMD: &builtincmd = &builtincmd[37];
pub static UMASKCMD: &builtincmd = &builtincmd[38];
pub static UNALIASCMD: &builtincmd = &builtincmd[39];
pub static UNSETCMD: &builtincmd = &builtincmd[40];
pub static WAITCMD: &builtincmd = &builtincmd[41];

#[cfg(test)]
mod tests {
    use super::*;
    use bstr::BString;

    use crate::expand::strlist;

    /// A field's bytes end with the NUL its C readers stop at, and a
    /// builtin stops at a length, so exactly one byte comes off -- not the
    /// trailing NUL of a word that ends in one.
    #[test]
    fn args_drop_only_the_terminator() {
        let fields = vec![
            strlist {
                text: BString::from(&b"echo\0"[..]),
            },
            strlist {
                text: BString::from(&b"\0"[..]),
            },
            strlist {
                text: BString::from(&b"a b\0"[..]),
            },
        ];
        let args = args(&fields);
        assert_eq!(
            args,
            vec![BStr::new("echo"), BStr::new(""), BStr::new("a b")]
        );
    }

    /// Every row the table names resolves, which is the check that a
    /// module move did not leave a name pointing at the wrong function.
    #[test]
    fn every_row_has_an_entry_point() {
        for cmd in &builtincmd {
            let name = cmd.name.to_bytes();
            assert_eq!(
                cmd.builtin.is_none(),
                name == b"eval",
                "only `eval` has a special entry point"
            );
        }
        for cmd in &bash_builtincmd {
            assert!(
                cmd.builtin.is_some(),
                "Bash-only rows use ordinary entry points"
            );
        }
    }
}
