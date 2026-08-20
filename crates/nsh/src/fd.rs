//! Per-shell logical descriptors.
//!
//! Shell syntax names descriptor *slots*. Those slots are state owned by a
//! [`Shell`](crate::context::Shell), not borrowed views of the host process's
//! descriptor table. Each open slot shares an owned, close-on-exec backing
//! descriptor above the shell-language range. Redirection changes the slot;
//! dropping the last handle closes the backing descriptor automatically.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard};

use nsh_platform::{AsDescriptor, BorrowedDescriptor, Descriptor};

/// A validated descriptor identity in the shell language.
///
/// This is an index into a shell-owned table, never an operating-system
/// handle. Constructing one does not acquire ownership and dropping one does
/// not close anything.
// [spec:nsh:def:idiom.logical-descriptors]
#[derive(Clone, Copy, Debug, Eq, Hash, Ord, PartialEq, PartialOrd)]
pub(crate) struct LogicalDescriptor(u8);

impl LogicalDescriptor {
    pub(crate) const COUNT: usize = 10;
    pub(crate) const STDIN: Self = Self(0);
    pub(crate) const STDOUT: Self = Self(1);
    pub(crate) const STDERR: Self = Self(2);

    pub(crate) const fn new(number: i32) -> Option<Self> {
        if number >= 0 && number < Self::COUNT as i32 {
            Some(Self(number as u8))
        } else {
            None
        }
    }

    pub(crate) const fn from_index(index: usize) -> Option<Self> {
        if index < Self::COUNT {
            Some(Self(index as u8))
        } else {
            None
        }
    }

    pub(crate) const fn from_digit(digit: u8) -> Option<Self> {
        if digit >= b'0' && digit <= b'9' {
            Self::new((digit - b'0') as i32)
        } else {
            None
        }
    }

    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) const fn as_i32(self) -> i32 {
        self.0 as i32
    }

    pub(crate) const fn as_digit(self) -> u8 {
        b'0' + self.0
    }
}

impl fmt::Display for LogicalDescriptor {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(formatter)
    }
}

/// Shared ownership of one open file description.
///
/// Sharing models `2>&1`: the two logical slots are independently replaceable
/// while retaining the same underlying open file description and offset.
// [spec:nsh:req:idiom.no-raw-fd-core]
#[derive(Clone, Debug)]
pub(crate) struct SharedFd(Arc<Descriptor>);

impl SharedFd {
    /// Move an owned descriptor into the shell's hidden backing range.
    pub(crate) fn from_owned(fd: Descriptor) -> std::io::Result<Self> {
        let fd = nsh_platform::move_fd_cloexec(fd, LogicalDescriptor::COUNT as i32)?;
        Ok(Self(Arc::new(fd)))
    }
}

impl From<Descriptor> for SharedFd {
    fn from(fd: Descriptor) -> Self {
        Self(Arc::new(fd))
    }
}

impl AsDescriptor for SharedFd {
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_> {
        self.0.as_platform_descriptor()
    }
}

/// A stable reference to one logical descriptor slot.
///
/// Writers retain this reference, so `echo >file` changes where an existing
/// buffered `Output` writes without changing or borrowing the host's process
/// descriptor table.
#[derive(Clone, Debug, Default)]
pub(crate) struct FdRef(Arc<Mutex<Option<SharedFd>>>);

impl FdRef {
    fn lock(&self) -> MutexGuard<'_, Option<SharedFd>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn get(&self) -> Option<SharedFd> {
        self.lock().clone()
    }

    pub(crate) fn replace(&self, value: Option<SharedFd>) -> Option<SharedFd> {
        std::mem::replace(&mut *self.lock(), value)
    }

    pub(crate) fn is_open(&self) -> bool {
        self.lock().is_some()
    }

    // [spec:nsh:req:idiom.platform-errors]
    pub(crate) fn write_once(&self, bytes: &[u8]) -> std::io::Result<usize> {
        let fd = self.get().ok_or_else(|| {
            nsh_platform::platform_error(nsh_platform::PlatformErrorKind::BadDescriptor)
        })?;
        nsh_platform::write_once(&fd, bytes)
    }

    pub(crate) fn write_all(&self, bytes: &[u8]) -> std::io::Result<()> {
        let fd = self.get().ok_or_else(|| {
            nsh_platform::platform_error(nsh_platform::PlatformErrorKind::BadDescriptor)
        })?;
        nsh_platform::write_all(&fd, bytes)
    }
}

