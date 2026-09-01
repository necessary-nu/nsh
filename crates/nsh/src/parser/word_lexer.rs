use super::*;
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
     * `$((\$))` and `$(($))` are one expression and the byte is data.
     * `$[ ]` is the same expression read to a different terminator. */
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    if matches!(
        lexer.current_syntax().syntax,
        SyntaxContext::Arithmetic
            | SyntaxContext::ArithmeticBracket
            | SyntaxContext::ArithmeticDoubleQuoted
    ) {
        lexer.quoted = true;
        lexer.push_literal(lexer.input.expect_byte());
        return Ok(());
    }

    /* Inside double quotes a backslash only escapes the four bytes that mean
     * something there; before anything else it is data, and so the
     * backslash itself is one of the word's bytes rather than a spelling
     * of the byte after it. Only the parser knows which happened. */
    // [spec:nsh:req:idiom.printable-ast+2]
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

/*
 * If eofmark is NULL, read a word or a redirection symbol.  If eofmark
 * is not NULL, read a here document.  In the latter case, eofmark is the
 * word which marks the end of the document and strip_tabs is true if
 * leading tabs should be stripped from the document.  The argument firstc
 * is the first character of the input token or document.
 *
 * The word lexer delegates here-document checks, redirections,
 * substitutions, backquotes, and arithmetic to focused helpers that borrow
 * the current lexer state.
 */

/// The locals of `readtoken1` that its internal subroutines share.
pub(super) struct WordLexer<'a> {
    /// Owned parse contexts, base first and current last. Popping retains the
    /// allocation, matching the C's reuse of its most recently popped level.
    pub(super) syntax_frames: Vec<SyntaxFrame>,
    /// The unquoted context a closing quote returns to.
    base_syntax: SyntaxContext,
    pub(super) check_here_document_end: bool,
    pub(super) preserve_escapes: bool,
    pub(super) dollar_single_quoted: bool,
    pub(super) input: InputUnit,
    quoted: bool,
    /// How many `X(` extended-glob groups are open in this word. While
    /// one is, `(`, `)`, `|`, and blanks are the pattern's own bytes.
    // [spec:nsh:req:compat.bash.expansion-globbing]
    pub(super) extglob_depth: usize,
    /// Where a `[` in this word opens a subscript, which is what lets
    /// Bash's lexer read `name[` as the opening of a subscript rather
    /// than as the end of a name.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    pub(super) subscript_position: SubscriptPosition,
    /// How many `[` of an assignment word's subscript are open. While
    /// one is, blanks and shell operators are the subscript's own bytes.
    // [spec:nsh:req:compat.bash.arrays-declarations]
    pub(super) subscript_depth: usize,
    /// Typed lexer events for the word being built.
    pub(super) output: Vec<WordToken>,
    pub(super) delimiter: EofMark<'a>,
    strip_tabs: bool,
}

