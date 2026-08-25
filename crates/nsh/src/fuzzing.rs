//! The deliberately narrow interface used by the separate cargo-fuzz
//! workspace.
//!
//! This module is public only with the `fuzzing` feature. It provides an
//! opaque parse-and-print operation so the fuzzer can exercise the printer's
//! semantic fixed-point without exposing the AST or parser as library API.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;

/// Parse `source` without executing it and return its canonical rendering.
///
/// The input frame and all parser-local resources are restored before this
/// returns, including when parsing rejects the source. Calling this twice on
/// its own output is the parse-and-print fuzzing property's fixed-point.
pub fn canonical_source(shell: &mut Shell, source: &BStr) -> Result<BString, Error> {
    crate::resource::with_resources(shell, |shell, _resources| {
        crate::input::set_input_string(shell, source);
        let mut rendered = BString::new(Vec::new());
        loop {
            match crate::parser::parse_command(shell, false)? {
                crate::parser::ParseResult::Eof => break,
                crate::parser::ParseResult::Tree(Some(node)) => {
                    rendered.extend_from_slice(&crate::nodes::source::command(&node));
                    rendered.push(b'\n');
                }
                crate::parser::ParseResult::Tree(None) => {}
            }
        }
        Ok(rendered)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Streams;

    fn shell() -> Shell {
        let streams = Streams::capture().expect("captured streams");
        Shell::builder()
            .streams(streams)
            .option(BStr::new(b"bash"), true)
            .build()
            .expect("shell")
    }

    #[test]
    fn canonical_source_is_a_fixed_point() {
        let mut shell = shell();
        let once = canonical_source(
            &mut shell,
            BStr::new(b"v=abc\nif [[ $v == a* ]]; then printf '%s\\n' \"$v\"; fi\n"),
        )
        .expect("first canonicalization");
        let twice =
            canonical_source(&mut shell, BStr::new(&once)).expect("second canonicalization");

        assert_eq!(once, twice);
    }
}
