//! The conversation a cloned process has with the one that cloned it.
//!
//! `RtlCloneUserProcess` gives the shell the `fork` it cannot otherwise
//! have, but the copy it produces is not a process Win32 will let create
//! another: its loader and console registration were never initialized
//! in it. So a clone does not create processes -- it asks. The process
//! that made the clone runs a broker thread, and the two speak over a
//! pair of anonymous pipes: a length-framed request one way, a
//! fixed-size reply the other.
//!
//! Four requests, and the last is what makes the arrangement survive a
//! clone of a clone: spawn a program, make a pipe, make a channel for a
//! new clone, and register that clone's channel with the broker that is
//! already running. Handles cannot simply be sent as numbers, so every
//! reply that carries one duplicates it into the asking process first,
//! and closes it there if the reply never arrives.

use std::cell::RefCell;
use std::ffi::{OsStr, OsString};
use std::os::windows::ffi::{OsStrExt as _, OsStringExt as _};
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};

use windows_sys::Win32::Foundation::{
    DUPLICATE_CLOSE_SOURCE, DUPLICATE_SAME_ACCESS, DuplicateHandle, HANDLE,
};
use windows_sys::Win32::Storage::FileSystem::{ReadFile, WriteFile};
use windows_sys::Win32::System::Threading::GetCurrentProcess;

use super::*;

pub(super) struct CloneBrokerChild {
    pub(super) request: Descriptor,
    pub(super) response: Descriptor,
}

const BROKER_SPAWN: u32 = 1;

pub(super) const BROKER_PIPE: u32 = 2;

pub(super) const BROKER_CHANNEL: u32 = 3;

const BROKER_REGISTER: u32 = 4;

thread_local! {
    pub(super) static CLONE_BROKER: RefCell<Option<CloneBrokerChild>> = const { RefCell::new(None) };
}

pub(super) fn execute_through_clone_broker(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
) -> Option<std::io::Result<u32>> {
    let request = match encode_spawn_request(path, argv, environment) {
        Ok(request) => request,
        Err(error) => return Some(Err(error)),
    };
    let response = match clone_broker_exchange(&request, 8)? {
        Ok(response) => response,
        Err(error) => return Some(Err(error)),
    };
    let kind = u32::from_le_bytes(response[..4].try_into().unwrap());
    let value = u32::from_le_bytes(response[4..].try_into().unwrap());
    Some(if kind == 0 {
        Ok(value)
    } else {
        Err(std::io::Error::from_raw_os_error(value as i32))
    })
}

fn clone_broker_exchange(
    payload: &[u8],
    response_length: usize,
) -> Option<std::io::Result<Vec<u8>>> {
    CLONE_BROKER.with(|broker| {
        let broker = broker.borrow();
        let broker = broker.as_ref()?;
        Some(broker_exchange_on(
            &broker.request,
            &broker.response,
            payload,
            response_length,
        ))
    })
}

fn broker_exchange_on(
    request: &Descriptor,
    response: &Descriptor,
    payload: &[u8],
    response_length: usize,
) -> std::io::Result<Vec<u8>> {
    let length = u32::try_from(payload.len()).map_err(|_| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "broker request is too large",
        )
    })?;
    let mut framed = Vec::with_capacity(payload.len() + 4);
    framed.extend_from_slice(&length.to_le_bytes());
    framed.extend_from_slice(payload);
    write_all(request, &framed)?;
    read_exact(response, response_length)
}

pub(super) fn broker_handle_pair(
    operation: u32,
) -> Option<std::io::Result<(Descriptor, Descriptor)>> {
    let response = match clone_broker_exchange(&operation.to_le_bytes(), 24)? {
        Ok(response) => response,
        Err(error) => return Some(Err(error)),
    };
    let kind = u32::from_le_bytes(response[..4].try_into().unwrap());
    let value = u32::from_le_bytes(response[4..8].try_into().unwrap());
    if kind != 0 {
        return Some(Err(std::io::Error::from_raw_os_error(value as i32)));
    }
    let first = u64::from_le_bytes(response[8..16].try_into().unwrap());
    let second = u64::from_le_bytes(response[16..24].try_into().unwrap());
    let result = (|| {
        let first = usize::try_from(first)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?
            as HANDLE;
        let second = usize::try_from(second)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?
            as HANDLE;
        let first = owned_handle(first, 0)?;
        match owned_handle(second, 0) {
            Ok(second) => Ok((first, second)),
            Err(error) => Err(error),
        }
    })();
    Some(result)
}

