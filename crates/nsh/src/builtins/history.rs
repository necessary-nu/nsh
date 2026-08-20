//! The interactive `history` compatibility builtin.

use crate::context::Shell;
use crate::error::Error;
use crate::eval::Flow;
use crate::output::Dest;
use bstr::BStr;

// [spec:nsh:req:compat.smoosh.history-builtin]
pub fn historycmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut clear = false;
    let mut opts = crate::options::Options::new(args);
    while let Some(option) = opts.next(&mut sh.diagnostics(), b"c")? {
        debug_assert_eq!(option, b'c');
        clear = true;
    }

    if clear {
        let Some(history) = crate::histedit::history_mut(sh) else {
            return Ok(Flow::Done((1).into()));
        };
        *history = crate::linedit::History::new();
        let size = crate::var::histsizeval(sh);
        crate::histedit::sethistsize(sh, BStr::new(size.as_slice()));
        crate::histedit::save_history(sh);
        return Ok(Flow::Done((0).into()));
    }

    let Some(contents) = crate::histedit::history_mut(sh).map(|history| history.file_contents())
    else {
        return Ok(Flow::Done((1).into()));
    };
    sh.write_output(Dest::Stdout, &contents)?;
    Ok(Flow::Done((0).into()))
}
