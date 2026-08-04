//! Literal port of `src/alias.c` / `src/alias.h`.
//! Rules: `docs/spec/port/src/alias.md`.

use libc::{c_char, c_int, c_void, size_t};
use core::ptr::{addr_of, addr_of_mut, null_mut};

use crate::error::{INTOFF, INTON};
use crate::memalloc::{ckfree, ckmalloc, savestr};
use crate::options::{argptr, nextopt};
use crate::output::VaArg;
use crate::shell::cstr;
use crate::var::{hashval, varequal};

pub const ALIASINUSE: c_int = 1;
pub const ALIASDEAD: c_int = 2;

// [spec:dash:def:alias.alias]
#[repr(C)]
pub struct alias {
    pub next: *mut alias,
    pub name: *mut c_char,
    pub val: *mut c_char,
    pub flag: c_int,
}

const ATABSIZE: usize = 39;

pub static mut atab: [*mut alias; ATABSIZE] = [null_mut(); ATABSIZE];

// [spec:dash:def:alias.setalias-fn]
// [spec:dash:sem:alias.setalias-fn]
unsafe fn setalias(name: *const c_char, val: *const c_char) {
    let mut ap: *mut alias;
    let app: *mut *mut alias;
    let mut p: *const c_char = name;
    let namelen: size_t;

    loop {
        if crate::syntax::BASESYNTAX(*p as i8 as c_int) != crate::syntax::CWORD {
            crate::error::sh_error(cstr(b"Invalid alias name: %s\0"), &[VaArg::Str(name)]);
        }
        p = p.add(1);
        if *p == b'=' as c_char {
            break;
        }
    }

    app = __lookupalias(name);
    ap = *app;
    INTOFF();
    if !ap.is_null() {
        if ((*ap).flag & ALIASINUSE) == 0 {
            ckfree((*ap).name as *mut c_void);
        }
        (*ap).flag &= !ALIASDEAD;
    } else {
        /* not found */
        ap = ckmalloc(core::mem::size_of::<alias>() as size_t) as *mut alias;
        (*ap).flag = 0;
        (*ap).next = null_mut();
        *app = ap;
    }
    namelen = (val as usize - name as usize) as size_t;
    (*ap).name = savestr(name);
    (*ap).val = (*ap).name.add(namelen as usize);
    INTON();
}

// [spec:dash:def:alias.unalias-fn]
// [spec:dash:sem:alias.unalias-fn]
pub unsafe fn unalias(name: *const c_char) -> c_int {
    let app: *mut *mut alias;

    app = __lookupalias(name);

    if !(*app).is_null() {
        INTOFF();
        *app = freealias(*app);
        INTON();
        return 0;
    }

    1
}

// [spec:dash:def:alias.rmaliases-fn]
// [spec:dash:sem:alias.rmaliases-fn]
pub unsafe fn rmaliases() {
    let mut ap: *mut alias;
    let mut app: *mut *mut alias;
    let mut i: c_int;

    INTOFF();
    i = 0;
    while i < ATABSIZE as c_int {
        app = addr_of_mut!(atab[i as usize]);
        ap = *app;
        while !ap.is_null() {
            *app = freealias(*app);
            if ap == *app {
                app = addr_of_mut!((*ap).next);
            }
            ap = *app;
        }
        i += 1;
    }
    INTON();
}

// [spec:dash:def:alias.lookupalias-pub-fn]
// [spec:dash:sem:alias.lookupalias-pub-fn]
/// Public lookup.  Absent from `plan/.port-manifest.styx` — the extractor
/// folded it into `alias.lookupalias-fn` after stripping the leading
/// underscores from the distinct static `__lookupalias`.
pub unsafe fn lookupalias(name: *const c_char, check: c_int) -> *mut alias {
    let ap: *mut alias = *__lookupalias(name);

    if check != 0 && !ap.is_null() && ((*ap).flag & ALIASINUSE) != 0 {
        return null_mut();
    }
    ap
}

/*
 * TODO - sort output
 */

// [spec:dash:def:alias.aliascmd-fn]
// [spec:dash:sem:alias.aliascmd-fn]
pub unsafe fn aliascmd(argc: c_int, mut argv: *mut *mut c_char) -> c_int {
    let mut n: *mut c_char;
    let mut ret: c_int = 0;
    let mut ap: *mut alias;

    if argc == 1 {
        let mut i: c_int;

        i = 0;
        while i < ATABSIZE as c_int {
            ap = atab[i as usize];
            while !ap.is_null() {
                printalias(ap);
                ap = (*ap).next;
            }
            i += 1;
        }
        return 0;
    }
    loop {
        argv = argv.add(1);
        n = *argv;
        if n.is_null() {
            break;
        }
        /* n + 1: funny ksh stuff (from 44lite) */
        let vv = if *n == 0 {
            null_mut()
        } else {
            libc::strchr(n.add(1), b'=' as c_int)
        };
        if *n == 0 || vv.is_null() {
            ap = *__lookupalias(n);
            if ap.is_null() {
                crate::output::outfmt(
                    crate::output::out2,
                    cstr(b"%s: %s not found\n\0"),
                    &[VaArg::Str(cstr(b"alias\0")), VaArg::Str(n)],
                );
                ret = 1;
            } else {
                printalias(ap);
            }
        } else {
            setalias(n, vv.add(1));
        }
    }

    ret
}

// [spec:dash:def:alias.unaliascmd-fn]
// [spec:dash:sem:alias.unaliascmd-fn]
pub unsafe fn unaliascmd(argc: c_int, argv: *mut *mut c_char) -> c_int {
    let mut i: c_int;

    loop {
        i = nextopt(b"a\0".as_ptr() as *const c_char);
        if i == b'\0' as c_int {
            break;
        }
        if i == b'a' as c_int {
            rmaliases();
            return 0;
        }
    }
    i = 0;
    while !(*argptr).is_null() {
        if unalias(*argptr) != 0 {
            crate::output::outfmt(
                crate::output::out2,
                cstr(b"%s: %s not found\n\0"),
                &[VaArg::Str(cstr(b"unalias\0")), VaArg::Str(*argptr)],
            );
            i = 1;
        }
        argptr = argptr.add(1);
    }

    i
}

// [spec:dash:def:alias.freealias-fn]
// [spec:dash:sem:alias.freealias-fn]
unsafe fn freealias(ap: *mut alias) -> *mut alias {
    let next: *mut alias;

    if ((*ap).flag & ALIASINUSE) != 0 {
        (*ap).flag |= ALIASDEAD;
        return ap;
    }

    next = (*ap).next;
    ckfree((*ap).name as *mut c_void);
    ckfree(ap as *mut c_void);
    next
}

// [spec:dash:def:alias.printalias-fn]
// [spec:dash:sem:alias.printalias-fn]
pub unsafe fn printalias(ap: *const alias) {
    crate::output::out1fmt(
        addr_of!(crate::mystring::snlfmt) as *const c_char,
        &[VaArg::Str(crate::mystring::single_quote((*ap).name))],
    );
}

// [spec:dash:def:alias.lookupalias-fn]
// [spec:dash:sem:alias.lookupalias-fn]
unsafe fn __lookupalias(name: *const c_char) -> *mut *mut alias {
    let mut app: *mut *mut alias;

    app = addr_of_mut!(atab[(hashval(name) % ATABSIZE as libc::c_uint) as usize]);

    while !(*app).is_null() {
        if varequal(name, (**app).name) != 0 {
            break;
        }
        app = addr_of_mut!((**app).next);
    }

    app
}