pub(super) fn register_clone_broker(
    request: &Descriptor,
    response: &Descriptor,
    process: HANDLE,
    job: Option<HANDLE>,
) -> std::io::Result<()> {
    let mut payload = Vec::with_capacity(20);
    payload.extend_from_slice(&BROKER_REGISTER.to_le_bytes());
    payload.extend_from_slice(&(process as usize as u64).to_le_bytes());
    payload.extend_from_slice(&(job.unwrap_or(std::ptr::null_mut()) as usize as u64).to_le_bytes());
    let response = broker_exchange_on(request, response, &payload, 8)?;
    let kind = u32::from_le_bytes(response[..4].try_into().unwrap());
    let value = u32::from_le_bytes(response[4..].try_into().unwrap());
    if kind == 0 {
        Ok(())
    } else {
        Err(std::io::Error::from_raw_os_error(value as i32))
    }
}

fn encode_spawn_request(
    path: &OsStr,
    argv: &[OsString],
    environment: &[(OsString, OsString)],
) -> std::io::Result<Vec<u8>> {
    let mut payload = Vec::new();
    payload.extend_from_slice(&BROKER_SPAWN.to_le_bytes());
    encode_wide_value(&mut payload, path)?;
    encode_count(&mut payload, argv.len())?;
    for argument in argv {
        encode_wide_value(&mut payload, argument)?;
    }
    encode_count(&mut payload, environment.len())?;
    for (name, value) in environment {
        encode_wide_value(&mut payload, name)?;
        encode_wide_value(&mut payload, value)?;
    }
    for handle in materialized_standard_handles() {
        payload.extend_from_slice(&(handle as usize as u64).to_le_bytes());
    }
    Ok(payload)
}

fn encode_count(output: &mut Vec<u8>, count: usize) -> std::io::Result<()> {
    let count =
        u32::try_from(count).map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidInput))?;
    output.extend_from_slice(&count.to_le_bytes());
    Ok(())
}

fn encode_wide_value(output: &mut Vec<u8>, value: &OsStr) -> std::io::Result<()> {
    let wide: Vec<u16> = value.encode_wide().collect();
    if wide.contains(&0) {
        return Err(std::io::Error::from(std::io::ErrorKind::InvalidInput));
    }
    encode_count(output, wide.len())?;
    for unit in wide {
        output.extend_from_slice(&unit.to_le_bytes());
    }
    Ok(())
}

struct SpawnRequest {
    path: OsString,
    argv: Vec<OsString>,
    environment: Vec<(OsString, OsString)>,
    handles: [u64; 3],
}

struct RequestCursor<'a> {
    remaining: &'a [u8],
}

impl<'a> RequestCursor<'a> {
    fn u32(&mut self) -> std::io::Result<u32> {
        let value = self.take(4)?;
        Ok(u32::from_le_bytes(value.try_into().unwrap()))
    }

    fn u64(&mut self) -> std::io::Result<u64> {
        let value = self.take(8)?;
        Ok(u64::from_le_bytes(value.try_into().unwrap()))
    }

    fn native(&mut self) -> std::io::Result<OsString> {
        let length = usize::try_from(self.u32()?)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
        let bytes = self.take(
            length
                .checked_mul(2)
                .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidData))?,
        )?;
        let wide: Vec<u16> = bytes
            .chunks_exact(2)
            .map(|unit| u16::from_le_bytes([unit[0], unit[1]]))
            .collect();
        Ok(OsString::from_wide(&wide))
    }

    fn take(&mut self, length: usize) -> std::io::Result<&'a [u8]> {
        let (value, remaining) = self.remaining.split_at_checked(length).ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::InvalidData, "truncated spawn request")
        })?;
        self.remaining = remaining;
        Ok(value)
    }
}

fn decode_spawn_request(payload: &[u8]) -> std::io::Result<SpawnRequest> {
    let mut cursor = RequestCursor { remaining: payload };
    let path = cursor.native()?;
    let argument_count = usize::try_from(cursor.u32()?)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
    let mut argv = Vec::with_capacity(argument_count.min(4096));
    for _ in 0..argument_count {
        argv.push(cursor.native()?);
    }
    let environment_count = usize::try_from(cursor.u32()?)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
    let mut environment = Vec::with_capacity(environment_count.min(16_384));
    for _ in 0..environment_count {
        environment.push((cursor.native()?, cursor.native()?));
    }
    let handles = [cursor.u64()?, cursor.u64()?, cursor.u64()?];
    if !cursor.remaining.is_empty() {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "spawn request has trailing data",
        ));
    }
    Ok(SpawnRequest {
        path,
        argv,
        environment,
        handles,
    })
}

