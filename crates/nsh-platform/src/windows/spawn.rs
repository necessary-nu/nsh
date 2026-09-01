//! Creating a Windows process from a program image.
//!
//! `CreateProcessW` wants one command line rather than an argument
//! vector and one environment block rather than pairs, so most of this
//! module is the two encoders that build them and the attribute list
//! that names exactly which handles the child may inherit. The quoting
//! rule in `append_windows_argument` is the one the C runtime parses
//! back, and it is the only place in the crate that has to know it.
//!
//! A POSIX `exec` cannot be spelled here at all: Windows will not
//! replace a running image. `execute_program` therefore spawns, waits,
//! and exits with the child's status, which is what the caller of an
//! `exec` observes anyway -- and if this process is a clone, it hands
//! the whole request to the broker instead, because a clone may not
//! create processes itself.

use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::path::Path;

use windows_sys::Win32::Foundation::{CloseHandle, HANDLE, INVALID_HANDLE_VALUE, WAIT_OBJECT_0};
use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
use windows_sys::Win32::System::Threading::{
    CREATE_UNICODE_ENVIRONMENT, CreateProcessW, DeleteProcThreadAttributeList,
    EXTENDED_STARTUPINFO_PRESENT, GetExitCodeProcess, INFINITE, InitializeProcThreadAttributeList,
    LPPROC_THREAD_ATTRIBUTE_LIST, PROC_THREAD_ATTRIBUTE_HANDLE_LIST, PROCESS_INFORMATION,
    ResumeThread, STARTF_USESTDHANDLES, STARTUPINFOEXW, STARTUPINFOW, TerminateProcess,
    UpdateProcThreadAttribute, WaitForSingleObject,
};

use super::*;

pub fn execute_program(program: crate::ProgramImage) -> std::io::Error {
    if let Some(result) = execute_through_clone_broker(
        program.path.as_os_str(),
        &program.arguments,
        &program.environment,
    ) {
        return match result {
            Ok(code) => exit_immediately(code as i32),
            Err(error) => error,
        };
    }
    execute_program_here(
        program.path.as_os_str(),
        &program.arguments,
        &program.environment,
        materialized_standard_handles(),
        None,
    )
}

fn execute_program_here(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
    handles: [HANDLE; 3],
    job: Option<HANDLE>,
) -> std::io::Error {
    match spawn_program_here(path, argv, environment, handles, job) {
        Ok(code) => exit_immediately(code as i32),
        Err(error) => error,
    }
}

