//! `read` in the Bash dialect.
//!
//! POSIX's `read` is a small command with a tight specification, and the
//! module above this one is a faithful port of it. Bash's is a different
//! command wearing the same name: it can read into an array, count
//! characters rather than fields, take its bytes from a descriptor the
//! shell is not parsing, and fall back on `REPLY` when no operand names
//! a variable. Layering all of that onto the POSIX loop would have made
//! one function whose behaviour depended on the dialect at every step,
//! so the dialect chooses between two readers once, at the top.
//!
//! What the two share is [`super::stream`], the byte source, and
//! `crate::expand::split_fields`, the field splitting -- so `IFS` means
//! the same thing in both.
//!
//! Two options are accepted and do less than Bash's:
//!
//! * `-e` and `-i` ask for line editing, which is only observable on a
//!   terminal; a non-interactive read behaves identically with or
//!   without them, which is every case a script can test.
//! * `-t` waits for the descriptor to become readable and then reads
//!   without a further bound, so a delimiter that never arrives on a
//!   ready descriptor still blocks. It is honest about waiting rather
//!   than spinning, which is what `[dec:nsh:safety-trumps-compatibility]`
//!   asks of a bounded read.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::descriptors::LogicalDescriptor;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::expand::ExpandedFields;
use crate::output::OutputDestination;
use crate::status::ExitStatus;
use crate::syntax::InputUnit;
use crate::variables::arrays;
use crate::variables::value::{VariableKind, VariableValue};
use crate::variables::{VariableAttributes, set_bytes};

use super::stream::ReadStream;

/// The name a `read` with no operand fills.
const REPLY: &[u8] = b"REPLY";

/// How many characters a record may hold before the read gives up.
///
/// A delimiter that never arrives would otherwise let one `read` grow
/// without bound on a descriptor that keeps producing bytes, which is
/// the unbounded-resource case `[dec:nsh:safety-trumps-compatibility]`
/// names. Sixteen mebibytes is far past any record a script means to
/// read and far short of anything that troubles a host.
const RECORD_LIMIT: usize = 16 * 1024 * 1024;

/// What one invocation asked for.
struct Requested {
    array: Option<BString>,
    delimiter: u8,
    prompt: Option<BString>,
    raw: bool,
    silent: bool,
    /// `-n`: stop after this many characters, or at the delimiter.
    limit: Option<usize>,
    /// `-N`: stop after exactly this many characters, delimiter or not.
    exact: Option<usize>,
    timeout: Option<f64>,
    descriptor: Option<u8>,
}

impl Requested {
    const fn new() -> Self {
        Self {
            array: None,
            delimiter: b'\n',
            prompt: None,
            raw: false,
            silent: false,
            limit: None,
            exact: None,
            timeout: None,
            descriptor: None,
        }
    }

    /// How many characters the read stops after, if it stops at all.
    const fn character_limit(&self) -> Option<usize> {
        match (self.exact, self.limit) {
            (Some(exact), _) => Some(exact),
            (None, limit) => limit,
        }
    }
}

/// The bytes one record held, and which of them a backslash protected
/// from field splitting.
struct Record {
    bytes: BString,
    protected: Vec<bool>,
    characters: usize,
}

impl Record {
    fn new() -> Self {
        Self {
            bytes: BString::default(),
            protected: Vec::new(),
            characters: 0,
        }
    }

