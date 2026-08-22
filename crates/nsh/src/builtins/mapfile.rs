//! Bash's `mapfile`, and `readarray`, which is the same command.
//!
//! `read -a` splits *one* record into fields; `mapfile` reads *every*
//! record into an element and never splits, so `IFS` has nothing to do
//! here. That is the whole difference, and it is why this is its own
//! module rather than an option on `read`.
//!
//! Records arrive through [`crate::builtins::read::stream`], the same
//! byte source `read` uses, so a `mapfile` on standard input consumes
//! exactly the bytes it stores and leaves the rest for whatever reads
//! next.
//!
//! `-c` and `-C` -- run a callback every so many records -- are refused
//! rather than implemented: they hand a data-derived string back to the
//! evaluator in the middle of a read, which is the ambient
//! data-to-syntax path `[dec:nsh:safety-trumps-compatibility]` says not
//! to import.

use bstr::{BStr, BString};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::output::OutputDestination;
use crate::status::ExitStatus;
use crate::variables::arrays;
use crate::variables::value::{VariableKind, VariableValue};

use super::read::stream::ReadStream;

/// The default name, which is why `mapfile` with no operand still fills
/// an array.
const DEFAULT_NAME: &[u8] = b"MAPFILE";

/// What one invocation asked for.
struct Requested {
    delimiter: u8,
    strip_delimiter: bool,
    /// `-n`: stop after this many records; zero means every record.
    count: u64,
    /// `-O`: the first index written.
    origin: u64,
    /// `-s`: records discarded before the first one stored.
    skip: u64,
    /// `-u`: which descriptor to read.
    descriptor: Option<u8>,
    /// Whether `-O` was given, which decides if the array is cleared.
    keeps_existing: bool,
}

impl Requested {
    const fn new() -> Self {
        Self {
            delimiter: b'\n',
            strip_delimiter: false,
            count: 0,
            origin: 0,
            skip: 0,
            descriptor: None,
            keeps_existing: false,
        }
    }
}

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let (requested, operands) = match parse(args) {
        Ok(parsed) => parsed,
        Err(complaint) => {
            shell.write_output(OutputDestination::Stderr, &complaint)?;
            return Ok(Flow::Done(ExitStatus::ERROR));
        }
    };
    let name = operands
        .first()
        .map_or_else(|| BString::from(DEFAULT_NAME), |word| (*word).to_owned());
    if !crate::parser::is_valid_name(&shell.locale, BStr::new(name.as_slice())) {
        let mut message = b"mapfile: ".to_vec();
        message.extend_from_slice(name.as_slice());
        message.extend_from_slice(b": not a valid identifier\n");
        shell.write_output(OutputDestination::Stderr, &message)?;
        return Ok(Flow::Done(ExitStatus::ERROR));
    }

    let records = read_records(shell, &requested)?;
    store(shell, BStr::new(name.as_slice()), &requested, records)?;
    Ok(Flow::Done(ExitStatus::SUCCESS))
}

fn read_records(shell: &mut Shell, requested: &Requested) -> Result<Vec<BString>, Error> {
    let mut source = ReadStream::open(shell, requested.descriptor)?;
    let mut records = Vec::new();
    let mut seen = 0u64;
    loop {
        if requested.count != 0 && records.len() as u64 >= requested.count {
            break;
        }
        let Some(mut record) = source.record(shell, requested.delimiter)? else {
            break;
        };
        seen += 1;
        if seen <= requested.skip {
            continue;
        }
        if requested.strip_delimiter && record.last() == Some(&requested.delimiter) {
            record.pop();
        }
        records.push(record);
    }
    source.close(shell);
    Ok(records)
}

fn store(
    shell: &mut Shell,
    name: &BStr,
    requested: &Requested,
    records: Vec<BString>,
) -> Result<(), Error> {
    let mut value = if requested.keeps_existing {
        match crate::variables::value::variable_value(shell, name).cloned() {
            Some(value) if value.kind() == VariableKind::Indexed => value,
            _ => VariableValue::empty(VariableKind::Indexed),
        }
    } else {
        VariableValue::empty(VariableKind::Indexed)
    };
    for (position, record) in records.iter().enumerate() {
        value.set_indexed(
            requested.origin.saturating_add(position as u64),
            BStr::new(record.as_slice()),
        );
    }
    arrays::assign_value(shell, name, value)
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
            if letter == b't' {
                requested.strip_delimiter = true;
                continue;
            }
            if !matches!(letter, b'd' | b'n' | b'O' | b's' | b'u' | b'c' | b'C') {
                let mut message = b"mapfile: -".to_vec();
                message.push(letter);
                message.extend_from_slice(b": invalid option\n");
                return Err(message);
            }
            let argument: &[u8] = if offset < letters.len() {
                let rest = &letters[offset..];
                offset = letters.len();
                rest
            } else {
                at += 1;
                let Some(next) = args.get(at) else {
                    let mut message = b"mapfile: -".to_vec();
                    message.push(letter);
                    message.extend_from_slice(b": option requires an argument\n");
                    return Err(message);
                };
                next.as_ref()
            };
            apply(&mut requested, letter, argument)?;
        }
        at += 1;
    }
    Ok((requested, &args[at..]))
}

fn apply(requested: &mut Requested, letter: u8, argument: &[u8]) -> Result<(), Vec<u8>> {
    match letter {
        b'd' => requested.delimiter = argument.first().copied().unwrap_or(b'\0'),
        b'u' => requested.descriptor = Some(number(letter, argument)? as u8),
        b'n' => requested.count = number(letter, argument)?,
        b's' => requested.skip = number(letter, argument)?,
        b'O' => {
            requested.origin = number(letter, argument)?;
            requested.keeps_existing = true;
        }
        _ => {
            return Err(
                b"mapfile: -C: running a callback while reading is not supported\n".to_vec(),
            );
        }
    }
    Ok(())
}

fn number(letter: u8, argument: &[u8]) -> Result<u64, Vec<u8>> {
    std::str::from_utf8(argument)
        .ok()
        .and_then(|text| text.trim().parse::<u64>().ok())
        .ok_or_else(|| {
            let mut message = b"mapfile: ".to_vec();
            message.extend_from_slice(argument);
            message.extend_from_slice(b": invalid ");
            message.push(letter);
            message.extend_from_slice(b" argument\n");
            message
        })
}
