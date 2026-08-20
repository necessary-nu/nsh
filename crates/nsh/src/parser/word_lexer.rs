use super::{
    BackquoteContext, Error, InputUnit, MultibyteInput, MultibyteMode, Shell, SyntaxContext,
    SyntaxFrame, WordLexer, read_input_unit, read_multibyte_character, unread_input_unit,
};
use crate::word::{QuoteBoundary, WordToken};
use bstr::BString;

impl WordLexer<'_> {
    #[inline]
    pub(super) fn current_syntax(&self) -> &SyntaxFrame {
        self.syntax_frames.last().unwrap()
    }

    #[inline]
    pub(super) fn current_syntax_mut(&mut self) -> &mut SyntaxFrame {
        self.syntax_frames.last_mut().unwrap()
    }

    pub(super) fn record_quote_boundary(
        &mut self,
        boundary: QuoteBoundary,
        toggle_nested_double_quote: bool,
    ) {
        if toggle_nested_double_quote && self.current_syntax().variable_depth != 0 {
            self.current_syntax_mut().inner_double_quote ^= true;
        }
        if self.delimiter.is_none() {
            self.output.push(WordToken::Quote(boundary));
        }
    }

    pub(super) fn push_literal(&mut self, byte: u8) {
        self.output.push(WordToken::Literal(byte));
    }

    pub(super) fn push_escaped(&mut self, byte: u8) {
        self.output.push(WordToken::Escaped(byte));
    }

    pub(super) fn push_multibyte(&mut self, bytes: BString, escaped: bool) {
        self.output.push(WordToken::Multibyte { bytes, escaped });
    }

    pub(super) fn literal_bytes(&self, range: core::ops::Range<usize>) -> Option<BString> {
        range
            .map(|at| match self.output.get(at) {
                Some(WordToken::Literal(byte)) => Some(*byte),
                _ => None,
            })
            .collect::<Option<Vec<_>>>()
            .map(BString::from)
    }

    pub(super) fn close_quote(&mut self) {
        if !self.delimiter.is_none() && self.current_syntax().variable_depth == 0 {
            self.push_literal(self.input.expect_byte());
            return;
        }

        if self.current_syntax().double_quote_variable_depth == 0 {
            if self.dollar_single_quoted {
                let end = self
                    .output
                    .iter()
                    .position(|token| {
                        matches!(token, WordToken::Literal(0) | WordToken::Escaped(0))
                    })
                    .unwrap_or(self.output.len());
                self.output.truncate(end);
                self.dollar_single_quoted = false;
            }

            self.current_syntax_mut().syntax = SyntaxContext::Base;
            self.current_syntax_mut().double_quoted = false;
        }

        self.quoted = true;
        self.record_quote_boundary(QuoteBoundary::Close, self.input.is(b'"'));
    }

    pub(super) fn close_parameter_expansion(&mut self) {
        if self.current_syntax().inner_double_quote || self.current_syntax().variable_depth == 0 {
            self.push_literal(self.input.expect_byte());
            return;
        }

        self.current_syntax_mut().variable_depth -= 1;
        if self.current_syntax().variable_depth == 0
            && self.current_syntax().variable_context_pushed
        {
            super::syntax_stack::pop(&mut self.syntax_frames);
        } else if self.current_syntax().double_quote_variable_depth > 0 {
            self.current_syntax_mut().double_quote_variable_depth -= 1;
        }
        if !self.check_here_document_end {
            self.output.push(WordToken::ParameterEnd);
        } else {
            self.push_literal(self.input.expect_byte());
        }
    }
}

pub(super) fn read_backslash(shell: &mut Shell, lexer: &mut WordLexer<'_>) -> Result<(), Error> {
    lexer.input = read_input_unit(shell)?;
    if lexer.input == InputUnit::EndOfInput {
        lexer.push_escaped(b'\\');
        unread_input_unit(shell);
        return Ok(());
    }

    if (lexer.current_syntax().double_quoted
        || lexer.current_syntax().backquote != BackquoteContext::None)
        && !lexer.input.is(b'\\')
        && !lexer.input.is(b'`')
        && !lexer.input.is(b'$')
        && (!lexer.input.is(b'"')
            || (!lexer.delimiter.is_none() && lexer.current_syntax().variable_depth == 0))
        && (!lexer.input.is(b'}') || lexer.current_syntax().variable_depth == 0)
    {
        lexer.push_escaped(b'\\');
    }
    lexer.quoted = true;

    match read_multibyte_character(shell, lexer.input, MultibyteMode::Escaped)? {
        MultibyteInput::Character { bytes, escaped } => {
            lexer.push_multibyte(bytes, escaped);
        }
        MultibyteInput::SingleByte | MultibyteInput::FieldBoundary => {
            lexer.push_escaped(lexer.input.expect_byte());
        }
    }
    Ok(())
}