pub(super) fn spawn_program_here(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
    handles: [HANDLE; 3],
    job: Option<HANDLE>,
) -> std::io::Result<u32> {
    let is_batch = Path::new(path).extension().is_some_and(|extension| {
        let extension = extension.to_string_lossy();
        extension.eq_ignore_ascii_case("bat") || extension.eq_ignore_ascii_case("cmd")
    });
    let application_path = if is_batch {
        environment
            .iter()
            .find(|(name, _)| name.to_string_lossy().eq_ignore_ascii_case("COMSPEC"))
            .map_or_else(|| OsString::from("cmd.exe"), |(_, value)| value.clone())
    } else {
        path.to_os_string()
    };
    let batch_path = is_batch.then(|| {
        OsString::from_wide(
            &path
                .encode_wide()
                .map(|unit| {
                    if unit == u16::from(b'/') {
                        u16::from(b'\\')
                    } else {
                        unit
                    }
                })
                .collect::<Vec<_>>(),
        )
    });
    let mut application: Vec<u16> = application_path.encode_wide().collect();
    if application.contains(&0) {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    application.push(0);

    let mut command_line = Vec::new();
    if is_batch {
        append_windows_argument(&mut command_line, &application_path)?;
        for option in ["/d", "/s", "/c"] {
            command_line.push(u16::from(b' '));
            command_line.extend(option.encode_utf16());
        }
        command_line.extend([u16::from(b' '), u16::from(b'"')]);
        append_windows_argument(
            &mut command_line,
            batch_path
                .as_deref()
                .expect("batch path exists for a batch file"),
        )?;
        for argument in argv.iter().skip(1) {
            command_line.push(u16::from(b' '));
            append_windows_argument(&mut command_line, argument)?;
        }
        command_line.push(u16::from(b'"'));
    } else {
        for (index, argument) in argv.iter().enumerate() {
            if index != 0 {
                command_line.push(u16::from(b' '));
            }
            append_windows_argument(&mut command_line, argument)?;
        }
    }
    command_line.push(0);

    let environment = match windows_environment_block(environment) {
        Ok(environment) => environment,
        Err(error) => return Err(error),
    };
    let mut inherited_handles: Vec<_> = handles
        .into_iter()
        .filter(|handle| !handle.is_null() && *handle != INVALID_HANDLE_VALUE)
        .collect();
    inherited_handles.sort_unstable_by_key(|handle| *handle as usize);
    inherited_handles.dedup();
    let attribute_list = if inherited_handles.is_empty() {
        None
    } else {
        Some(ProcessAttributeList::for_handles(&inherited_handles)?)
    };
    let mut startup = STARTUPINFOEXW::default();
    startup.StartupInfo = STARTUPINFOW {
        cb: if attribute_list.is_some() {
            std::mem::size_of::<STARTUPINFOEXW>() as u32
        } else {
            std::mem::size_of::<STARTUPINFOW>() as u32
        },
        dwFlags: STARTF_USESTDHANDLES,
        hStdInput: handles[0],
        hStdOutput: handles[1],
        hStdError: handles[2],
        ..STARTUPINFOW::default()
    };
    startup.lpAttributeList = attribute_list
        .as_ref()
        .map_or(std::ptr::null_mut(), |list| list.pointer);
    let mut information = PROCESS_INFORMATION::default();
    // SAFETY: application, command line, and environment are terminated and
    // live for the call; the command line is writable as CreateProcessW
    // requires; startup references live inheritable handles; and the process
    // information record is writable.
    let succeeded = unsafe {
        CreateProcessW(
            application.as_ptr(),
            command_line.as_mut_ptr(),
            std::ptr::null(),
            std::ptr::null(),
            i32::from(attribute_list.is_some()),
            CREATE_UNICODE_ENVIRONMENT
                | windows_sys::Win32::System::Threading::CREATE_SUSPENDED
                | if attribute_list.is_some() {
                    EXTENDED_STARTUPINFO_PRESENT
                } else {
                    0
                },
            environment.as_ptr().cast(),
            std::ptr::null(),
            &startup.StartupInfo,
            &mut information,
        )
    };
    if succeeded == 0 {
        return Err(std::io::Error::last_os_error());
    }
    if let Some(job) = job {
        // SAFETY: both handles were kept live by the broker and the process
        // is still suspended, so no descendant can escape before assignment.
        if unsafe { AssignProcessToJobObject(job, information.hProcess) } == 0 {
            let error = std::io::Error::last_os_error();
            // SAFETY: both handles are owned here and the suspended process
            // must not be left behind on an assignment failure.
            unsafe {
                TerminateProcess(information.hProcess, 1);
                CloseHandle(information.hThread);
                CloseHandle(information.hProcess);
            }
            return Err(error);
        }
    }
    // SAFETY: CreateProcessW returned the initial thread suspended.
    if unsafe { ResumeThread(information.hThread) } == u32::MAX {
        let error = std::io::Error::last_os_error();
        // SAFETY: both process-information handles remain owned here.
        unsafe {
            TerminateProcess(information.hProcess, 1);
            CloseHandle(information.hThread);
            CloseHandle(information.hProcess);
        }
        return Err(error);
    }
    // SAFETY: CreateProcessW initialized both owned handles. The initial
    // thread need not remain open while the process runs.
    unsafe { CloseHandle(information.hThread) };
    // SAFETY: the process handle remains live through the wait.
    if unsafe { WaitForSingleObject(information.hProcess, INFINITE) } != WAIT_OBJECT_0 {
        let error = std::io::Error::last_os_error();
        // SAFETY: this is the one remaining owned process handle.
        unsafe { CloseHandle(information.hProcess) };
        return Err(error);
    }
    let mut code = 1;
    // SAFETY: the signalled process has a final status and `code` is writable.
    let got_status = unsafe { GetExitCodeProcess(information.hProcess, &mut code) };
    // SAFETY: waiting and status collection are complete.
    unsafe { CloseHandle(information.hProcess) };
    if got_status == 0 {
        return Err(std::io::Error::last_os_error());
    }
    Ok(code)
}

struct ProcessAttributeList {
    _storage: Vec<usize>,
    pointer: LPPROC_THREAD_ATTRIBUTE_LIST,
}

impl ProcessAttributeList {
    fn for_handles(handles: &[HANDLE]) -> std::io::Result<Self> {
        let mut bytes = 0;
        // SAFETY: a null first call asks Windows for the required byte count.
        unsafe {
            InitializeProcThreadAttributeList(std::ptr::null_mut(), 1, 0, &mut bytes);
        }
        if bytes == 0 {
            return Err(std::io::Error::last_os_error());
        }
        let mut storage = vec![0_usize; bytes.div_ceil(std::mem::size_of::<usize>())];
        let pointer = storage.as_mut_ptr().cast();
        // SAFETY: `storage` is aligned and has at least the requested size.
        if unsafe { InitializeProcThreadAttributeList(pointer, 1, 0, &mut bytes) } == 0 {
            return Err(std::io::Error::last_os_error());
        }
        // SAFETY: the list is initialized, and the handle slice remains live
        // until CreateProcessW returns.
        if unsafe {
            UpdateProcThreadAttribute(
                pointer,
                0,
                PROC_THREAD_ATTRIBUTE_HANDLE_LIST as usize,
                handles.as_ptr().cast(),
                std::mem::size_of_val(handles),
                std::ptr::null_mut(),
                std::ptr::null(),
            )
        } == 0
        {
            let error = std::io::Error::last_os_error();
            // SAFETY: initialization succeeded, so the list must be deleted.
            unsafe { DeleteProcThreadAttributeList(pointer) };
            return Err(error);
        }
        Ok(Self {
            _storage: storage,
            pointer,
        })
    }
}

impl Drop for ProcessAttributeList {
    fn drop(&mut self) {
        // SAFETY: `pointer` remains backed by `_storage` and is deleted once.
        unsafe { DeleteProcThreadAttributeList(self.pointer) };
    }
}

fn append_windows_argument(output: &mut Vec<u16>, argument: &OsStr) -> std::io::Result<()> {
    let argument: Vec<u16> = argument.encode_wide().collect();
    if argument.contains(&0) {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    let quoted = argument.is_empty()
        || argument
            .iter()
            .any(|value| matches!(*value, 0x09 | 0x20 | 0x22));
    if !quoted {
        output.extend_from_slice(&argument);
        return Ok(());
    }
    output.push(u16::from(b'"'));
    let mut backslashes = 0_usize;
    for value in argument {
        if value == u16::from(b'\\') {
            backslashes += 1;
            continue;
        }
        if value == u16::from(b'"') {
            output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2 + 1));
            output.push(value);
        } else {
            output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes));
            output.push(value);
        }
        backslashes = 0;
    }
    output.extend(std::iter::repeat_n(u16::from(b'\\'), backslashes * 2));
    output.push(u16::from(b'"'));
    Ok(())
}

fn windows_environment_block(environment: &[(OsString, OsString)]) -> std::io::Result<Vec<u16>> {
    let mut entries: Vec<Vec<u16>> = environment
        .iter()
        .map(|(name, value)| {
            let mut entry: Vec<u16> = name.encode_wide().collect();
            entry.push(u16::from(b'='));
            entry.extend(value.encode_wide());
            if entry.contains(&0) {
                Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))
            } else {
                Ok(entry)
            }
        })
        .collect::<std::io::Result<_>>()?;
    entries.sort_by_key(|entry| {
        entry
            .iter()
            .map(|value| char::from_u32(u32::from(*value)).unwrap_or('\u{fffd}'))
            .flat_map(char::to_lowercase)
            .collect::<String>()
    });
    let mut block = Vec::new();
    for entry in entries {
        block.extend(entry);
        block.push(0);
    }
    block.push(0);
    if block.len() == 1 {
        block.push(0);
    }
    Ok(block)
}
