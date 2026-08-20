use super::{
    BackquoteContext, Error, InputUnit, LEGACY_END_PARAMETER, LEGACY_ESCAPE, MultibyteMode, Shell,
    SyntaxContext, WordLexer, decode_multibyte_character_at, read_input_unit, unread_input_unit,
};

impl WordLexer<'_> {
    pub(super) fn close_quote(&mut self) {
        if !self.delimiter.is_none() && self.current_syntax().variable_depth == 0 {
            self.output.push(self.input.expect_byte());
            return;
        }

        if self.current_syntax().double_quote_variable_depth == 0 {
            if self.dollar_single_quoted {
                let end = self
                    .output
                    .iter()
                    .position(|&byte| byte == 0)
                    .unwrap_or(self.output.len());
                self.output.truncate(end);
                self.dollar_single_quoted = false;
            }

            self.current_syntax_mut().syntax = SyntaxContext::Base;
            self.current_syntax_mut().double_quoted = false;
        }

        self.quoted = true;
        self.record_quote_boundary(self.input.is(b'"'));
    }

    pub(super) fn close_parameter_expansion(&mut self) {
        if self.current_syntax().inner_double_quote || self.current_syntax().variable_depth == 0 {
            self.output.push(self.input.expect_byte());
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
            self.input = InputUnit::Byte(LEGACY_END_PARAMETER as u8);
        }
        self.output.push(self.input.expect_byte());
    }
}

pub(super) fn read_backslash(shell: &mut Shell, lexer: &mut WordLexer<'_>) -> Result<(), Error> {
    lexer.input = read_input_unit(shell)?;
    if lexer.input == InputUnit::EndOfInput {
        lexer.output.push(LEGACY_ESCAPE as u8);
        lexer.output.push(b'\\');
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
        lexer.output.push(LEGACY_ESCAPE as u8);
        lexer.output.push(b'\\');
    }
    lexer.quoted = true;

    if decode_multibyte_character_at(
        shell,
        &mut lexer.output,
        lexer.input,
        MultibyteMode::Escaped,
    )? == 0
    {
        lexer.output.push(LEGACY_ESCAPE as u8);
        lexer.output.push(lexer.input.expect_byte());
    }
    Ok(())
}
