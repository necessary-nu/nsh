//! Naming an open descriptor of this process as a path.
//!
//! Process substitution hands a program the *name* of a pipe the shell holds
//! open. Such a name exists only where the system publishes its descriptor
//! table in the file namespace, so every answer here is an `Option`: a system
//! without such a directory cannot name a pipe at all, and saying so is
//! better than returning a path that will not open.

use std::path::{Path, PathBuf};

use crate::Descriptor;

/// The directory in which this system names an open descriptor.
#[cfg(unix)]
pub fn descriptor_name_directory() -> Option<&'static Path> {
    static DIRECTORY: std::sync::OnceLock<Option<&'static Path>> = std::sync::OnceLock::new();
    *DIRECTORY.get_or_init(|| {
        [Path::new("/dev/fd"), Path::new("/proc/self/fd")]
            .into_iter()
            .find(|candidate| crate::path_is_directory(candidate))
    })
}

/// Windows publishes no descriptor-table directory, so no handle of this
/// process can be named as a path.
#[cfg(windows)]
pub fn descriptor_name_directory() -> Option<&'static Path> {
    None
}

/// The path naming one descriptor this process holds open.
///
/// The name is only as good as the descriptor: it resolves in this process,
/// and in any child that still holds the same number.
#[cfg(unix)]
pub fn descriptor_name(fd: &Descriptor) -> Option<PathBuf> {
    Some(descriptor_name_directory()?.join(fd.number().to_string()))
}

#[cfg(windows)]
pub fn descriptor_name(_fd: &Descriptor) -> Option<PathBuf> {
    None
}

/// Keep one descriptor open across the `exec` this process is about to make.
///
/// Every descriptor the shell owns is close-on-exec, so a program the shell
/// runs inherits only what it was given. Clearing that flag hands this one
/// descriptor to whatever image the calling process becomes, so it is
/// meaningful only at a process terminus where nothing will ever fork
/// again. Which descriptors a terminus publishes is the caller's decision,
/// not this function's.
///
/// It borrows rather than takes: the caller keeps the owner, and no
/// descriptor number crosses this boundary.
// [spec:nsh:req:compat.bash.safe-core]
#[cfg(unix)]
pub fn publish_descriptor_across_exec(fd: &Descriptor) -> std::io::Result<()> {
    let flags = rustix::io::fcntl_getfd(fd).map_err(std::io::Error::from)?;
    rustix::io::fcntl_setfd(fd, flags.difference(rustix::io::FdFlags::CLOEXEC))
        .map_err(std::io::Error::from)
}

#[cfg(windows)]
pub fn publish_descriptor_across_exec(_fd: &Descriptor) -> std::io::Result<()> {
    Err(std::io::Error::from(std::io::ErrorKind::Unsupported))
}

#[cfg(all(test, unix))]
mod tests {
    use super::*;

    fn shell_writing_to(name: &Path, word: &str) -> crate::ProgramImage {
        crate::ProgramImage::new(
            std::path::PathBuf::from("/bin/sh"),
            vec![
                std::ffi::OsString::from("sh"),
                std::ffi::OsString::from("-c"),
                std::ffi::OsString::from(format!("echo {word} >{}", name.display())),
            ],
            Vec::new(),
        )
    }

    #[test]
    fn a_name_reopens_the_same_stream() {
        let (read, write) = crate::pipe().unwrap();
        let name = descriptor_name(&read).expect("this system names descriptors");

        assert!(name.starts_with(descriptor_name_directory().unwrap()));
        crate::write_all(&write, b"named").unwrap();
        let reopened = crate::open_path(&name, crate::OpenMode::ReadOnly).unwrap();
        assert_eq!(crate::read_exact(&reopened, 5).unwrap(), b"named");
    }

    #[test]
    fn a_published_descriptor_survives_exec() {
        let (read, write) = crate::pipe().unwrap();
        let name = descriptor_name(&write).expect("this system names descriptors");
        let status = crate::run_in_child(move || {
            publish_descriptor_across_exec(&write).unwrap();
            drop(crate::execute_program(shell_writing_to(&name, "survived")));
            crate::exit_immediately(1);
        })
        .unwrap();

        assert_eq!(status, 0);
        assert_eq!(crate::read_exact(&read, 9).unwrap(), b"survived\n");
    }

    /// The default is the opposite, and that is the point: an unpublished
    /// descriptor is gone by the time the program starts, so its name names
    /// nothing and the program fails rather than writing somewhere else.
    #[test]
    fn an_unpublished_descriptor_is_gone() {
        let (read, write) = crate::pipe().unwrap();
        let name = descriptor_name(&write).expect("this system names descriptors");
        let status = crate::run_in_child(move || {
            drop(crate::execute_program(shell_writing_to(&name, "leaked")));
            crate::exit_immediately(1);
        })
        .unwrap();

        assert_ne!(status, 0);
        drop(write);
        assert_eq!(crate::read_to_end(&read).unwrap(), b"");
    }
}
