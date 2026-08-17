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
pub fn unaliascmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut i: c_int;

    let mut opts = Options::new(args);
    while let Some(opt) = opts.next(sh, b"a")? {
        if opt == b'a' {
            rmaliases(sh);
            return Ok(Flow::Done(0));
        }
    }
    i = 0;
    for name in opts.operands() {
        if unalias(sh, name) != 0 {
            let mut message = b"unalias: ".to_vec();
            message.extend_from_slice(name);
            message.extend_from_slice(b" not found\n");
            let _ = sh.io.stderr().write_all(&message);
            i = 1;
        }
    }

    Ok(Flow::Done(i))
}
