#![allow(dead_code)]

use bstr::BStr;
use nsh::{Shell, Streams};
use std::ffi::OsStr;
use std::io::Write as _;
use std::os::unix::ffi::OsStrExt as _;
use std::process::{Command, Stdio};

pub struct Outcome {
    pub stdout: Vec<u8>,
    pub status: i32,
    pub refused: bool,
}

pub fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325u64, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0000_0100_0000_01b3)
    })
}

pub fn under_nsh(script: &[u8], env: Vec<(Vec<u8>, Vec<u8>)>) -> Option<Outcome> {
    let streams = Streams::capture().ok()?;
    let mut shell = Shell::builder()
        .streams(streams)
        .option(BStr::new(b"bash"), true)
        .env(env)
        .build()
        .ok()?;
    let outcome = shell.run(script);
    let refused = outcome.is_err();
    let status = outcome.map_or(0, |status| status.code().into());
    let stdout = shell.take_captured_stdout().ok()?.to_vec();
    drop(shell.take_captured_stderr());
    Some(Outcome {
        stdout,
        status,
        refused,
    })
}

pub fn under_bash(script: &[u8], env: &[(Vec<u8>, Vec<u8>)]) -> Option<(Vec<u8>, i32)> {
    let mut child = Command::new("bash")
        .arg("-s")
        .env_clear()
        .env("PATH", "/usr/bin:/bin")
        .envs(
            env.iter()
                .map(|(name, value)| (OsStr::from_bytes(name), OsStr::from_bytes(value))),
        )
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .ok()?;

    let write_result = child.stdin.take()?.write_all(script);
    if write_result.is_err() {
        drop(child.kill());
        drop(child.wait());
        return None;
    }

    let out = child.wait_with_output().ok()?;
    Some((out.stdout, out.status.code().unwrap_or(-1)))
}

pub fn assert_matches_bash(label: &str, data: &[u8], script: &[u8], env: Vec<(Vec<u8>, Vec<u8>)>) {
    let (Some(ours), Some((theirs, their_status))) =
        (under_nsh(script, env.clone()), under_bash(script, &env))
    else {
        return;
    };

    if ours.refused && their_status != 0 {
        return;
    }
    assert!(
        !ours.refused,
        "{label}: nsh refused Bash script input={:016x} bash_status={their_status} bash_stdout={:016x}",
        fingerprint(data),
        fingerprint(&theirs),
    );
    assert!(
        ours.stdout == theirs && ours.status == their_status,
        "{label}: nsh/Bash disagreement input={:016x} nsh_status={} bash_status={their_status} nsh_stdout={:016x} bash_stdout={:016x}",
        fingerprint(data),
        ours.status,
        fingerprint(&ours.stdout),
        fingerprint(&theirs),
    );
}
