//! The drop-in directories a package and an administrator may write to.
//!
//! The chain this shell inherited from `sh` -- `/etc/profile`, then
//! `$HOME/.profile`, then `$ENV` -- has no place a *package* may write.
//! The first two are the administrator's file and the user's, and `$ENV`
//! is a single pathname the user chooses, so two integrations that both
//! want it means one of them loses. The two directories here are the
//! answer to that: one under `$XDG_DATA_DIRS` a package manager owns, and
//! one under `/etc` the administrator owns, read second so a local setting
//! wins.
//!
//! Only `/etc/nsh/conf.d` is written down, because the rule fixes it. The
//! vendor search path is read out of the environment on every startup: a
//! path fixed at build time is a reader that cannot be taught where it is
//! looking, which is the thing that forces a package into `/etc` in the
//! first place.

use bstr::{BStr, BString};
use nsh_platform::{NativeStrExt as _, ShellBytesExt as _};

use crate::context::Shell;
use crate::error::Error;
use crate::evaluation::Flow;

/// Where `$XDG_DATA_DIRS` points when the environment does not say.
const DEFAULT_DATA_DIRS: &[u8] = b"/usr/local/share:/usr/share";

/// What a package owns, beneath each entry of `$XDG_DATA_DIRS`.
const VENDOR_SUFFIX: &[u8] = b"/nsh/vendor_conf.d";

/// What the administrator owns. Read after every vendor directory, which
/// is the whole of how a local setting overrides a packaged one.
const ADMINISTRATOR_DIRECTORY: &[u8] = b"/etc/nsh/conf.d";

/// The directories to read, in the order they are read.
///
/// An entry of `$XDG_DATA_DIRS` that is not absolute is skipped rather
/// than resolved against the working directory. That is what the XDG base
/// directory specification says to do with one, and here it is also the
/// difference between a shell that reads startup code from a fixed place
/// and one that reads it from wherever a terminal happened to open.
// [spec:nsh:req:interactive.vendor-startup-path]
pub(super) fn search_path(shell: &mut Shell) -> Vec<BString> {
    let configured = crate::variables::lookup_bytes(shell, BStr::new(b"XDG_DATA_DIRS"))
        .filter(|value| !value.is_empty());
    let data_dirs = configured.unwrap_or_else(|| BString::from(DEFAULT_DATA_DIRS));

    let mut directories: Vec<BString> = data_dirs
        .split(|byte| *byte == b':')
        .filter(|entry| entry.first() == Some(&b'/'))
        .map(|entry| {
            let mut directory = BString::from(entry);
            directory.extend_from_slice(VENDOR_SUFFIX);
            directory
        })
        .collect();
    directories.push(BString::from(ADMINISTRATOR_DIRECTORY));
    directories
}

/// Read every drop-in, vendor directories first and the administrator's
/// last.
///
/// The caller decides *whether* to read them at all; this decides only
/// what and in what order. A file is read through the same path
/// `/etc/profile` is read through, so an `exit` in a drop-in ends the
/// shell and a file that cannot be opened is passed over, exactly as in a
/// profile.
///
/// A failure inside a drop-in leaves through `?` and is recovered where
/// every other startup failure is, by moving to the next startup task --
/// so the rest of the sweep is abandoned rather than resumed. That is the
/// same bargain the profile chain makes: a broken `/etc/profile` does not
/// half-run.
pub(super) fn read_all(shell: &mut Shell) -> Result<Flow, Error> {
    for directory in search_path(shell) {
        for file in files_in(shell, BStr::new(&directory)) {
            let flow = super::read_startup_file(shell, BStr::new(&file))?;
            if !matches!(flow, Flow::Done(_)) {
                return Ok(flow);
            }
        }
    }
    Ok(Flow::Done((0).into()))
}

/// The regular files in one directory, by name, each as a full path.
///
/// Regular *after* following a symlink, which decides three cases the rule
/// leaves to the reader and that a startup mechanism is judged on. A
/// symlink to a file is read, because that is how a package manager
/// installs one. A dangling symlink is skipped, because there is nothing
/// to read. A symlink to `/dev/null` is skipped, which is the idiom for
/// masking a drop-in without deleting the package's copy of it.
///
/// Sorting is by the entry name and not by the full path: within one
/// directory the two agree, and it is the name a package chooses when it
/// puts itself before or after another.
fn files_in(shell: &mut Shell, directory: &BStr) -> Vec<BString> {
    let Ok(path) = directory.try_to_path_buf() else {
        return Vec::new();
    };
    let entries = match nsh_platform::read_directory(&path) {
        Ok(entries) => entries,
        Err(error) => {
            report_unreadable(shell, directory, &error);
            return Vec::new();
        }
    };

    let mut names: Vec<Vec<u8>> = entries
        .into_iter()
        .map(|entry| entry.name.to_shell_bytes())
        .collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let mut file = BString::from(directory.to_vec());
            file.push(b'/');
            file.extend_from_slice(&name);
            file
        })
        .filter(|file| {
            file.try_to_path_buf()
                .is_ok_and(|file| nsh_platform::path_is_file(&file))
        })
        .collect()
}

