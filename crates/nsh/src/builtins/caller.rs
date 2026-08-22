//! `caller`, Bash's read-out of the shell call stack.
//!
//! It is `FUNCNAME`, `BASH_SOURCE` and `BASH_LINENO` printed as one line,
//! and it reads them at the skew those arrays are defined with: frame
//! `n`'s *call* is `BASH_LINENO[n]`, while the function and file the call
//! was written in are `FUNCNAME[n + 1]` and `BASH_SOURCE[n + 1]`. The
//! operand-less form answers a narrower question -- where the running
//! subroutine was called from -- and prints `NULL` for a file the shell
//! does not have, which is Bash's own spelling for it.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

/// What Bash prints where a frame has no file.
const NO_FILE: &[u8] = b"NULL";

/// `caller [expr]`.
// [spec:nsh:req:compat.bash.traps-introspection]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    if shell.variables.call_stack.depth() == 0 {
        return Ok(Flow::Done(ExitStatus::FAILURE));
    }
    let Some(line) = frame_report(shell, args.get(1).copied())? else {
        return Ok(Flow::Done(ExitStatus::FAILURE));
    };
    shell.write_output(OutputDestination::Stdout, &line)?;
    Ok(Flow::Done(ExitStatus::SUCCESS))
}

/// The line to print, or `None` when the requested frame is out of range.
fn frame_report(shell: &mut Shell, operand: Option<&BStr>) -> Result<Option<Vec<u8>>, Error> {
    let Some(operand) = operand else {
        let Some(call) = shell.variables.call_stack.call_line(0) else {
            return Ok(None);
        };
        let file = shell
            .variables
            .call_stack
            .frame_source(1)
            .unwrap_or_else(|| BString::from(NO_FILE));
        return Ok(Some(joined(&[call, file])));
    };

    let level = crate::arithmetic::evaluate(shell, operand)?;
    let Ok(level) = usize::try_from(level) else {
        return Ok(None);
    };
    let stack = &shell.variables.call_stack;
    let (Some(call), Some(name), Some(file)) = (
        stack.call_line(level),
        stack.frame_name(level + 1),
        stack.frame_source(level + 1),
    ) else {
        return Ok(None);
    };
    Ok(Some(joined(&[call, name, file])))
}

fn joined(fields: &[BString]) -> Vec<u8> {
    let mut line = Vec::new();
    for (index, field) in fields.iter().enumerate() {
        if index != 0 {
            line.push(b' ');
        }
        line.extend_from_slice(field);
    }
    line.push(b'\n');
    line
}