    fn push(&mut self, bytes: &[u8], protected: bool) {
        self.bytes.extend_from_slice(bytes);
        self.protected
            .extend(core::iter::repeat_n(protected, bytes.len()));
        self.characters += 1;
    }
}

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(super) fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let (requested, names) = match parse(args) {
        Ok(parsed) => parsed,
        Err(complaint) => {
            shell.write_output(OutputDestination::Stderr, &complaint)?;
            return Ok(Flow::Done(ExitStatus::FAILURE));
        }
    };
    if let Some(prompt) = &requested.prompt
        && reading_from_terminal(shell, requested.descriptor)
    {
        shell.write_output(OutputDestination::Stderr, prompt)?;
    }

    // `-t 0` asks only whether a read would find anything, and answers
    // without consuming a byte.
    if requested.timeout == Some(0.0) {
        return Ok(Flow::Done(if input_is_available(shell, &requested, 0.0)? {
            ExitStatus::SUCCESS
        } else {
            ExitStatus::FAILURE
        }));
    }
    if let Some(seconds) = requested.timeout
        && seconds > 0.0
        && !input_is_available(shell, &requested, seconds)?
    {
        return Ok(Flow::Done(ExitStatus::from_code(128 + 14)));
    }

    let echo = requested
        .silent
        .then(|| silence(shell, &requested))
        .flatten();
    let outcome = crate::resource::with_resources(shell, |shell, _resources| {
        let mut source = ReadStream::open(shell, requested.descriptor)?;
        let outcome = read_record(shell, &mut source, &requested);
        source.close(shell);
        outcome
    });
    if let Some(echo) = &echo {
        restore(shell, &requested, echo);
    }
    /* Bash reports a descriptor it could not read -- a directory, most
     * often -- as a plain failure of `read`, where the shell's own input
     * stack calls an unreadable source unrecoverable because it is
     * normally the script itself. The diagnostic has already been
     * written; only the status is decided here. */
    let (record, complete) = match outcome {
        Ok(record) => record,
        Err(error) if error.is_unrecoverable_read() => {
            return Ok(Flow::Done(ExitStatus::FAILURE));
        }
        Err(error) => return Err(error),
    };

    assign(shell, &requested, &record, names)?;
    Ok(Flow::Done(if complete {
        ExitStatus::SUCCESS
    } else {
        ExitStatus::FAILURE
    }))
}

/// Read one record, and report whether it ended the way it was asked to
/// rather than at end of input.
fn read_record(
    shell: &mut Shell,
    source: &mut ReadStream,
    requested: &Requested,
) -> Result<(Record, bool), Error> {
    let mut record = Record::new();
    let limit = requested.character_limit();
    let mut escaped = false;
    if limit == Some(0) {
        return Ok((record, true));
    }

    loop {
        let unit = source.next_unit(shell, requested.delimiter == b'\0')?;
        if unit == InputUnit::EndOfInput {
            return Ok((record, false));
        }
        if unit.is(b'\0') && requested.delimiter != b'\0' {
            continue;
        }
        let byte = unit.expect_byte();

        // A character wider than one byte is never a delimiter and never
        // an escape, so it is appended whole and counted once.
        if !byte.is_ascii()
            && let Some(bytes) = wide_character(shell, source, byte)?
        {
            record.push(&bytes, escaped);
            escaped = false;
            if limit.is_some_and(|limit| record.characters >= limit) {
                return Ok((record, true));
            }
            continue;
        }

        if escaped {
            escaped = false;
            if byte == b'\n' {
                // A backslash-newline is swallowed whatever `-d` says,
                // and costs the character count nothing.
                continue;
            }
            record.push(&[byte], true);
        } else if !requested.raw && byte == b'\\' {
            escaped = true;
            continue;
        } else if byte == requested.delimiter && requested.exact.is_none() {
            return Ok((record, true));
        } else {
            record.push(&[byte], false);
        }

        if limit.is_some_and(|limit| record.characters >= limit) {
            return Ok((record, true));
        }
        if record.bytes.len() >= RECORD_LIMIT {
            return Err(shell
                .diagnostics()
                .shell_error(b"read: record is too long to hold"));
        }
    }
}

/// How wide the character beginning at `first` is, when the source is
/// already holding the rest of it, or `None` when it is not.
///
/// One thread-locale selection settles the whole character, and settling
/// it before consuming anything is what lets the caller skip the
/// put-back: a width of one means `first` stands alone, and nothing was
/// taken to give back.
///
/// Only the input stack can answer. `read -u N` names a descriptor the
/// shell is not parsing from and reads it a byte at a time on purpose --
/// everything after the record belongs to whoever reads that descriptor
/// next -- so it has no buffer to look into and keeps the incremental
/// decoder.
fn buffered_character_width(shell: &mut Shell, source: &ReadStream, first: u8) -> Option<usize> {
    const LONGEST: usize = 6;
    if !matches!(source, ReadStream::Standard) {
        return None;
    }
    let buffered = crate::input::buffered_line_bytes(&mut shell.input);
    if buffered.is_empty() {
        return None;
    }
    let mut window = [0_u8; LONGEST];
    window[0] = first;
    let taken = buffered.len().min(LONGEST - 1);
    window[1..=taken].copy_from_slice(&buffered[..taken]);
    match shell.locale.decode_prefix(&window[..=taken]) {
        nsh_platform::LocaleCharacter::Complete { width, .. } => Some(width),
        nsh_platform::LocaleCharacter::Invalid => Some(1),
        nsh_platform::LocaleCharacter::Incomplete => None,
    }
}