// [spec:dash:sem:parser.readtoken1-fn]
// [spec:posix:req:shell.hashbang-unspecified]
// [spec:posix:sem:shell.tokenization-and-parsing]
// [spec:posix:def:quote.purpose]
// [spec:posix:req:quote.always-special-characters]
// [spec:posix:req:quote.conditionally-special-characters]
// [spec:posix:req:quote.future-special-characters]
// [spec:posix:def:quote.mechanisms]
// [spec:posix:req:quote.backslash-literal]
// [spec:posix:req:quote.single-quotes]
// [spec:posix:req:quote.double-quotes-literal]
// [spec:posix:req:quote.double-quotes-dollar-sign]
// [spec:posix:req:quote.double-quotes-command-substitution]
// [spec:posix:req:quote.double-quotes-substring-parameter-expansion]
// [spec:posix:req:quote.double-quotes-other-parameter-expansion]
// [spec:posix:req:quote.double-quotes-backquote]
// [spec:posix:req:quote.double-quotes-backquote-undefined]
// [spec:posix:req:quote.double-quotes-backslash]
// [spec:posix:req:quote.double-quotes-expansion-result]
// [spec:posix:req:quote.double-quotes-embedded-double-quote]
// [spec:posix:req:quote.dollar-single-quotes]
// [spec:posix:syn:token.quoting-characters]
// [spec:posix:syn:token.expansion-candidates]
// [spec:posix:syn:token.append-to-word]
// [spec:nsh:req:idiom.parser-control-flow]
pub(super) fn read_word_token(
    shell: &mut Shell,
    first_input: InputUnit,
    syntax: SyntaxContext,
    delimiter: EofMark<'_>,
    strip_tabs: bool,
    check_here_document_end: bool,
    subscript_position: SubscriptPosition,
) -> Result<Token, Error> {
    let mut lexer = WordLexer {
        syntax_frames: vec![SyntaxFrame {
            syntax,
            inner_double_quote: false,
            variable_context_pushed: false,
            double_quoted: syntax == SyntaxContext::DoubleQuoted,
            backquote: BackquoteContext::None,
            variable_depth: 0,
            parenthesis_depth: 0,
            double_quote_variable_depth: 0,
        }],
        base_syntax: if syntax == SyntaxContext::Regex {
            SyntaxContext::Regex
        } else {
            SyntaxContext::Base
        },
        check_here_document_end,
        preserve_escapes: syntax == SyntaxContext::SingleQuoted,
        dollar_single_quoted: false,
        input: first_input,
        quoted: false,
        extglob_depth: 0,
        subscript_position: if bash::active(shell) && delimiter.is_none() {
            subscript_position
        } else {
            SubscriptPosition::None
        },
        subscript_depth: 0,
        output: Vec::new(),
        delimiter,
        strip_tabs,
    };
    'word: loop {
        /* for each line, until end of word */
        finish_word_if_delimited(shell, &mut lexer)?;
        /* Until end of line or end of word */
        loop {
            let position = WordPosition::of(lexer.current_syntax());
            let field_splitting =
                position.field_splitting && lexer.extglob_depth == 0 && lexer.subscript_depth == 0;
            bash::process_substitutions(shell, &mut lexer, field_splitting)?;
            if bash::open_extended_glob(shell, &mut lexer)? {
                continue;
            }
            /* The C's CHECKSTRSPACE, which permits max(MB_LEN_MAX, 23)
             * calls to USTPUTC, has no counterpart here: `getmbc`
             * writes into its own scratch and `getmbc_at` appends
             * what it reports, so there is no room for this frame to
             * make on its behalf. */
            let multibyte_mode = MultibyteMode::for_word(field_splitting, lexer.preserve_escapes);
            match read_multibyte_character(shell, lexer.input, multibyte_mode)? {
                MultibyteInput::FieldBoundary => {
                    if lexer.output.is_empty() {
                        return Ok(Token::plain(TokenKind::Blank));
                    }
                    lexer.input = read_input_unit(shell)?;
                    break 'word;
                }
                MultibyteInput::Character { bytes, escaped } => {
                    lexer.push_character(bytes, escaped);
                    lexer.input = read_unit_for_syntax(shell, lexer.current_syntax())?;
                    continue;
                }
                MultibyteInput::SingleByte => {}
            }

            let class = lexer.current_syntax().syntax.classify(lexer.input);

            match class {
                SyntaxClass::Newline => {
                    if field_splitting || position.regex_word {
                        break 'word;
                    }
                    lexer.push_literal(lexer.input.expect_byte());
                    prompt_after_newline(shell)?;
                    lexer.input = read_unit_for_syntax(shell, lexer.current_syntax())?;
                    continue 'word;
                }
                SyntaxClass::Word => {
                    bash::track_assignment_subscript(shell, &mut lexer);
                    if !bash::scan_arithmetic_bracket(&mut lexer) {
                        lexer.push_literal(lexer.input.expect_byte());
                    }
                }
                SyntaxClass::Control => {
                    if lexer.dollar_single_quoted && lexer.input.is(b'\\') {
                        parse_dollar_single_quote_escape(shell, &mut lexer.output)?;
                    } else {
                        if lexer.delimiter.is_none()
                            || lexer.current_syntax().double_quoted
                            || lexer.current_syntax().variable_depth != 0
                        {
                            // The quoting is what makes this byte data,
                            // and being data is all the tree records.
                            // [spec:nsh:req:idiom.canonical-tree+1]
                            lexer.push_inert(lexer.input.expect_byte());
                        } else {
                            lexer.push_literal(lexer.input.expect_byte());
                        }
                    }
                }
                SyntaxClass::Backslash => word_lexer::read_backslash(shell, &mut lexer)?,
                SyntaxClass::SingleQuote => {
                    lexer.current_syntax_mut().syntax = SyntaxContext::SingleQuoted;
                    lexer.record_quote_boundary(true, false);
                }
                SyntaxClass::DoubleQuote => {
                    lexer.current_syntax_mut().syntax = SyntaxContext::DoubleQuoted;
                    lexer.current_syntax_mut().double_quoted = true;
                    lexer.record_quote_boundary(true, true);
                }
                SyntaxClass::EndQuote => lexer.close_quote(),
                SyntaxClass::Variable => parse_parameter_expansion(shell, &mut lexer)?,
                SyntaxClass::EndVariable => lexer.close_parameter_expansion(),
                SyntaxClass::LeftParen => {
                    lexer.current_syntax_mut().parenthesis_depth += 1;
                    lexer.push_literal(lexer.input.expect_byte());
                }
                SyntaxClass::RightParen => match close_parenthesis(shell, &mut lexer, position)? {
                    ParenthesisOutcome::EndWord => break 'word,
                    ParenthesisOutcome::Advanced => continue,
                    ParenthesisOutcome::Consumed => {}
                },
                SyntaxClass::Backquote => {
                    if lexer.current_syntax().backquote == BackquoteContext::Legacy {
                        syntax_stack::pop(&mut lexer.syntax_frames);
                        lexer.preserve_escapes = false;
                        lexer.push_literal(lexer.input.expect_byte());
                    } else {
                        lexer.push_literal(b'`');
                        parse_command_substitution(shell, &mut lexer, true)?;
                    }
                }
                SyntaxClass::EndOfInput | SyntaxClass::EndOfAlias => break 'word,
                SyntaxClass::WordSeparator => {
                    if bash::inside_extended_glob(&mut lexer) {
                        lexer.push_literal(lexer.input.expect_byte());
                    } else if lexer.input.is(b')')
                        && lexer.current_syntax().backquote == BackquoteContext::Modern
                    {
                        syntax_stack::pop(&mut lexer.syntax_frames);
                        lexer.preserve_escapes = false;
                        lexer.push_literal(lexer.input.expect_byte());
                    } else if field_splitting || position.regex_boundary {
                        break 'word;
                    } else {
                        lexer.push_literal(lexer.input.expect_byte());
                    }
                }
            }

            lexer.input = read_unit_for_syntax(shell, lexer.current_syntax())?;
        }
    }
    finish_word_token(shell, &mut lexer)
}

