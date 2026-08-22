//! Where `read` and `mapfile` get their bytes.
//!
//! Standard input already has a reader: the shell's own input stack,
//! which knows how to avoid consuming more of a script's stdin than the
//! command asked for, how to run the line editor, and how to be
//! interrupted. `read` has always used it and still does.
//!
//! `read -u N` has none of that. It names a descriptor the shell holds
//! but is not parsing from, so this reads it a byte at a time -- which
//! is not a performance compromise but the requirement: a shell must
//! leave everything after the record for whoever reads the descriptor
//! next, and one byte at a time is the only way to promise that without
//! seeking.
//!
//! Both spellings answer the same three questions -- next byte, put one
//! back, next record -- so nothing above this module has to know which
//! one it is holding.

use bstr::BString;

use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::syntax::InputUnit;

pub(crate) enum ReadStream {
    /// The shell's input stack, positioned on standard input.
    Standard,
    /// One logical descriptor, read a byte at a time.
    Descriptor {
        descriptor: LogicalDescriptor,
        /// Bytes taken from the descriptor and handed back by `unread`.
        pushback: Vec<u8>,
        /// A read has already reported end of input.
        exhausted: bool,
    },
}

impl ReadStream {
    /// Open the source `-u` selected, or standard input when it did not.
    pub(crate) fn open(shell: &mut Shell, descriptor: Option<u8>) -> Result<Self, Error> {
        let Some(number) = descriptor else {
            crate::input::push_standard_input(shell);
            return Ok(Self::Standard);
        };
        let Some(descriptor) = LogicalDescriptor::new(i32::from(number)) else {
            let mut message = b"read: ".to_vec();
            message.extend_from_slice(number.to_string().as_bytes());
            message.extend_from_slice(b": invalid file descriptor");
            return Err(shell.diagnostics().shell_error(&message));
        };
        if shell.descriptors.get(descriptor).is_none() {
            let mut message = b"read: ".to_vec();
            message.extend_from_slice(number.to_string().as_bytes());
            message.extend_from_slice(b": bad file descriptor");
            return Err(shell.diagnostics().shell_error(&message));
        }
        Ok(Self::Descriptor {
            descriptor,
            pushback: Vec::new(),
            exhausted: false,
        })
    }

    /// Give back the input frame `open` pushed, if it pushed one.
    pub(crate) fn close(self, shell: &mut Shell) {
        if matches!(self, Self::Standard) {
            crate::input::pop_input_frame(shell);
        }
    }

    /// Whether the source keeps the parser's NUL filtering, which only
    /// the input stack has.
    pub(crate) fn next_unit(
        &mut self,
        shell: &mut Shell,
        preserve_nul: bool,
    ) -> Result<InputUnit, Error> {
        match self {
            Self::Standard => {
                if preserve_nul {
                    crate::input::read_input_unit_preserving_nul(shell)
                } else {
                    crate::input::read_input_unit(shell)
                }
            }
            Self::Descriptor {
                descriptor,
                pushback,
                exhausted,
            } => {
                if let Some(byte) = pushback.pop() {
                    return Ok(InputUnit::Byte(byte));
                }
                if *exhausted {
                    return Ok(InputUnit::EndOfInput);
                }
                let Some(source) = shell.descriptors.get(*descriptor) else {
                    *exhausted = true;
                    return Ok(InputUnit::EndOfInput);
                };
                let mut byte = [0_u8; 1];
                loop {
                    return match nsh_platform::read_once(&source, &mut byte) {
                        Ok(0) => {
                            *exhausted = true;
                            Ok(InputUnit::EndOfInput)
                        }
                        Ok(_) => Ok(InputUnit::Byte(byte[0])),
                        Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {
                            if let Some(error) =
                                crate::error::poll_interrupt(shell.interrupt_context())
                            {
                                return Err(error);
                            }
                            continue;
                        }
                        Err(_) => {
                            *exhausted = true;
                            Ok(InputUnit::EndOfInput)
                        }
                    };
                }
            }
        }
    }

    /// Put back the bytes a speculative read consumed.
    pub(crate) fn unread(&mut self, shell: &mut Shell, bytes: &[u8]) {
        match self {
            Self::Standard => crate::input::unread_input_units(shell, bytes.len()),
            Self::Descriptor { pushback, .. } => pushback.extend(bytes.iter().rev()),
        }
    }

    /// One `delimiter`-terminated record, delimiter included, or `None`
    /// when the source held nothing further at all.
    ///
    /// A final record with no delimiter is still a record, which is what
    /// makes `mapfile` on a file with no trailing newline store its last
    /// line.
    pub(crate) fn record(
        &mut self,
        shell: &mut Shell,
        delimiter: u8,
    ) -> Result<Option<BString>, Error> {
        let mut record = BString::default();
        loop {
            match self.next_unit(shell, true)? {
                InputUnit::EndOfInput => {
                    return Ok((!record.is_empty()).then_some(record));
                }
                unit => {
                    let byte = unit.expect_byte();
                    record.push(byte);
                    if byte == delimiter {
                        return Ok(Some(record));
                    }
                }
            }
        }
    }
}