/// All descriptor slots in one shell execution environment.
#[derive(Debug)]
pub(crate) struct FdTable {
    slots: [FdRef; LogicalDescriptor::COUNT],
}

impl FdTable {
    pub(crate) fn from_streams(streams: &crate::streams::Streams) -> std::io::Result<Self> {
        let initial = streams.initial_descriptors()?;
        Ok(Self {
            slots: std::array::from_fn(|number| {
                let slot = FdRef::default();
                slot.replace(initial[number].clone());
                slot
            }),
        })
    }

    pub(crate) fn slot(&self, descriptor: LogicalDescriptor) -> FdRef {
        self.slots[descriptor.index()].clone()
    }

    pub(crate) fn get(&self, descriptor: LogicalDescriptor) -> Option<SharedFd> {
        self.slot(descriptor).get()
    }

    pub(crate) fn replace(
        &self,
        descriptor: LogicalDescriptor,
        value: Option<SharedFd>,
    ) -> Option<SharedFd> {
        self.slot(descriptor).replace(value)
    }

    pub(crate) fn install_owned(
        &self,
        descriptor: LogicalDescriptor,
        fd: Descriptor,
    ) -> std::io::Result<Option<SharedFd>> {
        Ok(self.replace(descriptor, Some(SharedFd::from_owned(fd)?)))
    }

    pub(crate) fn is_open(&self, descriptor: LogicalDescriptor) -> bool {
        self.slot(descriptor).is_open()
    }

    /// Install this shell's logical table into exact process slots.
    ///
    /// This is the only route from logical descriptor state to process-wide
    /// state. It is called at the process terminus immediately before exec.
    // [spec:nsh:req:idiom.descriptor-materialization]
    pub(crate) fn materialize(&self) -> std::io::Result<()> {
        let mut changes = Vec::with_capacity(LogicalDescriptor::COUNT);
        for (number, slot) in self.slots.iter().enumerate() {
            let source = match slot.get() {
                Some(fd) => Some(nsh_platform::duplicate_cloexec(
                    &fd,
                    LogicalDescriptor::COUNT as i32,
                )?),
                None => None,
            };
            let descriptor = LogicalDescriptor::from_index(number)
                .expect("the table contains only logical descriptors");
            changes.push((descriptor.as_i32(), source));
        }
        nsh_platform::ProcessDescriptorTransaction::new(changes)?.apply()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:idiom.filesystem-account-bytes/test]
    fn shared(name: &str) -> SharedFd {
        SharedFd::from_owned(nsh_platform::anonymous_file(name).unwrap()).unwrap()
    }

    fn empty_table() -> FdTable {
        FdTable {
            slots: std::array::from_fn(|_| FdRef::default()),
        }
    }

    fn detached(fd: SharedFd) -> FdRef {
        let slot = FdRef::default();
        slot.replace(Some(fd));
        slot
    }

    // [spec:nsh:def:idiom.logical-descriptors/test]
    #[test]
    fn descriptor_identity_is_validated() {
        assert_eq!(LogicalDescriptor::new(0), Some(LogicalDescriptor::STDIN));
        assert_eq!(LogicalDescriptor::new(9).unwrap().as_digit(), b'9');
        assert!(LogicalDescriptor::new(-1).is_none());
        assert!(LogicalDescriptor::new(LogicalDescriptor::COUNT as i32).is_none());
        assert!(LogicalDescriptor::from_digit(b'x').is_none());
    }

    #[test]
    fn slot_refs_follow_replacement() {
        let table = empty_table();
        let descriptor = LogicalDescriptor::new(4).unwrap();
        let retained = table.slot(descriptor);
        let first = shared("fd-first");
        let second = shared("fd-second");

        assert!(retained.get().is_none());
        table.replace(descriptor, Some(first.clone()));
        assert!(Arc::ptr_eq(&retained.get().unwrap().0, &first.0));
        table.replace(descriptor, Some(second.clone()));
        assert!(Arc::ptr_eq(&retained.get().unwrap().0, &second.0));
    }

