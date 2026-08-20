//! `pwd`.
//!
//! Port of `pwdcmd` from `src/cd.c`. It prints what the shell believes
//! the current directory to be -- the logical path it has been
//! maintaining, or with `-P` the one the kernel would give.
//!
//! It shares `cd`'s option scan, because `-L` and `-P` mean the same
//! thing to both.

use crate::context::Shell;
use crate::error::Error;
use crate::output::OutputDestination;
use bstr::BStr;

use crate::builtins::cd::parse_cd_options;
use crate::evaluation::Flow;
use crate::options::Options;
use crate::working_directory::{DirectoryUpdate, update_current_directory};

// [spec:dash:def:cd.pwdcmd-fn]
// [spec:dash:sem:cd.pwdcmd-fn]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let options = parse_cd_options(shell, &mut Options::new(args))?;
    let mut dir = if options.physical {
        if shell.working_directory.physical.is_none() {
            update_current_directory(shell, DirectoryUpdate::Current, false)?;
        }
        shell.working_directory.physical.clone().unwrap_or_default()
    } else {
        shell.working_directory.logical.clone().unwrap_or_default()
    };
    dir.push(b'\n');
    shell.write_output(OutputDestination::Stdout, &dir)?;
    Ok(Flow::Done((0).into()))
}
