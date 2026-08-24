//! `type -a -f -p -P -t`, and the `command -v`/`-V` spellings of the
//! same question, in the Bash dialect.
//!
//! POSIX's `type` answers one question -- *what would this name run?* --
//! and dash's `describe_command` answers it in one pass, stopping at the
//! first hit. Bash's options ask a different question: *what are all the
//! things this name could be?* So the resolution is built as a list here
//! and the options only decide how much of it is printed. Keeping the
//! list and the rendering apart is what stops `-t`, `-p`, `-P` and `-a`
//! becoming four searches that can disagree.
//!
//! The `PATH` walk deliberately does not reuse `find_command`: that
//! function stops at the first executable and caches what it found,
//! neither of which `type -a` wants.

use bstr::{BStr, BString, ByteSlice as _};
use nsh_platform::ShellBytesExt as _;

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;
use crate::execution::PathCursor;
use crate::output::OutputDestination;
use crate::status::ExitStatus;

/// One way a name resolves, in the order Bash reports them.
enum Resolution {
    Alias(BString),
    Keyword,
    Function,
    Builtin { special: bool },
    File(BString),
}

impl Resolution {
    /// What `type -t` prints for this resolution.
    const fn terse(&self) -> &'static [u8] {
        match self {
            Self::Alias(_) => b"alias",
            Self::Keyword => b"keyword",
            Self::Function => b"function",
            Self::Builtin { .. } => b"builtin",
            Self::File(_) => b"file",
        }
    }
}

/// Which of the resolutions an invocation asked to see.
#[derive(Clone, Copy, Default)]
pub(crate) struct Requested {
    all: bool,
    terse: bool,
    path_only: bool,
    force_path: bool,
    skip_functions: bool,
    /// `command -v`: the shortest thing a caller could run to get this
    /// resolution -- a path for a file, and the bare name for anything
    /// the shell resolves itself.
    identify: bool,
}

impl Requested {
    /// `command -V` is plain `type`, and `command -v` is the same
    /// resolution rendered as something runnable. Both take the Bash
    /// renderer in Bash mode so a name is described the same way
    /// whichever spelling asked.
    pub(crate) const fn describing(verbose: bool) -> Self {
        Self {
            all: false,
            terse: false,
            path_only: false,
            force_path: false,
            skip_functions: false,
            identify: !verbose,
        }
    }
}

/// Scan the option cluster Bash accepts, which is not the POSIX
/// `type`'s: POSIX has none at all, so this runs only in Bash mode.
pub(super) fn parse<'a>(args: &'a [&'a BStr]) -> Result<(Requested, &'a [&'a BStr]), u8> {
    let mut requested = Requested::default();
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
        for letter in letters {
            match letter {
                b'a' => requested.all = true,
                b't' => requested.terse = true,
                b'p' => requested.path_only = true,
                b'P' => {
                    requested.path_only = true;
                    requested.force_path = true;
                }
                b'f' => requested.skip_functions = true,
                other => return Err(*other),
            }
        }
        at += 1;
    }
    Ok((requested, &args[at..]))
}

// [spec:nsh:req:compat.bash.builtins-special-variables]
pub(crate) fn run(
    shell: &mut Shell,
    requested: Requested,
    dest: OutputDestination,
    names: &[&BStr],
) -> Result<Flow, Error> {
    let mut status = ExitStatus::SUCCESS;
    for name in names {
        let resolutions = resolve(shell, requested, name);
        if resolutions.is_empty() {
            if !requested.terse && !requested.path_only {
                let mut message = b"nsh: type: ".to_vec();
                message.extend_from_slice(name.as_bytes());
                message.extend_from_slice(b": not found\n");
                shell.write_output(OutputDestination::Stderr, &message)?;
            }
            status = ExitStatus::FAILURE;
            continue;
        }
        if !write_resolutions(shell, requested, dest, name, &resolutions)? {
            status = ExitStatus::FAILURE;
        }
    }
    Ok(Flow::Done(status))
}

