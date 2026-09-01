//! Printing a redirection, including the here-document it defers.
//!
//! A here-document is the one construct whose text does not sit where its
//! operator does: the operator is written in the command and the body
//! after the line ends, so the printer carries a queue of pending bodies
//! and flushes it at every newline.

use super::*;

impl<'a> Printer<'a> {
    pub(super) fn redirections(&mut self, redirections: &[Redirection], indent: usize) {
        for redirection in redirections {
            self.out.push(b' ');
            match redirection {
                Redirection::File(file) => self.file_redirection(file, indent),
                Redirection::Descriptor(descriptor) => self.descriptor_redirection(descriptor),
                Redirection::HereDocument(document) => self.here_document(document, indent),
                Redirection::HereString(here) => self.here_string(here, indent),
            }
        }
    }

    /// `<<< word`, which unlike a here-document carries its whole body in
    /// the word and so needs no queueing to the end of the line.
    fn here_string(&mut self, redirection: &HereString, indent: usize) {
        self.descriptor_prefix(&redirection.descriptor, 0);
        self.out.extend_from_slice(b"<<< ");
        self.word(&redirection.word, indent);
    }

    fn file_redirection(&mut self, redirection: &FileRedirection, indent: usize) {
        let (operator, default): (&[u8], usize) = match redirection.operator {
            FileRedirectionOperator::Write => (b">", 1),
            FileRedirectionOperator::Clobber => (b">|", 1),
            FileRedirectionOperator::Read => (b"<", 0),
            FileRedirectionOperator::ReadWrite => (b"<>", 0),
            FileRedirectionOperator::Append => (b">>", 1),
        };
        self.descriptor_prefix(&redirection.descriptor, default);
        self.out.extend_from_slice(operator);
        self.out.push(b' ');
        self.word(&redirection.target, indent);
    }

    fn descriptor_redirection(&mut self, redirection: &DescriptorRedirection) {
        let (operator, default): (&[u8], usize) = match redirection.operator {
            DescriptorRedirectionOperator::Input => (b"<&", 0),
            DescriptorRedirectionOperator::Output => (b">&", 1),
        };
        self.descriptor_prefix(&redirection.descriptor, default);
        self.out.extend_from_slice(operator);
        match &redirection.target {
            DescriptorTarget::Number(number) => {
                self.out
                    .extend_from_slice(number.index().to_string().as_bytes());
            }
            DescriptorTarget::Close => self.out.push(b'-'),
            DescriptorTarget::Word(word) => self.word(word, 0),
        }
    }

    /// Write the descriptor number, unless the operator already implies it.
    /// The number, or `{name}`, before a redirection operator.
    ///
    /// A fixed slot the operator would have taken anyway is left unwritten,
    /// which is why the default comes in. `{name}` is never the default and
    /// is always written: it is the request to allocate.
    // [spec:nsh:req:compat.bash.parser-ast]
    fn descriptor_prefix(&mut self, descriptor: &RedirectionDescriptor, default: usize) {
        if descriptor
            .fixed()
            .is_some_and(|fixed| fixed.index() == default)
        {
            return;
        }
        self.out.extend_from_slice(&descriptor.text());
    }

    /// Write `<<DELIM` and queue the body for the end of the line.
    ///
    /// The tree keeps the body but not the delimiter the source spelled,
    /// so one is chosen that no line of the body can be mistaken for.
    fn here_document(&mut self, document: &HereDocument, indent: usize) {
        /* The body's run is the body and the delimiter line that ended
         * it, read together at the newline after this redirection. When
         * there is one it is the whole document, terminator included, and
         * the delimiter below is the one the source wrote. */
        // [spec:nsh:req:idiom.printable-ast+2]
        let read = document.body.tokens.text();
        if !self.ignore_runs && !read.is_empty() && !document.delimiter.as_bstr().is_empty() {
            self.descriptor_prefix(&document.descriptor, 0);
            self.out.extend_from_slice(b"<<");
            if document.expand {
                self.out.extend_from_slice(document.delimiter.as_bstr());
            } else {
                self.out.push(b'\'');
                self.out.extend_from_slice(document.delimiter.as_bstr());
                self.out.push(b'\'');
            }
            self.pending.push(read);
            return;
        }
        let mut body = Self::new(self.locale);
        body.ignore_runs = self.ignore_runs;
        body.spelled_body(&document.body.word, document.expand, indent);
        let mut body = body.out;
        if !body.is_empty() && body.last() != Some(&b'\n') {
            body.push(b'\n');
        }
        // The source's own delimiter, unless the body holds a line spelling
        // it -- which only happens when the input ended before the terminator
        // did, and then any delimiter is a guess.
        let spelled = document.delimiter.as_bstr();
        let delimiter = if spelled.is_empty()
            || body
                .split(|byte| *byte == b'\n')
                .any(|line| line == spelled)
        {
            unused_delimiter(&body)
        } else {
            BString::from(spelled)
        };

        self.descriptor_prefix(&document.descriptor, 0);
        self.out.extend_from_slice(b"<<");
        if document.expand {
            self.out.extend_from_slice(&delimiter);
        } else {
            self.out.push(b'\'');
            self.out.extend_from_slice(&delimiter);
            self.out.push(b'\'');
        }

        body.extend_from_slice(&delimiter);
        body.push(b'\n');
        self.pending.push(body);
    }
}