/// Say so when a directory is there and will not open.
///
/// A directory that is not there is the ordinary case on a machine with no
/// packages installed, and saying so on every startup is how a startup
/// mechanism becomes something people turn off. One that exists and
/// refuses is a misconfiguration, and the only way anyone learns their
/// drop-ins are not running is being told.
///
/// `PathErrorKind::NotFound` covers `ENOTDIR` as well as `ENOENT`, and
/// that is wanted here: `/etc/nsh` existing as a file means the conf
/// directory does not exist, not that it cannot be read.
fn report_unreadable(shell: &mut Shell, directory: &BStr, error: &std::io::Error) {
    if nsh_platform::is_path_error(error, nsh_platform::PathErrorKind::NotFound) {
        return;
    }
    let mut message = directory.to_vec();
    message.extend_from_slice(b": ");
    message.extend_from_slice(shell.locale.error_message(error).as_bytes());
    shell.diagnostics().shell_warning(&message);
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The search path a shell whose `$XDG_DATA_DIRS` is `value` would
    /// read.
    ///
    /// The unset case unsets rather than trusting a fresh shell to have
    /// nothing: this process is a `cargo test` run, and it inherits
    /// whatever the machine exports.
    fn search_path_with(value: Option<&[u8]>) -> Vec<BString> {
        let _guard = crate::test_support::lock();
        let mut owned = crate::context::Shell::new(crate::streams::Streams::INHERIT);
        let shell = &mut owned;
        let name = BStr::new(b"XDG_DATA_DIRS");
        match value {
            Some(value) => crate::variables::set_bytes(
                shell,
                name,
                Some(BStr::new(value)),
                crate::variables::VariableAttributes::NONE,
            )
            .expect("a plain assignment cannot fail"),
            None => crate::variables::unset_bytes(shell, name).expect("unsetting cannot fail"),
        }
        search_path(shell)
    }

    // [spec:nsh:req:interactive.vendor-startup-path/test]
    #[test]
    fn an_unset_search_path_takes_the_documented_default() {
        assert_eq!(
            search_path_with(None),
            vec![
                BString::from("/usr/local/share/nsh/vendor_conf.d"),
                BString::from("/usr/share/nsh/vendor_conf.d"),
                BString::from("/etc/nsh/conf.d"),
            ]
        );
    }

    /* An empty value is the default too, and it is worth its own case:
     * `XDG_DATA_DIRS=` is what a `env -i` or a stripped systemd unit
     * leaves behind, and treating it as "no vendor directories" rather
     * than as "unset" would make a package's drop-in disappear in exactly
     * the environments that are hardest to look at. */
    // [spec:nsh:req:interactive.vendor-startup-path/test]
    #[test]
    fn an_empty_search_path_takes_the_documented_default() {
        assert_eq!(search_path_with(Some(b"")), search_path_with(None));
    }

    // [spec:nsh:req:interactive.vendor-startup-path/test]
    #[test]
    fn the_administrator_directory_is_read_last() {
        assert_eq!(
            search_path_with(Some(b"/one:/two")),
            vec![
                BString::from("/one/nsh/vendor_conf.d"),
                BString::from("/two/nsh/vendor_conf.d"),
                BString::from("/etc/nsh/conf.d"),
            ]
        );
    }

    /* The XDG specification says a relative entry must be ignored. Here
     * that is also what keeps a shell started in an unpleasant directory
     * from sourcing every file under a `nsh/vendor_conf.d` beneath it. */
    // [spec:nsh:req:interactive.vendor-startup-path/test]
    #[test]
    fn a_relative_search_path_entry_is_ignored() {
        assert_eq!(
            search_path_with(Some(b".:relative:/absolute")),
            vec![
                BString::from("/absolute/nsh/vendor_conf.d"),
                BString::from("/etc/nsh/conf.d"),
            ]
        );
    }
}
