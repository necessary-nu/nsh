//! Bash's directory stack: `dirs`, `pushd` and `popd`.
//!
//! The stack's top entry is not stored. It is `$PWD`, always, which is
//! the only way `cd` can appear to "replace the lowest entry" without
//! knowing this module exists -- and Bash's stack does behave that way.
//! What [`DirectoryStack`] holds is everything *underneath* the current
//! directory, so `pushd` pushes where the shell was and `popd` returns
//! to where it came from.
//!
//! Rendering is the other half. Bash abbreviates a path under `$HOME` to
//! `~`, and `dirs -l` is the option that asks it not to; the abbreviation
//! is textual and re-derived at print time, because `$HOME` can change
//! between the `pushd` and the `dirs`.

use bstr::{BStr, BString, ByteSlice as _};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

/// The directories below the current one, innermost first.
#[derive(Default)]
pub struct DirectoryStack {
    below: Vec<BString>,
}

impl DirectoryStack {
    pub(crate) const fn new() -> Self {
        Self { below: Vec::new() }
    }

    /// The entries under the current directory, for `$DIRSTACK`.
    pub(crate) fn below(&self) -> &[BString] {
        &self.below
    }
}

/// What `dirs` was asked to print.
#[derive(Clone, Copy, Default)]
struct Format {
    clear: bool,
    numbered: bool,
    per_line: bool,
    literal: bool,
}

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run_dirs(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut format = Format::default();
    for word in &args[1.min(args.len())..] {
        let Some(letters) = word.strip_prefix(b"-").filter(|rest| !rest.is_empty()) else {
            // Bash's `dirs` takes no operands at all; a `+N`/`-N` index
            // selector would be one, and is not accepted here.
            return Err(refuse(shell, b"dirs", word));
        };
        for letter in letters {
            match letter {
                b'c' => format.clear = true,
                b'v' => format.numbered = true,
                b'p' => format.per_line = true,
                b'l' => format.literal = true,
                _ => return Err(refuse(shell, b"dirs", word)),
            }
        }
    }
    if format.clear {
        shell.directory_stack.below.clear();
        crate::variables::special::publish_directory_stack(shell);
        return Ok(Flow::Done(ExitStatus::SUCCESS));
    }
    write_stack(shell, format)?;
    Ok(Flow::Done(ExitStatus::SUCCESS))
}

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run_pushd(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let operands = match options_end(shell, b"pushd", args) {
        Ok(operands) => operands,
        Err(status) => return Ok(Flow::Done(status)),
    };
    if operands.len() > 1 {
        return Ok(Flow::Done(usage(shell, b"pushd: too many arguments\n")?));
    }

    let Some(target) = operands.first().copied() else {
        // A bare `pushd` exchanges the top two entries, which is what
        // makes it a way to flip between two directories.
        let Some(other) = shell.directory_stack.below.first().cloned() else {
            return Ok(Flow::Done(usage(
                shell,
                b"pushd: no other directory on the directory stack\n",
            )?));
        };
        let here = shell.working_directory.logical.clone().unwrap_or_default();
        if !change_to(shell, BStr::new(other.as_slice()))? {
            return Ok(Flow::Done(ExitStatus::FAILURE));
        }
        shell.directory_stack.below[0] = here;
        crate::variables::special::publish_directory_stack(shell);
        write_stack(shell, Format::default())?;
        return Ok(Flow::Done(ExitStatus::SUCCESS));
    };

    let here = shell.working_directory.logical.clone().unwrap_or_default();
    if !change_to(shell, target)? {
        return Ok(Flow::Done(ExitStatus::FAILURE));
    }
    shell.directory_stack.below.insert(0, here);
    crate::variables::special::publish_directory_stack(shell);
    write_stack(shell, Format::default())?;
    Ok(Flow::Done(ExitStatus::SUCCESS))
}

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub fn run_popd(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let operands = match options_end(shell, b"popd", args) {
        Ok(operands) => operands,
        Err(status) => return Ok(Flow::Done(status)),
    };
    if !operands.is_empty() {
        return Ok(Flow::Done(usage(shell, b"popd: too many arguments\n")?));
    }
    if shell.directory_stack.below.is_empty() {
        shell.write_output(OutputDestination::Stderr, b"popd: directory stack empty\n")?;
        return Ok(Flow::Done(ExitStatus::FAILURE));
    }
    let target = shell.directory_stack.below.remove(0);
    if !change_to(shell, BStr::new(target.as_slice()))? {
        return Ok(Flow::Done(ExitStatus::FAILURE));
    }
    crate::variables::special::publish_directory_stack(shell);
    write_stack(shell, Format::default())?;
    Ok(Flow::Done(ExitStatus::SUCCESS))
}