/// Render one name's resolutions, and report whether the request was
/// satisfied.
///
/// `-p` and `-P` are unsatisfied by a name that resolves to nothing on
/// disk, which is why this returns a flag rather than writing a status
/// itself: `type -P cd` finds `cd` and still fails.
fn write_resolutions(
    shell: &mut Shell,
    requested: Requested,
    dest: OutputDestination,
    name: &BStr,
    resolutions: &[Resolution],
) -> Result<bool, Error> {
    if requested.terse {
        for resolution in resolutions {
            let mut line = resolution.terse().to_vec();
            line.push(b'\n');
            shell.write_output(dest, &line)?;
            if !requested.all {
                break;
            }
        }
        return Ok(true);
    }

    if requested.identify {
        for resolution in resolutions {
            let mut line = match resolution {
                Resolution::Alias(text) => {
                    crate::alias::format_alias(name, BStr::new(text.as_slice())).to_vec()
                }
                Resolution::File(path) => path.to_vec(),
                _ => name.as_bytes().to_vec(),
            };
            line.push(b'\n');
            shell.write_output(dest, &line)?;
            if !requested.all {
                break;
            }
        }
        return Ok(true);
    }

    if requested.path_only {
        let files: Vec<&BString> = resolutions
            .iter()
            .filter_map(|resolution| match resolution {
                Resolution::File(path) => Some(path),
                _ => None,
            })
            .collect();
        // Plain `-p` reports a file only when the name *is* a file;
        // `-a -p` reports every file behind whatever else the name is.
        let selected: &[&BString] = if requested.all {
            &files
        } else if matches!(resolutions.first(), Some(Resolution::File(_))) {
            &files[..1.min(files.len())]
        } else {
            &[]
        };
        for path in selected {
            let mut line = path.to_vec();
            line.push(b'\n');
            shell.write_output(dest, &line)?;
        }
        return Ok(!requested.force_path || !files.is_empty());
    }

    for resolution in resolutions {
        let mut line = name.as_bytes().to_vec();
        match resolution {
            Resolution::Alias(text) => {
                line.extend_from_slice(b" is aliased to `");
                line.extend_from_slice(text);
                line.push(b'\'');
            }
            Resolution::Keyword => line.extend_from_slice(b" is a shell keyword"),
            Resolution::Function => {
                line.extend_from_slice(b" is a function");
                // Bash follows the sentence with the definition itself,
                // which is the same text `declare -f` prints.
                if let Some(source) = crate::builtins::declare::functions::source(shell, name) {
                    line.push(b'\n');
                    line.extend_from_slice(&source);
                }
            }
            Resolution::Builtin { special } => {
                line.extend_from_slice(if *special {
                    b" is a special shell builtin" as &[u8]
                } else {
                    b" is a shell builtin"
                });
            }
            Resolution::File(path) => {
                line.extend_from_slice(b" is ");
                line.extend_from_slice(path);
            }
        }
        line.push(b'\n');
        shell.write_output(dest, &line)?;
        if !requested.all {
            break;
        }
    }
    Ok(true)
}

fn resolve(shell: &mut Shell, requested: Requested, name: &BStr) -> Vec<Resolution> {
    let mut resolutions = Vec::new();
    if !requested.force_path {
        if let Some(alias) = shell.aliases.lookup(name, false) {
            resolutions.push(Resolution::Alias(BString::from(alias.to_vec())));
        }
        if is_keyword(shell, name) {
            resolutions.push(Resolution::Keyword);
        }
        if !requested.skip_functions && shell.commands.is_function(name) {
            resolutions.push(Resolution::Function);
        }
        if let Some(spec) = crate::execution::builtin(shell, name) {
            resolutions.push(Resolution::Builtin {
                special: spec.attributes().is_special(),
            });
        }
    }

    // `-t` calls a name a file whenever `PATH` holds one, executable or
    // not, which is what Bash reports and what `command -v` does not.
    let executable_only = !requested.terse;
    let mut files = search_path(shell, name, executable_only);
    if !requested.all {
        files.truncate(1);
    }
    resolutions.extend(files.into_iter().map(Resolution::File));
    resolutions
}

/// Bash's reserved words, restricted to the ones this shell's parser
/// actually treats as such -- claiming `select` or `time` here would
/// report a grammar the shell does not have.
fn is_keyword(shell: &Shell, name: &BStr) -> bool {
    if crate::parser::reserved_word(name, shell.options.dialect()).is_some() {
        return true;
    }
    shell.options.dialect() == crate::options::Dialect::Bash
        && matches!(name.as_bytes(), b"[[" | b"]]" | b"function")
}

/// Every `PATH` entry that names `name`, in `PATH` order.
///
/// A directory is never a candidate: `PATH=_tmp` with a directory
/// `_tmp/cat` must not make `cat` a command.
fn search_path(shell: &mut Shell, name: &BStr, executable_only: bool) -> Vec<BString> {
    if nsh_platform::shell_path_has_separator(name) {
        return usable(name, executable_only).into_iter().collect();
    }
    let path = crate::variables::path_value(shell);
    let mut found = Vec::new();
    let mut cursor = PathCursor::literal(BStr::new(path.as_slice()));
    while let Some(candidate) = cursor.advance(name) {
        if let Some(path) = usable(BStr::new(candidate.path.as_slice()), executable_only) {
            found.push(path);
        }
    }
    found
}

fn usable(candidate: &BStr, executable_only: bool) -> Option<BString> {
    let native = candidate.as_bytes().try_to_path_buf().ok()?;
    let metadata = nsh_platform::path_metadata(&native, true).ok()?;
    if metadata.kind == nsh_platform::FileKind::Directory {
        return None;
    }
    if executable_only
        && !nsh_platform::effective_access(&native, nsh_platform::AccessMode::EXEC_OK)
    {
        return None;
    }
    Some(candidate.to_owned())
}