/// Collect the remaining bytes of a multi-byte character, or put back
/// what turned out not to be one.
fn wide_character(
    shell: &mut Shell,
    source: &mut ReadStream,
    first: u8,
) -> Result<Option<BString>, Error> {
    const LONGEST: usize = 6;

    if let Some(width) = buffered_character_width(shell, source, first) {
        if width == 1 {
            return Ok(None);
        }
        let mut bytes = BString::default();
        bytes.push(first);
        for _ in 1..width {
            let unit = source.next_unit(shell, true)?;
            let Some(byte) = unit.byte() else { break };
            bytes.push(byte);
        }
        return Ok(Some(bytes));
    }

    let mut decoder = shell.locale.decoder();
    let mut bytes = BString::default();
    let mut byte = first;
    loop {
        bytes.push(byte);
        match decoder.push(byte) {
            nsh_platform::LocaleDecode::Complete(_) if bytes.len() > 1 => {
                return Ok(Some(bytes));
            }
            nsh_platform::LocaleDecode::Complete(_) | nsh_platform::LocaleDecode::Invalid => break,
            nsh_platform::LocaleDecode::Incomplete => {}
        }
        if bytes.len() >= LONGEST {
            break;
        }
        let next = source.next_unit(shell, true)?;
        let Some(next_byte) = next.byte() else {
            break;
        };
        byte = next_byte;
    }
    if bytes.len() > 1 {
        source.unread(shell, &bytes[1..]);
    }
    Ok(None)
}

fn assign(
    shell: &mut Shell,
    requested: &Requested,
    record: &Record,
    names: &[&BStr],
) -> Result<(), Error> {
    if let Some(array) = &requested.array {
        let name = BStr::new(array.as_slice());
        let mut fields = ExpandedFields::new();
        crate::expand::split_fields(
            shell,
            &record.bytes,
            &record.protected,
            usize::MAX,
            &mut fields,
        );
        let mut value = VariableValue::empty(VariableKind::Indexed);
        for (index, field) in fields.fields.iter().enumerate() {
            value.set_indexed(index as u64, field.as_bstr());
        }
        return arrays::assign_value(shell, name, value);
    }

    // No operand, or `-N`, means the bytes are not fields: they are one
    // value, kept exactly as they were read.
    if names.is_empty() || requested.exact.is_some() {
        let name = names.first().map_or_else(|| BStr::new(REPLY), |name| *name);
        set_bytes(
            shell,
            name,
            Some(BStr::new(record.bytes.as_slice())),
            VariableAttributes::NONE,
        )?;
        for name in names.iter().skip(1) {
            set_bytes(shell, name, Some(BStr::new(b"")), VariableAttributes::NONE)?;
        }
        return Ok(());
    }

    let mut fields = ExpandedFields::new();
    crate::expand::split_fields(
        shell,
        &record.bytes,
        &record.protected,
        names.len(),
        &mut fields,
    );
    for (index, name) in names.iter().enumerate() {
        let value = fields
            .fields
            .get(index)
            .map_or_else(|| BStr::new(b""), crate::expand::ExpandedField::as_bstr);
        set_bytes(shell, name, Some(value), VariableAttributes::NONE)?;
    }
    Ok(())
}

fn descriptor_of(requested: &Requested) -> Option<LogicalDescriptor> {
    match requested.descriptor {
        None => Some(LogicalDescriptor::STDIN),
        Some(number) => LogicalDescriptor::new(i32::from(number)),
    }
}

fn reading_from_terminal(shell: &Shell, descriptor: Option<u8>) -> bool {
    let requested = Requested {
        descriptor,
        ..Requested::new()
    };
    descriptor_of(&requested)
        .and_then(|descriptor| shell.descriptors.get(descriptor))
        .as_ref()
        .is_some_and(nsh_platform::is_terminal)
}

fn input_is_available(
    shell: &mut Shell,
    requested: &Requested,
    seconds: f64,
) -> Result<bool, Error> {
    let Some(source) = descriptor_of(requested).and_then(|d| shell.descriptors.get(d)) else {
        return Ok(false);
    };
    Ok(nsh_platform::wait_for_input(&source, Some(seconds)).unwrap_or(true))
}

