//! `type`.
//!
//! Port of `typecmd` and `describe_command` from `src/exec.c`.
//!
//! `describe_command` is here rather than in `crate::execution` because `type`
//! is what it is for: `command -v` and `command -V` are documented as
//! describing a name the way `type` does, so `builtins::command` calls
//! this one rather than either keeping a copy or pushing it back down
//! into the search machinery.

// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::execution::{Command, CommandSearch, PathCursor, find_command};
use crate::output::OutputDestination;
use bstr::{BStr, BString, ByteSlice};
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};

// [spec:dash:sem:exec.typecmd-fn]
// [spec:posix:syn:builtin.type.synopsis]
// [spec:posix:req:builtin.type.indicate-interpretation]
// [spec:posix:def:builtin.type.operand-name]
// [spec:posix:req:builtin.type.env-locale]
// [spec:posix:sem:builtin.type.env-nlspath]
// [spec:posix:sem:builtin.type.env-path]
// [spec:posix:sem:builtin.type.stdout]
// [spec:posix:req:builtin.type.stderr]
// [spec:posix:req:builtin.type.interfaces]
// [spec:posix:req:builtin.type.exit-status]
pub fn run(shell: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut failed = false;

    let mut option_scan = crate::options::Options::new(args);
    option_scan.next(&mut shell.diagnostics(), b"")?;
    for name in option_scan.operands() {
        match describe_command(shell, OutputDestination::Stdout, name, None, true)? {
            Flow::Done(status) => failed |= !status.success(),
            control => return Ok(control),
        }
    }
    Ok(Flow::Done(i32::from(failed).into()))
}

// [spec:dash:sem:exec.describe-command-fn]
// [spec:nsh:req:idiom.command-dispatch]
pub(crate) fn describe_command(
    shell: &mut Shell,
    dest: OutputDestination,
    command: &BStr,
    path: Option<&BStr>,
    verbose: bool,
) -> Result<Flow, Error> {
    let standard_search = path.is_none();
    let path_value = path
        .map(BString::from)
        .unwrap_or_else(|| crate::variables::path_value(shell));
    let path = path_value.as_slice().as_bstr();

    'resolution_complete: {
        if verbose {
            shell.write_output(dest, command)?;
        }

        /* First look at the keywords */
        if crate::parser::reserved_word(command).is_some() {
            let bytes = if verbose {
                b" is a shell keyword" as &[u8]
            } else {
                command.as_bytes()
            };
            shell.write_output(dest, bytes)?;
            break 'resolution_complete;
        }

        /* Then look at the aliases */
        if let Some(alias) = shell.aliases.lookup(command, false) {
            if verbose {
                let mut record = b" is an alias for ".to_vec();
                record.extend_from_slice(&alias);
                shell.write_output(dest, &record)?;
            } else {
                let line = crate::alias::format_alias(command, alias.as_slice().as_bstr());
                let mut record = b"alias ".to_vec();
                record.extend_from_slice(&line);
                shell.write_output(dest, &record)?;
                return Ok(Flow::Done((0).into()));
            }
            break 'resolution_complete;
        }

        /* Then if the standard search path is used, check if it is
         * a tracked alias.
         */
        let tracked = standard_search
            .then(|| shell.commands.resolved(command))
            .flatten();
        let was_tracked = tracked.is_some();
        let mut entry = tracked.unwrap_or(Command::Unknown);
        if !was_tracked {
            /* Finally use brute force */
            match find_command(
                shell,
                command,
                &mut entry,
                CommandSearch::DEFAULT.checking_absolute(),
                path,
            )? {
                Flow::Done(_) => {}
                control => return Ok(control),
            }
        }

        match entry {
            Command::External { path_index } => {
                let resolved: BString;
                let path_bytes: &BStr = if let Some(path_index) = path_index {
                    let mut cursor = PathCursor::new(path);
                    let candidate = (0..=path_index)
                        .map(|_| cursor.advance(command))
                        .last()
                        .flatten();
                    resolved = candidate
                        .expect("a resolved PATH index must name a PATH element")
                        .path;
                    resolved.as_bstr()
                } else {
                    // [spec:posix:req:builtin.command.opt-v]
                    if !verbose && nsh_platform::shell_path_has_separator(command) {
                        resolved = command
                            .try_to_path_buf()
                            .and_then(|path| nsh_platform::absolute_path(&path))
                            .map(|path| BString::from(path.to_shell_bytes()))
                            .unwrap_or_else(|_| command.to_owned());
                        resolved.as_slice().as_bstr()
                    } else {
                        command
                    }
                };
                if verbose {
                    let mut record = b" is".to_vec();
                    if was_tracked {
                        record.extend_from_slice(b" a tracked alias for");
                    }
                    record.push(b' ');
                    record.extend_from_slice(path_bytes);
                    shell.write_output(dest, &record)?;
                } else {
                    shell.write_output(dest, path_bytes)?;
                }
            }

            Command::Function(_) => {
                if verbose {
                    shell.write_output(dest, b" is a shell function")?;
                } else {
                    shell.write_output(dest, command)?;
                }
            }

            Command::Builtin(builtin) => {
                if verbose {
                    let record: &[u8] = if builtin.attributes().is_special() {
                        b" is a special shell builtin"
                    } else {
                        b" is a shell builtin"
                    };
                    shell.write_output(dest, record)?;
                } else {
                    shell.write_output(dest, command)?;
                }
            }

            Command::Unknown => {
                if verbose {
                    shell.write_output(dest, b": not found\n")?;
                }
                return Ok(Flow::Done((127).into()));
            }
        }
    }
    // out:
    shell.write_output(dest, b"\n")?;
    Ok(Flow::Done((0).into()))
}
