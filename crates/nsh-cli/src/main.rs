//! Entry point. The shell itself is the `nsh` library; this frontend owns
//! the host-process setup and returns the library's status to the OS.
//!
//! Three of the things this file does exist for one reason: Rust's
//! runtime does work between `_start` and this `main` that C's does not,
//! and a shell is close enough to the operating system that all of it
//! shows. What `std::rt::init` does, and what dash does instead:
//!
//!   * sets SIGPIPE to SIG_IGN — dash never touches SIGPIPE
//!   * opens /dev/null over any closed fd 0/1/2 — dash leaves them closed
//!   * catches SIGSEGV/SIGBUS on an alternate stack, to report a stack
//!     overflow — dash has no handler and dies on the signal
//!
//! Each is inherited or observable, so `nsh-platform` undoes each before
//! the shell starts. They were
//! found by diffing `/proc/self/status` and `/proc/self/fd` between the
//! two shells, which is worth repeating after any toolchain bump: this
//! list is a property of the Rust runtime, not of the port.

#![deny(unsafe_code)]

use bstr::{BStr, ByteSlice as _};
use std::io::{IsTerminal as _, Write as _};

mod invocation;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FrontendAction {
    Help,
    Version,
}

const HELP: &[u8] = concat!(
    "nsh ",
    env!("CARGO_PKG_VERSION"),
    "\n\n",
    "Usage:\n",
    "  nsh [OPTION]... [SCRIPT [ARG]...]\n",
    "  nsh [OPTION]... -c COMMAND [NAME [ARG]...]\n\n",
    "Frontend options:\n",
    "      --help       show this help and exit\n",
    "      --version    show version information and exit\n\n",
    "Shell options include -i, -s, -o NAME, and the ordinary set flags.\n",
    "Run `nsh -c 'set -o'` to list the complete shell-option state.\n",
)
.as_bytes();

const VERSION: &[u8] = concat!("nsh ", env!("CARGO_PKG_VERSION"), "\n").as_bytes();

// [spec:nsh:req:cli.metadata-options]
fn frontend_action(argv: &[Vec<u8>]) -> Option<FrontendAction> {
    match argv.get(1).map(Vec::as_slice) {
        Some(b"--help") => Some(FrontendAction::Help),
        Some(b"--version") => Some(FrontendAction::Version),
        _ => None,
    }
}

fn write_frontend_output(bytes: &[u8]) {
    if std::io::stdout().lock().write_all(bytes).is_err() {
        std::process::exit(1);
    }
}

fn write_invocation_error(argument_zero: Option<&Vec<u8>>, message: &[u8]) {
    let mut stderr = std::io::stderr().lock();
    let _ = stderr.write_all(argument_zero.map_or(b"sh", Vec::as_slice));
    let _ = stderr.write_all(b": 0: ");
    let _ = stderr.write_all(message);
    let _ = stderr.write_all(b"\n");
}

use nsh_platform::NativeStrExt as _;

fn main() {
    // A panic hook sat here, filtering out the `error::Longjmp` payload
    // the port used to implement C's `longjmp`: those unwinds were
    // ordinary control flow -- every shell error, interrupt, `exit` and
    // `set -e` went through one -- and the default hook printed a
    // "thread 'main' panicked" banner each time one was *raised*.
    //
    // `errors-are-values` deleted the mechanism, so there is no payload
    // to filter and every panic that reaches the hook is a genuine bug.
    // The default hook is the right one for that.

    // C's `main(int argc, char **argv)` receives raw NUL-terminated byte
    // strings. An argument need not be valid UTF-8, and dash passes such
    // bytes through untouched — `dash -c $'x=\xff; echo $x'` prints the
    // byte. `std::env::args()` unwraps a UTF-8 conversion and panics on
    // any non-UTF-8 argument, so the port died with status 101 where the
    // C ran normally.
    //
    // Keep the operating-system representation intact until this explicit
    // handoff to the byte-oriented shell language engine.
    let argv: Vec<Vec<u8>> = nsh_platform::process_arguments()
        .iter()
        .map(|argument| argument.to_shell_bytes())
        .collect();
    if let Some(action) = frontend_action(&argv) {
        write_frontend_output(match action {
            FrontendAction::Help => HELP,
            FrontendAction::Version => VERSION,
        });
        return;
    }

    nsh_platform::restore_shell_process_runtime_state();
    let stdin_is_terminal = std::io::stdin().is_terminal();
    let stderr_is_terminal = std::io::stderr().is_terminal();
    let invocation = {
        let mut stdout = std::io::stdout().lock();
        invocation::Invocation::parse(&argv, stdin_is_terminal, stderr_is_terminal, |bytes| {
            stdout.write_all(bytes)
        })
    };
    let invocation = match invocation {
        Ok(invocation) => invocation,
        Err(invocation::ParseError::Invocation(message)) => {
            write_invocation_error(argv.first(), &message);
            std::process::exit(2);
        }
        Err(invocation::ParseError::Output) => std::process::exit(1),
    };

    let parameters: Vec<&BStr> = invocation
        .parameters
        .iter()
        .map(|parameter| parameter.as_bstr())
        .collect();
    let mut builder = nsh::Shell::builder()
        .invocation_name(invocation.invocation_name.as_bstr())
        .argument_zero(invocation.argument_zero.as_bstr())
        .args(&parameters)
        .inherit_env()
        .streams(nsh::Streams::INHERIT)
        .host(nsh::ProcessHost);
    for option in nsh::ShellOption::ALL {
        builder = builder.shell_option(option, invocation.options.enabled(option));
    }
    let startup = invocation.startup();
    let mut shell = match builder.build() {
        Ok(shell) => shell,
        Err(error) => std::process::exit(error.status().code().into()),
    };
    // The frontend is the thing entitled to the process's standard
    // descriptors, so it hands them to the shell explicitly rather than
    // letting the shell assume them. See [dec:nsh:host-owns-streams].
    // The library returns its status rather than ending the process:
    // [dec:nsh:host-owns-the-process] makes that the frontend's act, and
    // this is the frontend. `exitshell` has already flushed and torn down
    // job control, so there is nothing left to do but leave.
    let status = shell.run_to_completion(startup);
    std::process::exit(status.code().into());
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:cli.metadata-options/test]
    #[test]
    fn metadata_options_are_frontend_only() {
        let argv = vec![b"nsh".to_vec(), b"--help".to_vec()];
        assert_eq!(frontend_action(&argv), Some(FrontendAction::Help));
        let argv = vec![b"nsh".to_vec(), b"--version".to_vec()];
        assert_eq!(frontend_action(&argv), Some(FrontendAction::Version));
        let argv = vec![b"nsh".to_vec(), b"-h".to_vec()];
        assert_eq!(frontend_action(&argv), None);

        for argv in [
            vec![b"nsh".to_vec(), b"script".to_vec(), b"--help".to_vec()],
            vec![
                b"nsh".to_vec(),
                b"-c".to_vec(),
                b":".to_vec(),
                b"--version".to_vec(),
            ],
        ] {
            assert_eq!(frontend_action(&argv), None);
        }
    }
}