/// Close one word: reject an unterminated construct, hand a bare
/// descriptor digit to the redirection parser, and otherwise publish the
/// structural word the lexer built.
// [spec:dash:sem:parser.readtoken1-fn]
fn finish_word_token(shell: &mut Shell, lexer: &mut WordLexer<'_>) -> Result<Token, Error> {
    if lexer.current_syntax().syntax == SyntaxContext::ArithmeticBracket {
        return Err(syntax_error(shell, b"Missing ']'"));
    }
    if lexer.current_syntax().syntax == SyntaxContext::Arithmetic {
        return Err(syntax_error(shell, b"Missing '))'"));
    }
    if (!matches!(
        lexer.current_syntax().syntax,
        SyntaxContext::Base | SyntaxContext::Regex
    ) && lexer.delimiter.is_none())
        || lexer.current_syntax().backquote != BackquoteContext::None
    {
        return Err(syntax_error(shell, b"Unterminated quoted string"));
    }
    if lexer.current_syntax().variable_depth != 0 {
        /* { */
        return Err(syntax_error(shell, b"Missing '}'"));
    }
    /* An assignment word's subscript swallowed the blanks and operators
     * inside it, so one that never closed has swallowed the rest of the
     * input. Bash reports the same unterminated construct. */
    // [spec:nsh:req:compat.bash.arrays-declarations]
    if lexer.subscript_depth != 0 {
        return Err(syntax_error(shell, b"Missing ']'"));
    }
    /* The outer `Option` is whether what was just read is a redirection
     * prefix at all; the inner one is whether it carried one, since an
     * operator with nothing before it takes its own default. A prefix is
     * literal bytes and nothing else: a word holding an expansion or a
     * quoted run is a word. */
    /* IO_NUMBER is "a string consisting solely of digits", not one digit:
     * `exec 42>file` names slot 42. A run too large to name a slot is not
     * an IO_NUMBER, and the standard says the token identifier is then
     * TOKEN -- an ordinary word, which is what falling through to the
     * bottom of this function produces. Bash adds `{name}`, which names no
     * slot and asks for one to be allocated.
     *
     * The first byte settles it for almost every word, and settling it
     * there is not a micro-optimisation: the collect below copies the
     * word, and a script of 200,000-byte lines would otherwise copy each
     * one to be told it is not a redirection. Digits go on being read as
     * digits, so only a word that opens with a brace costs more than it
     * did, and only in the dialect that has the form. */
    // [spec:posix:syn:grammar.token-classification]
    // [spec:posix:syn:redir.format]
    let braced =
        bash::active(shell) && matches!(lexer.output.first(), Some(WordToken::Literal(b'{')));
    let prefix: Option<Option<RedirectionDescriptor>> = if lexer.output.is_empty() {
        Some(None)
    } else {
        lexer
            .output
            .iter()
            .map(|token| match token {
                WordToken::Literal(byte) if braced || byte.is_ascii_digit() => Some(*byte),
                _ => None,
            })
            .collect::<Option<Vec<u8>>>()
            .and_then(|bytes| {
                LogicalDescriptor::from_digits(&bytes)
                    .map(RedirectionDescriptor::Fixed)
                    .or_else(|| {
                        bash::allocated_descriptor(shell, &bytes)
                            .map(RedirectionDescriptor::Allocated)
                    })
            })
            .map(Some)
    };
    if lexer.delimiter.is_none() {
        if let Some(explicit) =
            prefix.filter(|_| (lexer.input.is(b'>') || lexer.input.is(b'<')) && !lexer.quoted)
        {
            parse_redirection(shell, lexer, explicit)?;
            shell.input.last_token = TokenKind::Redirection;
            shell.input.last_token_quoted = false;
            return Ok(Token::plain(TokenKind::Redirection));
        }
        unread_input_unit(shell);
    }
    shell.input.last_token_quoted = lexer.quoted;
    keywords::nested_expansions(shell, &lexer.output)?;
    /* `grabstackblock(len)` reserved the bytes the C had been writing into
     * scratch space, which is what made `wordtext` outlive the next token.
     * Moving the buffer out is the same guarantee. */
    // [spec:nsh:def:idiom.word-ir]
    shell.input.word = ParsedWord::from_tokens(mem::take(&mut lexer.output));
    shell.input.last_token = TokenKind::Word;
    Ok(Token {
        kind: TokenKind::Word,
        quoted: lexer.quoted,
    })
}
/* end of readtoken routine */

