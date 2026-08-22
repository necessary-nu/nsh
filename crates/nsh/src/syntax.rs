//! Typed byte classification for the shell lexer.
//!
//! Dash generated four 257-byte tables so C could sign-extend input bytes,
//! bias a pointer into the middle of each table, and reserve two additional
//! negative integers for end-of-input and end-of-alias. None of those
//! representation tricks are part of shell syntax. The lexer now carries
//! input boundaries and syntax context explicitly and classifies ordinary
//! bytes without offset arithmetic.

/// One item returned by the shell's byte input stream.
// [spec:nsh:req:idiom.lexer-tokens]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum InputUnit {
    /// An ordinary shell input byte.
    Byte(u8),
    /// The current input source has no more bytes.
    EndOfInput,
    /// The current alias expansion ended at this position.
    EndOfAlias,
}

impl InputUnit {
    /// Whether this item is the specified byte.
    #[inline]
    pub(crate) const fn is(self, byte: u8) -> bool {
        matches!(self, Self::Byte(value) if value == byte)
    }

    /// The ordinary byte, if this item carries one.
    #[inline]
    pub(crate) const fn byte(self) -> Option<u8> {
        match self {
            Self::Byte(byte) => Some(byte),
            Self::EndOfInput | Self::EndOfAlias => None,
        }
    }

    /// Return the carried byte when control flow has already excluded a
    /// boundary item.
    #[inline]
    pub(crate) fn expect_byte(self) -> u8 {
        self.byte().expect("input boundary has no byte")
    }

    #[inline]
    pub(crate) fn begins_name(self, locale: &nsh_platform::Locale) -> bool {
        self.byte().is_some_and(|byte| is_name(locale, byte))
    }

    #[inline]
    pub(crate) fn continues_name(self, locale: &nsh_platform::Locale) -> bool {
        self.byte().is_some_and(|byte| is_in_name(locale, byte))
    }

    #[inline]
    pub(crate) fn is_digit(self) -> bool {
        self.byte().is_some_and(|byte| byte.is_ascii_digit())
    }

    #[inline]
    pub(crate) fn is_special_parameter(self) -> bool {
        self.byte().is_some_and(is_special)
    }
}

/// The lexical role of one input item in a syntax context.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxClass {
    Word,
    Newline,
    Backslash,
    SingleQuote,
    DoubleQuote,
    EndQuote,
    Backquote,
    Variable,
    EndVariable,
    LeftParen,
    RightParen,
    EndOfInput,
    EndOfAlias,
    Control,
    WordSeparator,
}

/// The quoting context used to classify an input item.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SyntaxContext {
    Base,
    DoubleQuoted,
    SingleQuoted,
    Arithmetic,
    /// The operand of Bash's `=~`, where regular-expression syntax wins.
    ///
    /// A regular expression and the shell disagree about `(`, `|` and `)`,
    /// and Bash resolves it by lexing this one operand differently rather
    /// than by making scripts quote every group. Blanks and the operators
    /// the shell still needs end the word only outside parentheses.
    // [spec:nsh:req:compat.bash.conditionals-arithmetic]
    Regex,
}

