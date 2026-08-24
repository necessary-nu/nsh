//! Per-shell logical descriptors.
//!
//! Shell syntax names descriptor *slots*. Those slots are state owned by a
//! [`Shell`](crate::context::Shell), not borrowed views of the host process's
//! descriptor table. Each open slot shares an owned, close-on-exec backing
//! descriptor above the shell-language range. Redirection changes the slot;
//! dropping the last handle closes the backing descriptor automatically.

use std::collections::BTreeMap;
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
pub(crate) struct LogicalDescriptor(u32);

impl LogicalDescriptor {
    /// How many descriptors the shell snapshots from the process at
    /// startup, and the floor its own backing descriptors are moved above.
    ///
    /// Two jobs in one number, and they are the same number for a reason:
    /// everything below the floor is a slot a script could be naming, and
    /// everything the shell holds for itself has to sit above whatever it
    /// might be asked to install. Ten is where the *inherited* range ends,
    /// not where the *nameable* range does -- POSIX's IO_NUMBER is "a
    /// string consisting solely of digits" and `[spec:posix:syn:redir.format]`
    /// says "one or more digit", so `exec 42>file` names slot 42. Slots
    /// past the inherited range are created on demand, and
    /// [`DescriptorTable::materialize`] raises the floor above the highest
    /// one in use before it duplicates anything.
    pub(crate) const INHERITED: usize = 10;
    pub(crate) const STDIN: Self = Self(0);
    pub(crate) const STDOUT: Self = Self(1);
    pub(crate) const STDERR: Self = Self(2);

    /// No upper bound of our own. POSIX leaves the maximum
    /// implementation-defined, and the honest maximum is the one the host
    /// enforces: `exec 1000000>file` fails at `dup2` with the same "bad
    /// file descriptor" Bash reports, rather than at a constant invented
    /// here.
    pub(crate) const fn new(number: i32) -> Option<Self> {
        if number >= 0 {
            Some(Self(number as u32))
        } else {
            None
        }
    }

    pub(crate) const fn from_index(index: usize) -> Option<Self> {
        if index <= u32::MAX as usize {
            Some(Self(index as u32))
        } else {
            None
        }
    }

    /// Read an IO_NUMBER: a run of decimal digits, and nothing else.
    ///
    /// A run that will not fit is not a descriptor this shell can name, so
    /// it is no more a redirection than `abc>file` is -- the caller falls
    /// back to treating the text as an ordinary word.
    // [spec:posix:syn:redir.format]
    // [spec:posix:syn:grammar.token-classification]
    pub(crate) fn from_digits(digits: &[u8]) -> Option<Self> {
        if digits.is_empty() || !digits.iter().all(u8::is_ascii_digit) {
            return None;
        }
        digits
            .iter()
            .try_fold(0u32, |value, digit| {
                value.checked_mul(10)?.checked_add(u32::from(*digit - b'0'))
            })
            .map(Self)
    }

    pub(crate) const fn index(self) -> usize {
        self.0 as usize
    }

    pub(crate) const fn as_i32(self) -> i32 {
        self.0 as i32
    }

