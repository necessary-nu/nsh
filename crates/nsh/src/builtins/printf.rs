//! The `printf` builtin.
//!
//! Port of the `printf` half of `src/bltin/printf.c`.
//! Rules: `docs/spec/port/src/bltin/printf.md`.
//!
//! The utility's contract is to read a format string *at runtime* and
//! render each argument by a pattern found in it, so parsing `%`
//! conversions here is inherent to `printf` and not a habit leaking in
//! from the C. What the port does not do is what the C did with the
//! result: `printf.c` found only the *end* of each specification and
//! handed the text straight back to C's `printf` as a format string,
//! which needed `mklong` to widen the conversion to `PRIdMAX` and the
//! `PF`/`ASPF` macros to pick between three varargs arities. Here the
//! specification is parsed into a [`Spec`] -- flags, width and precision
//! as fields -- and [`conv`] renders it against a typed argument through
//! Rust's own formatting and an `io::Write`. See
//! `[dec:nsh:printf-is-parsed-not-interpreted]`.
//!
//! `%b`'s escape dialect and the `\` escapes in the format string are
//! [`crate::escape`], shared with `echo` and the parser.

use crate::context::Shell;
use crate::error::Error;

use bstr::{BStr, BString, ByteSlice as _};

use crate::escape::{append_escape, parse_escape};
use crate::evaluation::Flow;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

mod conv;
mod scan;

use conv::{LIMIT, Spec};
use scan::{leading_number, scan_double, scan_integer};

/// What the C skipped with `strspn(fmt, SKIP2)` when the width or
/// precision was written out rather than taken from an argument.
///
/// The C's companion `SKIP1`, the flag characters, has no constant here:
/// [`Spec::flag`] recognises them one at a time and records what each
/// means, so a second spelling of the same set could only disagree with
/// it.
const WIDTH: &[u8] = b"*0123456789";

/// Where the rendered conversions go.
///
/// Bash's `-v NAME` does not change how anything is rendered, only where
/// the rendering lands, so it is one value threaded through the writers
/// rather than a second copy of the format loop. A `printf` without
/// `-v` writes each conversion straight out, exactly as the C did.
enum Destination {
    Standard,
    /// `-v NAME`: the variable to assign, and what has been rendered so
    /// far. It has to be collected because one format string can be
    /// reused over several arguments and the variable takes all of it.
    Variable {
        name: BString,
        rendered: Vec<u8>,
    },
}

impl Destination {
    /// Write one rendered conversion to wherever this invocation sends
    /// its output.
    fn emit(&mut self, shell: &mut crate::context::Shell, bytes: &[u8]) -> Result<(), Error> {
        match self {
            Self::Standard => shell.write_output(OutputDestination::Stdout, bytes),
            Self::Variable { rendered, .. } => {
                rendered.extend_from_slice(bytes);
                Ok(())
            }
        }
    }

    /// Write one rendered conversion, or raise what the C raised when it
    /// could not render it.
    ///
    /// `None` is a field longer than `vsnprintf` counts in an `int`. The C
    /// asked glibc to lay every conversion out and `xvasprintf` treated the
    /// refusal as fatal, so the builtin stops there: whatever the format had
    /// already printed stays printed, and the shell's status is 2.
    fn emit_field(
        &mut self,
        shell: &mut crate::context::Shell,
        rendered: Option<Vec<u8>>,
    ) -> Result<(), Error> {
        match rendered {
            Some(bytes) => self.emit(shell, &bytes),
            None => Err(shell.diagnostics().shell_error(b"xvsnprintf failed")),
        }
    }

    /// Land a `-v NAME` result, once the format loop has finished.
    fn finish(self, shell: &mut crate::context::Shell) -> Result<(), Error> {
        let Self::Variable { name, rendered } = self else {
            return Ok(());
        };
        /* `-v` names a variable the way an assignment does, so a
         * subscript in it selects an element rather than being part of
         * the name: `printf -v 'a[k]'` writes that element. */
        crate::variables::arrays::assign_text_target(
            shell,
            BStr::new(name.as_slice()),
            BStr::new(rendered.as_slice()),
            false,
        )
    }
}

