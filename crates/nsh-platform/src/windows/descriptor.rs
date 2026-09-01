//! The owned endpoint, and the shell descriptor number it answers to.
//!
//! Windows has handles; a shell script has numbers -- `exec 7>file` --
//! and the two have to be the same object. A [`Descriptor`] therefore
//! carries both: an owned handle, which is the real thing, and a number
//! allocated above the standard three, which means something only inside
//! a tree of nsh processes and is deliberately not a CRT file
//! descriptor. Duplication is where the pairing is maintained, so every
//! way of making a second owner for one endpoint lives here.

use std::fs::File;
use std::os::windows::io::{AsHandle, AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle};
use std::sync::atomic::{AtomicU32, Ordering as AtomicOrdering};

use windows_sys::Win32::Foundation::{
    CloseHandle, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE, HANDLE_FLAG_INHERIT,
    INVALID_HANDLE_VALUE, SetHandleInformation,
};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

#[derive(Debug)]
pub struct Descriptor {
    handle: OwnedHandle,
    number: i32,
}

#[derive(Clone, Copy)]
pub struct BorrowedDescriptor<'a>(BorrowedHandle<'a>);

pub trait AsDescriptor {
    #[doc(hidden)]
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_>;
}

impl AsDescriptor for Descriptor {
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_> {
        BorrowedDescriptor(self.handle.as_handle())
    }
}

impl AsDescriptor for &Descriptor {
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_> {
        BorrowedDescriptor(self.handle.as_handle())
    }
}

impl<T: AsHandle> AsDescriptor for T {
    fn as_platform_descriptor(&self) -> BorrowedDescriptor<'_> {
        BorrowedDescriptor(self.as_handle())
    }
}

static NEXT_DESCRIPTOR: AtomicU32 = AtomicU32::new(10);

fn descriptor_number(minimum: i32) -> std::io::Result<i32> {
    if minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let minimum = u32::try_from(minimum.max(10))
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    let mut current = NEXT_DESCRIPTOR.load(AtomicOrdering::Relaxed);
    loop {
        let number = current.max(minimum);
        let next = number.checked_add(1).ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::OutOfMemory,
                "descriptor namespace exhausted",
            )
        })?;
        match NEXT_DESCRIPTOR.compare_exchange_weak(
            current,
            next,
            AtomicOrdering::Relaxed,
            AtomicOrdering::Relaxed,
        ) {
            Ok(_) => return i32::try_from(number).map_err(std::io::Error::other),
            Err(observed) => current = observed,
        }
    }
}

pub(super) fn owned_handle(raw: HANDLE, minimum: i32) -> std::io::Result<Descriptor> {
    if raw.is_null() || raw == INVALID_HANDLE_VALUE {
        return Err(std::io::Error::last_os_error());
    }
    // RtlCloneUserProcess only copies inheritable handles. Descriptor
    // lifetime remains owned here; this flag exists for the clone boundary,
    // while normal image creation is constrained by the standard-handle list.
    // SAFETY: `raw` is a live handle and both flag arguments are valid.
    if unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: failure leaves the freshly returned handle owned here.
        unsafe { CloseHandle(raw) };
        return Err(error);
    }
    // SAFETY: a successful Windows creation/duplication API returned a fresh
    // owned handle which is transferred into `OwnedHandle` exactly once.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.cast()) };
    Ok(Descriptor {
        handle,
        number: descriptor_number(minimum)?,
    })
}

pub(super) fn raw_handle(fd: &impl AsDescriptor) -> HANDLE {
    fd.as_platform_descriptor().0.as_raw_handle() as HANDLE
}

pub(super) fn descriptor_from_file(file: File, minimum: i32) -> std::io::Result<Descriptor> {
    let raw = file.as_raw_handle() as HANDLE;
    // SAFETY: `file` keeps the handle live and both flag values are valid.
    if unsafe { SetHandleInformation(raw, HANDLE_FLAG_INHERIT, HANDLE_FLAG_INHERIT) } == 0 {
        return Err(std::io::Error::last_os_error());
    }
    let handle: OwnedHandle = file.into();
    Ok(Descriptor {
        handle,
        number: descriptor_number(minimum)?,
    })
}

pub(super) fn duplicate_at(fd: &impl AsDescriptor, minimum: i32) -> std::io::Result<Descriptor> {
    if minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: source and target process pseudo-handles are always valid, the
    // source is borrowed for this call, and `duplicate` is writable.
    let succeeded = unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            raw_handle(fd),
            GetCurrentProcess(),
            &mut duplicate,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    };
    if succeeded == 0 {
        Err(std::io::Error::last_os_error())
    } else {
        owned_handle(duplicate, minimum)
    }
}

impl Descriptor {
    // [spec:nsh:req:idiom.no-raw-fd-core]
    pub(crate) fn number(&self) -> i32 {
        self.number
    }

    pub fn into_file(self) -> File {
        self.handle.into()
    }
}

pub fn duplicate_cloexec(fd: &impl AsDescriptor, minimum: i32) -> std::io::Result<Descriptor> {
    duplicate_at(fd, minimum)
}

pub fn duplicate_fd(fd: &impl AsDescriptor) -> std::io::Result<Descriptor> {
    duplicate_at(fd, 0)
}

pub fn duplicate_file(fd: &impl AsDescriptor) -> std::io::Result<File> {
    duplicate_at(fd, 0).map(Descriptor::into_file)
}

pub fn move_fd_cloexec(fd: Descriptor, minimum: i32) -> std::io::Result<Descriptor> {
    if minimum < 0 {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    if fd.number() >= minimum {
        Ok(fd)
    } else {
        duplicate_at(&fd, minimum)
    }
}