/// Turn terminal echo off for `-s`, returning what to put back.
///
/// A descriptor that is not a terminal has no echo to turn off, and a
/// read from a pipe is silent already.
fn silence(shell: &mut Shell, requested: &Requested) -> Option<nsh_platform::TerminalSettings> {
    let source = descriptor_of(requested).and_then(|d| shell.descriptors.get(d))?;
    if !nsh_platform::is_terminal(&source) {
        return None;
    }
    let saved = nsh_platform::TerminalSettings::capture(&source).ok()?;
    saved.without_echo().apply(&source).ok()?;
    Some(saved)
}

fn restore(shell: &mut Shell, requested: &Requested, saved: &nsh_platform::TerminalSettings) {
    if let Some(source) = descriptor_of(requested).and_then(|d| shell.descriptors.get(d)) {
        drop(saved.apply(&source));
    }
}

fn parse<'a>(args: &'a [&'a BStr]) -> Result<(Requested, &'a [&'a BStr]), Vec<u8>> {
    let mut requested = Requested::new();
    let mut at = 1;
    while at < args.len() {
        let word: &[u8] = args[at].as_ref();
        let Some(letters) = word.strip_prefix(b"-").filter(|rest| !rest.is_empty()) else {
            break;
        };
        if letters == b"-" {
            at += 1;
            break;
        }
        let mut offset = 0;
        while offset < letters.len() {
            let letter = letters[offset];
            offset += 1;
            match letter {
                b'r' => requested.raw = true,
                b's' => requested.silent = true,
                b'e' => continue,
                _ => {
                    if !matches!(
                        letter,
                        b'a' | b'd' | b'i' | b'n' | b'N' | b'p' | b't' | b'u'
                    ) {
                        return Err(invalid_option(letter));
                    }
                    let argument: &[u8] = if offset < letters.len() {
                        let rest = &letters[offset..];
                        offset = letters.len();
                        rest
                    } else {
                        at += 1;
                        let Some(next) = args.get(at) else {
                            return Err(missing_argument(letter));
                        };
                        next.as_ref()
                    };
                    apply(&mut requested, letter, argument)?;
                }
            }
        }
        at += 1;
    }
    Ok((requested, &args[at..]))
}

fn apply(requested: &mut Requested, letter: u8, argument: &[u8]) -> Result<(), Vec<u8>> {
    match letter {
        b'a' => requested.array = Some(BString::from(argument)),
        b'd' => requested.delimiter = argument.first().copied().unwrap_or(b'\0'),
        b'i' => {}
        b'p' => requested.prompt = Some(BString::from(argument)),
        b'n' => requested.limit = Some(count(letter, argument)?),
        b'N' => requested.exact = Some(count(letter, argument)?),
        b't' => {
            requested.timeout = Some(
                std::str::from_utf8(argument)
                    .ok()
                    .and_then(|text| text.trim().parse::<f64>().ok())
                    .ok_or_else(|| invalid_number(letter, argument))?,
            );
        }
        _ => {
            requested.descriptor = Some(
                u8::try_from(count(letter, argument)?)
                    .map_err(|_| invalid_number(letter, argument))?,
            );
        }
    }
    Ok(())
}

fn count(letter: u8, argument: &[u8]) -> Result<usize, Vec<u8>> {
    std::str::from_utf8(argument)
        .ok()
        .and_then(|text| text.trim().parse::<usize>().ok())
        .ok_or_else(|| invalid_number(letter, argument))
}

fn invalid_option(letter: u8) -> Vec<u8> {
    let mut message = b"read: -".to_vec();
    message.push(letter);
    message.extend_from_slice(b": invalid option\n");
    message
}

fn missing_argument(letter: u8) -> Vec<u8> {
    let mut message = b"read: -".to_vec();
    message.push(letter);
    message.extend_from_slice(b": option requires an argument\n");
    message
}

fn invalid_number(letter: u8, argument: &[u8]) -> Vec<u8> {
    let mut message = b"read: ".to_vec();
    message.extend_from_slice(argument);
    message.extend_from_slice(b": invalid number argument to -");
    message.push(letter);
    message.push(b'\n');
    message
}