/// Step over the options `pushd` and `popd` accept, which is only `--`.
///
/// Bash's `+N`/`-N` rotation selectors are not accepted, so a word
/// beginning with `-` and holding anything else is a usage error with
/// status 2 -- which is what the survey's `pushd -z` case asks for.
fn options_end<'a>(
    shell: &mut Shell,
    name: &[u8],
    args: &'a [&'a BStr],
) -> Result<&'a [&'a BStr], ExitStatus> {
    let mut at = 1;
    if let Some(word) = args.get(at) {
        let bytes: &[u8] = word.as_ref();
        if bytes == b"--" {
            at += 1;
        } else if bytes.len() >= 2 && bytes[0] == b'-' {
            let mut message = name.to_vec();
            message.extend_from_slice(b": ");
            message.extend_from_slice(bytes);
            message.extend_from_slice(b": invalid option\n");
            drop(shell.write_output(OutputDestination::Stderr, &message));
            return Err(ExitStatus::ERROR);
        }
    }
    Ok(&args[at.min(args.len())..])
}

fn usage(shell: &mut Shell, message: &[u8]) -> Result<ExitStatus, Error> {
    shell.write_output(OutputDestination::Stderr, message)?;
    Ok(ExitStatus::ERROR)
}

fn refuse(shell: &mut Shell, name: &[u8], word: &BStr) -> Error {
    let mut message = name.to_vec();
    message.extend_from_slice(b": ");
    message.extend_from_slice(word.as_bytes());
    message.extend_from_slice(b": invalid argument");
    shell.diagnostics().shell_error(&message)
}

/// Move the shell, reporting whether it went.
///
/// `cd` is reused rather than reimplemented so `pushd ..` resolves the
/// same way `cd ..` does, and a failure leaves the stack untouched.
fn change_to(shell: &mut Shell, target: &BStr) -> Result<bool, Error> {
    let arguments = [BStr::new(b"cd"), target];
    match super::cd::run(shell, &arguments) {
        Ok(Flow::Done(status)) => Ok(status.success()),
        Ok(_) => Ok(false),
        Err(error) => {
            shell.status = error.status();
            Ok(false)
        }
    }
}

fn write_stack(shell: &mut Shell, format: Format) -> Result<(), Error> {
    let home = crate::variables::lookup_bytes(shell, BStr::new(b"HOME")).unwrap_or_default();
    let mut entries = vec![shell.working_directory.logical.clone().unwrap_or_default()];
    entries.extend(shell.directory_stack.below.iter().cloned());

    let mut line = Vec::new();
    for (position, entry) in entries.iter().enumerate() {
        let rendered = if format.literal {
            entry.clone()
        } else {
            abbreviate(BStr::new(entry.as_slice()), BStr::new(home.as_slice()))
        };
        if format.numbered {
            line.extend_from_slice(format!("{position:>2}  ").as_bytes());
        } else if format.per_line {
        } else if position > 0 {
            line.push(b' ');
        }
        line.extend_from_slice(&rendered);
        if format.numbered || format.per_line {
            line.push(b'\n');
        }
    }
    if !format.numbered && !format.per_line {
        line.push(b'\n');
    }
    shell.write_output(OutputDestination::Stdout, &line)
}

/// `$HOME` at the front of a path becomes `~`, and nothing else does.
///
/// The match is on a whole component: `/tmp/oil_tests` is not under a
/// `$HOME` of `/tmp/oil_test` even though its bytes begin with it.
fn abbreviate(path: &BStr, home: &BStr) -> BString {
    if home.is_empty() {
        return path.to_owned();
    }
    if path == home {
        return BString::from("~");
    }
    let bytes: &[u8] = path.as_ref();
    let Some(rest) = bytes.strip_prefix(home.as_bytes()) else {
        return path.to_owned();
    };
    if rest.first() != Some(&nsh_platform::shell_directory_separator()) {
        return path.to_owned();
    }
    let mut abbreviated = BString::from("~");
    abbreviated.extend_from_slice(rest);
    abbreviated
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:compat.bash.builtins-special-variables/test]
    #[test]
    fn home_is_abbreviated_only_on_a_component_boundary() {
        let home = BStr::new(b"/tmp/oil_test");
        assert_eq!(abbreviate(BStr::new(b"/tmp/oil_test"), home), "~");
        assert_eq!(abbreviate(BStr::new(b"/tmp/oil_test/x"), home), "~/x");
        assert_eq!(
            abbreviate(BStr::new(b"/tmp/oil_tests"), home),
            "/tmp/oil_tests"
        );
        assert_eq!(
            abbreviate(BStr::new(b"/elsewhere"), BStr::new(b"")),
            "/elsewhere"
        );
    }
}