impl SyntaxContext {
    /// Classify one input item under this quoting context.
    #[inline]
    pub(crate) const fn classify(self, input: InputUnit) -> SyntaxClass {
        let InputUnit::Byte(byte) = input else {
            return match input {
                InputUnit::EndOfInput => SyntaxClass::EndOfInput,
                InputUnit::EndOfAlias => SyntaxClass::EndOfAlias,
                InputUnit::Byte(_) => unreachable!(),
            };
        };

        match self {
            Self::Base => match byte {
                b'\n' => SyntaxClass::Newline,
                b'\\' => SyntaxClass::Backslash,
                b'\'' => SyntaxClass::SingleQuote,
                b'"' => SyntaxClass::DoubleQuote,
                b'`' => SyntaxClass::Backquote,
                b'$' => SyntaxClass::Variable,
                b'}' => SyntaxClass::EndVariable,
                b' ' | b'\t' | b'&' | b';' | b'<' | b'>' | b'|' | b'(' | b')' => {
                    SyntaxClass::WordSeparator
                }
                _ => SyntaxClass::Word,
            },
            Self::DoubleQuoted => match byte {
                b'\n' => SyntaxClass::Newline,
                b'\\' => SyntaxClass::Backslash,
                b'"' => SyntaxClass::EndQuote,
                b'`' => SyntaxClass::Backquote,
                b'$' => SyntaxClass::Variable,
                b'}' => SyntaxClass::EndVariable,
                b'!' | b'*' | b'-' | b'/' | b':' | b'=' | b'?' | b'[' | b']' | b'^' | b'~' => {
                    SyntaxClass::Control
                }
                _ => SyntaxClass::Word,
            },
            Self::SingleQuoted => match byte {
                b'\n' => SyntaxClass::Newline,
                b'\'' => SyntaxClass::EndQuote,
                b'!' | b'*' | b'-' | b'/' | b':' | b'=' | b'?' | b'[' | b'\\' | b']' | b'^'
                | b'~' => SyntaxClass::Control,
                _ => SyntaxClass::Word,
            },
            Self::Regex => match byte {
                b'\n' => SyntaxClass::Newline,
                b'\\' => SyntaxClass::Backslash,
                b'\'' => SyntaxClass::SingleQuote,
                b'"' => SyntaxClass::DoubleQuote,
                b'`' => SyntaxClass::Backquote,
                b'$' => SyntaxClass::Variable,
                b'}' => SyntaxClass::EndVariable,
                b'(' => SyntaxClass::LeftParen,
                b')' => SyntaxClass::RightParen,
                b' ' | b'\t' | b'&' | b';' | b'<' | b'>' => SyntaxClass::WordSeparator,
                _ => SyntaxClass::Word,
            },
            Self::Arithmetic => match byte {
                b'\n' => SyntaxClass::Newline,
                b'\\' => SyntaxClass::Backslash,
                b'`' => SyntaxClass::Backquote,
                b'$' => SyntaxClass::Variable,
                b'}' => SyntaxClass::EndVariable,
                b'(' => SyntaxClass::LeftParen,
                b')' => SyntaxClass::RightParen,
                _ => SyntaxClass::Word,
            },
        }
    }
}

/// Whether a byte can begin a shell name.
#[inline]
pub fn is_name(locale: &nsh_platform::Locale, byte: u8) -> bool {
    byte == b'_' || locale.is_alpha(byte)
}

/// Whether a byte can continue a shell name.
#[inline]
pub fn is_in_name(locale: &nsh_platform::Locale, byte: u8) -> bool {
    byte == b'_' || locale.is_alphanumeric(byte)
}

/// Whether a byte names a positional or special parameter.
#[inline]
pub fn is_special(byte: u8) -> bool {
    matches!(
        byte,
        b'0'..=b'9' | b'!' | b'$' | b'-' | b'?' | b'@' | b'#' | b'*'
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    // [spec:nsh:req:idiom.lexer-tokens/test]
    fn input_boundaries_are_not_bytes() {
        for context in [
            SyntaxContext::Base,
            SyntaxContext::DoubleQuoted,
            SyntaxContext::SingleQuoted,
            SyntaxContext::Arithmetic,
            SyntaxContext::Regex,
        ] {
            assert_eq!(
                context.classify(InputUnit::EndOfInput),
                SyntaxClass::EndOfInput
            );
            assert_eq!(
                context.classify(InputUnit::EndOfAlias),
                SyntaxClass::EndOfAlias
            );
        }
    }

    #[test]
    fn quoting_context_changes_classification() {
        assert_eq!(
            SyntaxContext::Base.classify(InputUnit::Byte(b' ')),
            SyntaxClass::WordSeparator
        );
        assert_eq!(
            SyntaxContext::DoubleQuoted.classify(InputUnit::Byte(b' ')),
            SyntaxClass::Word
        );
        assert_eq!(
            SyntaxContext::SingleQuoted.classify(InputUnit::Byte(b'\'')),
            SyntaxClass::EndQuote
        );
        assert_eq!(
            SyntaxContext::Arithmetic.classify(InputUnit::Byte(b'(')),
            SyntaxClass::LeftParen
        );
    }

    // [spec:nsh:req:compat.bash.conditionals-arithmetic/test]
    #[test]
    fn the_regex_context_keeps_group_syntax() {
        for (byte, class) in [
            (b'(', SyntaxClass::LeftParen),
            (b')', SyntaxClass::RightParen),
            (b'|', SyntaxClass::Word),
            (b'{', SyntaxClass::Word),
            (b';', SyntaxClass::WordSeparator),
            (b' ', SyntaxClass::WordSeparator),
        ] {
            assert_eq!(
                SyntaxContext::Regex.classify(InputUnit::Byte(byte)),
                class,
                "byte {byte:?}"
            );
        }
    }

    #[test]
    fn non_ascii_bytes_remain_word_data() {
        for byte in 0x80..=u8::MAX {
            assert_eq!(
                SyntaxContext::Base.classify(InputUnit::Byte(byte)),
                SyntaxClass::Word
            );
        }
    }
}
