//! The three expansions the word lexer has to parse rather than read.
//!
//! `${name op word}`, `$(list)` and `$((expression))` all nest a whole
//! grammar inside a word, so the lexer cannot cut them the way it cuts
//! everything else: it has to hand the bytes back to the parser and be
//! told where the construct ended.

use super::*;

// [spec:posix:req:expand.param-hash-requires-word]
fn parse_parameter_operator(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    bad_substitution: bool,
    parameter_syntax: &mut ParameterSyntax,
    nested_syntax: &mut SyntaxContext,
) -> Result<(), Error> {
    if bad_substitution {
        unread_input_unit(shell);
    } else if parameter_syntax.operation == ParameterOperation::Invalid {
        let current_unit = lexer.input;

        /* A here-document delimiter is recorded literally so the body can
         * be matched against it later, but the input can end in the
         * middle of one -- `<<${e` is the whole file -- and then there is
         * no byte to record. Ending is not an operator either, so this
         * falls through and the parse fails for the reason it really
         * failed: an unterminated construct, which is what every other
         * shell reports. Found by fuzzing; it panicked here. */
        lexer.record_delimiter_byte();

        if let Some(operation) = bash::parameter_operator(shell, lexer)? {
            parameter_syntax.operation = operation;
            *nested_syntax = SyntaxContext::Base;
        } else if lexer.input.is(b'%') || lexer.input.is(b'#') {
            let trim_prefix = lexer.input.is(b'#');
            parameter_syntax.operation = if trim_prefix {
                ParameterOperation::RemoveSmallestPrefix
            } else {
                ParameterOperation::RemoveSmallestSuffix
            };
            lexer.input = read_unit_skipping_line_continuations(shell)?;
            if lexer.input == current_unit {
                lexer.record_delimiter_byte();
                parameter_syntax.operation = if trim_prefix {
                    ParameterOperation::RemoveLargestPrefix
                } else {
                    ParameterOperation::RemoveLargestSuffix
                };
            } else {
                unread_input_unit(shell);
            }

            *nested_syntax = SyntaxContext::Base;
        } else {
            if lexer.input.is(b':') {
                parameter_syntax.colon = true;
                lexer.input = read_unit_skipping_line_continuations(shell)?;
                lexer.record_delimiter_byte();
            }
            parameter_syntax.operation = match lexer.input.byte() {
                Some(b'}') if !parameter_syntax.colon || !bash::active(shell) => {
                    ParameterOperation::Value
                }
                Some(b'-') => ParameterOperation::Default,
                Some(b'+') => ParameterOperation::Alternate,
                Some(b'?') => ParameterOperation::Error,
                Some(b'=') => ParameterOperation::Assign,
                _ if parameter_syntax.colon && bash::active(shell) => {
                    // `${name:offset:length}` reuses the colon that
                    // `${name:-word}` spends on its own operator, so the
                    // byte that decided against those forms belongs to
                    // the offset expression and is read again.
                    unread_input_unit(shell);
                    parameter_syntax.colon = false;
                    *nested_syntax = SyntaxContext::Base;
                    ParameterOperation::Substring
                }
                // The byte that is not an operator is the source's, and
                // the word's run is where the source is kept.
                // [spec:nsh:req:idiom.canonical-tree+1]
                _ => ParameterOperation::Invalid,
            };
        }
    } else {
        if parameter_syntax.operation == ParameterOperation::Length && !lexer.input.is(b'}') {
            parameter_syntax.operation = ParameterOperation::Invalid;
        }
        unread_input_unit(shell);
    }
    Ok(())
}

/*
 * Parse a substitution.  At this point, we have read the dollar sign
 * and nothing else.
 */