/// The operands a conversion reads its value from.
///
/// The C walked a `char **` cursor named `gargv`, and each `get*` helper
/// advanced it and returned a benign default once it ran off the end --
/// which is how one format string renders however many arguments it was
/// given. The cursor is an index here and the defaults are the same.
struct Operands<'a> {
    words: &'a [&'a BStr],
    next: usize,
    /// The C's `rval`: 1 once a numeric argument was malformed, and the
    /// builtin's exit status. Output still proceeds.
    status: ExitStatus,
}

impl<'a> Operands<'a> {
    fn new(words: &'a [&'a BStr]) -> Self {
        Self {
            words,
            next: 0,
            status: ExitStatus::SUCCESS,
        }
    }

    /// The C's `gargv != argv && *gargv`: the format is scanned again
    /// only while arguments remain *and* a pass consumed at least one,
    /// which is what keeps a format that reads nothing from looping for
    /// ever.
    fn reuse_format(&self) -> bool {
        self.next != 0 && self.next < self.words.len()
    }

    fn next_word(&mut self) -> Option<&'a BStr> {
        let word = *self.words.get(self.next)?;
        self.next += 1;
        Some(word)
    }

    /// One argument's first byte, or 0 once they are exhausted.
    // [spec:dash:sem:printf.getchr-fn]
    fn next_character(&mut self) -> u8 {
        self.next_word()
            .and_then(|word| word.first().copied())
            .unwrap_or(0)
    }

    /// One argument, or the empty string once they are exhausted.
    // [spec:dash:sem:printf.getstr-fn]
    fn next_string(&mut self) -> &'a [u8] {
        self.next_word().map_or(&[][..], |word| word)
    }

    /// One argument as an integer, or 0 once they are exhausted.
    ///
    /// `signed` picks between the C's `strtoimax` and `strtoumax`, which
    /// differ in where they saturate; both read base 0.
    // [spec:dash:sem:printf.getuintmax-fn]
    fn next_unsigned(&mut self, shell: &mut crate::context::Shell, signed: bool) -> u64 {
        let Some(word) = self.next_word() else {
            return 0;
        };
        let bytes = word;

        /* The POSIX rule that lets `printf %d "'A"` print 65: an
         * argument that opens with a quote is the character after it,
         * and nothing else is looked at. */
        if let Some(b'"' | b'\'') = bytes.first() {
            return u64::from(bytes.get(1).copied().unwrap_or(0));
        }

        let (value, end, range) = scan_integer(&shell.locale, bytes, signed);
        self.check_conversion(shell, bytes, end, range);
        value
    }

    /// One argument as a floating-point value, or 0 once they are
    /// exhausted.
    // [spec:dash:sem:printf.getdouble-fn]
    fn next_float(&mut self, shell: &mut crate::context::Shell) -> f64 {
        let Some(word) = self.next_word() else {
            return 0.0;
        };
        let bytes = word;

        if let Some(b'"' | b'\'') = bytes.first() {
            return f64::from(bytes.get(1).copied().unwrap_or(0));
        }

        let (value, end, range) = scan_double(&shell.locale, bytes);
        self.check_conversion(shell, bytes, end, range);
        value
    }

    /// Report a malformed numeric argument.
    ///
    /// `end` is where the conversion stopped and `range` whether the
    /// value ran past what the type holds. Either sets the exit status to
    /// 1 while the builtin goes on printing the value it did derive --
    /// text left over is the louder complaint of the two, so it is
    /// checked first, exactly as the C checked `*ep` before `errno`.
    // [spec:dash:sem:printf.check-conversion-fn]
    fn check_conversion(
        &mut self,
        shell: &mut crate::context::Shell,
        word: &[u8],
        end: usize,
        range: bool,
    ) {
        let mut message = word.to_vec();
        if end < word.len() {
            message.extend_from_slice(if end == 0 {
                &b": expected numeric value"[..]
            } else {
                &b": not completely converted"[..]
            });
        } else if range {
            message.extend_from_slice(b": ");
            /* The C's `strerror(ERANGE)`, so the wording is the
             * platform's rather than this file's. */
            message.extend_from_slice(shell.locale.range_error_message().as_bytes());
        } else {
            return;
        }

        shell.diagnostics().shell_warning(&message);
        self.status = ExitStatus::FAILURE;
    }
}

