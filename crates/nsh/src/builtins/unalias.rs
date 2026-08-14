//! `unalias`.
//!
//! Port of `unaliascmd` from `src/alias.c`. Removing an entry is
//! `crate::alias`'s business -- an alias being read has to survive its
//! own removal -- so this is the option scan and the diagnostic.

use crate::error::Error;
use bstr::BStr;
use libc::c_int;
use std::io::Write;

use crate::alias::{rmaliases, unalias};
use crate::options::Options;

// [spec:dash:def:alias.unaliascmd-fn]
// [spec:dash:sem:alias.unaliascmd-fn]
pub unsafe fn unaliascmd(args: &[&BStr]) -> Result<c_int, Error> {
    let mut i: c_int;

    let mut opts = Options::new(args);
    while let Some(opt) = opts.next(b"a")? {
        if opt == b'a' {
            rmaliases();
            return Ok(0);
        }
    }
    i = 0;
    for name in opts.operands() {
        let name = crate::shell::cstring(name);
        if unalias(name.as_ptr()) != 0 {
            let mut message = b"unalias: ".to_vec();
            message.extend_from_slice(name.as_bytes());
            message.extend_from_slice(b" not found\n");
            let _ = (*crate::output::stderr()).write_all(&message);
            i = 1;
        }
    }

    Ok(i)
}
