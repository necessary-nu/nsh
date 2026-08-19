//! The interactive `history` compatibility builtin.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use bstr::BStr;
use std::io::Write as _;

// [spec:nsh:req:compat.smoosh.history-builtin]
pub fn historycmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut clear = false;
    let mut opts = crate::options::Options::new(args);
    while let Some(option) = opts.next(sh, b"c")? {
        debug_assert_eq!(option, b'c');
        clear = true;
    }

    if clear {
        let Some(history) = crate::histedit::history_mut(sh) else {
            return Ok(Flow::Done(1));
        };
        *history = crate::linedit::History::new();
        let size = crate::var::histsizeval(sh);
        crate::histedit::sethistsize(sh, BStr::new(size.as_slice()));
        crate::histedit::save_history(sh);
        return Ok(Flow::Done(0));
    }

    let Some(contents) = crate::histedit::history_mut(sh).map(|history| history.file_contents())
    else {
        return Ok(Flow::Done(1));
    };
    let _ = sh.io.stdout().write_all(&contents);
    Ok(Flow::Done(0))
}
