//! `hash`.
//!
//! Port of `hashcmd` and `printentry` from `src/exec.c`. The command
//! table it prints and clears stays in `crate::exec`, which is what fills
//! it during a PATH search.

use bstr::BStr;
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io::Write;

use crate::exec::{
    CMDNORMAL, CMDUNKNOWN, DO_ERR, clearcmdentry, cmdentry, cmdlookup, cmdtable_mut,
    delete_cmd_entry, find_command, padvance, padvance_result, param, pathopt, tblentry,
};

// [spec:dash:def:exec.hashcmd-fn]
// [spec:dash:sem:exec.hashcmd-fn]
pub unsafe fn hashcmd(args: &[&BStr]) -> c_int {
    let mut cmdp: *mut tblentry;
    let mut c: c_int;
    let mut entry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };
    let mut clear: bool;

    clear = false;
    let mut opts = crate::options::Options::new(args);
    while opts.next(b"r").is_some() {
        clear = true;
    }
    if clear {
        clearcmdentry();
        return 0;
    }

    let operands = opts.operands();
    if operands.is_empty() {
        for (name, cmdp) in cmdtable_mut().iter() {
            if cmdp.cmdtype() == CMDNORMAL {
                printentry(BStr::new(name.as_slice()), cmdp);
            }
        }
        return 0;
    }
    c = 0;
    for name in operands {
        let name = crate::shell::cstring(name);
        let name = name.as_ptr() as *mut c_char;
        cmdp = cmdlookup(name, 0);
        if !cmdp.is_null() && (*cmdp).path_dependent() {
            delete_cmd_entry(name);
        }
        find_command(name, &mut entry, DO_ERR, crate::var::pathval());
        if entry.cmdtype == CMDUNKNOWN {
            c = 1;
        }
    }
    c
}

// [spec:dash:def:exec.printentry-fn]
// [spec:dash:sem:exec.printentry-fn]
unsafe fn printentry(name: &BStr, cmdp: &tblentry) {
    let mut idx: c_int;
    let mut path: *const c_char;
    let fullname: *mut c_char;

    idx = cmdp.path_index();
    path = crate::var::pathval();
    loop {
        padvance(&mut path, name.as_ptr() as *const c_char);
        idx -= 1;
        if idx < 0 {
            break;
        }
    }
    fullname = padvance_result();
    let output = &mut *crate::output::stdout();
    let _ = output.write_all(CStr::from_ptr(fullname).to_bytes());
    let suffix: &[u8] = if cmdp.rehash { b"*\n" } else { b"\n" };
    let _ = output.write_all(suffix);
}
