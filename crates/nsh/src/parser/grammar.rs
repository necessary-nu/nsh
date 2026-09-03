//! The grammar: how tokens make a program.
//!
//! A list of and-or lists, each a pipeline of commands, each a command
//! with its redirections -- and the here-documents whose bodies arrive
//! after the line their operator was written on. Everything here asks the
//! reader for the next token and never for the next byte.

use super::*;

pub(super) fn list(shell: &mut Shell, mode: ListMode) -> Result<ParseResult, Error> {
    let mut stop_at_terminator = mode == ListMode::StopAtTerminator;
    let newline_context = if mode == ListMode::TopLevel {
        TokenContext::NONE
    } else {
        TokenContext::SKIP_NEWLINES
    };
    let mut parsed_command: Option<Node>;
    let mut token: TokenKind;

    parsed_command = None;
    /* Where the whole list began, so a `Sequence` covers every command in
     * it and not only the two it joins. */
    // [spec:nsh:def:idiom.token-stream]
    let mut list_mark: Option<TokenMark> = None;
    loop {
        let command_mark = tokens::mark(shell);
        token = read_token(shell, newline_context.with(TokenContext::COMMAND_START))?.kind;
        match token {
            TokenKind::Newline => {
                parse_here_documents(shell)?;
                return Ok(ParseResult::Tree(parsed_command));
            }

            TokenKind::Eof => {
                let eof = parsed_command.is_none() && newline_context == TokenContext::NONE;
                /* out_eof: */
                parse_here_documents(shell)?;
                shell.input.token_pushed_back = true;
                shell.input.last_token = TokenKind::Eof;
                return if eof {
                    Ok(ParseResult::Eof)
                } else {
                    Ok(ParseResult::Tree(parsed_command))
                };
            }
            _ => {}
        }

        shell.input.token_pushed_back = true;
        if stop_at_terminator && token.ends_list() {
            return Ok(ParseResult::Tree(parsed_command));
        }
        // Top-level input has no enclosing grammar production whose
        // terminator it may return to.  A stray `do`, `}`, and similar
        // token is therefore a syntax error here; only compound lists
        // begin accepting terminators after their first command.
        if mode != ListMode::TopLevel {
            stop_at_terminator = true;
        }

        /* The line the backgrounded command starts on, captured before
         * anything consumes it. `command()?` and `pipeline()?` both take
         * their `savelinno` at this same point, so a wrapper built here
         * records the line its contents record. */
        let saved_line_number = crate::input::current_input_frame(&mut shell.input).line_number;

        let list_mark = *list_mark.get_or_insert(command_mark);
        let mut next = parse_and_or(shell)?.ok_or_else(|| expected_token_error(shell, None))?;
        token = read_token(shell, TokenContext::NONE)?.kind;
        if token == TokenKind::Background {
            /* The `&` is part of what was read, so every shape the
             * backgrounding takes is re-tagged rather than only the one
             * built here. */
            // [spec:nsh:def:idiom.token-stream]
            let backgrounded = tokens::run(shell, command_mark);
            next = match next {
                Node::Pipeline(mut pipeline) => {
                    pipeline.background = true;
                    Node::Pipeline(pipeline)
                }
                Node::Redirect(wrapper) => Node::Background(wrapper),
                command => Node::Background(CompoundCommand {
                    tokens: SourceTokens::none(),
                    line: SourceLine::new(saved_line_number),
                    command: Box::new(command),
                    redirections: Vec::new(),
                }),
            }
            .with_tokens(backgrounded);
        }
        if let Some(left) = parsed_command.take() {
            parsed_command = Some(Node::Sequence(BinaryCommand {
                tokens: tokens::run(shell, list_mark),
                left: Box::new(left),
                right: Box::new(next),
            }));
        } else {
            parsed_command = Some(next);
        }
        match token {
            TokenKind::Eof => {
                parse_here_documents(shell)?;
                shell.input.token_pushed_back = true;
                shell.input.last_token = TokenKind::Eof;
                return Ok(ParseResult::Tree(parsed_command));
            }
            TokenKind::Newline => {
                shell.input.token_pushed_back = true;
            }
            TokenKind::Background | TokenKind::Semicolon => {}
            _ => {
                if newline_context == TokenContext::NONE {
                    return Err(expected_token_error(shell, None));
                }
                shell.input.token_pushed_back = true;
                return Ok(ParseResult::Tree(parsed_command));
            }
        }
    }
}

