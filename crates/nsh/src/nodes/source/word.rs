//! Printing a word: the run it was read from, and the parts inside it.
//!
//! A parsed word carries its own tokens, so almost nothing here decides
//! how to spell one -- the bytes that were read are the bytes that go
//! back. What is left is the cases where a node was built rather than
//! read, and the expansions whose insides are themselves a tree.

use super::*;

impl<'a> Printer<'a> {
    pub(super) fn word(&mut self, word: &WordNode, indent: usize) {
        let run = word.tokens.written();
        if self.ignore_runs || run.is_empty() {
            self.spelled(&word.word, indent);
        } else {
            self.out.extend_from_slice(&run);
        }
    }

    /// Spell a word the shell built rather than read.
    ///
    /// The obligation is only that what it writes parses back to a
    /// structurally equal word, so it picks one rule and applies it
    /// everywhere: an inert run goes inside single quotes, an ordinary
    /// run goes as it is, and an expansion keeps the quoting it was
    /// under. Choosing per byte is what the deleted grammar was for, and
    /// choosing at all is only sound because the bytes here were never
    /// written by anyone.
    // [spec:nsh:req:idiom.printable-ast+2]
    fn spelled(&mut self, word: &ParsedWord, indent: usize) {
        if word.parts().is_empty() {
            self.out.extend_from_slice(b"''");
            return;
        }
        for part in word.parts() {
            if let WordPart::Text { bytes, quoted } = part {
                if *quoted {
                    push_single_quoted(&mut self.out, bytes);
                } else {
                    self.out.extend_from_slice(bytes);
                }
                continue;
            }
            if part.quoted() {
                self.out.push(b'"');
            }
            match part {
                WordPart::Parameter(parameter) => self.spelled_parameter(parameter, indent),
                WordPart::Command { command, .. } => {
                    self.command_substitution(command.as_deref(), indent);
                }
                WordPart::Arithmetic { expression, .. } => {
                    self.out.extend_from_slice(b"$((");
                    self.spelled(expression, indent);
                    self.out.extend_from_slice(b"))");
                }
                WordPart::Text { .. } => unreachable!("a text run was written above"),
            }
            if part.quoted() {
                self.out.push(b'"');
            }
        }
    }

    /// Spell a word as the body of a here-document.
    ///
    /// A body is not a shell word and cannot be spelled as one. Nothing
    /// quotes there: a `'` is a `'`, so the single quotes [`spelled`] puts
    /// around an inert run would be two more bytes of body, and the `"` it
    /// puts around an expansion likewise. What makes a run inert here is
    /// the delimiter, and that is already decided by `expand`.
    ///
    /// So the two cases are spelled by what the delimiter says. A body
    /// that does not expand is written exactly as it is, because its
    /// delimiter is quoted and every byte is already data. A body that
    /// does expand writes its expansions bare and backslash-escapes the
    /// three bytes that would otherwise start one.
    // [spec:nsh:req:idiom.canonical-tree+1]
    pub(super) fn spelled_body(&mut self, word: &ParsedWord, expand: bool, indent: usize) {
        for part in word.parts() {
            match part {
                WordPart::Text { bytes, .. } => {
                    if expand {
                        for byte in bytes.iter() {
                            if matches!(byte, b'\\' | b'$' | b'`') {
                                self.out.push(b'\\');
                            }
                            self.out.push(*byte);
                        }
                    } else {
                        self.out.extend_from_slice(bytes);
                    }
                }
                WordPart::Parameter(parameter) => self.spelled_parameter(parameter, indent),
                WordPart::Command { command, .. } => {
                    self.command_substitution(command.as_deref(), indent);
                }
                WordPart::Arithmetic { expression, .. } => {
                    self.out.extend_from_slice(b"$((");
                    self.spelled(expression, indent);
                    self.out.extend_from_slice(b"))");
                }
            }
        }
    }

    /// Spell an expansion from its fields, for a word nothing read.
    ///
    /// An expansion the shell refused never reaches here: the parser is
    /// the only thing that builds one, and a parsed word is written as
    /// its run.
    // [spec:nsh:req:idiom.printable-ast+2]
    fn spelled_parameter(&mut self, parameter: &ParameterExpansion, indent: usize) {
        self.out.extend_from_slice(b"${");
        if parameter.operation == ParameterOperation::Length {
            self.out.push(b'#');
        }
        if parameter.indirect {
            self.out.push(b'!');
        }
        self.out.extend_from_slice(&parameter.name);
        if parameter.colon {
            self.out.push(b':');
        }
        self.out.extend_from_slice(parameter.operation.operator());
        /* An operand that is there and empty is spelled by the operator
         * alone: `${a-}` is the empty operand, and the `''` that
         * [`spelled`] writes for a word with no parts would be two bytes
         * of operand rather than none. */
        // [spec:nsh:req:idiom.canonical-tree+1]
        if let Some(operand) = &parameter.operand {
            if !operand.parts().is_empty() {
                self.spelled(operand, indent);
            }
        }
        self.out.push(b'}');
    }

    fn command_substitution(&mut self, node: Option<&Node>, indent: usize) {
        if let Some(Node::Bash(BashNode::ProcessSubstitution(substitution))) = node {
            self.process_substitution(substitution, indent);
            return;
        }
        let outer_pending = core::mem::take(&mut self.pending);
        let start = self.out.len();
        self.out.extend_from_slice(b"$(");
        if let Some(node) = node {
            self.list(node, indent);
        }
        // `$((` would read as an arithmetic expansion, so a leading
        // subshell needs a blank between the two parentheses.
        if self.out.get(start + 2) == Some(&b'(') {
            self.out.insert(start + 2, b' ');
        }
        if !self.pending.is_empty() {
            self.newline(indent);
        }
        self.out.push(b')');
        self.pending = outer_pending;
    }
}
