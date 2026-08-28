use super::{
    BackquoteContext, Error, InputUnit, MultibyteInput, MultibyteMode, Shell, SyntaxContext,
    SyntaxFrame, WordLexer, read_input_unit, read_multibyte_character, read_unit_for_syntax,
    read_unit_skipping_line_continuations, syntax_stack, unread_input_unit,
};
use crate::word::WordToken;
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

    /// Record a quote opening or closing.
    ///
    /// A depth and not a kind: which quote it was is the node's run, and
    /// nothing below this ever asked. The four kinds were written by four
    /// callers and read by nobody once the printer stopped re-spelling.
    // [spec:nsh:req:idiom.canonical-tree+1]
    pub(super) fn record_quote_boundary(
        &mut self,
        opening: bool,
        toggle_nested_double_quote: bool,
    ) {
        if toggle_nested_double_quote && self.current_syntax().variable_depth != 0 {
            self.current_syntax_mut().inner_double_quote ^= true;
        }
        if self.delimiter.is_none() {
            self.output.push(if opening {
                WordToken::QuoteOpen
            } else {
                WordToken::QuoteClose
            });
        }
    }

    pub(super) fn push_literal(&mut self, byte: u8) {
        self.output.push(WordToken::Literal(byte));
    }

    /// Record the byte at the cursor into a here-document delimiter, if
    /// there is one.
    ///
    /// A delimiter is kept literally so the body can be matched against it
    /// later, and the input can end part-way through one -- `<<${e` and
    /// `<<${x:` are whole files. `check_here_document_end` says the
    /// delimiter is being recorded; it says nothing about whether the
    /// cursor is on a byte, and reading one out of an end-of-input item
    /// panicked. Recording nothing is right: ending is not an operator
    /// either, so the parse falls through and fails for the reason it
    /// really failed -- an unterminated construct.
    // [spec:nsh:req:idiom.bounded-recursion]
    pub(super) fn record_delimiter_byte(&mut self) {
        if self.check_here_document_end
            && let Some(byte) = self.input.byte()
        {
            self.push_literal(byte);
        }
    }

    /// Record a byte the source made inert.
    ///
    /// One method where there were two. A byte behind a backslash and a
    /// byte the quoting around it protected are the same byte and the
    /// same data; which of the two the source wrote is its run.
    // [spec:nsh:req:idiom.canonical-tree+1]
    pub(super) fn push_inert(&mut self, byte: u8) {
        self.output.push(WordToken::Inert(byte));
    }

    pub(super) fn push_character(&mut self, bytes: BString, inert: bool) {
        self.output.push(WordToken::Character { bytes, inert });
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
                    .position(|token| matches!(token, WordToken::Literal(0) | WordToken::Inert(0)))
                    .unwrap_or(self.output.len());
                self.output.truncate(end);
                self.dollar_single_quoted = false;
            }

            self.current_syntax_mut().syntax = self.base_syntax;
            self.current_syntax_mut().double_quoted = false;
        }

        self.quoted = true;
        self.record_quote_boundary(false, self.input.is(b'"'));
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
        /* A backslash with nothing after it is a line continuation joining to
         * nothing, and Bash drops it: `echo a\` over end of input writes `a`.
         * Keeping the byte wrote a word the source never had. */
        // [spec:nsh:req:compat.bash.expansion-globbing]
        unread_input_unit(shell);
        return Ok(());
    }

    /* Bash discards a backslash inside `$(( ))` before it evaluates, so
     * `$((\$))` and `$(($))` are one expression and the byte is data. */
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    if lexer.current_syntax().syntax == SyntaxContext::Arithmetic {
        lexer.quoted = true;
        lexer.push_literal(lexer.input.expect_byte());
        return Ok(());
    }

    /* Inside double quotes a backslash only escapes the four bytes that mean
     * something there; before anything else it is data, and so is what
     * follows it. Which of the two happened is the difference between a
     * spelling and a byte, and only the parser knows it. */
    // [spec:nsh:req:idiom.printable-ast]
    if (lexer.current_syntax().double_quoted
        || lexer.current_syntax().backquote != BackquoteContext::None)
        && !lexer.input.is(b'\\')
        && !lexer.input.is(b'`')
        && !lexer.input.is(b'$')
        && (!lexer.input.is(b'"')
            || (!lexer.delimiter.is_none() && lexer.current_syntax().variable_depth == 0))
        && (!lexer.input.is(b'}') || lexer.current_syntax().variable_depth == 0)
    {
        lexer.push_inert(b'\\');
    }
    lexer.quoted = true;

    match read_multibyte_character(shell, lexer.input, MultibyteMode::Escaped)? {
        MultibyteInput::Character { bytes, escaped } => {
            lexer.push_character(bytes, escaped);
        }
        MultibyteInput::SingleByte | MultibyteInput::FieldBoundary => {
            lexer.push_inert(lexer.input.expect_byte());
        }
    }
    Ok(())
}

/// What the current parse context says about where a word may end.
///
/// The three facts travel together because they are read together: a
/// Bash regular-expression operand ends at a blank or a shell operator,
/// but only outside its own parentheses, so `(a  b)` stays one word.
// [spec:nsh:req:compat.bash.conditionals-arithmetic]
#[derive(Clone, Copy)]
pub(super) struct WordPosition {
    pub(super) field_splitting: bool,
    pub(super) regex_word: bool,
    pub(super) regex_boundary: bool,
}

impl WordPosition {
    pub(super) fn of(frame: &SyntaxFrame) -> Self {
        let outermost = frame.variable_depth == 0 && frame.backquote == BackquoteContext::None;
        let regex_word = frame.syntax == SyntaxContext::Regex && outermost;
        Self {
            field_splitting: frame.syntax == SyntaxContext::Base && outermost,
            regex_word,
            regex_boundary: regex_word && frame.parenthesis_depth == 0,
        }
    }
}

/// What the caller must do once a `)` has been handled.
pub(super) enum ParenthesisOutcome {
    /// The word ends here.
    EndWord,
    /// The byte was consumed and the next input unit is already read.
    Advanced,
    /// The byte was consumed; the caller reads the next input unit.
    Consumed,
}

/// Handle one `)` in whichever parse context is current.
// [spec:dash:sem:parser.readtoken1-fn]
pub(super) fn close_parenthesis(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    position: WordPosition,
) -> Result<ParenthesisOutcome, Error> {
    if position.regex_boundary {
        return Ok(ParenthesisOutcome::EndWord);
    }
    if position.regex_word {
        lexer.current_syntax_mut().parenthesis_depth -= 1;
        lexer.push_literal(lexer.input.expect_byte());
        return Ok(ParenthesisOutcome::Consumed);
    }
    if lexer.current_syntax().parenthesis_depth > 0 {
        lexer.current_syntax_mut().parenthesis_depth -= 1;
    } else if read_unit_skipping_line_continuations(shell)?.is(b')') {
        syntax_stack::pop(&mut lexer.syntax_frames);
        if lexer.check_here_document_end {
            lexer.push_literal(lexer.input.expect_byte());
            lexer.push_literal(b')');
        } else {
            lexer.output.push(WordToken::ArithmeticEnd);
        }
        lexer.input = read_unit_for_syntax(shell, lexer.current_syntax())?;
        return Ok(ParenthesisOutcome::Advanced);
    } else {
        unread_input_unit(shell);
    }
    if lexer.current_syntax().syntax == SyntaxContext::Arithmetic {
        lexer.push_literal(lexer.input.expect_byte());
    }
    Ok(ParenthesisOutcome::Consumed)
}
