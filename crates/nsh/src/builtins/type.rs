//! `type`.
//!
//! Port of `typecmd` and `describe_command` from `src/exec.c`.
//!
//! `describe_command` is here rather than in `crate::exec` because `type`
//! is what it is for: `command -v` and `command -V` are documented as
//! describing a name the way `type` does, so `builtins::command` calls
//! this one rather than either keeping a copy or pushing it back down
//! into the search machinery.

// [spec:nsh:req:idiom.evaluator-control-flow]
use crate::context::Shell;
use crate::error::Error;
use bstr::{BStr, BString, ByteSlice};
use core::ffi::c_int;
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};
use std::io::Write;

use crate::eval::Flow;
use crate::exec::{Command, DO_ABS, PathCursor, find_command, padvance};
use crate::output::Dest;

// [spec:dash:def:exec.typecmd-fn]
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
pub fn typecmd(sh: &mut Shell, args: &[&BStr]) -> Result<Flow, Error> {
    let mut err: c_int = 0;

    let mut opts = crate::options::Options::new(args);
    opts.next(&mut sh.diagnostics(), b"")?;
    for name in opts.operands() {
        match describe_command(sh, Dest::Stdout, name, None, 1)? {
            Flow::Done(status) => err |= i32::from(status.code()),
            control => return Ok(control),
        }
    }
    Ok(Flow::Done((err).into()))
}

// [spec:dash:def:exec.describe-command-fn]
// [spec:dash:sem:exec.describe-command-fn]
// [spec:nsh:req:idiom.command-dispatch]
pub(crate) fn describe_command(
    sh: &mut Shell,
    dest: Dest,
    command: &BStr,
    path: Option<&BStr>,
    verbose: c_int,
) -> Result<Flow, Error> {
    let standard_search = path.is_none();
    let path_value = path
        .map(BString::from)
        .unwrap_or_else(|| crate::var::pathval(sh));
    let path = path_value.as_slice().as_bstr();

    'out_label: {
        if verbose != 0 {
            let _ = sh.io.get(dest).write_all(command);
        }

        /* First look at the keywords */
        if crate::parser::findkwd(command).is_some() {
            let bytes = if verbose != 0 {
                b" is a shell keyword" as &[u8]
            } else {
                command.as_bytes()
            };
            let _ = sh.io.get(dest).write_all(bytes);
            break 'out_label;
        }

        /* Then look at the aliases */
        if let Some(alias) = sh.aliases.lookup(command, false) {
            if verbose != 0 {
                let mut record = b" is an alias for ".to_vec();
                record.extend_from_slice(&alias);
                let _ = sh.io.get(dest).write_all(&record);
            } else {
                let line = crate::alias::printalias(command, alias.as_slice().as_bstr());
                let io = sh.io.get(dest);
                let _ = io.write_all(b"alias ");
                let _ = io.write_all(&line);
                return Ok(Flow::Done((0).into()));
            }
            break 'out_label;
        }

        /* Then if the standard search path is used, check if it is
         * a tracked alias.
         */
        let tracked = standard_search
            .then(|| sh.commands.resolved(command))
            .flatten();
        let was_tracked = tracked.is_some();
        let mut entry = tracked.unwrap_or(Command::Unknown);
        if !was_tracked {
            /* Finally use brute force */
            match find_command(sh, command, &mut entry, DO_ABS, path)? {
                Flow::Done(_) => {}
                control => return Ok(control),
            }
        }

        match entry {
            Command::External { path_index } => {
                let mut j = path_index;
                let resolved: BString;
                let path_bytes: &BStr = if j == -1 {
                    // [spec:posix:req:builtin.command.opt-v]
                    if verbose == 0 && nsh_platform::shell_path_has_separator(command) {
                        resolved = command
                            .try_to_path_buf()
                            .and_then(|path| nsh_platform::absolute_path(&path))
                            .map(|path| BString::from(path.to_shell_bytes()))
                            .unwrap_or_else(|_| command.to_owned());
                        resolved.as_slice().as_bstr()
                    } else {
                        command
                    }
                } else {
                    let mut cursor = PathCursor::new(path);
                    let candidate = loop {
                        let candidate = padvance(&mut cursor, command);
                        j -= 1;
                        if j < 0 {
                            break candidate;
                        }
                    };
                    resolved = candidate
                        .expect("a resolved PATH index must name a PATH element")
                        .path;
                    resolved.as_bstr()
                };
                if verbose != 0 {
                    let mut record = b" is".to_vec();
                    if was_tracked {
                        record.extend_from_slice(b" a tracked alias for");
                    }
                    record.push(b' ');
                    record.extend_from_slice(path_bytes);
                    let _ = sh.io.get(dest).write_all(&record);
                } else {
                    let _ = sh.io.get(dest).write_all(path_bytes);
                }
            }

            Command::Function(_) => {
                if verbose != 0 {
                    let _ = sh.io.get(dest).write_all(b" is a shell function");
                } else {
                    let _ = sh.io.get(dest).write_all(command);
                }
            }

            Command::Builtin(builtin) => {
                if verbose != 0 {
                    let record: &[u8] = if builtin.attributes().is_special() {
                        b" is a special shell builtin"
                    } else {
                        b" is a shell builtin"
                    };
                    let _ = sh.io.get(dest).write_all(record);
                } else {
                    let _ = sh.io.get(dest).write_all(command);
                }
            }

            Command::Unknown => {
                if verbose != 0 {
                    let _ = sh.io.get(dest).write_all(b": not found\n");
                }
                return Ok(Flow::Done((127).into()));
            }
        }
    }
    // out:
    let _ = sh.io.get(dest).write_all(b"\n");
    Ok(Flow::Done((0).into()))
}