/// The tail of a specification, as the C handed it to `printf`.
///
/// `mklong` had rewritten the integer conversions to `PRIdMAX` before
/// the text was passed, so the length modifier it inserted is part of
/// what a specification glibc could not read prints -- one `l` on this
/// target, where `i64` is a `long`. `%b` was passed with its
/// conversion character set to `s`, which is how the C reached `printf`
/// with a conversion character C has.
fn passed_tail(tail: &[u8], conversion: u8) -> Vec<u8> {
    let mut text = tail.to_vec();
    if matches!(conversion, b'd' | b'i' | b'o' | b'u' | b'x' | b'X') {
        text.push(b'l');
    }
    text.push(if conversion == b'b' { b's' } else { conversion });
    text
}

/// Where a written-out width or precision holds a `*`, which is where
/// C's `printf` stops reading the specification.
///
/// The C's own scan ran `strspn` over `*` along with the digits and
/// never looked further, so the digits it collected may straddle a `*`
/// that C's `printf` would never have got past. Only the ones in front
/// of it were ever read as a number.
fn unreadable_at(field: &[u8]) -> Option<usize> {
    field.iter().position(|&byte| byte == b'*')
}

/// How many leading bytes of `bytes` from `at` are in `set`.
fn span(bytes: &[u8], at: usize, set: &[u8]) -> usize {
    bytes[at..]
        .iter()
        .position(|byte| !set.contains(byte))
        .unwrap_or(bytes.len() - at)
}

/// `%b`: the argument with its escapes expanded, laid out in the field.
///
/// Returns non-zero when a `\c` stopped the conversion, which stops the
/// whole builtin.
///
/// The C could not lay this out itself. A `%b` result may contain NUL,
/// so it could not be handed to a C string function at all: the C
/// formatted a run of `X`s of the same length, let `printf` compute the
/// padding around them, then copied the real bytes back over the run.
/// Laying out bytes needs no stand-in.
// [spec:dash:sem:printf.print-escape-str-fn]
fn write_escaped_text(
    shell: &mut crate::context::Shell,
    destination: &mut Destination,
    spec: &Spec,
    word: &BStr,
) -> Result<bool, Error> {
    let mut buffer = BString::default();
    let done = append_escape(&shell.locale, word, &mut buffer);
    destination.emit_field(shell, spec.string(&buffer))?;
    Ok(done)
}