fn duplicate_handle_from_process(
    process: &OwnedHandle,
    raw: u64,
    inherit: bool,
) -> std::io::Result<Option<OwnedHandle>> {
    if raw == 0 || raw == usize::MAX as u64 {
        return Ok(None);
    }
    let raw = usize::try_from(raw)
        .map_err(|_| std::io::Error::from(std::io::ErrorKind::InvalidData))?
        as HANDLE;
    let mut duplicate = std::ptr::null_mut();
    // SAFETY: the broker owns a query/duplication-capable child process
    // handle, the raw handle was supplied by that child, and the output slot
    // receives one parent-owned inheritable duplicate on success.
    if unsafe {
        DuplicateHandle(
            process.as_raw_handle() as HANDLE,
            raw,
            GetCurrentProcess(),
            &mut duplicate,
            0,
            i32::from(inherit),
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        // SAFETY: DuplicateHandle returned one newly owned handle.
        Ok(Some(unsafe {
            OwnedHandle::from_raw_handle(duplicate.cast())
        }))
    }
}

fn duplicate_handle_into_process(
    process: &OwnedHandle,
    source: &impl AsDescriptor,
) -> std::io::Result<u64> {
    let mut remote = std::ptr::null_mut();
    // SAFETY: both process handles are valid, `source` is live, and the
    // returned handle is created in the target process's table.
    if unsafe {
        DuplicateHandle(
            GetCurrentProcess(),
            raw_handle(source),
            process.as_raw_handle() as HANDLE,
            &mut remote,
            0,
            1,
            DUPLICATE_SAME_ACCESS,
        )
    } == 0
    {
        Err(std::io::Error::last_os_error())
    } else {
        Ok(remote as usize as u64)
    }
}

fn close_handle_in_process(process: &OwnedHandle, raw: u64) {
    let Ok(raw) = usize::try_from(raw) else {
        return;
    };
    // SAFETY: DUPLICATE_CLOSE_SOURCE closes the target-process handle; no
    // local duplicate is requested.
    unsafe {
        DuplicateHandle(
            process.as_raw_handle() as HANDLE,
            raw as HANDLE,
            GetCurrentProcess(),
            std::ptr::null_mut(),
            0,
            0,
            DUPLICATE_CLOSE_SOURCE,
        );
    }
}

fn pipe_handles_for_process(process: &OwnedHandle) -> std::io::Result<[u64; 2]> {
    let (read, write) = direct_pipe()?;
    let remote_read = duplicate_handle_into_process(process, &read)?;
    match duplicate_handle_into_process(process, &write) {
        Ok(remote_write) => Ok([remote_read, remote_write]),
        Err(error) => {
            close_handle_in_process(process, remote_read);
            Err(error)
        }
    }
}

fn broker_result_message(result: std::io::Result<u32>) -> Vec<u8> {
    let (kind, value) = match result {
        Ok(value) => (0_u32, value),
        Err(error) => (1_u32, error.raw_os_error().unwrap_or(1) as u32),
    };
    let mut message = Vec::with_capacity(8);
    message.extend_from_slice(&kind.to_le_bytes());
    message.extend_from_slice(&value.to_le_bytes());
    message
}

fn broker_handles_message(result: std::io::Result<[u64; 2]>) -> Vec<u8> {
    let mut message = Vec::with_capacity(24);
    match result {
        Ok(handles) => {
            message.extend_from_slice(&0_u32.to_le_bytes());
            message.extend_from_slice(&0_u32.to_le_bytes());
            message.extend_from_slice(&handles[0].to_le_bytes());
            message.extend_from_slice(&handles[1].to_le_bytes());
        }
        Err(error) => {
            message.extend_from_slice(&1_u32.to_le_bytes());
            message.extend_from_slice(&(error.raw_os_error().unwrap_or(1) as u32).to_le_bytes());
            message.extend_from_slice(&0_u64.to_le_bytes());
            message.extend_from_slice(&0_u64.to_le_bytes());
        }
    }
    message
}

pub(super) fn clone_broker_main(
    request: Descriptor,
    response: Descriptor,
    mut process: OwnedHandle,
    mut job: Option<OwnedHandle>,
) {
    loop {
        let length = match broker_read_exact(&request, 4) {
            Ok(length) => u32::from_le_bytes(length.try_into().unwrap()) as usize,
            Err(_) => return,
        };
        if !(4..=64 * 1024 * 1024).contains(&length) {
            return;
        }
        let payload = match broker_read_exact(&request, length) {
            Ok(payload) => payload,
            Err(_) => return,
        };
        let operation = u32::from_le_bytes(payload[..4].try_into().unwrap());
        let body = &payload[4..];
        let message = match operation {
            BROKER_SPAWN => {
                let result = decode_spawn_request(body).and_then(|request| {
                    let input = duplicate_handle_from_process(&process, request.handles[0], true)?;
                    let output = duplicate_handle_from_process(&process, request.handles[1], true)?;
                    let error = duplicate_handle_from_process(&process, request.handles[2], true)?;
                    let handles = [
                        input.as_ref().map_or(std::ptr::null_mut(), |handle| {
                            handle.as_raw_handle() as HANDLE
                        }),
                        output.as_ref().map_or(std::ptr::null_mut(), |handle| {
                            handle.as_raw_handle() as HANDLE
                        }),
                        error.as_ref().map_or(std::ptr::null_mut(), |handle| {
                            handle.as_raw_handle() as HANDLE
                        }),
                    ];
                    spawn_program_here(
                        &request.path,
                        &request.argv,
                        &request.environment,
                        handles,
                        job.as_ref().map(|job| job.as_raw_handle() as HANDLE),
                    )
                });
                broker_result_message(result)
            }
            BROKER_PIPE => broker_handles_message(pipe_handles_for_process(&process)),
            BROKER_CHANNEL => {
                let result = (|| {
                    let (nested_request, client_request) = direct_pipe()?;
                    let (client_response, nested_response) = direct_pipe()?;
                    set_descriptor_inherit(&nested_request, false)?;
                    set_descriptor_inherit(&nested_response, false)?;
                    let remote_request = duplicate_handle_into_process(&process, &client_request)?;
                    let remote_response =
                        match duplicate_handle_into_process(&process, &client_response) {
                            Ok(handle) => handle,
                            Err(error) => {
                                close_handle_in_process(&process, remote_request);
                                return Err(error);
                            }
                        };
                    let nested_process =
                        duplicate_owned_handle(process.as_raw_handle() as HANDLE, false)?;
                    let nested_job = job
                        .as_ref()
                        .map(|job| duplicate_owned_handle(job.as_raw_handle() as HANDLE, false))
                        .transpose()?;
                    if let Err(error) = std::thread::Builder::new()
                        .name("nsh-spawn-broker".into())
                        .spawn(move || {
                            clone_broker_main(
                                nested_request,
                                nested_response,
                                nested_process,
                                nested_job,
                            )
                        })
                    {
                        close_handle_in_process(&process, remote_request);
                        close_handle_in_process(&process, remote_response);
                        return Err(error);
                    }
                    Ok([remote_request, remote_response])
                })();
                broker_handles_message(result)
            }
            BROKER_REGISTER => {
                let result = (|| {
                    let mut cursor = RequestCursor { remaining: body };
                    let child_process =
                        duplicate_handle_from_process(&process, cursor.u64()?, false)?
                            .ok_or_else(|| std::io::Error::from(std::io::ErrorKind::InvalidData))?;
                    let child_job = duplicate_handle_from_process(&process, cursor.u64()?, false)?;
                    if !cursor.remaining.is_empty() {
                        return Err(std::io::Error::from(std::io::ErrorKind::InvalidData));
                    }
                    process = child_process;
                    if child_job.is_some() {
                        job = child_job;
                    }
                    Ok(0)
                })();
                broker_result_message(result)
            }
            _ => broker_result_message(Err(std::io::Error::from(std::io::ErrorKind::InvalidInput))),
        };
        if broker_write_all(&response, &message).is_err() {
            return;
        }
    }
}

// Broker I/O deliberately avoids making temporary inheritable duplicates.
// Another shell child may be cloned while this thread is blocked on a pipe.
fn broker_read_exact(fd: &impl AsDescriptor, length: usize) -> std::io::Result<Vec<u8>> {
    let mut output = vec![0; length];
    let mut offset = 0;
    while offset < output.len() {
        let chunk = (output.len() - offset).min(u32::MAX as usize) as u32;
        let mut read = 0;
        // SAFETY: the descriptor remains borrowed for the call and the
        // remaining output slice is writable for `chunk` bytes.
        if unsafe {
            ReadFile(
                raw_handle(fd),
                output[offset..].as_mut_ptr(),
                chunk,
                &mut read,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if read == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::UnexpectedEof));
        }
        offset += read as usize;
    }
    Ok(output)
}

fn broker_write_all(fd: &impl AsDescriptor, bytes: &[u8]) -> std::io::Result<()> {
    let mut offset = 0;
    while offset < bytes.len() {
        let chunk = (bytes.len() - offset).min(u32::MAX as usize) as u32;
        let mut written = 0;
        // SAFETY: the descriptor remains borrowed for the call and the input
        // slice is readable for `chunk` bytes.
        if unsafe {
            WriteFile(
                raw_handle(fd),
                bytes[offset..].as_ptr(),
                chunk,
                &mut written,
                std::ptr::null_mut(),
            )
        } == 0
        {
            return Err(std::io::Error::last_os_error());
        }
        if written == 0 {
            return Err(std::io::Error::from(std::io::ErrorKind::WriteZero));
        }
        offset += written as usize;
    }
    Ok(())
}
