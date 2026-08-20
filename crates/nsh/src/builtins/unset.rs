//! `unset`.
//!
//! Port of `unsetcmd` from `src/var.c`. `-v` and `-f` choose between the
//! variable table and the function table; the last one given wins, and
//! with neither it is the variable table.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use crate::options::Options;
use crate::var::{VREADONLY, flags_bytes, unset_bytes};
use bstr::BStr;

// [spec:dash:def:var.unsetcmd-fn]
// [spec:dash:sem:var.unsetcmd-fn]
// [spec:posix:syn:builtin.unset.synopsis]
// [spec:posix:req:builtin.unset.unset-names]
// [spec:posix:req:builtin.unset.v-option]
// [spec:posix:req:builtin.unset.f-option]
// [spec:posix:req:builtin.unset.no-option]
// [spec:posix:req:builtin.unset.not-previously-set]
// [spec:posix:req:builtin.unset.utility-syntax-guidelines]
// [spec:posix:sem:builtin.unset.empty-assignment-and-special-parameters]
// [spec:posix:req:builtin.unset.stderr]
// [spec:posix:req:builtin.unset.exit-status]
// [spec:posix:sem:builtin.unset.utility-defaults]
pub fn unsetcmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut flag: u8 = 0;

    let mut opts = Options::new(args);
    while let Some(i) = opts.next(sh, b"vf")? {
        flag = i;
    }

    for name in opts.operands() {
        if flag != b'f' {
            if flags_bytes(sh, name).is_some_and(|flags| flags & VREADONLY != 0) {
                let mut message = name.to_vec();
                message.extend_from_slice(b" is read-only");
                return Err(sh.builtin_error_value(1, &message));
            }
            unset_bytes(sh, name)?;
            continue;
        }
        if flag != b'v' {
            crate::exec::unsetfunc(sh, name);
        }
    }
    Ok(Flow::Done((0).into()))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testutil::lock;
    use crate::var::{lookup_bytes, set_bytes};

    /// With neither option, and with `-v`, the variable goes; `-f` leaves
    /// it alone because it is looking at the other table.
    #[test]
    fn the_option_picks_the_table() {
        let _g = lock();
        let name = BStr::new("Tunset");
        let sh = &mut Shell::new(crate::streams::Streams::INHERIT);

        set_bytes(sh, name, Some(BStr::new("v")), 0).unwrap();
        assert_eq!(
            unsetcmd(sh, &[BStr::new("unset"), BStr::new("Tunset")]).unwrap(),
            Flow::Done((0).into())
        );
        assert!(lookup_bytes(sh, name).is_none());

        set_bytes(sh, name, Some(BStr::new("v")), 0).unwrap();
        assert_eq!(
            unsetcmd(
                sh,
                &[BStr::new("unset"), BStr::new("-v"), BStr::new("Tunset")]
            )
            .unwrap(),
            Flow::Done((0).into())
        );
        assert!(lookup_bytes(sh, name).is_none());

        set_bytes(sh, name, Some(BStr::new("v")), 0).unwrap();
        assert_eq!(
            unsetcmd(
                sh,
                &[BStr::new("unset"), BStr::new("-f"), BStr::new("Tunset")]
            )
            .unwrap(),
            Flow::Done((0).into())
        );
        assert!(lookup_bytes(sh, name).is_some(), "-f is the function table");
        unset_bytes(sh, name).unwrap();
    }

    /// The last option given wins, so `-f -v` unsets the variable.
    #[test]
    fn the_last_option_wins() {
        let _g = lock();
        let mut owned = Shell::new(crate::streams::Streams::INHERIT);
        let sh = &mut owned;
        let name = BStr::new("Tunset2");
        set_bytes(sh, name, Some(BStr::new("v")), 0).unwrap();
        assert_eq!(
            unsetcmd(
                sh,
                &[
                    BStr::new("unset"),
                    BStr::new("-f"),
                    BStr::new("-v"),
                    BStr::new("Tunset2"),
                ]
            )
            .unwrap(),
            Flow::Done((0).into())
        );
        assert!(lookup_bytes(sh, name).is_none());
    }
}