// [spec:dash:sem:printf.printfcmd-fn]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    /* `nextopt(nullstr)`: POSIX printf takes no options, so this exists
     * to reject `-x` and to step over a `--`. Bash's `-v` is the one
     * addition, and only in Bash mode. */
    // [spec:nsh:req:compat.bash.builtins-special-variables]
    let bash = shell.options.dialect() == crate::options::Dialect::Bash;
    let mut options = crate::options::Options::new(args);
    let mut destination = Destination::Standard;
    while let Some(option) =
        options.next(&mut shell.diagnostics(), if bash { b"v:" } else { b"" })?
    {
        if option == b'v' {
            destination = Destination::Variable {
                name: options.arg().to_owned(),
                rendered: Vec::new(),
            };
        }
    }

    let Some((format, arguments)) = options.operands().split_first() else {
        return Err(shell
            .diagnostics()
            .shell_error(b"usage: printf format [arg ...]"));
    };

    let format = format.as_bytes();
    let end = format.len();
    let mut operands = Operands::new(arguments);

    'format_arguments: loop {
        /*
         * Basic algorithm is to scan the format string for conversion
         * specifications -- once one is found, find out if the field
         * width or precision is a '*'; if it is, gather up value.
         * Note, format strings are reused as necessary to use up the
         * provided arguments, arguments of zero/null string are
         * provided to use up the format string.
         */
        let mut at = 0usize;
        while at < end {
            let character = format[at];
            at += 1;

            if character == b'\\' {
                let converted = parse_escape(&shell.locale, &format[at..], false);
                at += converted.consumed;
                destination.emit(shell, converted.bytes())?;
                continue;
            }
            /* A `%%` is one `%`; a `%` at the very end of the format
             * falls through and is the missing-conversion error. */
            if character != b'%' || format.get(at) == Some(&b'%') {
                if character == b'%' {
                    at += 1;
                }
                destination.emit(shell, &[character])?;
                continue;
            }

            /* Ok - we've found a format specification. The C saved its
             * address to hand back to `printf`; the port collects it. */
            let start = at - 1;
            let mut spec = Spec::bare();
            /* Where C's `printf` stopped reading. The C's scan carries on
             * past it either way -- it is the C's scan that says where
             * the specification ends and which operands it takes. */
            let mut stop = None;

            /* skip to field width */
            while at < end && spec.flag(format[at]) {
                at += 1;
            }
            if format.get(at) == Some(&b'*') {
                at += 1;
                spec.set_width(operands.next_unsigned(shell, true) as i32);
            } else {
                /* skip to possible '.', get following precision */
                let digits = span(&format[..end], at, WIDTH);
                let field = &format[at..at + digits];
                let read = unreadable_at(field);
                if let Some(k) = read {
                    stop = Some(at + k);
                }
                spec.set_written_width(leading_number(&field[..read.unwrap_or(digits)]));
                at += digits;
            }

            if format.get(at) == Some(&b'.') {
                at += 1;
                if format.get(at) == Some(&b'*') {
                    at += 1;
                    let value = operands.next_unsigned(shell, true) as i32;
                    if stop.is_none() {
                        spec.set_precision(value);
                    }
                } else {
                    let digits = span(&format[..end], at, WIDTH);
                    let field = &format[at..at + digits];
                    let read = unreadable_at(field);
                    if stop.is_none() {
                        if let Some(k) = read {
                            stop = Some(at + k);
                        }
                        spec.set_written_precision(leading_number(
                            &field[..read.unwrap_or(digits)],
                        ));
                    }
                    at += digits;
                }
            }

            let conversion = format.get(at).copied().unwrap_or(0);
            if conversion == 0 {
                return Err(shell.diagnostics().shell_error(b"missing format character"));
            }
            at += 1;
            if let Some(stop) = stop {
                let tail = passed_tail(&format[stop + 1..at - 1], conversion);
                spec.set_unreadable(format[stop], &tail);
            }

            match conversion {
                b'b' => {
                    /* escape if a \c was encountered */
                    if write_escaped_text(
                        shell,
                        &mut destination,
                        &spec,
                        BStr::new(operands.next_string()),
                    )? {
                        break 'format_arguments;
                    }
                }
                b'c' => {
                    let value = operands.next_character();
                    destination.emit_field(shell, spec.character(value))?;
                }
                /* `%q` renders its argument as the shell would have to
                 * write it to mean the same bytes, which is quoting and
                 * not formatting -- so the field layout applies to the
                 * quoted text, as it does for `%s`. */
                b'q' if bash => {
                    let value = operands.next_string();
                    let quoted = crate::escape::bash::requote(&shell.locale, BStr::new(value));
                    destination.emit_field(shell, spec.string(&quoted))?;
                }
                b's' => {
                    let value = operands.next_string();
                    destination.emit_field(shell, spec.string(value))?;
                }
                /* `mklong` widened the specification to `PRIdMAX` so
                 * that C's printf would pull a whole `i64` off the
                 * varargs. The value arrives typed. */
                b'd' | b'i' => {
                    let value = operands.next_unsigned(shell, true);
                    destination.emit_field(shell, spec.signed(value as i64))?;
                }
                b'o' | b'u' | b'x' | b'X' => {
                    let value = operands.next_unsigned(shell, false);
                    destination.emit_field(shell, spec.unsigned(value, conversion))?;
                }
                b'a' | b'A' | b'e' | b'E' | b'f' | b'F' | b'g' | b'G' => {
                    let value = operands.next_float(shell);
                    destination.emit_field(shell, spec.double(value, conversion))?;
                }
                _ => {
                    let mut message = format[start..at].to_vec();
                    message.extend_from_slice(b": invalid directive");
                    return Err(shell.diagnostics().shell_error(&message));
                }
            }
        }

        if !operands.reuse_format() {
            break;
        }
    }

    // out:
    destination.finish(shell)?;
    Ok(Flow::Done(operands.status))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The C's `get*` helpers each read one argument and hand back a
    /// benign default once the list runs out.
    #[test]
    fn exhausted_operands_yield_defaults() {
        let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned_sh;
        let words: Vec<&BStr> = vec![BStr::new("ab")];
        let mut operands = Operands::new(&words);
        assert_eq!(operands.next_character(), b'a');
        assert_eq!(operands.next_character(), 0);
        assert_eq!(operands.next_string(), b"");
        assert_eq!(operands.next_unsigned(shell, true), 0);
        assert_eq!(operands.next_float(shell), 0.0);
        assert_eq!(operands.status, ExitStatus::SUCCESS);
    }

    /// The format is scanned again only while arguments remain and a
    /// pass consumed one.
    #[test]
    fn a_format_repeats_while_words_remain() {
        let words: Vec<&BStr> = vec![BStr::new("a"), BStr::new("b")];
        let mut operands = Operands::new(&words);
        assert!(!operands.reuse_format());
        operands.next_string();
        assert!(operands.reuse_format());
        operands.next_string();
        assert!(!operands.reuse_format());
    }

    /// An argument that opens with a quote is the character after it,
    /// whichever quote it is, and a lone quote is nothing.
    #[test]
    fn a_quote_argument_is_one_byte() {
        let mut owned_sh = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned_sh;
        let words: Vec<&BStr> = vec![BStr::new("'A"), BStr::new("\"z"), BStr::new("'")];
        let mut operands = Operands::new(&words);
        assert_eq!(operands.next_unsigned(shell, true), 65);
        assert_eq!(operands.next_float(shell), 122.0);
        assert_eq!(operands.next_unsigned(shell, false), 0);
        assert_eq!(operands.status, ExitStatus::SUCCESS);
    }

    /// The digits of a written-out width are what the field asks for.
    #[test]
    fn a_written_width_is_its_digits() {
        assert_eq!(leading_number(b""), 0);
        assert_eq!(leading_number(b"12"), 12);
        assert_eq!(span(b"12.3d", 0, WIDTH), 2);
        assert_eq!(span(b"*", 0, WIDTH), 1);
        assert_eq!(span(b"d", 0, WIDTH), 0);
        /* Digits past what the C held saturate one place beyond the
         * limit, which is all anything asks of them. */
        assert_eq!(leading_number(b"2147483647"), LIMIT);
        assert_eq!(leading_number(b"2147483648"), LIMIT + 1);
        assert_eq!(leading_number(b"99999999999999999999"), LIMIT + 1);
    }

    /// The C widened the integer conversions before handing the text
    /// over, and passed `%b` as `%s`.
    #[test]
    fn a_passed_tail_carries_c_rewrites() {
        assert_eq!(passed_tail(b"", b'd'), b"ld");
        assert_eq!(passed_tail(b"2.5", b'i'), b"2.5li");
        assert_eq!(passed_tail(b".5", b'X'), b".5lX");
        assert_eq!(passed_tail(b"", b's'), b"s");
        assert_eq!(passed_tail(b"3", b'f'), b"3f");
        assert_eq!(passed_tail(b"", b'b'), b"s");
    }

    /// Only the digits in front of a `*` were ever read as a number.
    #[test]
    fn a_star_stops_the_digits() {
        assert_eq!(unreadable_at(b"5*"), Some(1));
        assert_eq!(unreadable_at(b"1*2"), Some(1));
        assert_eq!(unreadable_at(b"12"), None);
        assert_eq!(unreadable_at(b""), None);
    }
}
