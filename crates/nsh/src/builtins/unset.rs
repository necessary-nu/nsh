//! `unset`.
//!
//! Port of `unsetcmd` from `src/var.c`. `-v` and `-f` choose between the
//! variable table and the function table; the last one given wins, and
//! with neither it is the variable table.

use crate::error::Error;
use bstr::BStr;
use crate::eval::Flow;
use crate::options::Options;
use crate::var::unsetvar;

// [spec:dash:def:var.unsetcmd-fn]
// [spec:dash:sem:var.unsetcmd-fn]
pub unsafe fn unsetcmd(args: &[&BStr]) -> Result<Flow, Error> {
    let mut flag: u8 = 0;

    let mut opts = Options::new(args);
    while let Some(i) = opts.next(b"vf")? {
        flag = i;
    }

    for name in opts.operands() {
        let name = crate::shell::cstring(name);
        if flag != b'f' {
            unsetvar(name.as_ptr())?;
            continue;
        }
        if flag != b'v' {
            crate::exec::unsetfunc(name.as_ptr());
        }
    }
    Ok(Flow::Done(0))
}

#[cfg(test)]
mod tests {
    use super::*;

    use crate::testutil::{CStr0, lock};
    use crate::var::{lookupvar, setvar};

    /// With neither option, and with `-v`, the variable goes; `-f` leaves
    /// it alone because it is looking at the other table.
    #[test]
    fn the_option_picks_the_table() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("Tunset");

            setvar(name.p(), CStr0::new("v").p(), 0);
            assert_eq!(unsetcmd(&[BStr::new("unset"), BStr::new("Tunset")]).unwrap(), Flow::Done(0));
            assert!(lookupvar(name.p()).is_null());

            setvar(name.p(), CStr0::new("v").p(), 0);
            assert_eq!(
                unsetcmd(&[BStr::new("unset"), BStr::new("-v"), BStr::new("Tunset")]).unwrap(),
                Flow::Done(0)
            );
            assert!(lookupvar(name.p()).is_null());

            setvar(name.p(), CStr0::new("v").p(), 0);
            assert_eq!(
                unsetcmd(&[BStr::new("unset"), BStr::new("-f"), BStr::new("Tunset")]).unwrap(),
                Flow::Done(0)
            );
            assert!(!lookupvar(name.p()).is_null(), "-f is the function table");
            unsetvar(name.p());
        }
    }

    /// The last option given wins, so `-f -v` unsets the variable.
    #[test]
    fn the_last_option_wins() {
        let _g = lock();
        unsafe {
            let name = CStr0::new("Tunset2");
            setvar(name.p(), CStr0::new("v").p(), 0);
            assert_eq!(
                unsetcmd(&[
                    BStr::new("unset"),
                    BStr::new("-f"),
                    BStr::new("-v"),
                    BStr::new("Tunset2"),
                ])
                .unwrap(),
                Flow::Done(0)
            );
            assert!(lookupvar(name.p()).is_null());
        }
    }
}
