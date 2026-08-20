//! The interactive `history` compatibility builtin.

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::output::OutputDestination;
use bstr::BStr;

// [spec:nsh:req:compat.smoosh.history-builtin]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut clear = false;
    let mut option_scan = crate::options::Options::new(args);
    while let Some(option) = option_scan.next(&mut shell.diagnostics(), b"c")? {
        debug_assert_eq!(option, b'c');
        clear = true;
    }

    if clear {
        let Some(history) = crate::editor::history_mut(shell) else {
            return Ok(Flow::Done((1).into()));
        };
        *history = crate::editor::History::new();
        let size = crate::variables::history_size_value(shell);
        crate::editor::set_history_size(shell, BStr::new(size.as_slice()));
        crate::editor::save_history(shell);
        return Ok(Flow::Done((0).into()));
    }

    let Some(contents) = crate::editor::history_mut(shell).map(|history| history.file_contents())
    else {
        return Ok(Flow::Done((1).into()));
    };
    shell.write_output(OutputDestination::Stdout, &contents)?;
    Ok(Flow::Done((0).into()))
}