// [spec:posix:syn:expand.param-format]
// [spec:posix:syn:expand.param-braces-optional]
// [spec:posix:syn:expand.param-unbraced-resolution]
pub(super) fn parse_parameter_expansion(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
) -> Result<(), Error> {
    let mut nested_syntax = lexer.current_syntax().syntax;
    let substitution_start = lexer.output.len();

    lexer.push_literal(b'$');

    lexer.input = read_unit_skipping_line_continuations(shell)?;
    if lexer.input.is(b'(') {
        /* $(command) or $((arith)) */
        lexer.push_literal(lexer.input.expect_byte());
        if read_unit_skipping_line_continuations(shell)?.is(b'(') {
            parse_arithmetic_expansion(lexer)?;
        } else {
            unread_input_unit(shell);
            parse_command_substitution(shell, lexer, false)?;
        }
    } else if lexer.input.is(b'\'')
        && nested_syntax.classify(InputUnit::Byte(b'&')) != SyntaxClass::Word
    {
        lexer.output.truncate(substitution_start);
        lexer.dollar_single_quoted = true;
        lexer.current_syntax_mut().syntax = SyntaxContext::SingleQuoted;
        lexer.record_quote_boundary(true, false);
        return Ok(());
    } else if bash::locale_quote(shell, lexer, nested_syntax, substitution_start)
        || bash::arithmetic_bracket(shell, lexer, substitution_start)
    {
        return Ok(());
    } else if lexer.input.is(b'{')
        || lexer.input.begins_name(&shell.locale)
        || lexer.input.is_special_parameter()
    {
        let mut parameter_syntax = ParameterSyntax::unbraced();
        if lexer.input.is(b'{') {
            if lexer.check_here_document_end {
                lexer.push_literal(b'{');
            }
            lexer.input = read_unit_skipping_line_continuations(shell)?;
            parameter_syntax = ParameterSyntax::braced();
        }
        let indirection = bash::parameter_indirection(shell, lexer, parameter_syntax.braced)?;
        let indirect = indirection == bash::Indirection::Present;
        let mut bad_substitution = indirection == bash::Indirection::Invalid;
        let name_start = lexer.output.len();
        'assignment_name: loop {
            if bad_substitution {
                break 'assignment_name;
            }
            if lexer.input.begins_name(&shell.locale) {
                loop {
                    lexer.push_literal(lexer.input.expect_byte());
                    lexer.input = read_unit_skipping_line_continuations(shell)?;
                    if !lexer.input.continues_name(&shell.locale) {
                        break;
                    }
                }
            } else if lexer.input.is_digit() {
                loop {
                    lexer.push_literal(lexer.input.expect_byte());
                    lexer.input = read_unit_skipping_line_continuations(shell)?;
                    if !(parameter_syntax.accepts_multiple_name_digits() && lexer.input.is_digit())
                    {
                        break;
                    }
                }
            } else if !lexer.input.is(b'}') {
                let mut current_unit = lexer.input;

                lexer.input = read_unit_skipping_line_continuations(shell)?;

                if parameter_syntax.accepts_array_subscript() && current_unit.is(b'#') {
                    parameter_syntax.operation = ParameterOperation::Length;

                    if lexer.input.is(b'_')
                        || lexer
                            .input
                            .byte()
                            .is_some_and(|byte| shell.locale.is_alphanumeric(byte))
                    {
                        if lexer.check_here_document_end {
                            lexer.push_literal(b'#');
                        }
                        continue 'assignment_name;
                    }

                    current_unit = lexer.input;
                    lexer.input = read_unit_skipping_line_continuations(shell)?;
                    if current_unit.is(b'}') || !lexer.input.is(b'}') {
                        unread_input_unit(shell);
                        parameter_syntax.operation = ParameterOperation::Invalid;
                        lexer.input = current_unit;
                        current_unit = InputUnit::Byte(b'#');
                    } else if lexer.check_here_document_end {
                        lexer.push_literal(b'#');
                    }
                }

                if !current_unit.is_special_parameter() {
                    if parameter_syntax.operation == ParameterOperation::Length {
                        parameter_syntax.operation = ParameterOperation::Invalid;
                    }
                    bad_substitution = true;
                    break 'assignment_name;
                }

                lexer.push_literal(current_unit.expect_byte());
            } else {
                bad_substitution = true;
                break 'assignment_name;
            }
            break 'assignment_name;
        }

        bash::parameter_subscript(
            shell,
            lexer,
            bad_substitution,
            parameter_syntax.accepts_subscript_operand(),
        )?;
        bash::parameter_prefix_selector(shell, lexer, indirect && !bad_substitution)?;
        let name_end = lexer.output.len();

        parse_parameter_operator(
            shell,
            lexer,
            bad_substitution,
            &mut parameter_syntax,
            &mut nested_syntax,
        )?;

        if matches!(
            nested_syntax,
            SyntaxContext::Arithmetic
                | SyntaxContext::ArithmeticBracket
                | SyntaxContext::ArithmeticDoubleQuoted
        ) {
            nested_syntax = SyntaxContext::DoubleQuoted;
        }

        if (nested_syntax != lexer.current_syntax().syntax
            || lexer.current_syntax().inner_double_quote)
            && parameter_syntax.has_operand()
        {
            syntax_stack::push(&mut lexer.syntax_frames, nested_syntax);

            lexer.current_syntax_mut().variable_context_pushed = true;
            lexer.current_syntax_mut().double_quoted = nested_syntax != SyntaxContext::Base;
        }

        if parameter_syntax.has_operand() {
            lexer.current_syntax_mut().variable_depth += 1;
            if lexer.current_syntax().double_quoted {
                lexer.current_syntax_mut().double_quote_variable_depth += 1;
            }
        }
        if !lexer.check_here_document_end {
            let name = lexer
                .literal_bytes(name_start..name_end)
                .unwrap_or_default();
            lexer.output.truncate(substitution_start);
            lexer.output.push(WordToken::ParameterStart {
                name,
                operation: parameter_syntax.operation,
                colon: parameter_syntax.colon,
                indirect,
            });
        }
    } else {
        unread_input_unit(shell);
    }

    Ok(())
}

/*
 * Called to parse command substitutions.  oldstyle is set if the command
 * is enclosed inside `...` rather than $(...).
 */