/*
 * Check to see whether we are at the end of the here document.  When this
 * is called, c is set to the first character of the next input line.  If
 * we are at the end of the here document, this routine records an explicit
 * end-of-input boundary.
 */

/* checkend: */
// [spec:posix:req:redir.here-doc-delimiter]
// [spec:posix:req:redir.here-doc-tab-strip]
fn finish_word_if_delimited(shell: &mut Shell, lexer: &mut WordLexer<'_>) -> Result<(), Error> {
    if let Some(mark) = lexer.delimiter.real() {
        let mut index: usize;
        let mut more_heredoc = false;

        if lexer.strip_tabs {
            while lexer.input.is(b'\t') {
                lexer.input = read_input_unit(shell)?;
            }
        }

        let mut consumed = Vec::new();
        index = 0;
        loop {
            if let Some(byte) = lexer.input.byte() {
                consumed.push(byte);
            }
            if index == mark.len() {
                break;
            }
            if !lexer.input.is(mark[index]) {
                more_heredoc = true;
                break;
            }

            lexer.input = read_input_unit(shell)?;
            index += 1;
        }

        if !more_heredoc {
            if lexer.input.is(b'\n') || lexer.input == InputUnit::EndOfInput {
                lexer.input = InputUnit::EndOfInput;
                consume_newline_without_prompt(shell);
            } else {
                more_heredoc = true;
            }
        }

        if more_heredoc {
            if let Some((&first, rest)) = consumed.split_first() {
                lexer.input = InputUnit::Byte(first);
                if !rest.is_empty() {
                    /* These bytes were read once already and are about to
                     * be read again. A pushed string is not a frame of
                     * its own -- `strpush` stacks inside the one it was
                     * pushed on -- so the reader records them a second
                     * time unless the first is taken back. The byte in
                     * `lexer.input` is handed over rather than re-read,
                     * so it is not one of them. */
                    // [spec:nsh:def:idiom.token-stream]
                    let frame = shell.input.current;
                    shell.input.tokens.unrecord(frame, rest.len());
                    push_string_input(shell, BStr::new(rest), None);
                }
            }
        }
    }
    Ok(())
}