    #[test]
    fn replacement_returns_saved_value() {
        let table = empty_table();
        let first = shared("fd-saved");
        let second = shared("fd-current");
        let descriptor = LogicalDescriptor::new(6).unwrap();
        table.replace(descriptor, Some(first.clone()));

        let saved = table.replace(descriptor, Some(second.clone())).unwrap();

        assert!(Arc::ptr_eq(&saved.0, &first.0));
        assert!(Arc::ptr_eq(&table.get(descriptor).unwrap().0, &second.0));
    }

    #[test]
    fn logical_dup_survives_replacement() {
        let original = shared("fd-dup-original");
        let replacement = shared("fd-dup-replacement");
        let left = detached(original.clone());
        let right = detached(original.clone());

        left.replace(Some(replacement.clone()));

        assert!(Arc::ptr_eq(&left.get().unwrap().0, &replacement.0));
        assert!(Arc::ptr_eq(&right.get().unwrap().0, &original.0));
    }

    #[test]
    fn writes_follow_current_slot() {
        let first = shared("fd-write-first");
        let second = shared("fd-write-second");
        let slot = detached(first.clone());

        slot.write_all(b"before").unwrap();
        slot.replace(Some(second.clone()));
        slot.write_all(b"after").unwrap();

        assert_eq!(nsh_platform::take_file_contents(&first).unwrap(), b"before");
        assert_eq!(nsh_platform::take_file_contents(&second).unwrap(), b"after");
    }

    #[test]
    fn closed_slot_reports_bad_descriptor() {
        let slot = FdRef::default();

        let error = slot.write_all(b"nowhere").unwrap_err();

        assert!(nsh_platform::is_bad_descriptor_error(&error));
        assert!(!slot.is_open());
    }

    #[test]
    // [spec:nsh:req:idiom.no-raw-fd-core/test]
    fn owned_descriptor_remains_usable_after_hiding() {
        let source = nsh_platform::open_null_input().unwrap();
        let shared = SharedFd::from_owned(source).unwrap();

        assert_eq!(nsh_platform::read_to_end(&shared).unwrap(), b"");
    }

    #[test]
    fn install_owned_replaces_slot() {
        let table = empty_table();
        let original = shared("fd-install-original");
        let descriptor = LogicalDescriptor::new(5).unwrap();
        table.replace(descriptor, Some(original.clone()));
        let replacement = nsh_platform::anonymous_file("fd-install-new").unwrap();

        let saved = table
            .install_owned(descriptor, replacement)
            .unwrap()
            .unwrap();

        assert!(Arc::ptr_eq(&saved.0, &original.0));
        assert!(!Arc::ptr_eq(&table.get(descriptor).unwrap().0, &original.0));
    }

    #[test]
    fn poisoned_slot_keeps_its_state() {
        let value = shared("fd-poison");
        let slot = detached(value.clone());
        let other = slot.clone();
        let result = std::thread::spawn(move || {
            let _held = other.0.lock().unwrap();
            panic!("poison the test mutex");
        })
        .join();

        assert!(result.is_err());
        assert!(Arc::ptr_eq(&slot.get().unwrap().0, &value.0));
    }

    #[test]
    fn materialize_installs_complete_map() {
        let (read, write) = nsh_platform::pipe().unwrap();
        let status = nsh_platform::run_in_child(move || {
            let table = empty_table();
            let seven = LogicalDescriptor::new(7).unwrap();
            let eight = LogicalDescriptor::new(8).unwrap();
            table.install_owned(seven, write).unwrap();
            table.materialize().unwrap();

            let seven =
                nsh_platform::snapshot_process_fd(seven.as_i32(), LogicalDescriptor::COUNT as i32)
                    .unwrap()
                    .unwrap();
            nsh_platform::write_all(&seven, b"logical").unwrap();
            if nsh_platform::snapshot_process_fd(eight.as_i32(), LogicalDescriptor::COUNT as i32)
                .unwrap()
                .is_some()
            {
                nsh_platform::exit_immediately(2);
            }
            nsh_platform::exit_immediately(0);
        })
        .unwrap();

        assert_eq!(status, 0);
        assert_eq!(nsh_platform::read_exact(&read, 7).unwrap(), b"logical");
    }
}
