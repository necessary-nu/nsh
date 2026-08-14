//! `type`.
//!
//! Port of `typecmd` and `describe_command` from `src/exec.c`.
//!
//! `describe_command` is here rather than in `crate::exec` because `type`
//! is what it is for: `command -v` and `command -V` are documented as
//! describing a name the way `type` does, so `builtins::command` calls
//! this one rather than either keeping a copy or pushing it back down
//! into the search machinery.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ptr::{null, null_mut};
use libc::{c_char, c_int};
use std::ffi::CStr;
use std::io::Write;

use crate::builtins::BUILTIN_SPECIAL;
use crate::eval::Flow;
use crate::exec::{
    CMDBUILTIN, CMDFUNCTION, CMDNORMAL, DO_ABS, DO_ALTPATH, DO_NOFUNC, cmdentry, cmdlookup, find_builtin,
    find_command, padvance, padvance_result, param, pathopt, tblentry,
};
use crate::options::Options;
use crate::output::Output;

// [spec:dash:def:exec.typecmd-fn]
// [spec:dash:sem:exec.typecmd-fn]
pub unsafe fn typecmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut err: c_int = 0;

    let mut opts = crate::options::Options::new(args);
    opts.next(b"")?;
    for name in opts.operands() {
        let name = crate::shell::cstring(name);
        match describe_command(
            sh,
            crate::output::stdout(),
            name.as_ptr() as *mut c_char,
            null(),
            1,
        )? {
            Flow::Done(status) => err |= status,
            exit @ Flow::Exit { .. } => return Ok(exit),
        }
    }
    Ok(Flow::Done(err))
}

// [spec:dash:def:exec.describe-command-fn]
// [spec:dash:sem:exec.describe-command-fn]
pub(crate) unsafe fn describe_command(
    sh: &mut Shell,
    out: *mut Output,
    command: *mut c_char,
    mut path: *const c_char,
    verbose: c_int,
) -> Result<Flow, Error> {
    let mut entry: cmdentry = cmdentry {
        cmdtype: 0,
        u: param { index: 0 },
    };
    let cmdp: *mut tblentry;
    let ap: *const crate::alias::alias;

    'out_label: {
        if verbose != 0 {
            let _ = (&mut *out).write_all(CStr::from_ptr(command).to_bytes());
        }

        /* First look at the keywords */
        if !crate::parser::findkwd(command).is_null() {
            let bytes = if verbose != 0 {
                b" is a shell keyword" as &[u8]
            } else {
                CStr::from_ptr(command).to_bytes()
            };
            let _ = (&mut *out).write_all(bytes);
            break 'out_label;
        }

        /* Then look at the aliases */
        ap = crate::alias::lookupalias(command, 0);
        if !ap.is_null() {
            if verbose != 0 {
                let mut record = b" is an alias for ".to_vec();
                record.extend_from_slice(CStr::from_ptr((*ap).val).to_bytes());
                let _ = (&mut *out).write_all(&record);
            } else {
                let _ = (&mut *out).write_all(b"alias ");
                crate::alias::printalias(ap);
                return Ok(Flow::Done(0));
            }
            break 'out_label;
        }

        /* Then if the standard search path is used, check if it is
         * a tracked alias.
         */
        if path.is_null() {
            path = crate::var::pathval();
            cmdp = cmdlookup(command, 0);
        } else {
            cmdp = null_mut();
        }

        if !cmdp.is_null() {
            (*cmdp).write_to(&mut entry);
        } else {
            /* Finally use brute force */
            match find_command(sh, command, &mut entry, DO_ABS, path)? {
                Flow::Done(_) => {}
                exit @ Flow::Exit { .. } => return Ok(exit),
            }
        }

        match entry.cmdtype {
            CMDNORMAL => {
                let mut j: c_int = entry.u.index;
                let p: *mut c_char;
                if j == -1 {
                    p = command;
                } else {
                    loop {
                        padvance(&mut path, command);
                        j -= 1;
                        if j < 0 {
                            break;
                        }
                    }
                    p = padvance_result();
                }
                if verbose != 0 {
                    let mut record = b" is".to_vec();
                    if !cmdp.is_null() {
                        record.extend_from_slice(b" a tracked alias for");
                    }
                    record.push(b' ');
                    record.extend_from_slice(CStr::from_ptr(p).to_bytes());
                    let _ = (&mut *out).write_all(&record);
                } else {
                    let _ = (&mut *out).write_all(CStr::from_ptr(p).to_bytes());
                }
            }

            CMDFUNCTION => {
                if verbose != 0 {
                    let _ = (&mut *out).write_all(b" is a shell function");
                } else {
                    let _ = (&mut *out).write_all(CStr::from_ptr(command).to_bytes());
                }
            }

            CMDBUILTIN => {
                if verbose != 0 {
                    let record: &[u8] = if ((*entry.u.cmd).flags & BUILTIN_SPECIAL) != 0 {
                        b" is a special shell builtin"
                    } else {
                        b" is a shell builtin"
                    };
                    let _ = (&mut *out).write_all(record);
                } else {
                    let _ = (&mut *out).write_all(CStr::from_ptr(command).to_bytes());
                }
            }

            _ => {
                if verbose != 0 {
                    let _ = (&mut *out).write_all(b": not found\n");
                }
                return Ok(Flow::Done(127));
            }
        }
    }
    // out:
    let _ = (&mut *out).write_all(b"\n");
    Ok(Flow::Done(0))
}