/* parsebackq: */
// [spec:posix:def:expand.cmdsub-forms]
// [spec:posix:req:expand.cmdsub-backquote-backslash]
// [spec:posix:req:expand.cmdsub-backquote-matching]
// [spec:posix:syn:expand.cmdsub-dollar-paren-extent]
// [spec:posix:req:expand.cmdsub-parsing]
// [spec:posix:req:expand.cmdsub-redirections-only]
// [spec:posix:req:expand.cmdsub-alias-substitution]
// [spec:posix:req:expand.cmdsub-terminating-paren]
// [spec:posix:req:expand.cmdsub-nesting]
// [spec:posix:req:expand.cmdsub-arith-ambiguity]
pub(super) fn parse_command_substitution(
    shell: &mut Shell,
    lexer: &mut WordLexer<'_>,
    legacy: bool,
) -> Result<(), Error> {
    let mut saved_prompt_enabled = false;
    /* `grabstackstr(pout)` had to reserve the backquote's text because
     * `list(2)?` builds on the same stack; owning it says the same thing, and
     * it has to outlive the `popfile` below because `setinputstring` reads
     * through the pointer rather than copying. */
    let mut substitution_text: BString = BString::new(Vec::new());

    if lexer.check_here_document_end {
        syntax_stack::push(&mut lexer.syntax_frames, SyntaxContext::Base);
        lexer.current_syntax_mut().backquote = if legacy {
            BackquoteContext::Legacy
        } else {
            BackquoteContext::Modern
        };
        lexer.preserve_escapes = true;
        if legacy {
            shell.input.token_pushed_back = false;
        }
        return Ok(());
    }
    let introducer_length = if legacy { 1 } else { 2 };
    lexer
        .output
        .truncate(lexer.output.len().saturating_sub(introducer_length));
    if legacy {
        /* We must read until the closing backquote, giving special
        treatment to some slashes, and then push the string and
        reread it as input, interpreting it normally.  */
        let mut input: InputUnit;

        loop {
            if shell.input.prompt_needed {
                select_prompt(shell, PromptKind::Continuation)?;
            }
            input = read_unit_skipping_line_continuations(shell)?;
            if input.is(b'`') {
                break;
            } else if input.is(b'\\') {
                input = read_input_unit(shell)?;
                if input.byte().is_none() {
                    return Err(syntax_error(shell, b"EOF in backquote substitution"));
                }
                if !input.is(b'\\')
                    && !input.is(b'`')
                    && !input.is(b'$')
                    && (!lexer.current_syntax().double_quoted || !input.is(b'"'))
                {
                    substitution_text.push(b'\\');
                }
                if let MultibyteInput::Character { bytes, .. } =
                    read_multibyte_character(shell, input, MultibyteMode::Literal)?
                {
                    substitution_text.extend_from_slice(&bytes);
                    continue;
                }
            } else if input == InputUnit::EndOfInput {
                return Err(syntax_error(shell, b"EOF in backquote substitution"));
            } else if input.is(b'\n') {
                consume_newline_without_prompt(shell);
            }
            substitution_text.push(input.expect_byte());
        }
    }
    let saved_here_documents = core::mem::take(&mut shell.input.pending_here_documents);
    let completed_at = shell.input.completed_here_documents.len();

    if legacy {
        saved_prompt_enabled = shell.input.prompt_before_read;
        shell.input.prompt_before_read = false;
    }

    /* A substitution re-enters the grammar from inside a word, which is
     * a route `nested_command` cannot see: the word being read belongs to
     * the enclosing command, and the list inside `$( )` starts a fresh
     * descent. `$( $( ... ) )` recursed unbounded until this charged it. */
    // [spec:nsh:req:idiom.bounded-recursion]
    let parsed = keywords::nested(shell, |shell| {
        crate::resource::with_resources(shell, |shell, _resources| {
            if legacy {
                set_input_string(shell, BStr::new(&substitution_text));
            }
            let mut node = list(shell, ListMode::StopAtTerminator)?.into_node();

            if !legacy {
                if read_token(shell, TokenContext::NONE)?.kind != TokenKind::RightParen {
                    return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
                }
                set_input_string(shell, BStr::new(b""));
            }

            parse_here_documents(shell)?;
            finalize::node(shell, &mut node, completed_at)?;
            Ok(node)
        })
    });

    if legacy {
        shell.input.prompt_before_read = saved_prompt_enabled;
    }
    shell.input.pending_here_documents = saved_here_documents;
    if legacy {
        /* Ignore any pushed back tokens left from the backquote
         * parsing.
         */
        shell.input.token_pushed_back = false;
    }
    lexer.output.push(WordToken::Command(parsed?.map(Box::new)));
    Ok(())
}

/*
 * Parse an arithmetic expansion (indicate start of one and set state)
 */
/* parsearith: */
// [spec:posix:syn:expand.arith-format]
fn parse_arithmetic_expansion(lexer: &mut WordLexer<'_>) -> Result<(), Error> {
    syntax_stack::push(&mut lexer.syntax_frames, SyntaxContext::Arithmetic);
    lexer.current_syntax_mut().double_quoted = true;
    if lexer.check_here_document_end {
        lexer.record_delimiter_byte();
    } else {
        lexer.output.truncate(lexer.output.len().saturating_sub(2));
        lexer.output.push(WordToken::ArithmeticStart);
    }
    Ok(())
}