    /// The decimal text a job listing reprints this descriptor as.
    pub(crate) fn digits(self) -> Vec<u8> {
        self.0.to_string().into_bytes()
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
pub(crate) struct SharedDescriptor(Arc<Descriptor>);

impl SharedDescriptor {
    /// Move an owned descriptor into the shell's hidden backing range.
    pub(crate) fn from_owned(descriptor: Descriptor) -> std::io::Result<Self> {
        let descriptor =
            nsh_platform::move_fd_cloexec(descriptor, LogicalDescriptor::INHERITED as i32)?;
        Ok(Self(Arc::new(descriptor)))
    }
}

impl From<Descriptor> for SharedDescriptor {
    fn from(descriptor: Descriptor) -> Self {
        Self(Arc::new(descriptor))
    }
}

impl AsDescriptor for SharedDescriptor {
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
pub(crate) struct DescriptorSlot(Arc<Mutex<Option<SharedDescriptor>>>);

impl DescriptorSlot {
    fn lock(&self) -> MutexGuard<'_, Option<SharedDescriptor>> {
        self.0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    pub(crate) fn get(&self) -> Option<SharedDescriptor> {
        self.lock().clone()
    }

    pub(crate) fn replace(&self, value: Option<SharedDescriptor>) -> Option<SharedDescriptor> {
        std::mem::replace(&mut *self.lock(), value)
    }

    #[cfg(test)]
    pub(crate) fn is_open(&self) -> bool {
        self.lock().is_some()
    }

    // [spec:nsh:req:idiom.platform-errors]
    pub(crate) fn write_once(&self, bytes: &[u8]) -> std::io::Result<usize> {
        let descriptor = self.get().ok_or_else(|| {
            nsh_platform::platform_error(nsh_platform::PlatformErrorKind::BadDescriptor)
        })?;
        nsh_platform::write_once(&descriptor, bytes)
    }

    pub(crate) fn write_all(&self, bytes: &[u8]) -> std::io::Result<()> {
        let descriptor = self.get().ok_or_else(|| {
            nsh_platform::platform_error(nsh_platform::PlatformErrorKind::BadDescriptor)
        })?;
        nsh_platform::write_all(&descriptor, bytes)
    }
}

/// All descriptor slots in one shell execution environment.
///
/// Sparse, because the set of slots a script can name is the set of
/// decimal numbers and not a range: `exec 42>file` is an ordinary
/// redirection. The inherited range 0..[`LogicalDescriptor::INHERITED`] is
/// always present so that materialization keeps saying the same thing
/// about it -- an inherited descriptor the shell never touched is still
/// closed in the child if the slot is empty -- and anything above it
/// exists only once a script has named it.
#[derive(Debug)]
pub(crate) struct DescriptorTable {
    slots: Mutex<BTreeMap<LogicalDescriptor, DescriptorSlot>>,
}

impl DescriptorTable {
    pub(crate) fn from_streams(streams: &crate::streams::Streams) -> std::io::Result<Self> {
        let initial = streams.initial_descriptors()?;
        let mut slots = BTreeMap::new();
        for (number, value) in initial.iter().enumerate() {
            let descriptor =
                LogicalDescriptor::from_index(number).expect("the inherited range is in range");
            let slot = DescriptorSlot::default();
            slot.replace(value.clone());
            slots.insert(descriptor, slot);
        }
        Ok(Self {
            slots: Mutex::new(slots),
        })
    }

    fn locked(&self) -> MutexGuard<'_, BTreeMap<LogicalDescriptor, DescriptorSlot>> {
        self.slots
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
    }

    /// A stable handle to one slot, creating it if this is the first time
    /// the shell has named it.
    ///
    /// Creating is what makes the handle stable: a writer retains it, so
    /// `exec 42>file` followed by `echo >&42` has to reach the same slot.
    pub(crate) fn slot(&self, descriptor: LogicalDescriptor) -> DescriptorSlot {
        self.locked().entry(descriptor).or_default().clone()
    }

    /// What a slot holds, without bringing it into existence.
    ///
    /// Reading `>&99` must not grow the table by a slot nothing will ever
    /// write to, so this asks rather than creates.
    pub(crate) fn get(&self, descriptor: LogicalDescriptor) -> Option<SharedDescriptor> {
        self.locked().get(&descriptor).and_then(DescriptorSlot::get)
    }

    pub(crate) fn replace(
        &self,
        descriptor: LogicalDescriptor,
        value: Option<SharedDescriptor>,
    ) -> Option<SharedDescriptor> {
        self.slot(descriptor).replace(value)
    }

    pub(crate) fn install_owned(
        &self,
        target: LogicalDescriptor,
        owned: Descriptor,
    ) -> std::io::Result<Option<SharedDescriptor>> {
        Ok(self.replace(target, Some(SharedDescriptor::from_owned(owned)?)))
    }

    #[cfg(test)]
    pub(crate) fn is_open(&self, descriptor: LogicalDescriptor) -> bool {
        self.slot(descriptor).is_open()
    }

    /// Install this shell's logical table into exact process slots.
    ///
    /// This is the only route from logical descriptor state to process-wide
    /// state. It is called at the process terminus immediately before exec.
    // [spec:nsh:req:idiom.descriptor-materialization]
    pub(crate) fn materialize(&self) -> std::io::Result<()> {
        let entries: Vec<(LogicalDescriptor, DescriptorSlot)> = self
            .locked()
            .iter()
            .map(|(descriptor, slot)| (*descriptor, slot.clone()))
            .collect();
        /* Every source has to sit above every target, or installing one
         * slot would overwrite the descriptor another slot is about to be
         * installed from. With a fixed table that was the constant floor;
         * with a sparse one it is the highest slot actually in play, and
         * never below the floor, so the inherited range behaves exactly as
         * it did. */
        let floor = entries
            .iter()
            .map(|(descriptor, _)| descriptor.as_i32().saturating_add(1))
            .max()
            .unwrap_or(0)
            .max(LogicalDescriptor::INHERITED as i32);
        let mut changes = Vec::with_capacity(entries.len());
        for (descriptor, slot) in entries {
            let source = match slot.get() {
                Some(held) => Some(nsh_platform::duplicate_cloexec(&held, floor)?),
                None => None,
            };
            changes.push((descriptor.as_i32(), source));
        }
        nsh_platform::ProcessDescriptorTransaction::new(changes)?.apply()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // [spec:nsh:req:idiom.filesystem-account-bytes/test]
    fn shared(name: &str) -> SharedDescriptor {
        SharedDescriptor::from_owned(nsh_platform::anonymous_file(name).unwrap()).unwrap()
    }

    fn empty_table() -> DescriptorTable {
        let mut slots = BTreeMap::new();
        for number in 0..LogicalDescriptor::INHERITED {
            slots.insert(
                LogicalDescriptor::from_index(number).unwrap(),
                DescriptorSlot::default(),
            );
        }
        DescriptorTable {
            slots: Mutex::new(slots),
        }
    }

    fn detached(descriptor: SharedDescriptor) -> DescriptorSlot {
        let slot = DescriptorSlot::default();
        slot.replace(Some(descriptor));
        slot
    }

    // [spec:nsh:def:idiom.logical-descriptors/test]
    #[test]
    fn descriptor_identity_is_validated() {
        assert_eq!(LogicalDescriptor::new(0), Some(LogicalDescriptor::STDIN));
        assert_eq!(LogicalDescriptor::new(9).unwrap().digits(), b"9".to_vec());
        assert!(LogicalDescriptor::new(-1).is_none());
        /* Past the inherited range is a slot like any other: POSIX's
         * IO_NUMBER is a digit *string*, so `exec 42>file` names one. */
        // [spec:posix:syn:redir.format/test]
        let past = LogicalDescriptor::new(LogicalDescriptor::INHERITED as i32);
        assert_eq!(past, LogicalDescriptor::from_digits(b"10"));
        assert_eq!(past.unwrap().digits(), b"10".to_vec());
        assert_eq!(
            LogicalDescriptor::from_digits(b"007"),
            LogicalDescriptor::new(7),
        );
        assert!(LogicalDescriptor::from_digits(b"x").is_none());
        assert!(LogicalDescriptor::from_digits(b"").is_none());
        assert!(LogicalDescriptor::from_digits(b"1a").is_none());
        // A run that cannot be held is not an IO_NUMBER, so it stays a word.
        assert!(LogicalDescriptor::from_digits(b"99999999999999999999").is_none());
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
        let slot = DescriptorSlot::default();

        let error = slot.write_all(b"nowhere").unwrap_err();

        assert!(nsh_platform::is_bad_descriptor_error(&error));
        assert!(!slot.is_open());
    }

    #[test]
    // [spec:nsh:req:idiom.no-raw-fd-core/test]
    fn owned_descriptor_remains_usable_after_hiding() {
        let source = nsh_platform::open_null_input().unwrap();
        let shared = SharedDescriptor::from_owned(source).unwrap();

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

            let seven = nsh_platform::snapshot_process_fd(
                seven.as_i32(),
                LogicalDescriptor::INHERITED as i32,
            )
            .unwrap()
            .unwrap();
            nsh_platform::write_all(&seven, b"logical").unwrap();
            if nsh_platform::snapshot_process_fd(
                eight.as_i32(),
                LogicalDescriptor::INHERITED as i32,
            )
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