// [spec:dash:sem:parser.andor-fn]
// [spec:posix:syn:grammar.list-and-or]
// [spec:posix:def:cmd.and-or-list-definition]
// [spec:posix:req:cmd.and-or-precedence]
// [spec:posix:syn:cmd.and-list-format]
// [spec:posix:syn:cmd.or-list-format]
fn parse_and_or(shell: &mut Shell) -> Result<Option<Node>, Error> {
    let mut parsed_command: Option<Node>;

    let start = tokens::mark(shell);
    parsed_command = pipeline(shell, TokenContext::NONE)?;
    loop {
        let operator: fn(BinaryCommand) -> Node = match read_token(shell, TokenContext::NONE)?.kind
        {
            TokenKind::AndIf => Node::And,
            TokenKind::OrIf => Node::Or,
            _ => {
                shell.input.token_pushed_back = true;
                return Ok(parsed_command);
            }
        };
        let left = parsed_command
            .take()
            .ok_or_else(|| expected_token_error(shell, None))?;
        let right = pipeline(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?
            .ok_or_else(|| expected_token_error(shell, None))?;
        parsed_command = Some(operator(BinaryCommand {
            tokens: tokens::run(shell, start),
            left: Box::new(left),
            right: Box::new(right),
        }));
    }
}

// [spec:dash:sem:parser.pipeline-fn]
// [spec:posix:syn:grammar.pipeline]
// [spec:posix:def:cmd.pipeline-definition]
// [spec:posix:syn:cmd.pipeline-format]
// [spec:posix:req:cmd.pipeline-bang-subshell-separation]
pub(super) fn pipeline(shell: &mut Shell, context: TokenContext) -> Result<Option<Node>, Error> {
    let start = tokens::mark(shell);
    let line = crate::input::current_input_frame(&mut shell.input).line_number;
    let first = read_token(shell, context)?.kind;
    /* `time` prefixes the whole pipeline, `!` included -- `time ! true`
     * times the negation and answers 1 -- so it is read before the `!`
     * and wraps whatever the rest of this function builds. A bare `time`
     * has no pipeline to time and reports zeros, which is why the command
     * is optional. */
    // [spec:posix:req:token.reserved-word-time]
    // [spec:nsh:req:compat.bash.select-time-grammar]
    if first == TokenKind::Time {
        return Ok(Some(keywords::timed_command(shell, start, line)?));
    }
    let mut parsed_command: Option<Node>;
    let mut negate = false;
    let command_context = if first == TokenKind::Bang {
        negate = true;
        /* POSIX's grammar takes one `!` -- `pipeline: Bang pipe_sequence`
         * -- and dash refuses a second. Bash repeats it, and each one
         * negates: `! ! true` is 0 and `! ! ! true` is 1. A script that
         * writes it has to run here, so the dialect decides, and the
         * POSIX dialect keeps refusing what the grammar refuses.
         * Found by the `differential` fuzz target. */
        // [spec:posix:syn:grammar.pipeline]
        // [spec:nsh:req:compat.bash.select-time-grammar]
        if bash::active(shell) {
            while read_token(shell, TokenContext::COMMAND_START)?.kind == TokenKind::Bang {
                negate = !negate;
            }
            shell.input.token_pushed_back = true;
        }
        /* Bash takes `!` and `time` in either order, because both are
         * flags on the pipeline command rather than wrappers around it:
         * `! time false` is as ordinary as `time ! false` and reports the
         * same. Reading `time` only before the `!` made the commoner of
         * the two -- `if ! time cmd` -- a syntax error. */
        // [spec:nsh:req:compat.bash.select-time-grammar]
        if read_token(shell, TokenContext::COMMAND_START)?.kind == TokenKind::Time {
            let timed = keywords::timed_command(shell, start, line)?;
            return Ok(Some(if negate {
                Node::Not(NegatedCommand {
                    tokens: tokens::run(shell, start),
                    command: Box::new(timed),
                })
            } else {
                timed
            }));
        }
        shell.input.token_pushed_back = true;
        TokenContext::COMMAND_START
    } else {
        shell.input.token_pushed_back = true;
        TokenContext::NONE
    };
    /* `!` is the negation's, not the pipe sequence's: `! a | b` negates
     * a two-command pipeline whose own run starts at `a`. */
    // [spec:nsh:def:idiom.token-stream]
    let sequence_start = tokens::mark(shell);
    parsed_command = keywords::nested_command(shell, command_context)?;
    let mut render_command_list: Vec<Node> = Vec::new();
    if read_token(shell, TokenContext::NONE)?.kind == TokenKind::Pipe {
        /* Every `stalloc(sizeof(struct nodelist))` the C does here is one
         * `Vec` slot; the list is built front to back either way, and
         * `command()?` cannot return NULL without having raised first. */
        render_command_list.push(
            parsed_command
                .take()
                .ok_or_else(|| expected_token_error(shell, None))?,
        );
        loop {
            render_command_list.push(
                keywords::nested_command(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?
                    .ok_or_else(|| expected_token_error(shell, None))?,
            );
            if read_token(shell, TokenContext::NONE)?.kind != TokenKind::Pipe {
                break;
            }
        }
    }
    /* The token that ended the sequence goes back before either wrapper
     * takes its run, so neither claims the `;` that follows it. */
    // [spec:nsh:def:idiom.token-stream]
    shell.input.token_pushed_back = true;
    if !render_command_list.is_empty() {
        parsed_command = Some(Node::Pipeline(Pipeline {
            tokens: tokens::run(shell, sequence_start),
            background: false,
            commands: render_command_list,
        }));
    }
    if negate {
        let command = parsed_command.ok_or_else(|| expected_token_error(shell, None))?;
        Ok(Some(Node::Not(NegatedCommand {
            tokens: tokens::run(shell, start),
            command: Box::new(command),
        })))
    } else {
        Ok(parsed_command)
    }
}

// [spec:dash:sem:parser.command-fn]
// [spec:posix:syn:grammar.command]
// [spec:posix:syn:grammar.subshell-and-compound-list]
// [spec:posix:syn:grammar.for-clause]
// [spec:posix:syn:grammar.for-name]
// [spec:posix:syn:grammar.third-word-of-for-and-case]
// [spec:posix:syn:grammar.case-statement-termination]
// [spec:posix:syn:grammar.if-clause]
// [spec:posix:syn:grammar.while-until-clause]
// [spec:posix:syn:grammar.brace-group-and-do-group]
// [spec:posix:def:cmd.compound-definition]
// [spec:posix:req:cmd.group-double-paren-ambiguity]
// [spec:posix:req:cmd.for-do-done-delimiters]
// [spec:posix:syn:cmd.for-format]
// [spec:posix:syn:cmd.case-clause-syntax]
// [spec:posix:syn:cmd.case-format]
// [spec:posix:syn:cmd.if-format]
// [spec:posix:syn:cmd.while-format]
// [spec:posix:syn:cmd.until-format]
// [spec:nsh:req:idiom.structural-ast]
pub(super) fn command(shell: &mut Shell, context: TokenContext) -> Result<Option<Node>, Error> {
    let mut parsed_command: Option<Node>;
    let closing_token: Option<TokenKind>;
    let start = tokens::mark(shell);
    let saved_line_number = crate::input::current_input_frame(&mut shell.input).line_number;

    let token = read_token(shell, context)?.kind;
    if let Some(bash_node) = bash::command_prefix(shell, token, SourceLine::new(saved_line_number))?
    {
        parsed_command = Some(bash_node);
        closing_token = None;
    } else if token == TokenKind::If {
        parsed_command = keywords::if_command(shell, start)?;
        closing_token = Some(TokenKind::Fi);
    } else if token == TokenKind::While || token == TokenKind::Until {
        let constructor: fn(BinaryCommand) -> Node = if shell.input.last_token == TokenKind::While {
            Node::While
        } else {
            Node::Until
        };
        let parsed = list(shell, ListMode::Compound)?;
        let left_command = required_compound_node(shell, parsed, TokenKind::Do)?;
        let got = read_token(shell, TokenContext::NONE)?.kind;
        if got != TokenKind::Do {
            return Err(expected_token_error(shell, Some(TokenKind::Do)));
        }
        let parsed = list(shell, ListMode::Compound)?;
        let right_command = required_compound_node(shell, parsed, TokenKind::Done)?;
        parsed_command = Some(constructor(BinaryCommand {
            tokens: SourceTokens::none(),
            left: Box::new(left_command),
            right: Box::new(right_command),
        }));
        closing_token = Some(TokenKind::Done);
    } else if token == TokenKind::For {
        let var_token = read_token(shell, TokenContext::NONE)?;
        // The arithmetic form takes its own closing token, because Bash
        // lets it end at `}` as well as at `done`.
        let mut arithmetic_form = false;
        if var_token.kind == TokenKind::DoubleParen {
            arithmetic_form = true;
            parsed_command = Some(bash::arithmetic_for(
                shell,
                SourceLine::new(saved_line_number),
            )?);
        } else {
            parsed_command = Some(Node::For(Box::new(keywords::iteration_command(
                shell,
                saved_line_number,
                var_token,
                start,
            )?)));
        }
        closing_token = (!arithmetic_form).then_some(TokenKind::Done);
    } else if token == TokenKind::Select {
        /* `for`'s syntax exactly; the menu and the read are the
         * evaluator's, not the grammar's. */
        // [spec:nsh:req:compat.bash.select-time-grammar]
        let var_token = read_token(shell, TokenContext::NONE)?;
        parsed_command = Some(Node::Select(Box::new(keywords::iteration_command(
            shell,
            saved_line_number,
            var_token,
            start,
        )?)));
        closing_token = Some(TokenKind::Done);
    } else if token == TokenKind::Case {
        parsed_command = Some(keywords::case_command(shell, saved_line_number)?);
        closing_token = None;
    } else if token == TokenKind::LeftParen {
        let parsed = list(shell, ListMode::Compound)?;
        let inner = required_compound_node(shell, parsed, TokenKind::RightParen)?;
        parsed_command = Some(Node::Subshell(CompoundCommand {
            tokens: SourceTokens::none(),
            line: SourceLine::new(saved_line_number),
            command: Box::new(inner),
            redirections: Vec::new(),
        }));
        closing_token = Some(TokenKind::RightParen);
    } else if token == TokenKind::LeftBrace {
        parsed_command = list(shell, ListMode::Compound)?.into_node().map(|inner| {
            Node::Group(CompoundCommand {
                tokens: SourceTokens::none(),
                line: SourceLine::new(saved_line_number),
                command: Box::new(inner),
                redirections: Vec::new(),
            })
        });
        closing_token = Some(TokenKind::RightBrace);
    } else if token == TokenKind::Word || token == TokenKind::Redirection {
        shell.input.token_pushed_back = true;
        return parse_simple_command(shell);
    } else {
        return Err(expected_token_error(shell, None));
    }

    if let Some(closing_token) = closing_token {
        if read_token(shell, TokenContext::NONE)?.kind != closing_token {
            return Err(expected_token_error(shell, Some(closing_token)));
        }
    }

    /* A subshell is the one compound form Bash records at its closing
     * token rather than its opening one, and the line is read here
     * because that is where the `)` has just been consumed. dash records
     * the `(` and reports it in the diagnostic a failed redirection on a
     * subshell raises, so the two answers are kept apart by the dialect
     * that asks rather than one of them being made to serve both. */
    // [spec:nsh:req:compat.bash.traps-introspection]
    if bash::active(shell)
        && let Some(Node::Subshell(subshell)) = parsed_command.as_mut()
    {
        subshell.line = line_reached(shell);
    }

    /* Every compound form ends at a closing token the branch above left
     * to the check just made, so the run is taken here rather than where
     * the node was built, where it would stop a token short. */
    // [spec:nsh:def:idiom.token-stream]
    parsed_command = parsed_command.map(|node| node.with_tokens(tokens::run(shell, start)));

    /* Now check for redirection which may follow command */
    let mut redirections: Vec<Redirection> = Vec::new();
    let mut redirection_context = TokenContext::COMMAND_START;
    while read_token(shell, redirection_context)?.kind == TokenKind::Redirection {
        redirection_context = TokenContext::NONE;
        /* The C copies `redirnode` into a local *before* `parsefname`,
         * because the token read inside it can set the global again.
         * Taking ownership of it here is the same guarantee. */
        let pending = core::mem::take(&mut shell.input.pending_redirection)
            .ok_or_else(|| syntax_error(shell, b"missing redirection operator state"))?;
        redirections.push(parse_redirection_target(shell, pending)?);
    }
    shell.input.token_pushed_back = true;
    if !redirections.is_empty() {
        parsed_command = Some(
            match parsed_command.take() {
                Some(Node::Subshell(mut wrapper)) => {
                    wrapper.redirections = redirections;
                    Node::Subshell(wrapper)
                }
                Some(Node::Group(mut wrapper)) => {
                    wrapper.redirections = redirections;
                    Node::Group(wrapper)
                }
                Some(command) => Node::Redirect(CompoundCommand {
                    tokens: SourceTokens::none(),
                    line: SourceLine::new(saved_line_number),
                    command: Box::new(command),
                    redirections,
                }),
                None => return Err(expected_token_error(shell, None)),
            }
            .with_tokens(tokens::run(shell, start)),
        );
    }

    Ok(parsed_command)
}

// [spec:dash:sem:parser.simplecmd-fn]
// [spec:posix:req:redir.not-in-command-arguments]
// [spec:posix:syn:grammar.simple-command]
// [spec:posix:syn:grammar.assignment-first-word]
// [spec:posix:syn:grammar.assignment-word-recognition]
// [spec:posix:syn:grammar.function-definition]
// [spec:posix:syn:grammar.function-name]
// [spec:posix:req:grammar.function-body-no-expansion]
// [spec:posix:def:cmd.simple-definition]
// [spec:posix:def:cmd.function-definition-term]
// [spec:posix:syn:cmd.function-format]
// [spec:posix:req:cmd.function-name-requirements]
// [spec:posix:req:cmd.function-no-expansion-at-definition]
fn parse_simple_command(shell: &mut Shell) -> Result<Option<Node>, Error> {
    let mut args: Vec<Node> = Vec::new();
    let mut variables: Vec<Node> = Vec::new();
    let mut redirections: Vec<Redirection> = Vec::new();
    let mut word_context = TokenContext::ALIASES;
    let start = tokens::mark(shell);
    let saved_line_number = crate::input::current_input_frame(&mut shell.input).line_number;
    loop {
        let word_mark = tokens::mark(shell);
        let token = read_token(shell, word_context)?.kind;
        if token == TokenKind::Word {
            let ordinary_assignment = shell.input.word.is_assignment(&shell.locale);
            let tokens = tokens::run(shell, word_mark);
            let mut node = Node::Word(WordNode {
                tokens,
                word: mem::take(&mut shell.input.word),
            });
            if bash::active(shell)
                && (word_context != TokenContext::NONE || bash::declaration_context(&args))
            {
                node = match node {
                    Node::Word(word) => match bash::array_word(shell, word) {
                        Ok(array) => array,
                        Err(word) => Node::Word(word),
                    },
                    _ => unreachable!("a freshly parsed word is an argument node"),
                };
            }
            let bash_assignment =
                matches!(node, Node::Bash(crate::nodes::BashNode::ArrayAssignment(_)));
            if word_context != TokenContext::NONE && (ordinary_assignment || bash_assignment) {
                variables.push(node);
            } else {
                args.push(node);
                word_context = TokenContext::NONE;
            }
        } else if token == TokenKind::Redirection {
            let pending = core::mem::take(&mut shell.input.pending_redirection)
                .ok_or_else(|| syntax_error(shell, b"missing redirection operator state"))?;
            redirections.push(parse_redirection_target(shell, pending)?);
        } else {
            if token == TokenKind::LeftParen
                && bash::active(shell)
                && bash::compound_array(shell, &mut variables, &mut args)?
            {
                continue;
            }
            /* The C's `app == &args->narg.next` says the argument list holds
             * exactly one word, which is the name being defined. */
            if token == TokenKind::LeftParen
                && args.len() == 1
                && variables.is_empty()
                && redirections.is_empty()
            {
                /* We have a function */
                if read_token(shell, TokenContext::NONE)?.kind != TokenKind::RightParen {
                    return Err(expected_token_error(shell, Some(TokenKind::RightParen)));
                }
                /* the word becomes the function's name; the C keeps the same
                 * `char *` when it relabels the node */
                let Some(Node::Word(word)) = args.pop() else {
                    return Err(syntax_error(shell, b"Bad function name"));
                };
                let builtin_spec = crate::execution::builtin(shell, word.word.as_bstr());
                if !bash::accepts_function_name(shell, word.word.as_bstr())
                    || builtin_spec.is_some_and(|cmd| cmd.attributes().is_special())
                {
                    return Err(syntax_error(shell, b"Bad function name"));
                }
                /* Move the parsed name into a dedicated function variant so
                 * the tree never passes through an invalid intermediate. */
                let line_number = crate::input::current_input_frame(&mut shell.input).line_number;
                let body =
                    keywords::nested_command(shell, TokenContext::COMMAND_START_AFTER_NEWLINES)?
                        .ok_or_else(|| expected_token_error(shell, None))?;
                return Ok(Some(Node::Function(FunctionDefinition {
                    tokens: tokens::run(shell, start),
                    line: SourceLine::new(line_number),
                    name: NodeText::new(BString::from(word.word.as_bstr())),
                    body: Box::new(body),
                })));
            }
            shell.input.token_pushed_back = true;
            break;
        }
    }
    /* out: */
    Ok(Some(Node::Command(Box::new(SimpleCommand {
        tokens: tokens::run(shell, start),
        line: SourceLine::new(saved_line_number),
        assignments: variables,
        arguments: args,
        redirections,
    }))))
}

// [spec:dash:sem:parser.makename-fn]
pub(crate) fn make_name_node(shell: &mut Shell, mark: TokenMark) -> Node {
    Node::Word(WordNode {
        tokens: tokens::run(shell, mark),
        word: mem::take(&mut shell.input.word),
    })
}

// [spec:dash:sem:parser.parsefname-fn]
// [spec:dash:sem:parser.fixredir-fn]
// [spec:posix:req:redir.here-doc-quoted-delimiter]
// [spec:posix:req:redir.here-doc-unquoted-delimiter]
// [spec:posix:req:grammar.here-doc-redirection]
//
// The C reads the redirection node out of the `redirnode` global; here the
// caller has already taken ownership of it, because the `readtoken` below can
// set that global again before this function is done with it.
fn parse_redirection_target(
    shell: &mut Shell,
    pending: PendingRedirection,
) -> Result<Redirection, Error> {
    let is_here_document = matches!(pending, PendingRedirection::HereDocument { .. });
    let target_mark = tokens::mark(shell);
    let token = read_token(
        shell,
        if is_here_document {
            TokenContext::HERE_DOCUMENT_END
        } else {
            TokenContext::NONE
        },
    )?;
    if token.kind != TokenKind::Word {
        return Err(expected_token_error(shell, None));
    }
    let redirection = match pending {
        PendingRedirection::HereDocument { descriptor } => {
            let mut here = core::mem::take(&mut shell.input.pending_here_document)
                .ok_or_else(|| syntax_error(shell, b"missing here-document delimiter state"))?;
            let expand = !token.quoted;
            here.delimiter = BString::from(shell.input.word.as_bstr());
            here.expand = expand;
            let delimiter = NodeText::new(here.delimiter.clone());
            shell.input.pending_here_documents.push(here);
            Redirection::HereDocument(HereDocument {
                descriptor,
                expand,
                /* Replaced wholesale once the body has been read, at the
                 * newline that ends this line. */
                body: WordNode {
                    tokens: SourceTokens::none(),
                    word: ParsedWord::new(),
                },
                delimiter,
            })
        }
        PendingRedirection::HereString { descriptor } => Redirection::HereString(HereString {
            descriptor,
            word: WordNode {
                tokens: tokens::run(shell, target_mark),
                word: mem::take(&mut shell.input.word),
            },
        }),
        PendingRedirection::Descriptor {
            operator,
            descriptor,
        } => {
            let text = shell.input.word_text();
            /* "If word evaluates to one or more digits, the file
             * descriptor denoted by n shall be a duplicate" -- so `>&42`
             * duplicates onto slot 42, not only `>&2`. */
            // [spec:posix:req:redir.dup-output]
            let target = if let Some(number) = LogicalDescriptor::from_digits(text) {
                DescriptorTarget::Number(number)
            } else if text == BStr::new(b"-") {
                DescriptorTarget::Close
            } else {
                DescriptorTarget::Word(WordNode {
                    tokens: tokens::run(shell, target_mark),
                    word: mem::take(&mut shell.input.word),
                })
            };
            Redirection::Descriptor(DescriptorRedirection {
                operator,
                descriptor,
                target,
            })
        }
        PendingRedirection::File {
            operator,
            descriptor,
            with_stderr,
        } => Redirection::File(FileRedirection {
            operator,
            descriptor,
            target: WordNode {
                tokens: tokens::run(shell, target_mark),
                word: mem::take(&mut shell.input.word),
            },
            with_stderr,
        }),
    };
    Ok(redirection)
}

/*
 * Input any here documents.
 */

// [spec:dash:sem:parser.parseheredoc-fn]
// [spec:posix:req:redir.here-doc-line-continuation]
// [spec:posix:req:redir.here-doc-backslash]
// [spec:posix:req:redir.here-doc-multiple]
// [spec:posix:req:redir.here-doc-ps2]
// [spec:posix:req:token.here-document-mode]
pub(super) fn parse_here_documents(shell: &mut Shell) -> Result<(), Error> {
    let list: Vec<PendingHereDocument> = core::mem::take(&mut shell.input.pending_here_documents);

    for here in list {
        if shell.input.prompt_needed {
            select_prompt(shell, PromptKind::Continuation)?;
        }
        let mark = EofMark::Word(BStr::new(&here.delimiter));
        /* The C reads the first character inside the argument list. The
         * receiver is passed there too, so the read is its own statement:
         * evaluation order is unchanged, the first character is still
         * read before `readtoken1` runs. */
        if !here.expand {
            let firstc = read_input_unit(shell)?;
            read_word_token(
                shell,
                firstc,
                SyntaxContext::SingleQuoted,
                mark,
                here.strip_tabs,
                false,
                SubscriptPosition::None,
            )?;
        } else {
            let firstc = read_unit_skipping_line_continuations(shell)?;
            read_word_token(
                shell,
                firstc,
                SyntaxContext::DoubleQuoted,
                mark,
                here.strip_tabs,
                false,
                SubscriptPosition::None,
            )?;
        }
        let mut word = mem::take(&mut shell.input.word);
        /* A body that ended at the delimiter carries the newline before it.
         * One that ended at end of input does not, and Bash still reads the
         * document as a line: `cat <<a` over `x` with no newline writes
         * `x\n`. Recording the line the reader saw is also the only way the
         * body can be printed back, since a printed document has to close
         * with a delimiter on its own line. */
        // [spec:nsh:req:idiom.printable-ast+2]
        if !word.is_empty() && !word.as_bstr().ends_with(b"\n") {
            word.push_literal_byte(b'\n');
        }
        /* The body and the delimiter line that ended it were read here,
         * far from the redirection that named them. */
        // [spec:nsh:def:idiom.token-stream]
        let body = WordNode {
            tokens: shell.input.tokens.cut_run(SourceTokenKind::HereDocument),
            word,
        };
        shell.input.completed_here_documents.push(body);
    }
    Ok(())
}
