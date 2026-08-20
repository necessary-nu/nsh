//! `unalias`.
//!
//! Port of `unaliascmd` from `src/alias.c`. Removing an entry is
//! `crate::alias`'s business -- an alias being read has to survive its
//! own removal -- so this is the option scan and the diagnostic.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ffi::c_int;
use std::io::Write;

use crate::alias::{rmaliases, unalias};
use crate::eval::Flow;
use crate::options::Options;

// [spec:dash:def:alias.unaliascmd-fn]
// [spec:dash:sem:alias.unaliascmd-fn]
// [spec:posix:syn:builtin.unalias.synopsis]
// [spec:posix:req:builtin.unalias.remove-definitions]
// [spec:posix:req:builtin.unalias.utility-syntax-guidelines]
// [spec:posix:req:builtin.unalias.opt-a]
// [spec:posix:req:builtin.unalias.operand-alias-name]
// [spec:posix:req:builtin.unalias.env-locale]
// [spec:posix:sem:builtin.unalias.env-nlspath]
// [spec:posix:req:builtin.unalias.stderr]
// [spec:posix:req:builtin.unalias.interfaces]
// [spec:posix:req:builtin.unalias.exit-status]
pub fn unaliascmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut i: c_int;

    let mut opts = Options::new(args);
    while let Some(opt) = opts.next(&mut sh.diagnostics(), b"a")? {
        if opt == b'a' {
            rmaliases(&mut sh.interrupt_deferral, &mut sh.aliases);
            return Ok(Flow::Done((0).into()));
        }
    }
    i = 0;
    for name in opts.operands() {
        if unalias(&mut sh.interrupt_deferral, &mut sh.aliases, name) != 0 {
            let mut message = b"unalias: ".to_vec();
            message.extend_from_slice(name);
            message.extend_from_slice(b" not found\n");
            let _ = sh.io.stderr().write_all(&message);
            i = 1;
        }
    }

    Ok(Flow::Done((i).into()))
}
