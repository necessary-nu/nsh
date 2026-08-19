//! `export` and `readonly`.
//!
//! Port of `exportcmd` from `src/var.c`. One function under two names,
//! telling them apart by the word it was called as -- the two differ only
//! in which flag they set on the variable.
//!
//! The variable table stays in `crate::var`. What is here is the argument
//! handling: with no operands it prints the set, and with them it sets a
//! flag on names that exist and creates the ones that do not.

use crate::context::Shell;
use crate::error::Error;
use bstr::BStr;
use core::ffi::c_int;

use crate::eval::Flow;
use crate::options::Options;
use crate::var::{VEXPORT, VREADONLY, add_flags, flags_bytes, set_bytes, show_vars};

// [spec:dash:def:var.exportcmd-fn]
// [spec:dash:sem:var.exportcmd-fn]
// [spec:posix:syn:builtin.export.synopsis]
// [spec:posix:req:builtin.export.set-attribute]
// [spec:posix:req:builtin.export.declaration-utility]
// [spec:posix:req:builtin.export.utility-syntax-guidelines]
// [spec:posix:req:builtin.export.p-output-format]
// [spec:posix:req:builtin.export.p-output-reinput]
// [spec:posix:sem:builtin.export.no-arguments]
// [spec:posix:req:builtin.export.stderr]
// [spec:posix:req:builtin.export.exit-status]
// [spec:posix:sem:builtin.export.utility-defaults]
// [spec:posix:syn:builtin.readonly.synopsis]
// [spec:posix:req:builtin.readonly.set-attribute]
// [spec:posix:def:builtin.readonly.attribute]
// [spec:posix:req:builtin.readonly.application-constraint]
// [spec:posix:req:builtin.readonly.declaration-utility]
// [spec:posix:req:builtin.readonly.utility-syntax-guidelines]
// [spec:posix:sem:builtin.readonly.p-output-format]
// [spec:posix:req:builtin.readonly.p-output-reinput]
// [spec:posix:sem:builtin.readonly.no-arguments]
// [spec:posix:req:builtin.readonly.stderr]
// [spec:posix:req:builtin.readonly.exit-status]
// [spec:posix:sem:builtin.readonly.utility-defaults]
pub fn exportcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    /* `export` and `readonly` are one builtin telling itself apart by the
     * word it was called as. */
    let flag: c_int = if args[0].first() == Some(&b'r') {
        VREADONLY
    } else {
        VEXPORT
    };

    let mut opts = Options::new(args);
    let notp = opts.next(sh, b"p")?.is_none();
    let operands = opts.operands();
    if notp && !operands.is_empty() {
        for word in operands {
            match word.iter().position(|&b| b == b'=') {
                Some(at) => {
                    let name = BStr::new(&word[..at]);
                    if flags_bytes(sh, name).is_some_and(|flags| flags & VREADONLY != 0) {
                        let mut message = name.to_vec();
                        message.extend_from_slice(b": is read only");
                        return Err(sh.builtin_error_value(1, &message));
                    }
                    set_bytes(
                        sh,
                        name,
                        Some(BStr::new(&word[at + 1..])),
                        flag,
                    )?;
                }
                None => {
                    if !add_flags(sh, word, flag) {
                        set_bytes(sh, word, None, flag)?;
                    }
                }
            }
        }
    } else {
        show_vars(sh, args[0], flag, 0);
    }
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testutil::lock;
    use crate::var::{VSTRFIXED, flags_bytes, lookup_bytes, set_bytes};

    /// The shell is the caller's: `export` reads and writes the variable
    /// table, which belongs to an instance, so a `Shell` made in here
    /// would be a different set of variables from the one the test set up.
    fn run(sh: &mut Shell, name: &[u8], words: &[&[u8]]) -> Flow {
        let mut args = vec![BStr::new(name)];
        args.extend(words.iter().map(|w| BStr::new(*w)));
        exportcmd(sh, &args).unwrap()
    }

    /// The word the builtin was called as picks the flag, which is the
    /// whole of the difference between the two commands.
    #[test]
    fn the_calling_name_picks_the_flag() {
        let _g = lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        let name = BStr::new("Texport");
        set_bytes(sh, name, Some(BStr::new("v")), VSTRFIXED).unwrap();

        assert_eq!(run(sh, b"export", &[b"Texport"]), Flow::Done(0));
        assert_ne!(flags_bytes(sh, name).unwrap() & VEXPORT, 0);
        assert_eq!(flags_bytes(sh, name).unwrap() & VREADONLY, 0);

        assert_eq!(run(sh, b"readonly", &[b"Texport"]), Flow::Done(0));
        assert_ne!(flags_bytes(sh, name).unwrap() & VREADONLY, 0);
    }

    /// An operand carrying a value assigns as well as flags, which is
    /// what makes `export` one of the assignment builtins.
    #[test]
    fn an_operand_may_assign() {
        let _g = lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        assert_eq!(run(sh, b"export", &[b"Texport2=set"]), Flow::Done(0));
        let name = BStr::new("Texport2");
        assert_eq!(lookup_bytes(sh, name).map(Vec::from), Some(b"set".to_vec()));
        assert_ne!(flags_bytes(sh, name).unwrap() & VEXPORT, 0);
    }
}
